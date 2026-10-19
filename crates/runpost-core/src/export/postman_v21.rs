//! Formato nativo → Postman Collection v2.1 / entorno Postman (sección 11).
//!
//! Los ids nativos `col_/fld_/req_/env_` + UUID se exportan como UUID de Postman para que
//! reimportar el archivo devuelva el mismo árbol con los mismos ids.

use serde_json::{Map, Value, json};

use crate::model::{
    ApiKeyIn, Auth, Body, ClientAuth, Collection, Environment, FieldKind, Item, KeyValue, RawLang,
    Request, Scripts, Variable,
};

pub const SCHEMA_URL: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// `"col_1a2b…"` → `"1a2b3c4d-5e6f-…"`; ids sin UUID dentro → UUID nuevo.
fn postman_id(native: &str) -> String {
    native
        .split_once('_')
        .and_then(|(_, rest)| uuid::Uuid::parse_str(rest).ok())
        .unwrap_or_else(uuid::Uuid::new_v4)
        .to_string()
}

fn attrs(pairs: &[(&str, &str)]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|(k, v)| json!({"key": k, "value": v, "type": "string"}))
            .collect(),
    )
}

/// `None` = no escribir `auth` (Postman hereda del padre).
fn auth(auth: &Auth) -> Option<Value> {
    Some(match auth {
        Auth::Inherit => return None,
        Auth::None => json!({"type": "noauth"}),
        Auth::Bearer { token } => json!({"type": "bearer", "bearer": attrs(&[("token", token)])}),
        Auth::Basic { username, password } => json!({"type": "basic",
            "basic": attrs(&[("username", username), ("password", password)])}),
        Auth::ApiKey {
            key,
            value,
            location,
        } => json!({"type": "apikey", "apikey": attrs(&[
            ("key", key), ("value", value),
            ("in", if *location == ApiKeyIn::Query { "query" } else { "header" }),
        ])}),
        Auth::Digest { username, password } => json!({"type": "digest",
            "digest": attrs(&[("username", username), ("password", password)])}),
        // Postman: "Send as Basic Auth header" (`header`) o "Send client credentials in body".
        Auth::OAuth2Cc(cc) => json!({"type": "oauth2", "oauth2": attrs(&[
            ("grant_type", "client_credentials"),
            ("accessTokenUrl", &cc.token_url), ("clientId", &cc.client_id),
            ("clientSecret", &cc.client_secret), ("scope", &cc.scope),
            ("client_authentication", client_authentication(cc.client_auth)),
            ("addTokenTo", "header"),
        ])}),
        Auth::OAuth2Code(code) => json!({"type": "oauth2", "oauth2": attrs(&[
            ("grant_type", "authorization_code_with_pkce"),
            ("authUrl", &code.auth_url), ("accessTokenUrl", &code.token_url),
            ("clientId", &code.client_id), ("clientSecret", &code.client_secret),
            ("scope", &code.scope), ("redirect_uri", &code.redirect_uri),
            ("challengeAlgorithm", "S256"),
            ("client_authentication", client_authentication(code.client_auth)),
            ("addTokenTo", "header"),
        ])}),
    })
}

fn client_authentication(client_auth: ClientAuth) -> &'static str {
    match client_auth {
        ClientAuth::Basic => "header",
        ClientAuth::Body => "body",
    }
}

fn key_values(list: &[KeyValue]) -> Value {
    Value::Array(
        list.iter()
            .map(|kv| {
                let mut entry = json!({"key": kv.key, "value": kv.value});
                if !kv.enabled {
                    entry["disabled"] = json!(true);
                }
                entry
            })
            .collect(),
    )
}

fn events(scripts: &Scripts) -> Option<Value> {
    let list: Vec<Value> = [("prerequest", &scripts.pre), ("test", &scripts.test)]
        .into_iter()
        .filter(|(_, code)| !code.is_empty())
        .map(|(listen, code)| {
            json!({"listen": listen, "script": {"type": "text/javascript",
                   "exec": code.split('\n').collect::<Vec<_>>()}})
        })
        .collect();
    (!list.is_empty()).then_some(Value::Array(list))
}

fn variables(list: &[Variable], strip_secrets: bool) -> Value {
    Value::Array(
        list.iter()
            .map(|var| {
                let value = if var.secret && strip_secrets {
                    ""
                } else {
                    var.value.as_deref().unwrap_or_default()
                };
                let mut entry = json!({"key": var.key, "value": value,
                    "type": if var.secret { "secret" } else { "default" }});
                if !var.enabled {
                    entry["disabled"] = json!(true);
                }
                entry
            })
            .collect(),
    )
}

