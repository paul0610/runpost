//! Insomnia → colecciones (fase 4): exportación v4 (`_type: export`, JSON o YAML) y v5 (YAML:
//! `collection.insomnia.rest/5.0`, `spec.insomnia.rest/5.0` y el entorno global
//! `environment.insomnia.rest/5.0`).
//!
//! Cada espacio de trabajo es una colección: el entorno base pasa a variables de la colección y
//! cada subentorno a un entorno de RunPost. RunPost no tiene cabeceras ni variables por carpeta:
//! las cabeceras de las carpetas se copian a cada petición (Insomnia las envía todas) y las
//! variables de carpeta van al script previo de la carpeta (`pm.variables.set`). `{{ _.x }}` pasa
//! a `{{x}}`, y los secretos del vault (`{{ vault.x }}`) a variables secretas vacías. Los scripts
//! usan la API `insomnia.*`, copia de la de Postman: se traducen cambiando el prefijo, lo mismo que
//! hace Insomnia al importar de Postman.

use std::collections::HashMap;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::model::{
    ApiKeyIn, Auth, Body, ClientAuth, Collection, Environment, FieldKind, Folder, Item, KeyValue,
    MultipartField, OAuth2Cc, OAuth2Code, RawLang, Request, Scripts, Variable,
};

use super::{Imported, array, fill_path_params, split_query, stable_id, text};

#[cfg(test)]
mod tests;

/// Profundidad máxima de carpetas (y freno ante `parentId` circulares).
const MAX_DEPTH: usize = 64;

pub fn detect(value: &Value) -> bool {
    is_v4(value) || v5_type(value).is_some()
}

fn is_v4(value: &Value) -> bool {
    value.get("_type").and_then(Value::as_str) == Some("export")
        && value.get("resources").is_some_and(Value::is_array)
}

/// `collection`, `spec`, `environment`, `mock`, `mcpClient` de `type: <x>.insomnia…/5.x`.
fn v5_type(value: &Value) -> Option<&str> {
    let kind = value.get("type").and_then(Value::as_str)?;
    let (name, version) = kind.split_once('/')?;
    let (prefix, rest) = name.split_once('.')?;
    (version.starts_with('5') && rest.starts_with("insomnia")).then_some(prefix)
}

// ---------- árbol común a v4 y v5 ----------

/// Un espacio de trabajo: colección (`collection`, `design`) o solo entornos (`environment`).
struct Workspace<'a> {
    id: String,
    name: String,
    description: String,
    environments_only: bool,
    base: Option<&'a Value>,
    subs: Vec<&'a Value>,
    items: Vec<Node<'a>>,
}

enum Node<'a> {
    Folder(&'a Value, Vec<Node<'a>>),
    Request(&'a Value),
}

impl Node<'_> {
    fn value(&self) -> &Value {
        match self {
            Node::Folder(value, _) | Node::Request(value) => value,
        }
    }
}

fn meta<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get("meta").and_then(|m| m.get(key))
}

fn kind(value: &Value) -> &str {
    value.get("_type").and_then(Value::as_str).unwrap_or("")
}

fn id(value: &Value) -> String {
    text(value.get("_id").or_else(|| meta(value, "id")))
}

fn description(value: &Value) -> String {
    text(
        value
            .get("description")
            .or_else(|| meta(value, "description")),
    )
}

fn sort_key(value: &Value) -> f64 {
    value
        .get("metaSortKey")
        .or_else(|| meta(value, "sortKey"))
        .and_then(Value::as_f64)
        .unwrap_or_default()
}

/// Script de v4 (`preRequestScript`) o de v5 (`scripts.preRequest`).
fn script(value: &Value, v4: &str, v5: &str) -> String {
    text(
        value
            .get(v4)
            .or_else(|| value.get("scripts").and_then(|s| s.get(v5))),
    )
}

