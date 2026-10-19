//! Importadores (sección 11). v1: Postman v2.1 (colección y entorno). Fase 4: curl, HAR, OpenAPI 3,
//! Insomnia y Bruno.

pub mod bruno;
pub mod curl;
pub mod har;
pub mod insomnia;
pub mod openapi;
pub mod postman_v21;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Map, Number, Value};
use yaml_rust2::{Yaml, YamlLoader};

use crate::model::{Collection, Environment};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[serde(rename = "postman_v21")]
    Postman,
    Openapi,
    Har,
    Insomnia,
    Bruno,
    Curl,
}

impl Format {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "postman" | "postman_v21" => Some(Self::Postman),
            "openapi" | "openapi3" | "swagger" => Some(Self::Openapi),
            "har" => Some(Self::Har),
            "insomnia" => Some(Self::Insomnia),
            "bruno" => Some(Self::Bruno),
            "curl" => Some(Self::Curl),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postman => "postman_v21",
            Self::Openapi => "openapi",
            Self::Har => "har",
            Self::Insomnia => "insomnia",
            Self::Bruno => "bruno",
            Self::Curl => "curl",
        }
    }
}

/// Lo que sale de un importador antes de guardarlo en el workspace.
#[derive(Debug, Default)]
pub struct Imported {
    pub collections: Vec<Collection>,
    pub environments: Vec<Environment>,
    /// Lo que no se pudo trasladar.
    pub warnings: Vec<String>,
}

/// Detecta el formato de un texto pegado o leído de un archivo.
pub fn detect(text: &str) -> Option<Format> {
    let trimmed = text.trim_start();
    if curl::looks_like(trimmed) {
        return Some(Format::Curl);
    }
    let value = parse_document(text).ok()?;
    detect_value(&value)
}

pub fn detect_value(value: &Value) -> Option<Format> {
    if postman_v21::detect(value).is_some() {
        Some(Format::Postman)
    } else if openapi::detect(value) {
        Some(Format::Openapi)
    } else if har::detect(value) {
        Some(Format::Har)
    } else if insomnia::detect(value) {
        Some(Format::Insomnia)
    } else if bruno::detect(value) {
        Some(Format::Bruno)
    } else {
        None
    }
}

/// JSON o YAML → `Value`. En YAML las claves que no son texto (`200:` en OpenAPI) pasan a texto.
pub fn parse_document(text: &str) -> Result<Value> {
    let text = text.trim_start_matches('\u{feff}');
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Ok(value);
    }
    let docs = YamlLoader::load_from_str(text).context("not valid JSON or YAML")?;
    match docs.into_iter().next() {
        Some(doc) => Ok(yaml_to_json(doc)),
        None => bail!("empty document"),
    }
}

fn yaml_key(key: Yaml) -> String {
    match key {
        Yaml::String(text) | Yaml::Real(text) => text,
        Yaml::Integer(n) => n.to_string(),
        Yaml::Boolean(b) => b.to_string(),
        Yaml::Null => "null".into(),
        other => yaml_to_json(other).to_string(),
    }
}

fn yaml_to_json(yaml: Yaml) -> Value {
    match yaml {
        Yaml::String(text) => Value::String(text),
        Yaml::Integer(n) => Value::Number(n.into()),
        Yaml::Real(text) => text
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::String(text)),
        Yaml::Boolean(b) => Value::Bool(b),
        Yaml::Array(items) => Value::Array(items.into_iter().map(yaml_to_json).collect()),
        Yaml::Hash(hash) => {
            let mut map = Map::new();
            for (key, value) in hash {
                map.insert(yaml_key(key), yaml_to_json(value));
            }
            Value::Object(map)
        }
        Yaml::Null | Yaml::BadValue | Yaml::Alias(_) => Value::Null,
    }
}

// ---- utilidades comunes de los importadores ----

/// Texto de un campo JSON (número o booleano como texto; ausente o null = vacío).
pub(crate) fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub(crate) fn array(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// Separa la query de una URL: `(url sin query, params)`; los valores se dejan tal cual (sin
/// decodificar), como los escribe el usuario en Postman.
pub(crate) fn split_query(url: &str) -> (String, Vec<crate::model::KeyValue>) {
    let (base, fragment) = match url.find('#') {
        Some(at) => (&url[..at], &url[at..]),
        None => (url, ""),
    };
    let Some((path, query)) = base.split_once('?') else {
        return (url.to_owned(), Vec::new());
    };
    let params = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => crate::model::KeyValue::new(key, value),
            None => crate::model::KeyValue::new(pair, ""),
        })
        .collect();
    (format!("{path}{fragment}"), params)
}

