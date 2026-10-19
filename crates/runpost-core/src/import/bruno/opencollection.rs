//! OpenCollection (YAML), el formato de Bruno por defecto desde la 3.1: una carpeta con
//! `opencollection.yml`, `folder.yml`, un `.yml` por petición y `environments/*.yml`, o un único
//! archivo empaquetado (`bundled: true`). Se lee a las mismas estructuras que los `.bru`, con sus
//! nombres (`disabled` → `~`, `placement: query` → `queryparams`, `"eq 201"` en las aserciones).

use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use super::{BruEnv, BruFile, BruFolder, BruRequest, Imported, Pair, Shared, build};
use crate::import::{array, parse_document, text};

/// Profundidad máxima de carpetas en un archivo empaquetado.
const MAX_DEPTH: usize = 64;

pub(super) fn detect(value: &Value) -> bool {
    value
        .get("opencollection")
        .and_then(Value::as_str)
        .is_some_and(|version| !version.trim().is_empty())
        && value.get("info").is_some_and(Value::is_object)
}

fn enabled(entry: &Value) -> bool {
    !entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// La variante elegida de una lista `[{selected, …}]` (o la primera).
fn selected(variants: &[Value]) -> Option<&Value> {
    variants
        .iter()
        .find(|v| v.get("selected").and_then(Value::as_bool) == Some(true))
        .or_else(|| variants.first())
}

/// Valor de una variable: texto, `{type, data}` o variantes `[{selected, value}]`.
fn var_value(entry: Option<&Value>) -> String {
    match entry {
        Some(Value::Object(map)) if map.contains_key("data") => text(map.get("data")),
        Some(Value::Array(variants)) => selected(variants)
            .map(|v| var_value(v.get("value")))
            .unwrap_or_default(),
        other => text(other),
    }
}

fn pairs(list: Option<&Value>) -> Vec<Pair> {
    array(list)
        .iter()
        .map(|entry| Pair {
            key: text(entry.get("name")),
            value: var_value(entry.get("value")),
            enabled: enabled(entry),
        })
        .collect()
}

/// `docs`: texto o `{content, type}`.
fn docs(entry: Option<&Value>) -> String {
    match entry {
        Some(Value::Object(map)) => text(map.get("content")),
        other => text(other),
    }
}

/// `auth`: `inherit` o `{type, …}`. `missing` es lo que vale sin `auth`.
fn auth(entry: Option<&Value>, shared: &mut Shared, missing: &str) {
    match entry {
        Some(Value::String(mode)) => shared.auth_mode = mode.clone(),
        Some(Value::Object(map)) => {
            let kind = text(map.get("type"));
            let mut attrs: HashMap<String, String> = HashMap::new();
            for (key, value) in map.iter().filter(|(key, _)| key.as_str() != "type") {
                // OAuth 2.0 anida `credentials {clientId, clientSecret, placement}`.
                if let Value::Object(inner) = value {
                    for (inner_key, inner_value) in inner {
                        attrs.insert(format!("{key}.{inner_key}"), text(Some(inner_value)));
                    }
                }
                attrs.insert(key.clone(), text(Some(value)));
            }
            if kind == "apikey" && attrs.get("placement").is_some_and(|p| p == "query") {
                attrs.insert("placement".into(), "queryparams".into());
            }
            shared.auth_mode = kind.clone();
            shared.auth.insert(kind, attrs);
        }
        _ => shared.auth_mode = missing.to_owned(),
    }
}

/// `variables`, `scripts` y `actions`: en `runtime` (petición) o en `request` (carpeta, colección).
fn runtime(container: Option<&Value>, shared: &mut Shared) {
    let field = |key: &str| container.and_then(|c| c.get(key));
    shared.vars_pre.extend(pairs(field("variables")));
    for script in array(field("scripts")) {
        let code = text(script.get("code"));
        match text(script.get("type")).as_str() {
            "before-request" => shared.script_pre = code,
            "after-response" => shared.script_post = code,
            "tests" => shared.tests = code,
            _ => {}
        }
    }
    // Variables posteriores a la respuesta: `set-variable` con la expresión a leer.
    for action in array(field("actions")) {
        if text(action.get("type")) == "set-variable" {
            shared.vars_post.push(Pair {
                key: text(action.get("variable").and_then(|v| v.get("name"))),
                value: text(action.get("selector").and_then(|s| s.get("expression"))),
                enabled: enabled(action),
            });
        }
    }
}

/// Lo común de `opencollection.yml` y `folder.yml` (`request {headers, auth, variables, …}`).
fn shared(request: Option<&Value>, docs_entry: Option<&Value>) -> Shared {
    let mut out = Shared {
        headers: pairs(request.and_then(|r| r.get("headers"))),
        docs: docs(docs_entry),
        ..Default::default()
    };
    auth(request.and_then(|r| r.get("auth")), &mut out, "inherit");
    runtime(request, &mut out);
    out
}

fn body(entry: Option<&Value>, request: &mut BruRequest) {
    let entry = match entry {
        Some(Value::Array(variants)) => selected(variants).and_then(|v| v.get("body")),
        other => other,
    };
    let Some(body) = entry.filter(|b| b.is_object()) else {
        request.body_mode = "none".into();
        return;
    };
    let data = body.get("data");
    let target = &mut request.body;
    let mode = match text(body.get("type")).as_str() {
        "json" => {
            target.json = text(data);
            "json"
        }
        "text" => {
            target.text = text(data);
            "text"
        }
        "xml" => {
            target.xml = text(data);
            "xml"
        }
        "sparql" => {
            target.sparql = text(data);
            "sparql"
        }
        "form-urlencoded" => {
            target.form = pairs(data);
            "formUrlEncoded"
        }
        "multipart-form" => {
            target.multipart = array(data)
                .iter()
                .map(|part| {
                    let value = match part.get("value") {
                        Some(Value::Array(paths)) => paths
                            .iter()
                            .map(|p| text(Some(p)))
                            .collect::<Vec<_>>()
                            .join("|"),
                        other => text(other),
                    };
                    let pair = Pair {
                        key: text(part.get("name")),
                        value,
                        enabled: enabled(part),
                    };
                    (pair, text(part.get("type")) == "file")
                })
                .collect();
            "multipartForm"
        }
        "file" => {
            target.file = array(data)
                .iter()
                .map(|file| BruFile {
                    path: text(file.get("filePath")),
                    content_type: text(file.get("contentType")),
                    selected: file
                        .get("selected")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                })
                .collect();
            "file"
        }
        _ => "none",
    };
    request.body_mode = mode.into();
}

fn graphql_body(entry: Option<&Value>, request: &mut BruRequest) {
    request.body_mode = "graphql".into();
    let entry = match entry {
        Some(Value::Array(variants)) => selected(variants).and_then(|v| v.get("body")),
        other => other,
    };
    let Some(body) = entry else {
        return;
    };
    request.body.graphql_query = text(body.get("query"));
    request.body.graphql_vars = match body.get("variables") {
        Some(Value::String(vars)) => vars.clone(),
        Some(vars @ (Value::Object(_) | Value::Array(_))) => {
            serde_json::to_string_pretty(vars).unwrap_or_default()
        }
        _ => String::new(),
    };
}

/// Una petición (`info.type: http | graphql`); `None` para lo demás (gRPC, WebSocket, script).
fn request(item: &Value, fallback: &str) -> Option<BruRequest> {
    let info = item.get("info");
    let info_field = |key: &str| info.and_then(|i| i.get(key));
    let kind = match text(info_field("type")).as_str() {
        "" | "http" => "http",
        "graphql" => "graphql",
        _ => return None,
    };
    let section = item.get(kind);
    let field = |key: &str| section.and_then(|s| s.get(key));
    let name = text(info_field("name"));
    let method = text(field("method")).to_ascii_uppercase();
    let mut request = BruRequest {
        name: if name.is_empty() {
            fallback.to_owned()
        } else {
            name
        },
        kind: kind.into(),
        seq: info_field("seq")
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        method: match (method.is_empty(), kind) {
            (false, _) => method,
            (true, "graphql") => "POST".into(),
            (true, _) => "GET".into(),
        },
        url: text(field("url")),
        ..Default::default()
    };
    let mut query = Vec::new();
    for param in array(field("params")) {
        let pair = Pair {
            key: text(param.get("name")),
            value: var_value(param.get("value")),
            enabled: enabled(param),
        };
        if text(param.get("type")) == "path" {
            request.path.push(pair);
        } else {
            query.push(pair);
        }
    }
    request.query = (!query.is_empty()).then_some(query);
    request.shared.headers = pairs(field("headers"));
    if kind == "graphql" {
        graphql_body(field("body"), &mut request);
    } else {
        body(field("body"), &mut request);
    }
    let run = item.get("runtime");
    // La 3.0.0-rc1 guardaba el auth en `runtime.auth`.
    let auth_entry = field("auth").or_else(|| run.and_then(|r| r.get("auth")));
    auth(auth_entry, &mut request.shared, "none");
    runtime(run, &mut request.shared);
    request.asserts = array(run.and_then(|r| r.get("assertions")))
        .iter()
        .map(|assert| Pair {
            key: text(assert.get("expression")),
            value: format!(
                "{} {}",
                text(assert.get("operator")),
                text(assert.get("value"))
            )
            .trim()
            .to_owned(),
            enabled: enabled(assert),
        })
        .collect();
    let doc = docs(item.get("docs"));
    request.shared.docs = if doc.is_empty() {
        text(info_field("description"))
    } else {
        doc
    };
    Some(request)
}

/// `folder.yml`: `(nombre, seq, lo común)`.
pub(super) fn folder_file(source: &str) -> Result<(String, f64, Shared)> {
    let value = parse_document(source)?;
    let info = value.get("info");
    Ok((
        text(info.and_then(|i| i.get("name"))),
        info.and_then(|i| i.get("seq"))
            .and_then(Value::as_f64)
            .unwrap_or_default(),
        shared(value.get("request"), value.get("docs")),
    ))
}

pub(super) fn request_file(source: &str, fallback: &str) -> Result<Option<BruRequest>> {
    Ok(request(&parse_document(source)?, fallback))
}

fn env_from(name: &str, value: &Value) -> BruEnv {
    BruEnv {
        name: name.to_owned(),
        vars: array(value.get("variables"))
            .iter()
            .map(|var| {
                // Un secreto no lleva valor en el archivo.
                let secret = var.get("secret").and_then(Value::as_bool).unwrap_or(false);
                let pair = Pair {
                    key: text(var.get("name")),
                    value: if secret {
                        String::new()
                    } else {
                        var_value(var.get("value"))
                    },
                    enabled: enabled(var),
                };
                (pair, secret)
            })
            .collect(),
    }
}

/// `environments/<Nombre>.yml`: el nombre sale del archivo, como en Bruno.
pub(super) fn environment(name: &str, source: &str) -> Result<BruEnv> {
    Ok(env_from(name, &parse_document(source)?))
}

/// `opencollection.yml`: la colección (sin peticiones) y la lista `ignore`.
pub(super) fn root(config: &Value) -> (BruFolder, Vec<String>) {
    let collection = BruFolder {
        name: text(config.get("info").and_then(|i| i.get("name"))),
        shared: shared(config.get("request"), config.get("docs")),
        ..Default::default()
    };
    let ignore = array(
        config
            .get("extensions")
            .and_then(|e| e.get("bruno"))
            .and_then(|b| b.get("ignore")),
    )
    .iter()
    .map(|v| text(Some(v)))
    .collect();
    (collection, ignore)
}

fn items(list: &[Value], folder: &mut BruFolder, skipped: &mut usize, depth: usize) {
    for item in list {
        let info = item.get("info");
        if text(info.and_then(|i| i.get("type"))) == "folder" {
            if depth >= MAX_DEPTH {
                *skipped += 1;
                continue;
            }
            let mut sub = BruFolder {
                name: text(info.and_then(|i| i.get("name"))),
                seq: info
                    .and_then(|i| i.get("seq"))
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                shared: shared(item.get("request"), item.get("docs")),
                ..Default::default()
            };
            items(array(item.get("items")), &mut sub, skipped, depth + 1);
            folder.folders.push(sub);
        } else {
            match request(item, "") {
                Some(request) => folder.requests.push(request),
                None => *skipped += 1,
            }
        }
    }
}

/// Archivo empaquetado: carpetas con `items[]` y entornos en `config.environments[]`.
pub(super) fn import(value: &Value) -> Result<Imported> {
    let (mut collection, _) = root(value);
    let mut skipped = 0;
    items(array(value.get("items")), &mut collection, &mut skipped, 0);
    let environments = array(value.get("config").and_then(|c| c.get("environments")))
        .iter()
        .map(|env| env_from(&text(env.get("name")), env))
        .collect();
    Ok(build(collection, environments, skipped))
}