/// URL como texto si no hay `params`; si los hay, objeto con `raw`, `host`, `path` y `query`.
fn url(request: &Request) -> Value {
    if request.params.is_empty() {
        return json!(request.url);
    }
    let enabled: Vec<String> = request
        .params
        .iter()
        .filter(|p| p.enabled)
        .map(|p| format!("{}={}", p.key, p.value))
        .collect();
    let mut raw = request.url.clone();
    if !enabled.is_empty() {
        raw.push(if raw.contains('?') { '&' } else { '?' });
        raw.push_str(&enabled.join("&"));
    }
    let mut map = Map::new();
    map.insert("raw".into(), json!(raw));
    let (protocol, rest) = match request.url.split_once("://") {
        Some((protocol, rest)) => (Some(protocol), rest),
        None => (None, request.url.as_str()),
    };
    if let Some(protocol) = protocol {
        map.insert("protocol".into(), json!(protocol));
    }
    let base = rest.split(['?', '#']).next().unwrap_or_default();
    let (host_port, path) = base.split_once('/').unwrap_or((base, ""));
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, Some(port))
        }
        _ => (host_port, None),
    };
    map.insert("host".into(), json!(host.split('.').collect::<Vec<_>>()));
    if let Some(port) = port {
        map.insert("port".into(), json!(port));
    }
    if !path.is_empty() {
        map.insert("path".into(), json!(path.split('/').collect::<Vec<_>>()));
    }
    // La query de la URL nativa, si la tuviera, va también en `query` para no perderla.
    let mut query: Vec<KeyValue> = rest
        .split_once('?')
        .map(|(_, q)| q.split('#').next().unwrap_or_default())
        .into_iter()
        .flat_map(|q| q.split('&'))
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            KeyValue::new(k, v)
        })
        .collect();
    query.extend(request.params.iter().cloned());
    map.insert("query".into(), key_values(&query));
    Value::Object(map)
}

fn body(body: &Body) -> Option<Value> {
    Some(match body {
        Body::None => return None,
        Body::Raw { lang, text } => {
            let language = match lang {
                RawLang::Json => "json",
                RawLang::Text => "text",
                RawLang::Xml => "xml",
                RawLang::Html => "html",
                RawLang::Javascript => "javascript",
            };
            json!({"mode": "raw", "raw": text, "options": {"raw": {"language": language}}})
        }
        Body::Urlencoded { fields } => {
            json!({"mode": "urlencoded", "urlencoded": key_values(fields)})
        }
        Body::Multipart { fields } => {
            json!({"mode": "formdata", "formdata": fields.iter().map(|f| {
            let mut entry = match f.kind {
                FieldKind::Text => json!({"key": f.key, "value": f.value, "type": "text"}),
                FieldKind::File => json!({"key": f.key, "type": "file", "src": f.path}),
            };
            if !f.enabled {
                entry["disabled"] = json!(true);
            }
            entry
        }).collect::<Vec<_>>()})
        }
        Body::Binary { path } => json!({"mode": "file", "file": {"src": path}}),
        Body::Graphql { query, variables } => {
            json!({"mode": "graphql", "graphql": {"query": query, "variables": variables}})
        }
    })
}

fn request(request: &Request) -> Value {
    let mut req = Map::new();
    if let Some(auth) = auth(&request.auth) {
        req.insert("auth".into(), auth);
    }
    req.insert("method".into(), json!(request.method));
    req.insert("header".into(), key_values(&request.headers));
    req.insert("url".into(), url(request));
    if !request.description.is_empty() {
        req.insert("description".into(), json!(request.description));
    }
    if let Some(body) = body(&request.body) {
        req.insert("body".into(), body);
    }
    let mut item = Map::new();
    item.insert("id".into(), json!(postman_id(&request.id)));
    item.insert("name".into(), json!(request.name));
    item.insert("request".into(), Value::Object(req));
    if let Some(events) = events(&request.scripts) {
        item.insert("event".into(), events);
    }
    if !request.examples.is_empty() {
        item.insert(
            "response".into(),
            request
                .examples
                .iter()
                .map(|example| {
                    let mut original = json!({
                        "method": example.request.method,
                        "header": key_values(&example.request.headers),
                        "url": {"raw": example.request.url},
                    });
                    if !example.request.body.is_empty() {
                        original["body"] = json!({"mode": "raw", "raw": example.request.body});
                    }
                    json!({
                        "id": postman_id(&example.id),
                        "name": example.name,
                        "originalRequest": original,
                        "status": example.status_text,
                        "code": example.status,
                        "header": key_values(&example.headers),
                        "cookie": [],
                        "body": example.body,
                    })
                })
                .collect(),
        );
    }
    let mut behavior = Map::new();
    if let Some(follow) = request.settings.follow_redirects {
        behavior.insert("followRedirects".into(), json!(follow));
    }
    if let Some(verify) = request.settings.verify_tls {
        behavior.insert("strictSSL".into(), json!(verify));
    }
    if !behavior.is_empty() {
        item.insert("protocolProfileBehavior".into(), Value::Object(behavior));
    }
    Value::Object(item)
}

