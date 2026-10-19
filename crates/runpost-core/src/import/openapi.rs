//! OpenAPI 3.x (JSON o YAML) → colección con una petición por operación (sección 11, fase 4).
//!
//! `{{baseUrl}}` sale del primer servidor; una carpeta por etiqueta; los parámetros de ruta pasan a
//! `{{nombre}}` (variables de colección), los de query y cabecera llevan su ejemplo, y el cuerpo es el
//! ejemplo del documento o uno generado desde el esquema. La seguridad global es la auth de la
//! colección, con sus credenciales como variables secretas vacías.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

use crate::model::{
    ApiKeyIn, Auth, Body, ClientAuth, Collection, FieldKind, Folder, Item, KeyValue,
    MultipartField, OAUTH_REDIRECT_URI, OAuth2Cc, OAuth2Code, RawLang, Request, Variable,
};

use super::{Imported, array, text};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];
/// Profundidad máxima al generar ejemplos (esquemas recursivos).
const MAX_DEPTH: usize = 8;

pub fn detect(value: &Value) -> bool {
    value
        .get("openapi")
        .and_then(Value::as_str)
        .is_some_and(|v| v.starts_with('3'))
        || value.get("swagger").is_some()
}

/// Sigue los `$ref` locales (`#/components/...`).
fn resolve<'a>(doc: &'a Value, mut value: &'a Value) -> &'a Value {
    for _ in 0..20 {
        let Some(reference) = value.get("$ref").and_then(Value::as_str) else {
            return value;
        };
        let Some(pointer) = reference.strip_prefix('#') else {
            return &Value::Null;
        };
        value = doc.pointer(pointer).unwrap_or(&Value::Null);
    }
    value
}

/// Ejemplo de un valor según su esquema.
fn example(doc: &Value, schema: &Value, depth: usize) -> Value {
    let schema = resolve(doc, schema);
    if let Some(value) = schema.get("example") {
        return value.clone();
    }
    if let Some(value) = array(schema.get("examples")).first() {
        return value.clone();
    }
    if let Some(value) = schema.get("default") {
        return value.clone();
    }
    if let Some(value) = array(schema.get("enum")).first() {
        return value.clone();
    }
    if depth >= MAX_DEPTH {
        return Value::Null;
    }
    if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        for part in parts {
            if let Value::Object(map) = example(doc, part, depth + 1) {
                merged.extend(map);
            }
        }
        return Value::Object(merged);
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = array(schema.get(key)).first() {
            return example(doc, first, depth + 1);
        }
    }
    let kind = match schema.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .find(|k| *k != "null")
            .unwrap_or_default()
            .to_owned(),
        _ if schema.get("properties").is_some() => "object".into(),
        _ => String::new(),
    };
    match kind.as_str() {
        "object" => {
            let mut map = Map::new();
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (name, property) in properties {
                    map.insert(name.clone(), example(doc, property, depth + 1));
                }
            }
            Value::Object(map)
        }
        "array" => match schema.get("items") {
            Some(items) => json!([example(doc, items, depth + 1)]),
            None => json!([]),
        },
        "integer" | "number" => json!(0),
        "boolean" => json!(true),
        "string" => json!(match text(schema.get("format")).as_str() {
            "date-time" => "2026-01-01T00:00:00Z",
            "date" => "2026-01-01",
            "email" => "user@example.com",
            "uuid" => "00000000-0000-4000-8000-000000000000",
            "uri" | "url" => "https://example.com",
            "binary" | "byte" => "",
            _ => "string",
        }),
        _ => Value::Null,
    }
}

