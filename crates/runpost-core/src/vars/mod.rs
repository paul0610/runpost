//! Resolución de `{{variables}}` (sección 5.7) y enmascarado de secretos.
//!
//! Orden: override de petición > datos de iteración > entorno activo > colección > globales > dinámicas.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::model::Variable;

/// Pares clave → valor (overrides de MCP/runner, datos de iteración).
pub type Vars = BTreeMap<String, String>;

pub const MASK: &str = "***";

/// Profundidad máxima de variables anidadas (`{{a}}` cuyo valor contiene `{{b}}`).
const MAX_DEPTH: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Override,
    Iteration,
    Environment,
    Collection,
    Globals,
    Dynamic,
}

const LAYERED: [Scope; 5] = [
    Scope::Override,
    Scope::Iteration,
    Scope::Environment,
    Scope::Collection,
    Scope::Globals,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    pub value: String,
    pub secret: bool,
    pub scope: Scope,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub text: String,
    /// Nombres sin valor; se dejan como `{{nombre}}` en `text` (igual que Postman).
    pub unresolved: Vec<String>,
    /// Algún valor usado (directa o indirectamente) era secreto.
    pub secret: bool,
}

#[derive(Clone, Default)]
pub struct Resolver {
    layers: HashMap<Scope, HashMap<String, (String, bool)>>,
    masker: Masker,
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: BTreeMap<_, Vec<_>> = self
            .layers
            .iter()
            .map(|(scope, vars)| (scope, vars.keys().collect()))
            .collect();
        f.debug_struct("Resolver").field("keys", &keys).finish()
    }
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Valores planos (overrides, iteración). Nunca son secretos: el que los envía ya los conoce.
    pub fn with_pairs(mut self, scope: Scope, vars: &Vars) -> Self {
        let layer = self.layers.entry(scope).or_default();
        for (key, value) in vars {
            layer.insert(key.clone(), (value.clone(), false));
        }
        self
    }

    /// Como `with_pairs`, pero las claves que `secret` marca cuentan como secretas: p. ej. un
    /// `accessToken` que llega del archivo de datos del runner y es secreto en el entorno.
    pub fn with_pairs_where(
        mut self,
        scope: Scope,
        vars: &Vars,
        secret: impl Fn(&str) -> bool,
    ) -> Self {
        let layer = self.layers.entry(scope).or_default();
        for (key, value) in vars {
            let is_secret = secret(key);
            if is_secret {
                self.masker.add(value);
            }
            layer.insert(key.clone(), (value.clone(), is_secret));
        }
        self
    }

    /// Variables del modelo (entorno, colección, globales). Las deshabilitadas no cuentan.
    pub fn with_variables(mut self, scope: Scope, vars: &[Variable]) -> Self {
        let layer = self.layers.entry(scope).or_default();
        for var in vars.iter().filter(|var| var.enabled) {
            // Los secretos del llavero llegan con valor (`Workspace` los lee al cargar); si el
            // llavero no lo tiene, cuentan como variable sin valor y quedan como `{{nombre}}`.
            let Some(value) = &var.value else { continue };
            if var.secret {
                self.masker.add(value);
            }
            layer.insert(var.key.clone(), (value.clone(), var.secret));
        }
        self
    }

    /// Un secreto que no viene de una variable (p. ej. un token OAuth 2.0): se enmascara igual.
    pub fn with_secret(mut self, secret: &str) -> Self {
        self.masker.add(secret);
        self
    }

    pub fn masker(&self) -> &Masker {
        &self.masker
    }

    /// Valor de una variable siguiendo el orden 5.7; las dinámicas se generan en cada llamada.
    pub fn lookup(&self, name: &str) -> Option<Lookup> {
        for scope in LAYERED {
            if let Some((value, secret)) = self.layers.get(&scope).and_then(|layer| layer.get(name))
            {
                return Some(Lookup {
                    value: value.clone(),
                    secret: *secret,
                    scope,
                });
            }
        }
        dynamic(name).map(|value| Lookup {
            value,
            secret: false,
            scope: Scope::Dynamic,
        })
    }

    pub fn resolve(&self, input: &str) -> Resolved {
        let mut out = Resolved::default();
        out.text = self.expand(input, &mut Vec::new(), &mut out.unresolved, &mut out.secret);
        out
    }

    /// Para texto que sale de la app (snippets): sustituye solo las variables que no son secretas
    /// ni dinámicas; las demás, y las desconocidas, quedan como `{{nombre}}`.
    pub fn resolve_safe(&self, input: &str) -> String {
        self.expand_safe(input, &mut Vec::new())
    }

    fn expand_safe(&self, input: &str, stack: &mut Vec<String>) -> String {
        let mut text = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find("{{") {
            text.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                rest = &rest[start..];
                break;
            };
            let raw = &after[..end];
            let name = raw.trim();
            rest = &after[end + 2..];
            let cycle = stack.iter().any(|seen| seen == name) || stack.len() >= MAX_DEPTH;
            match self
                .lookup(name)
                .filter(|found| !cycle && !found.secret && found.scope != Scope::Dynamic)
            {
                Some(found) if !name.is_empty() && !name.contains('{') => {
                    stack.push(name.to_owned());
                    text.push_str(&self.expand_safe(&found.value, stack));
                    stack.pop();
                }
                _ => text.push_str(&format!("{{{{{raw}}}}}")),
            }
        }
        text.push_str(rest);
        text
    }

    fn expand(
        &self,
        input: &str,
        stack: &mut Vec<String>,
        unresolved: &mut Vec<String>,
        secret: &mut bool,
    ) -> String {
        let mut text = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find("{{") {
            text.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                rest = &rest[start..];
                break;
            };
            let raw = &after[..end];
            let name = raw.trim();
            rest = &after[end + 2..];
            let literal = || format!("{{{{{raw}}}}}");
            if name.is_empty() || name.contains('{') {
                text.push_str(&literal());
                continue;
            }
            let cycle = stack.iter().any(|seen| seen == name) || stack.len() >= MAX_DEPTH;
            match self.lookup(name).filter(|_| !cycle) {
                Some(found) => {
                    *secret |= found.secret;
                    stack.push(name.to_owned());
                    text.push_str(&self.expand(&found.value, stack, unresolved, secret));
                    stack.pop();
                }
                None => {
                    if !unresolved.iter().any(|seen| seen == name) {
                        unresolved.push(name.to_owned());
                    }
                    text.push_str(&literal());
                }
            }
        }
        text.push_str(rest);
        text
    }
}