fn follow_redirects(value: &Value) -> Option<bool> {
    let setting = value
        .get("settingFollowRedirects")
        .or_else(|| value.get("settings").and_then(|s| s.get("followRedirects")));
    match setting {
        Some(Value::String(mode)) if mode == "on" => Some(true),
        Some(Value::String(mode)) if mode == "off" => Some(false),
        Some(Value::Bool(follow)) => Some(*follow),
        _ => None,
    }
}

/// Orden de Insomnia: clave de orden ascendente; a igualdad, id descendente.
fn sort_nodes(nodes: &mut [Node<'_>]) {
    nodes.sort_by(|a, b| {
        let (a, b) = (a.value(), b.value());
        sort_key(a)
            .total_cmp(&sort_key(b))
            .then_with(|| id(b).cmp(&id(a)))
    });
}

fn v4_nodes<'a>(
    children: &HashMap<String, Vec<&'a Value>>,
    parent: &str,
    depth: usize,
    skipped: &mut usize,
) -> Vec<Node<'a>> {
    let mut nodes = Vec::new();
    if depth > MAX_DEPTH {
        return nodes;
    }
    for resource in children.get(parent).map(Vec::as_slice).unwrap_or_default() {
        match kind(resource) {
            "request_group" => nodes.push(Node::Folder(
                resource,
                v4_nodes(children, &id(resource), depth + 1, skipped),
            )),
            "request" => nodes.push(Node::Request(resource)),
            "grpc_request" | "websocket_request" | "socketio_request" | "mcp_request" => {
                *skipped += 1;
            }
            _ => {}
        }
    }
    sort_nodes(&mut nodes);
    nodes
}

fn v4_workspaces<'a>(value: &'a Value, skipped: &mut usize) -> Vec<Workspace<'a>> {
    let resources = array(value.get("resources"));
    let mut children: HashMap<String, Vec<&'a Value>> = HashMap::new();
    for resource in resources {
        children
            .entry(text(resource.get("parentId")))
            .or_default()
            .push(resource);
    }
    let environment = |parent: &str| -> Vec<&'a Value> {
        let mut envs: Vec<_> = children
            .get(parent)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|r| kind(r) == "environment")
            .collect();
        envs.sort_by(|a, b| sort_key(a).total_cmp(&sort_key(b)));
        envs
    };
    let mut out = Vec::new();
    for workspace in resources.iter().filter(|r| kind(r) == "workspace") {
        let scope = text(workspace.get("scope"));
        if matches!(scope.as_str(), "mock-server" | "mcp") {
            continue;
        }
        let ws_id = id(workspace);
        let base = environment(&ws_id).into_iter().next();
        let mut subs = base.map(|b| environment(&id(b))).unwrap_or_default();
        // Formato antiguo: subentornos colgados de `__BASE_ENVIRONMENT_ID__`.
        if out.is_empty() {
            subs.extend(environment("__BASE_ENVIRONMENT_ID__"));
        }
        out.push(Workspace {
            name: text(workspace.get("name")),
            description: description(workspace),
            environments_only: scope == "environment",
            base,
            subs,
            items: v4_nodes(&children, &ws_id, 0, skipped),
            id: ws_id,
        });
    }
    // Exportación parcial sin su espacio de trabajo: lo que cuelga de padres que no están.
    let known: std::collections::HashSet<String> = resources.iter().map(id).collect();
    let mut orphans: Vec<Node<'a>> = Vec::new();
    let mut parents: Vec<&String> = children
        .keys()
        .filter(|parent| !parent.is_empty() && !known.contains(*parent))
        .collect();
    parents.sort();
    for parent in parents {
        orphans.extend(v4_nodes(&children, parent, 0, skipped));
    }
    if !orphans.is_empty() {
        sort_nodes(&mut orphans);
        out.push(Workspace {
            id: "orphans".into(),
            name: "Insomnia import".into(),
            description: String::new(),
            environments_only: false,
            base: None,
            subs: Vec::new(),
            items: orphans,
        });
    }
    out
}

