//! HAR 1.2 (capturas de las herramientas de desarrollo del navegador) → colección (fase 4).
//!
//! Una petición por entrada, en carpetas por host si hay más de uno. Si la captura distingue el tipo
//! de recurso, solo entran las llamadas XHR/fetch. No se copian las cookies ni las cabeceras que pone
//! el cliente (pseudo-cabeceras de HTTP/2, `Host`, `Content-Length`, `Connection`).

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::model::{
    Body, Collection, FieldKind, Folder, Item, KeyValue, MultipartField, RawLang, Request,
};

use super::{Imported, array, split_query, text};

pub fn detect(value: &Value) -> bool {
    value
        .get("log")
        .is_some_and(|log| log.get("entries").is_some_and(Value::is_array))
}

const SKIP: &[&str] = &["host", "content-length", "connection", "cookie"];

pub fn import(value: &Value) -> Result<Imported> {
    let log = value.get("log").context("missing log")?;
    let entries = array(log.get("entries"));
    let typed = |entry: &&Value| {
        entry
            .get("_resourceType")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
    };
    let api_only = entries
        .iter()
        .any(|e| matches!(typed(&e).as_deref(), Some("xhr" | "fetch")));
    let mut warnings = Vec::new();
    let mut skipped = 0;
    let mut cookies = 0;
    let mut credentials = 0;
    let mut by_host: Vec<(String, Vec<Request>)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        if api_only && !matches!(typed(&entry).as_deref(), Some("xhr" | "fetch")) {
            skipped += 1;
            continue;
        }
        let Some(req) = entry.get("request") else {
            continue;
        };
        let method = text(req.get("method")).to_uppercase();
        let full = text(req.get("url"));
        let (url, params) = split_query(&full);
        let parsed = url::Url::parse(&url).ok();
        let host = parsed
            .as_ref()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let path = parsed
            .as_ref()
            .map(|u| u.path().to_owned())
            .unwrap_or_else(|| url.clone());

        let mut headers = Vec::new();
        for header in array(req.get("headers")) {
            let name = text(header.get("name"));
            let lower = name.to_ascii_lowercase();
            if name.starts_with(':') || SKIP.contains(&lower.as_str()) {
                if lower == "cookie" {
                    cookies += 1;
                }
                continue;
            }
            if lower == "authorization" {
                credentials += 1;
            }
            headers.push(KeyValue::new(name, text(header.get("value"))));
        }

        let body = match req.get("postData") {
            Some(data) => {
                let mime = text(data.get("mimeType")).to_ascii_lowercase();
                let fields = array(data.get("params"));
                if mime.contains("multipart/form-data") {
                    headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
                    Body::Multipart {
                        fields: fields
                            .iter()
                            .map(|f| {
                                let file = f.get("fileName").is_some_and(|n| !n.is_null());
                                MultipartField {
                                    key: text(f.get("name")),
                                    value: if file {
                                        String::new()
                                    } else {
                                        text(f.get("value"))
                                    },
                                    kind: if file {
                                        FieldKind::File
                                    } else {
                                        FieldKind::Text
                                    },
                                    path: if file {
                                        text(f.get("fileName"))
                                    } else {
                                        String::new()
                                    },
                                    enabled: true,
                                }
                            })
                            .collect(),
                    }
                } else if mime.contains("x-www-form-urlencoded") && !fields.is_empty() {
                    Body::Urlencoded {
                        fields: fields
                            .iter()
                            .map(|f| KeyValue::new(text(f.get("name")), text(f.get("value"))))
                            .collect(),
                    }
                } else {
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
                        text: text(data.get("text")),
                    }
                }
            }
            None => Body::None,
        };

        let base = format!("{method} {path}");
        let count = seen.entry(base.clone()).or_default();
        *count += 1;
        let name = if *count > 1 {
            format!("{base} #{count}")
        } else {
            base
        };
        let mut request = Request::new(name, method, url);
        request.params = params;
        request.headers = headers;
        request.body = body;
        match by_host.iter_mut().find(|(h, _)| *h == host) {
            Some((_, list)) => list.push(request),
            None => by_host.push((host, vec![request])),
        }
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} entries skipped: only XHR/fetch calls are imported"
        ));
    }
    if cookies > 0 {
        warnings.push(format!("{cookies} Cookie headers not imported"));
    }
    if credentials > 0 {
        warnings.push(format!(
            "{credentials} requests keep their Authorization header: move it to a secret variable before sharing the collection"
        ));
    }

    let title = array(log.get("pages"))
        .first()
        .map(|page| text(page.get("title")))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "HAR import".into());
    let mut collection = Collection::new(title);
    collection.items = if by_host.len() > 1 {
        by_host
            .into_iter()
            .map(|(host, requests)| {
                let mut folder = Folder::new(if host.is_empty() { "?" } else { &host });
                folder.items = requests
                    .into_iter()
                    .map(|r| Item::Request(Box::new(r)))
                    .collect();
                Item::Folder(folder)
            })
            .collect()
    } else {
        by_host
            .into_iter()
            .flat_map(|(_, requests)| requests)
            .map(|r| Item::Request(Box::new(r)))
            .collect()
    };
    Ok(Imported {
        collections: vec![collection],
        environments: Vec::new(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(kind: &str, method: &str, url: &str, post: Option<Value>) -> Value {
        let mut request = json!({"method": method, "url": url, "httpVersion": "HTTP/2", "headers": [
            {"name": ":authority", "value": "api.test"},
            {"name": "Accept", "value": "application/json"},
            {"name": "Cookie", "value": "session=abc"},
            {"name": "Authorization", "value": "Bearer abc"}
        ], "queryString": []});
        if let Some(post) = post {
            request["postData"] = post;
        }
        json!({"_resourceType": kind, "request": request, "response": {"status": 200}})
    }

    #[test]
    fn imports_api_calls_grouped_by_host_without_cookies() {
        let har = json!({"log": {"version": "1.2", "pages": [{"title": "Shop"}], "entries": [
            entry("document", "GET", "https://shop.test/", None),
            entry("xhr", "GET", "https://api.test/v1/items?page=2", None),
            entry("fetch", "POST", "https://api.test/v1/items",
                Some(json!({"mimeType": "application/json", "text": "{\"a\":1}"}))),
            entry("fetch", "POST", "https://auth.test/token", Some(json!({
                "mimeType": "application/x-www-form-urlencoded", "text": "grant=x",
                "params": [{"name": "grant", "value": "x"}]}))),
            entry("xhr", "GET", "https://api.test/v1/items?page=3", None),
        ]}});
        assert!(detect(&har));
        let imported = import(&har).unwrap();
        let collection = &imported.collections[0];
        assert_eq!(collection.name, "Shop");
        assert_eq!(collection.items.len(), 2, "two hosts → two folders");
        let Item::Folder(api) = &collection.items[0] else {
            panic!()
        };
        assert_eq!(api.name, "api.test");
        let names: Vec<_> = api.items.iter().map(Item::name).collect();
        assert_eq!(
            names,
            ["GET /v1/items", "POST /v1/items", "GET /v1/items #2"]
        );
        let Item::Request(first) = &api.items[0] else {
            panic!()
        };
        assert_eq!(first.url, "https://api.test/v1/items");
        assert_eq!(first.params, vec![KeyValue::new("page", "2")]);
        let header_names: Vec<_> = first.headers.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(header_names, ["Accept", "Authorization"]);
        let Item::Folder(auth) = &collection.items[1] else {
            panic!()
        };
        let Item::Request(token) = &auth.items[0] else {
            panic!()
        };
        assert_eq!(
            token.body,
            Body::Urlencoded {
                fields: vec![KeyValue::new("grant", "x")]
            }
        );
        assert!(
            imported
                .warnings
                .iter()
                .any(|w| w.starts_with("1 entries skipped"))
        );
        assert!(imported.warnings.iter().any(|w| w.contains("Cookie")));
        assert!(
            imported
                .warnings
                .iter()
                .any(|w| w.contains("Authorization"))
        );
    }
}