/// Valor de ejemplo como texto (query, cabeceras, formularios).
fn example_text(doc: &Value, parameter: &Value) -> String {
    let value = match parameter.get("example") {
        Some(value) => value.clone(),
        None => match parameter
            .get("examples")
            .and_then(Value::as_object)
            .and_then(|m| m.values().next())
        {
            Some(first) => resolve(doc, first)
                .get("value")
                .cloned()
                .unwrap_or(Value::Null),
            None => match parameter.get("schema") {
                Some(schema) => {
                    let schema = resolve(doc, schema);
                    // Solo ejemplos explícitos: "string" o 0 en una query confunden más que ayudan.
                    schema
                        .get("example")
                        .or_else(|| schema.get("default"))
                        .or_else(|| array(schema.get("enum")).first())
                        .cloned()
                        .unwrap_or(Value::Null)
                }
                None => Value::Null,
            },
        },
    };
    match value {
        Value::String(text) => text,
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `https://{region}.api.test/{version}` con los valores por defecto de sus variables.
fn server_url(server: &Value) -> String {
    let mut url = text(server.get("url"));
    if let Some(vars) = server.get("variables").and_then(Value::as_object) {
        for (name, var) in vars {
            url = url.replace(&format!("{{{name}}}"), &text(var.get("default")));
        }
    }
    url.trim_end_matches('/').to_owned()
}

/// Auth de un esquema de seguridad y las variables que necesita.
fn scheme_auth(
    name: &str,
    scheme: &Value,
    vars: &mut BTreeMap<String, bool>,
    warnings: &mut Vec<String>,
) -> Option<Auth> {
    let kind = text(scheme.get("type"));
    let mut secret = |key: &str| {
        vars.insert(key.to_owned(), true);
        format!("{{{{{key}}}}}")
    };
    match kind.as_str() {
        "http" => match text(scheme.get("scheme")).to_ascii_lowercase().as_str() {
            "bearer" => Some(Auth::Bearer {
                token: secret("bearerToken"),
            }),
            "basic" => Some(Auth::Basic {
                username: secret("basicUsername"),
                password: secret("basicPassword"),
            }),
            other => {
                warnings.push(format!(
                    "security scheme {name}: http {other} not supported"
                ));
                None
            }
        },
        "apiKey" => {
            let location = match text(scheme.get("in")).as_str() {
                "header" => ApiKeyIn::Header,
                "query" => ApiKeyIn::Query,
                other => {
                    warnings.push(format!(
                        "security scheme {name}: API key in {other} not supported"
                    ));
                    return None;
                }
            };
            Some(Auth::ApiKey {
                key: text(scheme.get("name")),
                value: secret("apiKey"),
                location,
            })
        }
        "oauth2" => {
            let flows = scheme.get("flows");
            let scopes = |flow: &Value| {
                flow.get("scopes")
                    .and_then(Value::as_object)
                    .map(|scopes| scopes.keys().cloned().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default()
            };
            // Una URL relativa cuelga del servidor.
            let absolute = |url: String| {
                if url.starts_with('/') {
                    format!("{{{{baseUrl}}}}{url}")
                } else {
                    url
                }
            };
            let client_credentials = flows.and_then(|f| f.get("clientCredentials"));
            let authorization_code = flows.and_then(|f| f.get("authorizationCode"));
            if client_credentials.is_none() && authorization_code.is_none() {
                warnings.push(format!(
                    "security scheme {name}: OAuth 2.0 flow not supported (only clientCredentials and authorizationCode): set as Bearer {{{{accessToken}}}}"
                ));
                return Some(Auth::Bearer {
                    token: secret("accessToken"),
                });
            }
            let client_secret = secret("clientSecret");
            vars.insert("clientId".into(), false);
            let client_id = "{{clientId}}".to_owned();
            // Client credentials no necesita navegador: va primero si hay los dos.
            Some(match (client_credentials, authorization_code) {
                (Some(flow), _) => Auth::oauth2_cc(OAuth2Cc {
                    token_url: absolute(text(flow.get("tokenUrl"))),
                    client_id,
                    client_secret,
                    scope: scopes(flow),
                    client_auth: ClientAuth::Basic,
                }),
                (None, Some(flow)) => Auth::oauth2_code(OAuth2Code {
                    auth_url: absolute(text(flow.get("authorizationUrl"))),
                    token_url: absolute(text(flow.get("tokenUrl"))),
                    client_id,
                    client_secret,
                    scope: scopes(flow),
                    redirect_uri: OAUTH_REDIRECT_URI.to_owned(),
                    client_auth: ClientAuth::Basic,
                }),
                (None, None) => unreachable!("checked above"),
            })
        }
        "openIdConnect" => {
            warnings.push(format!(
                "security scheme {name} (openIdConnect): set as Bearer {{{{accessToken}}}}"
            ));
            Some(Auth::Bearer {
                token: secret("accessToken"),
            })
        }
        other => {
            warnings.push(format!("security scheme {name}: {other} not supported"));
            None
        }
    }
}

/// Auth de una lista `security` (la primera opción con un esquema conocido).
fn security_auth(
    doc: &Value,
    security: &Value,
    vars: &mut BTreeMap<String, bool>,
    warnings: &mut Vec<String>,
) -> Option<Auth> {
    let requirements = security.as_array()?;
    if requirements.is_empty() {
        return Some(Auth::None);
    }
    for requirement in requirements {
        for name in requirement.as_object().into_iter().flat_map(|m| m.keys()) {
            let scheme = doc
                .pointer(&format!("/components/securitySchemes/{name}"))
                .map(|s| resolve(doc, s));
            if let Some(auth) = scheme.and_then(|s| scheme_auth(name, s, vars, warnings)) {
                return Some(auth);
            }
        }
    }
    None
}

fn request_body(doc: &Value, body: &Value, headers: &mut Vec<KeyValue>) -> Body {
    let body = resolve(doc, body);
    let Some(content) = body.get("content").and_then(Value::as_object) else {
        return Body::None;
    };
    let pick = |test: &dyn Fn(&str) -> bool| content.iter().find(|(media, _)| test(media));
    let chosen = pick(&|m| m == "application/json" || m.ends_with("+json"))
        .or_else(|| pick(&|m| m == "application/x-www-form-urlencoded"))
        .or_else(|| pick(&|m| m == "multipart/form-data"))
        .or_else(|| pick(&|m| m.starts_with("text/")))
        .or_else(|| pick(&|m| m.ends_with("xml")))
        .or_else(|| content.iter().next());
    let Some((media, spec)) = chosen else {
        return Body::None;
    };
    let schema = spec.get("schema").map_or(&Value::Null, |s| resolve(doc, s));
    let sample = match spec.get("example") {
        Some(value) => value.clone(),
        None => match spec
            .get("examples")
            .and_then(Value::as_object)
            .and_then(|m| m.values().next())
        {
            Some(first) => resolve(doc, first)
                .get("value")
                .cloned()
                .unwrap_or(Value::Null),
            None => example(doc, schema, 0),
        },
    };
    let properties = || {
        schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    match media.as_str() {
        "application/x-www-form-urlencoded" => Body::Urlencoded {
            fields: properties()
                .iter()
                .map(|(name, property)| {
                    let value = match example(doc, property, 1) {
                        Value::String(text) => text,
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    KeyValue::new(name, value)
                })
                .collect(),
        },
        "multipart/form-data" => Body::Multipart {
            fields: properties()
                .iter()
                .map(|(name, property)| {
                    let property = resolve(doc, property);
                    let file = matches!(text(property.get("format")).as_str(), "binary" | "byte");
                    MultipartField {
                        key: name.clone(),
                        value: if file {
                            String::new()
                        } else {
                            match example(doc, property, 1) {
                                Value::String(text) => text,
                                Value::Null => String::new(),
                                other => other.to_string(),
                            }
                        },
                        kind: if file {
                            FieldKind::File
                        } else {
                            FieldKind::Text
                        },
                        path: String::new(),
                        enabled: true,
                    }
                })
                .collect(),
        },
        media => {
            let is_json = media == "application/json" || media.ends_with("+json");
            let text = match &sample {
                Value::String(text) if !is_json => text.clone(),
                Value::Null => String::new(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            let lang = if is_json {
                RawLang::Json
            } else if media.ends_with("xml") {
                RawLang::Xml
            } else if media == "text/html" {
                RawLang::Html
            } else {
                RawLang::Text
            };
            if media != "application/json" && !matches!(lang, RawLang::Xml | RawLang::Html) {
                headers.push(KeyValue::new("Content-Type", media));
            }
            Body::Raw { lang, text }
        }
    }
}

pub fn import(doc: &Value) -> Result<Imported> {
    if doc.get("swagger").is_some() {
        bail!("Swagger 2.0 is not supported: convert it to OpenAPI 3 first");
    }
    let info = doc.get("info").cloned().unwrap_or(Value::Null);
    let mut warnings = Vec::new();
    let mut vars: BTreeMap<String, bool> = BTreeMap::new();
    let base_url = array(doc.get("servers"))
        .first()
        .map(server_url)
        .unwrap_or_default();
    if base_url.is_empty() {
        warnings.push("no servers: set the baseUrl variable".into());
    }

    let mut collection = Collection::new(match text(info.get("title")) {
        title if title.is_empty() => "OpenAPI".into(),
        title => title,
    });
    collection.description = [text(info.get("description")), text(info.get("version"))]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\nVersion: ");
    collection.auth = doc
        .get("security")
        .and_then(|s| security_auth(doc, s, &mut vars, &mut warnings))
        .filter(|auth| *auth != Auth::None);

    let tag_docs: BTreeMap<String, String> = array(doc.get("tags"))
        .iter()
        .map(|t| (text(t.get("name")), text(t.get("description"))))
        .collect();
    let mut folders: Vec<Folder> = Vec::new();
    let mut root: Vec<Item> = Vec::new();
    let mut path_vars: Vec<String> = Vec::new();

    for (path, item) in doc
        .get("paths")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let item = resolve(doc, item);
        let shared = array(item.get("parameters"));
        for method in METHODS {
            let Some(op) = item.get(method) else {
                continue;
            };
            // Parámetros de la ruta y de la operación (esta gana por nombre + ubicación).
            let mut parameters: Vec<&Value> = Vec::new();
            for parameter in shared.iter().chain(array(op.get("parameters"))) {
                let parameter = resolve(doc, parameter);
                let key = (text(parameter.get("name")), text(parameter.get("in")));
                parameters.retain(|p| (text(p.get("name")), text(p.get("in"))) != key);
                parameters.push(parameter);
            }
            let mut url = format!("{{{{baseUrl}}}}{path}");
            let mut params = Vec::new();
            let mut headers = Vec::new();
            for parameter in parameters {
                let name = text(parameter.get("name"));
                match text(parameter.get("in")).as_str() {
                    "path" => {
                        url = url.replace(&format!("{{{name}}}"), &format!("{{{{{name}}}}}"));
                        if !path_vars.contains(&name) {
                            path_vars.push(name);
                        }
                    }
                    "query" => {
                        let mut row = KeyValue::new(name, example_text(doc, parameter));
                        row.enabled = parameter
                            .get("required")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        params.push(row);
                    }
                    "header" => headers.push(KeyValue::new(name, example_text(doc, parameter))),
                    other => warnings.push(format!(
                        "{} {path}: {other} parameter {name} not imported",
                        method.to_uppercase()
                    )),
                }
            }
            let body = op
                .get("requestBody")
                .map_or(Body::None, |b| request_body(doc, b, &mut headers));
            let name = [text(op.get("summary")), text(op.get("operationId"))]
                .into_iter()
                .find(|n| !n.is_empty())
                .unwrap_or_else(|| format!("{} {path}", method.to_uppercase()));
            let mut request = Request::new(name, method.to_uppercase(), url);
            request.params = params;
            request.headers = headers;
            request.body = body;
            request.description = text(op.get("description"));
            if let Some(security) = op.get("security")
                && let Some(auth) = security_auth(doc, security, &mut vars, &mut warnings)
            {
                request.auth = auth;
            }
            let entry = Item::Request(Box::new(request));
            match array(op.get("tags")).first().map(|t| text(Some(t))) {
                Some(tag) if !tag.is_empty() => match folders.iter_mut().find(|f| f.name == tag) {
                    Some(folder) => folder.items.push(entry),
                    None => {
                        let mut folder = Folder::new(tag.clone());
                        folder.description = tag_docs.get(&tag).cloned().unwrap_or_default();
                        folder.items.push(entry);
                        folders.push(folder);
                    }
                },
                _ => root.push(entry),
            }
        }
    }

    collection
        .variables
        .push(Variable::new("baseUrl", base_url));
    for name in path_vars {
        collection.variables.push(Variable::new(name, ""));
    }
    for (name, secret) in vars {
        collection.variables.push(if secret {
            Variable::secret(name, "")
        } else {
            Variable::new(name, "")
        });
    }
    collection.items = folders.into_iter().map(Item::Folder).collect();
    collection.items.extend(root);
    Ok(Imported {
        collections: vec![collection],
        environments: Vec::new(),
        warnings,
    })
}

#[cfg(test)]
mod tests;