fn v5_nodes<'a>(list: &'a [Value], depth: usize, skipped: &mut usize) -> Vec<Node<'a>> {
    let mut nodes = Vec::new();
    if depth > MAX_DEPTH {
        return nodes;
    }
    for item in list {
        if item.get("method").is_some() {
            nodes.push(Node::Request(item));
        } else if item.get("reflectionApi").is_some() || item.get("url").is_some() {
            // gRPC, WebSocket, Socket.IO.
            *skipped += 1;
        } else {
            nodes.push(Node::Folder(
                item,
                v5_nodes(array(item.get("children")), depth + 1, skipped),
            ));
        }
    }
    sort_nodes(&mut nodes);
    nodes
}

fn v5_workspace<'a>(
    value: &'a Value,
    environments_only: bool,
    skipped: &mut usize,
) -> Workspace<'a> {
    let base = value.get("environments").filter(|e| e.is_object());
    Workspace {
        id: id(value),
        name: text(value.get("name")),
        description: description(value),
        environments_only,
        base,
        subs: base
            .map(|b| array(b.get("subEnvironments")).iter().collect())
            .unwrap_or_default(),
        items: v5_nodes(array(value.get("collection")), 0, skipped),
    }
}

// ---------- traducción a RunPost ----------

/// `ident(.ident | [n])*`.
fn is_path(path: &str) -> bool {
    let ident = |part: &str| {
        let mut chars = part.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    };
    let mut rest = path;
    let first_end = rest.find(['.', '[']).unwrap_or(rest.len());
    if !ident(&rest[..first_end]) {
        return false;
    }
    rest = &rest[first_end..];
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            if !ident(&after[..end]) {
                return false;
            }
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix('[') {
            let Some(end) = after.find(']') else {
                return false;
            };
            if end == 0 || !after[..end].chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            rest = &after[end + 1..];
        } else {
            return false;
        }
    }
    true
}

/// `{% uuid 'v4' %}` → `{{$guid}}`; `{% now 'iso-8601' %}` → `{{$isoTimestamp}}`;
/// `{% now 'unix' %}` → `{{$timestamp}}` (segundos, como en Insomnia).
fn tag(inner: &str) -> Option<&'static str> {
    let inner = inner.trim();
    let (name, args) = inner
        .split_once(char::is_whitespace)
        .map_or((inner, ""), |(name, args)| (name, args.trim()));
    let first = args
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['\'', '"']);
    match (name, first) {
        ("uuid", "" | "v4") => Some("{{$guid}}"),
        ("now", "iso-8601") => Some("{{$isoTimestamp}}"),
        ("now", "unix") => Some("{{$timestamp}}"),
        _ => None,
    }
}

/// Aplana el `data` de un entorno: `{"a": {"b": 1}, "c": [2]}` → `a.b = 1`, `c[0] = 2`.
fn flatten(prefix: &str, value: &Value, out: &mut Vec<(String, String)>, depth: usize) {
    match value {
        Value::Object(map) if depth < MAX_DEPTH => {
            for (key, value) in map {
                flatten(&format!("{prefix}.{key}"), value, out, depth + 1);
            }
        }
        Value::Array(items) if depth < MAX_DEPTH => {
            for (index, value) in items.iter().enumerate() {
                flatten(&format!("{prefix}[{index}]"), value, out, depth + 1);
            }
        }
        Value::String(text) => out.push((prefix.to_owned(), text.clone())),
        Value::Null => out.push((prefix.to_owned(), String::new())),
        other => out.push((prefix.to_owned(), other.to_string())),
    }
}

/// Nombres que suelen guardar credenciales: el aviso pide marcarlas como secretas.
fn looks_like_credential(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "token",
        "password",
        "passphrase",
        "secret",
        "apikey",
        "api_key",
        "api-key",
    ]
    .iter()
    .any(|word| name.contains(word))
}

