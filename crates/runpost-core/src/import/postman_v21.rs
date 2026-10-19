//! Postman Collection v2.1 y entornos Postman → formato nativo (sección 11).

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use crate::model::{
    ApiKeyIn, Auth, Body, ClientAuth, Collection, Environment, Example, ExampleRequest, FieldKind,
    Folder, Item, KeyValue, MultipartField, OAuth2Cc, OAuth2Code, RawLang, Request,
    RequestSettings, SCHEMA, Scripts, Variable, new_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PostmanKind {
    Collection,
    Environment,
}

impl std::str::FromStr for PostmanKind {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        match text {
            "collection" => Ok(Self::Collection),
            "environment" => Ok(Self::Environment),
            other => bail!("unknown kind {other:?} (collection | environment)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedCollection {
    pub collection: Collection,
    /// Lo que no se pudo trasladar (auth no soportada, ejemplos de respuesta, …).
    pub warnings: Vec<String>,
}

pub fn detect(value: &Value) -> Option<PostmanKind> {
    if value.get("info").is_some() && value.get("item").is_some() {
        Some(PostmanKind::Collection)
    } else if value.get("values").is_some_and(Value::is_array) {
        Some(PostmanKind::Environment)
    } else {
        None
    }
}

pub fn import_collection(text: &str) -> Result<ImportedCollection> {
    import_collection_value(&serde_json::from_str(text).context("invalid JSON")?)
}

pub fn import_environment(text: &str) -> Result<Environment> {
    import_environment_value(&serde_json::from_str(text).context("invalid JSON")?)
}

/// `"1a2b3c4d-5e6f-…"` → `"col_1a2b3c4d5e6f…"`; si no es un UUID, id nuevo.
pub(crate) fn id_from(prefix: &str, postman_id: Option<&Value>) -> String {
    postman_id
        .and_then(Value::as_str)
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .map(|id| format!("{prefix}_{}", id.simple()))
        .unwrap_or_else(|| new_id(prefix))
}

fn text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// Postman admite `description` como texto o como `{content, type}`.
fn description(value: Option<&Value>) -> String {
    match value {
        Some(Value::Object(map)) => text(map.get("content")),
        other => text(other),
    }
}

fn enabled(entry: &Value) -> bool {
    !entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn array(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn key_values(value: Option<&Value>) -> Vec<KeyValue> {
    match value {
        // `header` también puede venir como texto `"K: v\nK2: v2"`.
        Some(Value::String(block)) => block
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| KeyValue::new(k.trim(), v.trim()))
            .collect(),
        other => array(other)
            .iter()
            .filter(|entry| entry.is_object())
            .map(|entry| KeyValue {
                key: text(entry.get("key")),
                value: text(entry.get("value")),
                enabled: enabled(entry),
            })
            .collect(),
    }
}

fn variables(value: Option<&Value>) -> Vec<Variable> {
    array(value)
        .iter()
        .filter(|entry| entry.get("key").is_some())
        .map(|entry| {
            let secret = entry.get("type").and_then(Value::as_str) == Some("secret");
            let mut var = if secret {
                Variable::secret(text(entry.get("key")), text(entry.get("value")))
            } else {
                Variable::new(text(entry.get("key")), text(entry.get("value")))
            };
            var.enabled = entry
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true)
                && enabled(entry);
            var
        })
        .collect()
}

/// Atributo de un auth Postman: `bearer: [{key: "token", value: "…"}]`.
fn auth_attr(auth: &Value, kind: &str, key: &str) -> String {
    match auth.get(kind) {
        Some(Value::Array(attrs)) => attrs
            .iter()
            .find(|a| a.get("key").and_then(Value::as_str) == Some(key))
            .map(|a| text(a.get("value")))
            .unwrap_or_default(),
        // Formato antiguo v2.0: `bearer: {token: "…"}`.
        Some(Value::Object(map)) => text(map.get(key)),
        _ => String::new(),
    }
}

/// `None` = no hay `auth` (hereda del padre).
fn auth(value: Option<&Value>, at: &str, warnings: &mut Vec<String>) -> Option<Auth> {
    let auth = value.filter(|v| v.is_object())?;
    let kind = auth.get("type").and_then(Value::as_str).unwrap_or("noauth");
    Some(match kind {
        "noauth" => Auth::None,
        "inherit" => Auth::Inherit,
        "bearer" => Auth::Bearer {
            token: auth_attr(auth, "bearer", "token"),
        },
        "basic" => Auth::Basic {
            username: auth_attr(auth, "basic", "username"),
            password: auth_attr(auth, "basic", "password"),
        },
        "apikey" => Auth::ApiKey {
            key: auth_attr(auth, "apikey", "key"),
            value: auth_attr(auth, "apikey", "value"),
            location: if auth_attr(auth, "apikey", "in") == "query" {
                ApiKeyIn::Query
            } else {
                ApiKeyIn::Header
            },
        },
        "digest" => Auth::Digest {
            username: auth_attr(auth, "digest", "username"),
            password: auth_attr(auth, "digest", "password"),
        },
        "oauth2" => {
            let attr = |key: &str| auth_attr(auth, "oauth2", key);
            let client_auth = if attr("client_authentication") == "body" {
                ClientAuth::Body
            } else {
                ClientAuth::Basic
            };
            match attr("grant_type").as_str() {
                "client_credentials" => Auth::oauth2_cc(OAuth2Cc {
                    token_url: attr("accessTokenUrl"),
                    client_id: attr("clientId"),
                    client_secret: attr("clientSecret"),
                    scope: attr("scope"),
                    client_auth,
                }),
                // Sin `grant_type`, Postman usa authorization code.
                "" | "authorization_code" | "authorization_code_with_pkce" => {
                    Auth::oauth2_code(OAuth2Code {
                        auth_url: attr("authUrl"),
                        token_url: attr("accessTokenUrl"),
                        client_id: attr("clientId"),
                        client_secret: attr("clientSecret"),
                        scope: attr("scope"),
                        redirect_uri: super::local_redirect(&attr("redirect_uri"), at, warnings),
                        client_auth,
                    })
                }
                grant => {
                    warnings.push(format!(
                        "{at}: OAuth 2.0 grant {grant:?} not supported (only client credentials and authorization code), imported as none"
                    ));
                    Auth::None
                }
            }
        }
        other => {
            warnings.push(format!(
                "{at}: auth {other:?} not supported yet, imported as none"
            ));
            Auth::None
        }
    })
}

fn scripts(events: Option<&Value>) -> Scripts {
    let mut scripts = Scripts::default();
    for event in array(events) {
        let exec = match event.get("script").and_then(|s| s.get("exec")) {
            Some(Value::Array(lines)) => lines
                .iter()
                .map(|l| text(Some(l)))
                .collect::<Vec<_>>()
                .join("\n"),
            other => text(other),
        };
        let target = match event.get("listen").and_then(Value::as_str) {
            Some("prerequest") => &mut scripts.pre,
            Some("test") => &mut scripts.test,
            _ => continue,
        };
        if !target.is_empty() {
            target.push('\n');
        }
        target.push_str(&exec);
    }
    scripts
}

/// URL como texto o como objeto `{raw, host, path, query}`. Con `query`, la URL se guarda
/// sin la query y los pares van a `params` (así conservan `disabled`).
fn url(value: Option<&Value>, at: &str, warnings: &mut Vec<String>) -> (String, Vec<KeyValue>) {
    let Some(Value::Object(map)) = value else {
        return (text(value), Vec::new());
    };
    if !array(map.get("variable")).is_empty() {
        warnings.push(format!(
            "{at}: path variables (:name) are kept literally in the URL"
        ));
    }
    let raw = match map.get("raw") {
        Some(raw) => text(Some(raw)),
        None => {
            let host = match map.get("host") {
                Some(Value::Array(parts)) => parts
                    .iter()
                    .map(|p| text(Some(p)))
                    .collect::<Vec<_>>()
                    .join("."),
                other => text(other),
            };
            let path = match map.get("path") {
                Some(Value::Array(parts)) => parts
                    .iter()
                    .map(|p| text(Some(p)))
                    .collect::<Vec<_>>()
                    .join("/"),
                other => text(other),
            };
            let protocol = text(map.get("protocol"));
            let mut raw = if protocol.is_empty() {
                host
            } else {
                format!("{protocol}://{host}")
            };
            if !path.is_empty() {
                raw.push('/');
                raw.push_str(path.trim_start_matches('/'));
            }
            raw
        }
    };
    let query = key_values(map.get("query"));
    if query.is_empty() {
        (raw, query)
    } else {
        let base = raw
            .split_once('?')
            .map_or(raw.as_str(), |(base, _)| base)
            .to_owned();
        (base, query)
    }
}

fn body(value: Option<&Value>) -> Body {
    let Some(body) = value.filter(|v| v.is_object()) else {
        return Body::None;
    };
    match body.get("mode").and_then(Value::as_str) {
        Some("raw") => Body::Raw {
            lang: match body
                .pointer("/options/raw/language")
                .and_then(Value::as_str)
                .unwrap_or("text")
            {
                "json" => RawLang::Json,
                "xml" => RawLang::Xml,
                "html" => RawLang::Html,
                "javascript" => RawLang::Javascript,
                _ => RawLang::Text,
            },
            text: text(body.get("raw")),
        },
        Some("urlencoded") => Body::Urlencoded {
            fields: key_values(body.get("urlencoded")),
        },
        Some("formdata") => Body::Multipart {
            fields: array(body.get("formdata"))
                .iter()
                .map(|field| {
                    let file = field.get("type").and_then(Value::as_str) == Some("file");
                    let src = match field.get("src") {
                        Some(Value::Array(list)) => text(list.first()),
                        other => text(other),
                    };
                    MultipartField {
                        key: text(field.get("key")),
                        value: if file {
                            String::new()
                        } else {
                            text(field.get("value"))
                        },
                        kind: if file {
                            FieldKind::File
                        } else {
                            FieldKind::Text
                        },
                        path: if file { src } else { String::new() },
                        enabled: enabled(field),
                    }
                })
                .collect(),
        },
        Some("file") => Body::Binary {
            path: text(body.pointer("/file/src")),
        },
        Some("graphql") => Body::Graphql {
            query: text(body.pointer("/graphql/query")),
            variables: text(body.pointer("/graphql/variables")),
        },
        _ => Body::None,
    }
}

/// `response` de Postman (ejemplos guardados) → `examples`.
fn examples(list: Option<&Value>) -> Vec<Example> {
    array(list)
        .iter()
        .filter(|response| response.is_object())
        .map(|response| {
            let code = response
                .get("code")
                .and_then(Value::as_u64)
                .map_or(0, |code| u16::try_from(code).unwrap_or(0));
            let name = text(response.get("name"));
            let mut example = Example::new(
                if name.is_empty() {
                    "Example".into()
                } else {
                    name
                },
                code,
            );
            example.id = id_from("exm", response.get("id"));
            example.status_text = text(response.get("status"));
            example.headers = key_values(response.get("header"));
            example.body = text(response.get("body"));
            if let Some(original) = response.get("originalRequest").filter(|o| o.is_object()) {
                example.request = ExampleRequest {
                    method: text(original.get("method")).to_uppercase(),
                    url: match original.get("url") {
                        Some(Value::Object(url)) => text(url.get("raw")),
                        other => text(other),
                    },
                    headers: key_values(original.get("header")),
                    body: text(original.pointer("/body/raw")),
                };
            }
            example
        })
        .collect()
}

fn request(item: &Value, at: &str, warnings: &mut Vec<String>) -> Request {
    let name = text(item.get("name"));
    // `request` puede ser solo la URL en texto.
    let req = match item.get("request") {
        Some(Value::String(url)) => &serde_json::json!({"method": "GET", "url": url}),
        Some(other) => other,
        None => &Value::Null,
    };
    let (url, params) = url(req.get("url"), at, warnings);
    let behavior = item.get("protocolProfileBehavior");
    Request {
        id: id_from("req", item.get("id")),
        name,
        method: req
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_uppercase(),
        url,
        params,
        headers: key_values(req.get("header")),
        auth: auth(req.get("auth"), at, warnings).unwrap_or(Auth::Inherit),
        body: body(req.get("body")),
        scripts: scripts(item.get("event")),
        settings: RequestSettings {
            follow_redirects: behavior
                .and_then(|b| b.get("followRedirects"))
                .and_then(Value::as_bool),
            verify_tls: behavior
                .and_then(|b| b.get("strictSSL"))
                .and_then(Value::as_bool),
            timeout_ms: None,
        },
        description: description(req.get("description")),
        examples: examples(item.get("response")),
    }
}

fn items(list: &[Value], parent: &str, warnings: &mut Vec<String>) -> Vec<Item> {
    list.iter()
        .filter(|item| item.is_object())
        .map(|item| {
            let at = format!("{parent}/{}", text(item.get("name")));
            if let Some(children) = item.get("item").and_then(Value::as_array) {
                Item::Folder(Folder {
                    id: id_from("fld", item.get("id")),
                    name: text(item.get("name")),
                    description: description(item.get("description")),
                    auth: auth(item.get("auth"), &at, warnings).filter(|a| *a != Auth::Inherit),
                    scripts: scripts(item.get("event")),
                    items: items(children, &at, warnings),
                })
            } else {
                Item::Request(Box::new(request(item, &at, warnings)))
            }
        })
        .collect()
}

pub fn import_collection_value(value: &Value) -> Result<ImportedCollection> {
    let info = value.get("info").context("missing info")?;
    if let Some(schema) = info.get("schema").and_then(Value::as_str)
        && !schema.contains("v2.1")
        && !schema.contains("v2.0")
    {
        bail!("unsupported Postman schema {schema}");
    }
    let name = text(info.get("name"));
    let mut warnings = Vec::new();
    let collection = Collection {
        schema: SCHEMA,
        id: id_from("col", info.get("_postman_id")),
        description: description(info.get("description")),
        auth: auth(value.get("auth"), &name, &mut warnings).filter(|a| *a != Auth::Inherit),
        agent_policy: None,
        variables: variables(value.get("variable")),
        scripts: scripts(value.get("event")),
        items: items(array(value.get("item")), &name, &mut warnings),
        name,
    };
    Ok(ImportedCollection {
        collection,
        warnings,
    })
}

pub fn import_environment_value(value: &Value) -> Result<Environment> {
    let values = value.get("values").context("missing values")?;
    Ok(Environment {
        schema: SCHEMA,
        id: id_from("env", value.get("id")),
        name: text(value.get("name")),
        variables: variables(Some(values)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_kind() {
        assert_eq!(
            detect(&json!({"info":{},"item":[]})),
            Some(PostmanKind::Collection)
        );
        assert_eq!(
            detect(&json!({"name":"e","values":[]})),
            Some(PostmanKind::Environment)
        );
        assert_eq!(detect(&json!({"x":1})), None);
        assert_eq!(
            "environment".parse::<PostmanKind>().unwrap(),
            PostmanKind::Environment
        );
        assert!("x".parse::<PostmanKind>().is_err());
    }

    #[test]
    fn maps_every_postman_shape() {
        let collection = json!({
          "info": {"_postman_id": "00000000-0000-4000-8000-0000000000c1", "name": "C",
                   "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
                   "description": {"content": "docs", "type": "text/markdown"}},
          "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{t}}", "type": "string"}]},
          "variable": [{"key": "base", "value": "http://x"}, {"key": "s", "value": "v", "type": "secret"},
                       {"key": "off", "value": "1", "disabled": true}],
          "event": [{"listen": "prerequest", "script": {"exec": ["a", "b"]}},
                    {"listen": "test", "script": {"exec": "t"}}],
          "item": [
            {"name": "F", "auth": {"type": "basic", "basic": [{"key":"username","value":"u"},{"key":"password","value":"p"}]},
             "item": [
               {"name": "q", "request": {"method": "get",
                  "url": {"raw": "{{base}}/a?x=1&y=2", "host": ["{{base}}"], "path": ["a"],
                          "query": [{"key": "x", "value": "1"}, {"key": "y", "value": "2", "disabled": true}]},
                  "header": "Accept: application/json\nX-A: 1",
                  "auth": {"type": "apikey", "apikey": [{"key":"key","value":"K"},{"key":"value","value":"V"},{"key":"in","value":"query"}]},
                  "body": {"mode": "urlencoded", "urlencoded": [{"key": "a", "value": "1"}]}},
                "protocolProfileBehavior": {"followRedirects": false, "strictSSL": false},
                "response": [{"name": "example", "code": 200, "status": "OK",
                  "header": [{"key": "Content-Type", "value": "application/json"}],
                  "body": "{\"ok\":true}",
                  "originalRequest": {"method": "get", "url": {"raw": "{{base}}/q?x=1"}, "header": []}}]},
               {"name": "form", "request": {"method": "POST", "url": "{{base}}/f",
                  "body": {"mode": "formdata", "formdata": [{"key": "t", "value": "v", "type": "text"},
                                                            {"key": "f", "type": "file", "src": ["C:/a.bin"]}]}}},
               {"name": "bin", "request": {"method": "PUT", "url": "{{base}}/b", "body": {"mode": "file", "file": {"src": "x.bin"}}}},
               {"name": "gql", "request": {"method": "POST", "url": "{{base}}/g",
                  "body": {"mode": "graphql", "graphql": {"query": "{a}", "variables": "{}"}}}},
               {"name": "raw", "request": {"method": "POST", "url": "{{base}}/r",
                  "body": {"mode": "raw", "raw": "<a/>", "options": {"raw": {"language": "xml"}}},
                  "auth": {"type": "hawk"}}},
               {"name": "short", "request": "http://short"},
               {"name": "built", "request": {"url": {"protocol": "https", "host": ["api", "x", "com"], "path": ["v1", "u"],
                                                      "variable": [{"key": "id"}]}}}
             ]}
          ]
        });
        let ImportedCollection {
            collection: c,
            warnings,
        } = import_collection_value(&collection).unwrap();
        assert_eq!(c.id, "col_000000000000400080000000000000c1");
        assert_eq!(c.description, "docs");
        assert_eq!(
            c.auth,
            Some(Auth::Bearer {
                token: "{{t}}".into()
            })
        );
        assert_eq!(c.variables.len(), 3);
        assert!(c.variables[1].secret && !c.variables[2].enabled);
        assert_eq!(
            c.scripts,
            Scripts {
                pre: "a\nb".into(),
                test: "t".into()
            }
        );
        let Item::Folder(f) = &c.items[0] else {
            panic!()
        };
        assert_eq!(
            f.auth,
            Some(Auth::Basic {
                username: "u".into(),
                password: "p".into()
            })
        );
        let reqs: Vec<&Request> = f
            .items
            .iter()
            .map(|i| match i {
                Item::Request(r) => r.as_ref(),
                Item::Folder(_) => panic!(),
            })
            .collect();
        let q = reqs[0];
        assert_eq!((q.method.as_str(), q.url.as_str()), ("GET", "{{base}}/a"));
        assert_eq!(q.params.len(), 2);
        assert!(!q.params[1].enabled);
        assert_eq!(
            q.headers,
            [
                KeyValue::new("Accept", "application/json"),
                KeyValue::new("X-A", "1")
            ]
        );
        assert_eq!(
            q.auth,
            Auth::ApiKey {
                key: "K".into(),
                value: "V".into(),
                location: ApiKeyIn::Query
            }
        );
        assert_eq!(q.settings.follow_redirects, Some(false));
        assert_eq!(q.settings.verify_tls, Some(false));
        assert!(matches!(&q.body, Body::Urlencoded { fields } if fields.len() == 1));
        let Body::Multipart { fields } = &reqs[1].body else {
            panic!()
        };
        assert_eq!(
            (fields[1].kind, fields[1].path.as_str()),
            (FieldKind::File, "C:/a.bin")
        );
        assert_eq!(reqs[1].auth, Auth::Inherit);
        assert_eq!(
            reqs[2].body,
            Body::Binary {
                path: "x.bin".into()
            }
        );
        assert_eq!(
            reqs[3].body,
            Body::Graphql {
                query: "{a}".into(),
                variables: "{}".into()
            }
        );
        assert_eq!(
            reqs[4].body,
            Body::Raw {
                lang: RawLang::Xml,
                text: "<a/>".into()
            }
        );
        assert_eq!(reqs[4].auth, Auth::None);
        assert_eq!(
            (reqs[5].method.as_str(), reqs[5].url.as_str()),
            ("GET", "http://short")
        );
        assert_eq!(reqs[6].url, "https://api.x.com/v1/u");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        // Los ejemplos (`response`) se importan.
        let example = &reqs[0].examples[0];
        assert_eq!(
            (
                example.name.as_str(),
                example.status,
                example.status_text.as_str()
            ),
            ("example", 200, "OK")
        );
        assert_eq!(example.body, "{\"ok\":true}");
        assert_eq!(example.headers[0].key, "Content-Type");
        assert_eq!(
            (
                example.request.method.as_str(),
                example.request.url.as_str()
            ),
            ("GET", "{{base}}/q?x=1")
        );
    }

    #[test]
    fn environment_secret_becomes_file_secret() {
        let env = import_environment(
            r#"{"id":"00000000-0000-4000-8000-0000000000e1","name":"Acme DESA","values":[
                {"key":"gatewayUrl","value":"http://x","type":"default","enabled":true},
                {"key":"accessToken","value":"fake","type":"secret","enabled":true},
                {"key":"off","value":"","type":"default","enabled":false}]}"#,
        )
        .unwrap();
        assert_eq!(env.id, "env_000000000000400080000000000000e1");
        assert_eq!(env.variables[1], Variable::secret("accessToken", "fake"));
        assert!(!env.variables[2].enabled);
    }

    #[test]
    fn rejects_other_schemas_and_bad_input() {
        assert!(import_collection(r#"{"info":{"schema":"https://schema.getpostman.com/json/collection/v1.0.0/"},"item":[]}"#).is_err());
        assert!(import_collection("not json").is_err());
        assert!(import_environment(r#"{"name":"x"}"#).is_err());
        let fresh =
            import_collection(r#"{"info":{"name":"n","_postman_id":"nope"},"item":[]}"#).unwrap();
        assert!(fresh.collection.id.starts_with("col_"));
    }
}