/// Id estable (`col_` + 32 hex) para formatos sin UUID propio: reimportar lo mismo sustituye la
/// colección en vez de duplicarla.
pub(crate) fn stable_id(prefix: &str, seed: &str) -> String {
    use aws_lc_rs::digest;
    let hash = digest::digest(&digest::SHA256, seed.as_bytes());
    let hex: String = hash.as_ref()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{prefix}_{hex}")
}

/// Cabeceras heredadas (colección, carpetas) seguidas de las propias; una propia con el mismo
/// nombre manda. RunPost no tiene cabeceras compartidas: se copian a cada petición.
pub(crate) fn merge_headers(
    inherited: &[crate::model::KeyValue],
    own: Vec<crate::model::KeyValue>,
) -> Vec<crate::model::KeyValue> {
    let mut merged: Vec<_> = inherited
        .iter()
        .filter(|h| h.enabled && !own.iter().any(|o| o.key.eq_ignore_ascii_case(&h.key)))
        .cloned()
        .collect();
    merged.extend(own);
    merged
}

/// Parámetros de ruta `/:nombre` (Bruno, Insomnia): RunPost no los tiene, así que van en la URL.
/// Con valor se escribe el valor; sin valor, `{{nombre}}`, y el nombre va a `missing` para crear
/// la variable.
pub(crate) fn fill_path_params(
    url: &str,
    params: &[(String, String)],
    missing: &mut Vec<String>,
) -> String {
    let mut out = url.to_owned();
    for (name, value) in params {
        if name.is_empty() {
            continue;
        }
        let needle = format!("/:{name}");
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(&needle) {
            let after = &rest[at + needle.len()..];
            result.push_str(&rest[..at]);
            // Solo el segmento entero: `/:id` no toca `/:idx`.
            if after
                .chars()
                .next()
                .is_none_or(|c| matches!(c, '/' | '?' | '#' | '.' | ';' | '&'))
            {
                if value.is_empty() {
                    if !missing.contains(name) {
                        missing.push(name.clone());
                    }
                    result.push_str(&format!("/{{{{{name}}}}}"));
                } else {
                    result.push('/');
                    result.push_str(value);
                }
            } else {
                result.push_str(&needle);
            }
            rest = after;
        }
        result.push_str(rest);
        out = result;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_keys_become_text_and_json_wins() {
        let value = parse_document("openapi: 3.0.3\npaths:\n  /a:\n    get:\n      responses:\n        200:\n          description: ok\n").unwrap();
        assert_eq!(value["openapi"], "3.0.3");
        assert_eq!(
            value["paths"]["/a"]["get"]["responses"]["200"]["description"],
            "ok"
        );
        assert_eq!(parse_document("{\"a\": 1}").unwrap()["a"], 1);
        assert!(parse_document("a: [1, 2\n").is_err());
    }

    #[test]
    fn split_query_keeps_raw_values() {
        let (url, params) = split_query("{{base}}/v1/items?q=a%20b&flag&x=1=2#top");
        assert_eq!(url, "{{base}}/v1/items#top");
        let pairs: Vec<_> = params
            .iter()
            .map(|p| (p.key.as_str(), p.value.as_str()))
            .collect();
        assert_eq!(pairs, [("q", "a%20b"), ("flag", ""), ("x", "1=2")]);
    }

    #[test]
    fn path_params_and_stable_ids() {
        let mut missing = Vec::new();
        let url = fill_path_params(
            "{{base}}/v1/:currency/accounts/:accountId/:idx?x=:currency",
            &[
                ("currency".into(), "usd".into()),
                ("accountId".into(), String::new()),
                ("id".into(), "7".into()),
            ],
            &mut missing,
        );
        assert_eq!(
            url,
            "{{base}}/v1/usd/accounts/{{accountId}}/:idx?x=:currency"
        );
        assert_eq!(missing, ["accountId"]);

        let id = stable_id("col", "bruno:Acme");
        assert_eq!(id, stable_id("col", "bruno:Acme"));
        assert_ne!(id, stable_id("col", "bruno:Other"));
        assert!(id.starts_with("col_") && id.len() == 36);

        let inherited = [
            crate::model::KeyValue::new("Accept", "application/json"),
            crate::model::KeyValue::new("X-Trace", "1"),
        ];
        let merged = merge_headers(
            &inherited,
            vec![crate::model::KeyValue::new("accept", "*/*")],
        );
        let pairs: Vec<_> = merged
            .iter()
            .map(|h| (h.key.as_str(), h.value.as_str()))
            .collect();
        assert_eq!(pairs, [("X-Trace", "1"), ("accept", "*/*")]);
    }

    #[test]
    fn format_names() {
        assert_eq!(Format::parse("OpenAPI"), Some(Format::Openapi));
        assert_eq!(Format::parse("postman_v21"), Some(Format::Postman));
        assert_eq!(Format::Postman.as_str(), "postman_v21");
        assert_eq!(Format::parse("raml"), None);
    }
}