#[derive(Default)]
struct Builder {
    /// Secretos del vault citados (`{{ vault.x }}`).
    vault: Vec<String>,
    path_vars: Vec<String>,
    /// Etiquetas y expresiones sin equivalente, por nombre.
    kept: Vec<String>,
    scripts: usize,
    unsupported: Vec<String>,
    credentials: usize,
}

/// `credentialsInBody` de Insomnia.
fn client_auth(auth: &Value) -> ClientAuth {
    if auth.get("credentialsInBody").and_then(Value::as_bool) == Some(true) {
        ClientAuth::Body
    } else {
        ClientAuth::Basic
    }
}

impl Builder {
    /// Plantillas de Insomnia → `{{nombre}}`; los comentarios `{# … #}` se quitan.
    fn t(&mut self, source: &str) -> String {
        if !source.contains('{') {
            return source.to_owned();
        }
        let mut out = String::with_capacity(source.len());
        let mut rest = source;
        loop {
            let next = ["{{", "{%", "{#"]
                .into_iter()
                .filter_map(|open| rest.find(open).map(|at| (at, open)))
                .min_by_key(|(at, _)| *at);
            let Some((at, open)) = next else {
                break;
            };
            let close = match open {
                "{{" => "}}",
                "{%" => "%}",
                _ => "#}",
            };
            out.push_str(&rest[..at]);
            let after = &rest[at + 2..];
            let Some(end) = after.find(close) else {
                out.push_str(&rest[at..]);
                rest = "";
                break;
            };
            let inner = &after[..end];
            let token = &rest[at..at + end + 4];
            match open {
                "{{" => match self.variable(inner) {
                    Some(name) => {
                        out.push_str("{{");
                        out.push_str(&name);
                        out.push_str("}}");
                    }
                    None => {
                        self.keep("expression");
                        out.push_str(token);
                    }
                },
                "{%" => match tag(inner) {
                    Some(dynamic) => out.push_str(dynamic),
                    None => {
                        self.keep(inner.split_whitespace().next().unwrap_or("tag"));
                        out.push_str(token);
                    }
                },
                _ => {}
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    fn keep(&mut self, name: &str) {
        if !self.kept.iter().any(|k| k == name) {
            self.kept.push(name.to_owned());
        }
    }

    /// `_.a.b`, `a`, `_['my-key']`, `vault.x` → el nombre de la variable de RunPost.
    fn variable(&mut self, inner: &str) -> Option<String> {
        let inner = inner.trim();
        if let Some(index) = inner
            .strip_prefix('_')
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix('['))
            .and_then(|r| r.strip_suffix(']'))
        {
            let key = index.trim();
            let key = key
                .strip_prefix('\'')
                .and_then(|k| k.strip_suffix('\''))
                .or_else(|| key.strip_prefix('"').and_then(|k| k.strip_suffix('"')))?;
            return (!key.is_empty() && !key.contains(['{', '}'])).then(|| key.to_owned());
        }
        let path = inner.strip_prefix("_.").unwrap_or(inner);
        if !is_path(path) {
            return None;
        }
        if let Some(name) = path.strip_prefix("vault.") {
            if !self.vault.iter().any(|v| v == name) {
                self.vault.push(name.to_owned());
            }
            return Some(name.to_owned());
        }
        Some(path.to_owned())
    }

    /// `insomnia.` → `pm.` donde `insomnia` es un nombre suelto (no `x.insomnia.`, ni en `'…`).
    fn translate(&mut self, code: &str) -> String {
        if code.trim().is_empty() {
            return String::new();
        }
        self.scripts += 1;
        let mut out = String::with_capacity(code.len());
        let mut rest = code;
        while let Some(at) = rest.find("insomnia.") {
            out.push_str(&rest[..at]);
            let free = out.chars().next_back().is_none_or(|c| {
                !(c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | '-' | '"' | '\''))
            });
            out.push_str(if free { "pm." } else { "insomnia." });
            rest = &rest[at + "insomnia.".len()..];
        }
        out.push_str(rest);
        out
    }

    fn vars(&mut self, env: &Value) -> Vec<Variable> {
        let mut vars = Vec::new();
        // Vista de tabla (`kvPairData`), con tipo `secret`.
        if text(env.get("environmentType")) == "kv"
            && let Some(Value::Array(pairs)) = env.get("kvPairData")
        {
            for pair in pairs {
                let key = text(pair.get("name"));
                if key.is_empty() {
                    continue;
                }
                let value = self.t(&text(pair.get("value")));
                let mut var = if text(pair.get("type")) == "secret" {
                    Variable::secret(key, value)
                } else {
                    Variable::new(key, value)
                };
                var.enabled = pair.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                vars.push(var);
            }
        } else if let Some(Value::Object(data)) = env.get("data") {
            let mut flat = Vec::new();
            let mut vault = Vec::new();
            for (key, value) in data {
                if key == "__insomnia_vault" {
                    if let Value::Object(secrets) = value {
                        vault.extend(secrets.keys().cloned());
                    }
                    continue;
                }
                flatten(key, value, &mut flat, 0);
            }
            for (key, value) in flat {
                let value = self.t(&value);
                vars.push(Variable::new(key, value));
            }
            // Insomnia no exporta el valor de los secretos del vault.
            vars.extend(vault.into_iter().map(|name| Variable::secret(name, "")));
        }
        self.credentials += vars
            .iter()
            .filter(|v| !v.secret && looks_like_credential(&v.key))
            .count();
        vars
    }

    fn pairs(&mut self, list: Option<&Value>) -> Vec<KeyValue> {
        array(list)
            .iter()
            .filter(|entry| !text(entry.get("name")).trim().is_empty())
            .map(|entry| KeyValue {
                key: self.t(&text(entry.get("name"))),
                value: self.t(&text(entry.get("value"))),
                enabled: !entry
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
            .collect()
    }

    /// `None` = heredar: `{}`, sin `type` o desactivada. Un prefijo propio de bearer en una
    /// petición pasa a la cabecera `Authorization` (`extra`).
    fn auth(
        &mut self,
        value: Option<&Value>,
        at: &str,
        extra: Option<&mut Vec<KeyValue>>,
    ) -> Option<Auth> {
        let auth = value.filter(|v| v.is_object())?;
        if auth.get("disabled").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let kind = auth.get("type").and_then(Value::as_str)?;
        let mut field = |key: &str| self.t(&text(auth.get(key)));
        Some(match kind {
            "none" => Auth::None,
            "basic" => Auth::Basic {
                username: field("username"),
                password: field("password"),
            },
            "bearer" => {
                let token = field("token");
                let prefix = text(auth.get("prefix"));
                match extra {
                    Some(headers)
                        if !prefix.is_empty() && !prefix.eq_ignore_ascii_case("bearer") =>
                    {
                        let value = if prefix == "NO_PREFIX" {
                            token
                        } else {
                            format!("{prefix} {token}")
                        };
                        headers.push(KeyValue::new("Authorization", value));
                        Auth::None
                    }
                    _ => Auth::Bearer { token },
                }
            }
            "apikey" => {
                let key = field("key");
                let value = field("value");
                match text(auth.get("addTo")).as_str() {
                    "queryParams" => Auth::ApiKey {
                        key,
                        value,
                        location: ApiKeyIn::Query,
                    },
                    "cookie" => match extra {
                        Some(headers) => {
                            headers.push(KeyValue::new("Cookie", format!("{key}={value}")));
                            Auth::None
                        }
                        None => Auth::ApiKey {
                            key,
                            value,
                            location: ApiKeyIn::Header,
                        },
                    },
                    _ => Auth::ApiKey {
                        key,
                        value,
                        location: ApiKeyIn::Header,
                    },
                }
            }
            "digest" => Auth::Digest {
                username: field("username"),
                password: field("password"),
            },
            "oauth2" => match text(auth.get("grantType")).as_str() {
                "client_credentials" => Auth::oauth2_cc(OAuth2Cc {
                    token_url: field("accessTokenUrl"),
                    client_id: field("clientId"),
                    client_secret: field("clientSecret"),
                    scope: field("scope"),
                    client_auth: client_auth(auth),
                }),
                "" | "authorization_code" => {
                    let redirect = field("redirectUrl");
                    Auth::oauth2_code(OAuth2Code {
                        auth_url: field("authorizationUrl"),
                        token_url: field("accessTokenUrl"),
                        client_id: field("clientId"),
                        client_secret: field("clientSecret"),
                        scope: field("scope"),
                        redirect_uri: super::local_redirect(&redirect, at, &mut self.unsupported),
                        client_auth: client_auth(auth),
                    })
                }
                grant => {
                    self.unsupported.push(format!(
                        "{at}: OAuth 2.0 grant {grant:?} not supported (only client credentials and authorization code), imported as none"
                    ));
                    Auth::None
                }
            },
            other => {
                self.unsupported.push(format!(
                    "{at}: auth {other:?} not supported yet, imported as none"
                ));
                Auth::None
            }
        })
    }

    fn body(&mut self, value: Option<&Value>) -> Body {
        let Some(body) = value.filter(|v| v.is_object()) else {
            return Body::None;
        };
        let mime_type = text(body.get("mimeType"));
        let mime = mime_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let file = text(body.get("fileName"));
        match mime.as_str() {
            "application/x-www-form-urlencoded" => Body::Urlencoded {
                fields: self.pairs(body.get("params")),
            },
            "multipart/form-data" => {
                let mut fields = Vec::new();
                for part in array(body.get("params")) {
                    let key = self.t(&text(part.get("name")));
                    let enabled = !part
                        .get("disabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if text(part.get("type")) == "file" {
                        fields.push(MultipartField {
                            key,
                            value: String::new(),
                            kind: FieldKind::File,
                            path: text(part.get("fileName")),
                            enabled,
                        });
                    } else {
                        fields.push(MultipartField {
                            key,
                            value: self.t(&text(part.get("value"))),
                            kind: FieldKind::Text,
                            path: String::new(),
                            enabled,
                        });
                    }
                }
                Body::Multipart { fields }
            }
            "application/graphql" => {
                let source = text(body.get("text"));
                let parsed: Option<Value> = serde_json::from_str(&source).ok();
                let query = parsed
                    .as_ref()
                    .map(|p| text(p.get("query")))
                    .unwrap_or_else(|| source.clone());
                let variables = match parsed.as_ref().and_then(|p| p.get("variables")) {
                    Some(Value::String(vars)) => vars.clone(),
                    Some(vars @ (Value::Object(_) | Value::Array(_))) => {
                        serde_json::to_string_pretty(vars).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                Body::Graphql {
                    query: self.t(&query),
                    variables: self.t(&variables),
                }
            }
            // Cualquier otro cuerpo con `fileName` envía ese archivo.
            _ if !file.is_empty() => Body::Binary { path: file },
            _ => {
                let source = text(body.get("text"));
                if source.is_empty() && mime.is_empty() {
                    return Body::None;
                }
                let lang = if mime.contains("json") {
                    RawLang::Json
                } else if mime.contains("xml") {
                    RawLang::Xml
                } else if mime.contains("html") {
                    RawLang::Html
                } else if mime.contains("javascript") {
                    RawLang::Javascript
                } else {
                    RawLang::Text
                };
                Body::Raw {
                    lang,
                    text: self.t(&source),
                }
            }
        }
    }

    /// Variables de carpeta (`environment`) y scripts traducidos.
    fn scripts(&mut self, value: &Value, folder_vars: bool) -> Scripts {
        let mut pre = String::new();
        if folder_vars && let Some(Value::Object(map)) = value.get("environment") {
            let mut flat = Vec::new();
            for (key, value) in map {
                flatten(key, value, &mut flat, 0);
            }
            for (key, value) in flat {
                let value = self.t(&value);
                pre.push_str(&format!(
                    "pm.variables.set({}, {});\n",
                    serde_json::to_string(&key).unwrap_or_default(),
                    serde_json::to_string(&value).unwrap_or_default()
                ));
            }
        }
        pre.push_str(&self.translate(&script(value, "preRequestScript", "preRequest")));
        Scripts {
            pre: pre.trim_end().to_owned(),
            test: self
                .translate(&script(value, "afterResponseScript", "afterResponse"))
                .trim_end()
                .to_owned(),
        }
    }

    fn request(&mut self, value: &Value, inherited: &[KeyValue], parent: &str) -> Request {
        let name = text(value.get("name"));
        let at = format!("{parent}/{name}");
        let method = text(value.get("method")).to_ascii_uppercase();
        let url = self.t(&text(value.get("url")));
        let (base, mut params) = split_query(&url);
        // Insomnia añade `parameters` a la URL al enviar.
        for param in self.pairs(value.get("parameters")) {
            if !params
                .iter()
                .any(|p| p.key == param.key && p.value == param.value)
            {
                params.push(param);
            }
        }
        let path: Vec<(String, String)> = array(value.get("pathParameters"))
            .iter()
            .map(|p| (text(p.get("name")), self.t(&text(p.get("value")))))
            .collect();
        let url = fill_path_params(&base, &path, &mut self.path_vars);
        let mut headers: Vec<KeyValue> = inherited.to_vec();
        headers.extend(self.pairs(value.get("headers")));
        let auth = self
            .auth(value.get("authentication"), &at, Some(&mut headers))
            .unwrap_or(Auth::Inherit);
        let mut request = Request::new(
            name,
            if method.is_empty() {
                "GET".into()
            } else {
                method
            },
            url,
        );
        request.params = params;
        request.body = self.body(value.get("body"));
        request.headers = headers;
        request.auth = auth;
        request.scripts = self.scripts(value, false);
        request.settings.follow_redirects = follow_redirects(value);
        request.description = description(value);
        request
    }

    fn items(&mut self, nodes: Vec<Node<'_>>, inherited: &[KeyValue], parent: &str) -> Vec<Item> {
        let mut items = Vec::new();
        for node in nodes {
            match node {
                Node::Folder(value, children) => {
                    let name = text(value.get("name"));
                    let at = format!("{parent}/{name}");
                    let mut headers = inherited.to_vec();
                    headers.extend(
                        self.pairs(value.get("headers"))
                            .into_iter()
                            .filter(|h| h.enabled),
                    );
                    let mut folder = Folder::new(name);
                    folder.description = description(value);
                    folder.auth = self.auth(value.get("authentication"), &at, None);
                    folder.scripts = self.scripts(value, true);
                    folder.items = self.items(children, &headers, &at);
                    items.push(Item::Folder(folder));
                }
                Node::Request(value) => {
                    items.push(Item::Request(Box::new(
                        self.request(value, inherited, parent),
                    )));
                }
            }
        }
        items
    }

    fn environments(
        &mut self,
        workspace: &Workspace<'_>,
        base_vars: &[Variable],
    ) -> Vec<Environment> {
        workspace
            .subs
            .iter()
            .map(|sub| {
                let sub_name = text(sub.get("name"));
                let mut env = Environment::new(format!("{} - {sub_name}", workspace.name));
                env.id = stable_id("env", &format!("insomnia:{}", id(sub)));
                let mut vars = base_vars.to_vec();
                for var in self.vars(sub) {
                    match vars.iter_mut().find(|v| v.key == var.key) {
                        Some(existing) => *existing = var,
                        None => vars.push(var),
                    }
                }
                env.variables = vars;
                env
            })
            .collect()
    }
}

fn convert(workspaces: Vec<Workspace<'_>>, skipped: usize, extra: Vec<String>) -> Result<Imported> {
    let mut b = Builder::default();
    let mut imported = Imported::default();
    let mut vault: Vec<String> = Vec::new();
    for workspace in workspaces {
        let name = if workspace.name.trim().is_empty() {
            "Insomnia import".to_owned()
        } else {
            workspace.name.clone()
        };
        let base_vars = workspace.base.map(|base| b.vars(base)).unwrap_or_default();
        if workspace.environments_only {
            // Entorno global: un entorno por subentorno (con lo del base) o solo el base.
            let mut envs = b.environments(&workspace, &base_vars);
            if envs.is_empty() && !base_vars.is_empty() {
                let mut env = Environment::new(name.clone());
                env.id = stable_id("env", &format!("insomnia:{}", workspace.id));
                env.variables = base_vars;
                envs.push(env);
            }
            imported.environments.extend(envs);
            continue;
        }
        let mut collection = Collection::new(name.clone());
        collection.id = stable_id("col", &format!("insomnia:{}", workspace.id));
        collection.description = workspace.description.clone();
        collection.variables = base_vars;
        let environments = b.environments(&workspace, &[]);
        collection.items = b.items(workspace.items, &[], &name);
        for var in std::mem::take(&mut b.path_vars) {
            if !collection.variables.iter().any(|v| v.key == var) {
                collection.variables.push(Variable::new(var, ""));
            }
        }
        for var in std::mem::take(&mut b.vault) {
            let defined = collection.variables.iter().any(|v| v.key == var)
                || environments
                    .iter()
                    .any(|env| env.variables.iter().any(|v| v.key == var));
            if !defined {
                collection.variables.push(Variable::secret(var.clone(), ""));
            }
            if !vault.contains(&var) {
                vault.push(var);
            }
        }
        imported.collections.push(collection);
        imported.environments.extend(environments);
    }
    if imported.collections.is_empty() && imported.environments.is_empty() {
        bail!("the Insomnia export has no requests or environments to import");
    }
    let mut warnings = std::mem::take(&mut b.unsupported);
    vault.append(&mut b.vault);
    if !vault.is_empty() {
        warnings.push(format!(
            "Insomnia does not export vault secrets: fill in the secret variables {}",
            vault.join(", ")
        ));
    }
    if b.scripts > 0 {
        warnings.push(format!(
            "{} Insomnia scripts translated (insomnia.* → pm.*): review them",
            b.scripts
        ));
    }
    if !b.kept.is_empty() {
        warnings.push(format!(
            "template tags or expressions with no RunPost equivalent were kept as they are: {}",
            b.kept.join(", ")
        ));
    }
    if b.credentials > 0 {
        warnings.push(format!(
            "{} environment variables look like credentials (token, password, key): mark them as secret",
            b.credentials
        ));
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} gRPC, WebSocket or Socket.IO requests skipped: only HTTP requests are imported"
        ));
    }
    warnings.extend(extra);
    imported.warnings = warnings;
    Ok(imported)
}

pub fn import(value: &Value) -> Result<Imported> {
    let mut skipped = 0;
    let mut extra = Vec::new();
    let workspaces = if is_v4(value) {
        let workspaces = v4_workspaces(value, &mut skipped);
        let ignored = array(value.get("resources"))
            .iter()
            .filter(|r| {
                kind(r) == "workspace"
                    && matches!(text(r.get("scope")).as_str(), "mock-server" | "mcp")
            })
            .count();
        if ignored > 0 {
            extra.push(format!("{ignored} mock server or MCP workspaces skipped"));
        }
        workspaces
    } else {
        match v5_type(value) {
            Some("collection") => vec![v5_workspace(value, false, &mut skipped)],
            Some("spec") => {
                extra.push(
                    "the API spec of the design document was not imported: import it as OpenAPI"
                        .into(),
                );
                vec![v5_workspace(value, false, &mut skipped)]
            }
            Some("environment") => vec![v5_workspace(value, true, &mut skipped)],
            Some(other) => bail!("Insomnia {other} files cannot be imported"),
            None => bail!("not an Insomnia export"),
        }
    };
    convert(workspaces, skipped, extra)
}