fn items(list: &[Item]) -> Value {
    Value::Array(
        list.iter()
            .map(|item| match item {
                Item::Request(r) => request(r),
                Item::Folder(folder) => {
                    let mut map = Map::new();
                    map.insert("id".into(), json!(postman_id(&folder.id)));
                    map.insert("name".into(), json!(folder.name));
                    if !folder.description.is_empty() {
                        map.insert("description".into(), json!(folder.description));
                    }
                    if let Some(auth) = folder.auth.as_ref().and_then(auth) {
                        map.insert("auth".into(), auth);
                    }
                    if let Some(events) = events(&folder.scripts) {
                        map.insert("event".into(), events);
                    }
                    map.insert("item".into(), items(&folder.items));
                    Value::Object(map)
                }
            })
            .collect(),
    )
}

/// Colección Postman v2.1. `strip_secrets` vacía los valores de las variables secretas.
pub fn export_collection(collection: &Collection, strip_secrets: bool) -> Value {
    let mut info = Map::new();
    info.insert("_postman_id".into(), json!(postman_id(&collection.id)));
    info.insert("name".into(), json!(collection.name));
    if !collection.description.is_empty() {
        info.insert("description".into(), json!(collection.description));
    }
    info.insert("schema".into(), json!(SCHEMA_URL));
    let mut out = Map::new();
    out.insert("info".into(), Value::Object(info));
    out.insert("item".into(), items(&collection.items));
    if let Some(auth) = collection.auth.as_ref().and_then(auth) {
        out.insert("auth".into(), auth);
    }
    if let Some(events) = events(&collection.scripts) {
        out.insert("event".into(), events);
    }
    if !collection.variables.is_empty() {
        out.insert(
            "variable".into(),
            variables(&collection.variables, strip_secrets),
        );
    }
    Value::Object(out)
}