/// Variables dinámicas soportadas: `$guid`, `$timestamp`, `$isoTimestamp`, `$randomInt`.
pub fn dynamic(name: &str) -> Option<String> {
    match name {
        "$guid" => Some(uuid::Uuid::new_v4().to_string()),
        "$timestamp" => Some(chrono::Utc::now().timestamp().to_string()),
        "$isoTimestamp" => {
            Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        }
        // Igual que Postman: entero entre 0 y 1000.
        "$randomInt" => Some((getrandom::u32().unwrap_or_default() % 1001).to_string()),
        _ => None,
    }
}

/// Sustituye cualquier valor secreto (y sus formas codificadas) por `***`.
#[derive(Clone, Default)]
pub struct Masker {
    /// Ordenados de mayor a menor longitud para que un secreto contenido en otro no deje restos.
    secrets: Vec<String>,
}

impl std::fmt::Debug for Masker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Masker({} values)", self.secrets.len())
    }
}

impl Masker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra un secreto y sus variantes: codificado en query/form y escapado en JSON.
    pub fn add(&mut self, secret: &str) {
        if secret.is_empty() {
            return;
        }
        let form: String = url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
        let json = serde_json::to_string(secret).unwrap_or_default();
        let json = json.trim_matches('"').to_owned();
        for variant in [secret.to_owned(), form.replace('+', "%20"), form, json] {
            if !variant.is_empty() && !self.secrets.contains(&variant) {
                self.secrets.push(variant);
            }
        }
        self.secrets
            .sort_by_key(|value| std::cmp::Reverse(value.len()));
    }

    /// Añade los secretos de otro `Masker` (p. ej. los valores que dejó un script).
    pub fn merge(&mut self, other: &Masker) {
        for secret in &other.secrets {
            if !self.secrets.contains(secret) {
                self.secrets.push(secret.clone());
            }
        }
        self.secrets
            .sort_by_key(|value| std::cmp::Reverse(value.len()));
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    pub fn mask(&self, input: &str) -> String {
        let mut text = input.to_owned();
        for secret in &self.secrets {
            if text.contains(secret.as_str()) {
                text = text.replace(secret.as_str(), MASK);
            }
        }
        text
    }

    /// Enmascara bytes si son UTF-8; los binarios se devuelven tal cual.
    pub fn mask_bytes(&self, input: &[u8]) -> Vec<u8> {
        match std::str::from_utf8(input) {
            Ok(text) if !self.is_empty() => self.mask(text).into_bytes(),
            _ => input.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn model(pairs: &[(&str, &str)]) -> Vec<Variable> {
        pairs.iter().map(|(k, v)| Variable::new(*k, *v)).collect()
    }

    /// Un resolver con `x` definida en todas las capas, cada una con su nombre.
    fn layered() -> Resolver {
        Resolver::new()
            .with_variables(Scope::Globals, &model(&[("x", "globals"), ("g", "G")]))
            .with_variables(
                Scope::Collection,
                &model(&[("x", "collection"), ("c", "C")]),
            )
            .with_variables(
                Scope::Environment,
                &model(&[("x", "environment"), ("e", "E")]),
            )
            .with_pairs(Scope::Iteration, &vars(&[("x", "iteration"), ("i", "I")]))
            .with_pairs(Scope::Override, &vars(&[("x", "override")]))
    }

    #[test]
    fn order_override_beats_everything() {
        let found = layered().lookup("x").unwrap();
        assert_eq!(
            (found.value.as_str(), found.scope),
            ("override", Scope::Override)
        );
    }

    #[test]
    fn order_each_layer_beats_the_next() {
        let expected = [
            (Scope::Override, "iteration", Scope::Iteration),
            (Scope::Iteration, "environment", Scope::Environment),
            (Scope::Environment, "collection", Scope::Collection),
            (Scope::Collection, "globals", Scope::Globals),
        ];
        let mut resolver = layered();
        for (removed, value, scope) in expected {
            resolver.layers.remove(&removed);
            let found = resolver.lookup("x").unwrap();
            assert_eq!((found.value.as_str(), found.scope), (value, scope));
        }
        resolver.layers.remove(&Scope::Globals);
        assert_eq!(resolver.lookup("x"), None);
    }

    #[test]
    fn layers_are_merged_not_replaced() {
        let resolved = layered().resolve("{{i}}-{{e}}-{{c}}-{{g}}");
        assert_eq!(resolved.text, "I-E-C-G");
        assert!(resolved.unresolved.is_empty());
    }

    #[test]
    fn insertion_order_does_not_change_priority() {
        let resolver = Resolver::new()
            .with_pairs(Scope::Override, &vars(&[("x", "override")]))
            .with_variables(Scope::Globals, &model(&[("x", "globals")]));
        assert_eq!(resolver.resolve("{{x}}").text, "override");
    }

    #[test]
    fn user_variable_shadows_dynamic_one() {
        let resolver =
            Resolver::new().with_variables(Scope::Globals, &model(&[("$guid", "fixed")]));
        assert_eq!(resolver.resolve("{{$guid}}").text, "fixed");
    }

    #[test]
    fn dynamic_variables() {
        let resolver = Resolver::new();
        let guid = resolver.resolve("{{$guid}}").text;
        assert!(uuid::Uuid::parse_str(&guid).is_ok());
        assert_ne!(guid, resolver.resolve("{{$guid}}").text);
        let ts: i64 = resolver.resolve("{{$timestamp}}").text.parse().unwrap();
        assert!((ts - chrono::Utc::now().timestamp()).abs() < 5);
        let iso = resolver.resolve("{{$isoTimestamp}}").text;
        assert!(chrono::DateTime::parse_from_rfc3339(&iso).is_ok() && iso.ends_with('Z'));
        for _ in 0..50 {
            let n: u32 = resolver.resolve("{{$randomInt}}").text.parse().unwrap();
            assert!(n <= 1000);
        }
        let found = resolver.lookup("$timestamp").unwrap();
        assert_eq!(found.scope, Scope::Dynamic);
    }

    #[test]
    fn unknown_dynamic_and_missing_are_left_literal() {
        let resolved = Resolver::new().resolve("{{$nope}}/{{missing}}/{{missing}}");
        assert_eq!(resolved.text, "{{$nope}}/{{missing}}/{{missing}}");
        assert_eq!(resolved.unresolved, ["$nope", "missing"]);
    }

    #[test]
    fn disabled_variables_are_ignored() {
        let mut off = Variable::new("x", "disabled");
        off.enabled = false;
        let resolver = Resolver::new()
            .with_variables(Scope::Environment, &[off])
            .with_variables(Scope::Globals, &model(&[("x", "globals")]));
        assert_eq!(resolver.resolve("{{x}}").text, "globals");
    }

    #[test]
    fn variable_without_value_is_unresolved() {
        let mut keyring = Variable::secret("token", "");
        keyring.value = None;
        let resolved = Resolver::new()
            .with_variables(Scope::Environment, &[keyring])
            .resolve("{{token}}");
        assert_eq!(resolved.unresolved, ["token"]);
    }

    #[test]
    fn whitespace_inside_braces_is_trimmed() {
        let resolver = Resolver::new().with_pairs(Scope::Override, &vars(&[("a", "1")]));
        assert_eq!(resolver.resolve("{{ a }}").text, "1");
    }

    #[test]
    fn malformed_braces_are_kept() {
        let resolver = Resolver::new().with_pairs(Scope::Override, &vars(&[("a", "1")]));
        assert_eq!(resolver.resolve("{{}}").text, "{{}}");
        assert_eq!(resolver.resolve("x{{a").text, "x{{a");
        assert_eq!(resolver.resolve("{{{a}}}").text, "{{{a}}}");
        assert_eq!(resolver.resolve("no vars").text, "no vars");
        assert_eq!(resolver.resolve("{{a}}}").text, "1}");
    }

    #[test]
    fn nested_variables_resolve_recursively() {
        let resolver = Resolver::new().with_variables(
            Scope::Environment,
            &model(&[("url", "{{host}}/v2"), ("host", "http://h")]),
        );
        assert_eq!(resolver.resolve("{{url}}/ping").text, "http://h/v2/ping");
    }

    #[test]
    fn cycles_stop_and_report() {
        let resolver = Resolver::new().with_variables(
            Scope::Environment,
            &model(&[("a", "{{b}}"), ("b", "{{a}}"), ("self", "x{{self}}")]),
        );
        let resolved = resolver.resolve("{{a}}");
        assert_eq!(resolved.text, "{{a}}");
        assert_eq!(resolved.unresolved, ["a"]);
        assert_eq!(resolver.resolve("{{self}}").text, "x{{self}}");
    }

    #[test]
    fn depth_limit_stops_long_chains() {
        let chain: Vec<Variable> = (0..20)
            .map(|i| Variable::new(format!("v{i}"), format!("{{{{v{}}}}}", i + 1)))
            .collect();
        let resolved = Resolver::new()
            .with_variables(Scope::Environment, &chain)
            .resolve("{{v0}}");
        assert_eq!(resolved.text, "{{v10}}");
        assert_eq!(resolved.unresolved, ["v10"]);
    }

    #[test]
    fn secret_flag_propagates_through_nesting() {
        let resolver = Resolver::new().with_variables(
            Scope::Environment,
            &[
                Variable::new("auth", "Bearer {{token}}"),
                Variable::secret("token", "s3cr3t"),
                Variable::new("plain", "p"),
            ],
        );
        let resolved = resolver.resolve("{{auth}}");
        assert_eq!(resolved.text, "Bearer s3cr3t");
        assert!(resolved.secret);
        assert!(!resolver.resolve("{{plain}}").secret);
        assert_eq!(resolver.masker().mask(&resolved.text), "Bearer ***");
    }

    #[test]
    fn overrides_are_never_secret() {
        let resolver = Resolver::new()
            .with_variables(Scope::Environment, &[Variable::secret("token", "real")])
            .with_pairs(Scope::Override, &vars(&[("token", "given")]));
        let resolved = resolver.resolve("{{token}}");
        assert_eq!(resolved.text, "given");
        assert!(!resolved.secret);
        // El secreto del entorno sigue enmascarado aunque esté sombreado.
        assert_eq!(resolver.masker().mask("real"), MASK);
    }

    #[test]
    fn masker_handles_encoded_forms_and_overlaps() {
        let mut masker = Masker::new();
        masker.add("");
        assert!(masker.is_empty());
        masker.add("a b&c\"d");
        masker.add("abc");
        masker.add("abcdef");
        masker.add("abc");
        assert_eq!(masker.mask("x=a%20b%26c%22d"), "x=***");
        assert_eq!(masker.mask("x=a+b%26c%22d"), "x=***");
        assert_eq!(masker.mask(r#"{"t":"a b&c\"d"}"#), r#"{"t":"***"}"#);
        assert_eq!(masker.mask("abcdef abc"), "*** ***");
        assert_eq!(masker.mask("nothing"), "nothing");
        assert_eq!(format!("{masker:?}"), "Masker(6 values)");
    }

    #[test]
    fn mask_bytes_skips_binary_and_empty_masker() {
        let mut masker = Masker::new();
        assert_eq!(Masker::new().mask_bytes(b"abc"), b"abc");
        masker.add("abc");
        assert_eq!(masker.mask_bytes(b"-abc-"), b"-***-");
        assert_eq!(
            masker.mask_bytes(&[0xff, b'a', b'b', b'c']),
            [0xff, b'a', b'b', b'c']
        );
    }

    #[test]
    fn debug_never_prints_values() {
        let resolver = Resolver::new()
            .with_variables(Scope::Environment, &[Variable::secret("token", "s3cr3t")]);
        let debug = format!("{resolver:?}");
        assert!(debug.contains("token") && !debug.contains("s3cr3t"));
    }
}