/// Entorno Postman. `strip_secrets` vacía los secretos (los del llavero nunca tienen valor aquí).
pub fn export_environment(env: &Environment, strip_secrets: bool) -> Value {
    let values: Vec<Value> = env
        .variables
        .iter()
        .map(|var| {
            let value = if var.secret && strip_secrets {
                ""
            } else {
                var.value.as_deref().unwrap_or_default()
            };
            json!({"key": var.key, "value": value,
                   "type": if var.secret { "secret" } else { "default" }, "enabled": var.enabled})
        })
        .collect();
    json!({"id": postman_id(&env.id), "name": env.name, "values": values,
           "_postman_variable_scope": "environment"})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::postman_v21::{import_collection_value, import_environment_value};
    use crate::model::{Folder, MultipartField, OAuth2Cc, OAuth2Code, Variable};

    fn rich() -> Collection {
        let mut c = Collection::new("Rich");
        c.description = "d".into();
        c.auth = Some(Auth::Bearer {
            token: "{{t}}".into(),
        });
        c.scripts = Scripts {
            pre: "a\nb".into(),
            test: "c".into(),
        };
        c.variables = vec![
            Variable::new("base", "http://h:8080"),
            Variable::secret("s", "v"),
        ];
        c.variables[0].enabled = false;
        let mut folder = Folder::new("F");
        folder.auth = Some(Auth::None);
        folder.description = "fd".into();
        let mut q = Request::new("q", "GET", "https://{{host}}:8443/a/b?inline=1");
        q.params = vec![
            KeyValue::new("x", "1"),
            KeyValue {
                enabled: false,
                ..KeyValue::new("y", "2")
            },
        ];
        q.headers = vec![KeyValue {
            enabled: false,
            ..KeyValue::new("H", "v")
        }];
        q.auth = Auth::ApiKey {
            key: "k".into(),
            value: "v".into(),
            location: ApiKeyIn::Query,
        };
        q.settings.follow_redirects = Some(false);
        q.settings.verify_tls = Some(true);
        q.description = "rd".into();
        let mut p = Request::new("p", "POST", "{{base}}/p");
        p.auth = Auth::Basic {
            username: "u".into(),
            password: "pw".into(),
        };
        p.body = Body::Multipart {
            fields: vec![
                MultipartField {
                    key: "t".into(),
                    value: "v".into(),
                    kind: FieldKind::Text,
                    path: String::new(),
                    enabled: false,
                },
                MultipartField {
                    key: "f".into(),
                    value: String::new(),
                    kind: FieldKind::File,
                    path: "a.bin".into(),
                    enabled: true,
                },
            ],
        };
        let mut g = Request::new("g", "POST", "{{base}}/g");
        g.body = Body::Graphql {
            query: "{a}".into(),
            variables: "{}".into(),
        };
        let mut u = Request::new("u", "POST", "{{base}}/u");
        u.body = Body::Urlencoded {
            fields: vec![KeyValue::new("a", "1")],
        };
        let mut b = Request::new("b", "PUT", "{{base}}/b");
        b.body = Body::Binary {
            path: "x.bin".into(),
        };
        b.auth = Auth::None;
        let mut r = Request::new("r", "POST", "{{base}}/r");
        r.body = Body::Raw {
            lang: RawLang::Javascript,
            text: "x()".into(),
        };
        for req in [q, p, g, u, b, r] {
            folder.items.push(Item::Request(Box::new(req)));
        }
        c.items.push(Item::Folder(folder));
        c
    }

    #[test]
    fn round_trip_keeps_tree_ids_and_scripts() {
        let original = rich();
        let exported = export_collection(&original, false);
        assert_eq!(exported["info"]["schema"], SCHEMA_URL);
        let back = import_collection_value(&exported).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        let mut expected = original.clone();
        // La query embebida en la URL se reimporta como params (equivalente).
        let Item::Folder(folder) = &mut expected.items[0] else {
            panic!()
        };
        let Item::Request(q) = &mut folder.items[0] else {
            panic!()
        };
        q.url = "https://{{host}}:8443/a/b".into();
        q.params.insert(0, KeyValue::new("inline", "1"));
        assert_eq!(back.collection, expected);
        let q = &exported["item"][0]["item"][0]["request"]["url"];
        assert_eq!(q["raw"], "https://{{host}}:8443/a/b?inline=1&x=1");
        assert_eq!(q["port"], "8443");
        assert_eq!(q["host"], json!(["{{host}}"]));
        assert_eq!(q["path"], json!(["a", "b"]));
    }

    #[test]
    fn strip_secrets_empties_secret_values_only() {
        let exported = export_collection(&rich(), true);
        assert_eq!(exported["variable"][1]["value"], "");
        assert_eq!(exported["variable"][1]["type"], "secret");
        assert_eq!(exported["variable"][0]["value"], "http://h:8080");
        let mut env = Environment::new("E");
        env.variables = vec![Variable::new("u", "x"), Variable::secret("t", "real")];
        let mut keyring = Variable::secret("k", "");
        keyring.value = None;
        env.variables.push(keyring);
        let stripped = export_environment(&env, true);
        assert!(!stripped.to_string().contains("real"));
        let kept = export_environment(&env, false);
        assert_eq!(kept["values"][1]["value"], "real");
        assert_eq!(kept["values"][2]["value"], "");
        let back = import_environment_value(&kept).unwrap();
        assert_eq!(
            (back.id.as_str(), back.variables[1].clone()),
            (env.id.as_str(), Variable::secret("t", "real"))
        );
    }

    #[test]
    fn foreign_ids_get_fresh_uuids() {
        assert!(uuid::Uuid::parse_str(&postman_id("adhoc")).is_ok());
        assert_eq!(
            postman_id("col_000000000000400080000000000000c1"),
            "00000000-0000-4000-8000-0000000000c1"
        );
    }

    #[test]
    fn digest_and_oauth2_round_trip_through_postman() {
        let mut c = Collection::new("Auth");
        let auths = [
            Auth::Digest {
                username: "acme".into(),
                password: "{{digestPass}}".into(),
            },
            Auth::oauth2_cc(OAuth2Cc {
                token_url: "https://id.example.test/oauth/token".into(),
                client_id: "acme-client".into(),
                client_secret: "{{clientSecret}}".into(),
                scope: "accounts_read".into(),
                client_auth: ClientAuth::Body,
            }),
            Auth::oauth2_code(OAuth2Code {
                auth_url: "https://id.example.test/authorize".into(),
                token_url: "https://id.example.test/oauth/token".into(),
                client_id: "acme-app".into(),
                client_secret: String::new(),
                scope: "openid".into(),
                redirect_uri: "http://127.0.0.1:9123/cb".into(),
                client_auth: ClientAuth::Basic,
            }),
        ];
        for (i, auth) in auths.iter().enumerate() {
            let mut request = Request::new(format!("r{i}"), "GET", "https://api.example.test");
            request.auth = auth.clone();
            c.items.push(Item::Request(Box::new(request)));
        }
        let exported = export_collection(&c, false);
        let oauth = &exported["item"][1]["request"]["auth"];
        assert_eq!(oauth["type"], "oauth2");
        let attr = |auth: &Value, key: &str| {
            auth["oauth2"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["key"] == key)
                .map(|a| a["value"].clone())
        };
        assert_eq!(attr(oauth, "grant_type").unwrap(), "client_credentials");
        assert_eq!(attr(oauth, "client_authentication").unwrap(), "body");
        let code = &exported["item"][2]["request"]["auth"];
        assert_eq!(
            attr(code, "grant_type").unwrap(),
            "authorization_code_with_pkce"
        );
        assert_eq!(exported["item"][0]["request"]["auth"]["type"], "digest");

        let back = import_collection_value(&exported).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        let imported: Vec<Auth> = back
            .collection
            .items
            .iter()
            .map(|item| match item {
                Item::Request(request) => request.auth.clone(),
                Item::Folder(_) => panic!("request"),
            })
            .collect();
        assert_eq!(imported, auths);

        // El callback de Postman en la nube no sirve aquí: se cambia por el local y se avisa.
        let postman = json!({"info": {"name": "P", "schema": SCHEMA_URL}, "item": [
            {"name": "login", "request": {"method": "GET", "url": "https://api.example.test",
             "auth": {"type": "oauth2", "oauth2": [
                {"key": "redirect_uri", "value": "https://oauth.pstmn.io/v1/callback"},
                {"key": "authUrl", "value": "https://id.example.test/authorize"},
                {"key": "useBrowser", "value": true}]}}},
            {"name": "implicit", "request": {"method": "GET", "url": "https://api.example.test",
             "auth": {"type": "oauth2", "oauth2": [{"key": "grant_type", "value": "implicit"}]}}}]});
        let back = import_collection_value(&postman).unwrap();
        let Item::Request(login) = &back.collection.items[0] else {
            panic!("request")
        };
        let Auth::OAuth2Code(code) = &login.auth else {
            panic!("oauth2_code")
        };
        assert_eq!(code.redirect_uri, crate::model::OAUTH_REDIRECT_URI);
        assert_eq!(code.auth_url, "https://id.example.test/authorize");
        assert!(
            back.warnings[0].contains("register it with the provider"),
            "{:?}",
            back.warnings
        );
        assert!(back.warnings[1].contains("grant \"implicit\" not supported"));
    }

    #[test]
    fn examples_round_trip_through_postman() {
        let mut c = Collection::new("Examples");
        let mut request = Request::new("ping", "GET", "https://api.example.test/v1/ping");
        let mut example = crate::model::Example::new("Pong", 200);
        example.status_text = "OK".into();
        example.headers = vec![KeyValue::new("Content-Type", "application/json")];
        example.body = "{\"pong\":true}".into();
        example.request = crate::model::ExampleRequest {
            method: "GET".into(),
            url: "https://api.example.test/v1/ping".into(),
            headers: vec![KeyValue::new("Accept", "application/json")],
            body: String::new(),
        };
        request.examples.push(example);
        c.items.push(Item::Request(Box::new(request)));
        let exported = export_collection(&c, false);
        let response = &exported["item"][0]["response"][0];
        assert_eq!(response["name"], "Pong");
        assert_eq!(response["code"], 200);
        assert_eq!(response["status"], "OK");
        assert_eq!(
            response["originalRequest"]["url"]["raw"],
            "https://api.example.test/v1/ping"
        );
        let back = import_collection_value(&exported).unwrap();
        assert_eq!(back.collection, c);
    }
}
