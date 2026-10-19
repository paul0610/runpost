//! Motor HTTP: de `Request` (nativo) a petición real con `reqwest`, y de vuelta a `Response`.

pub mod auth;
pub mod digest;
pub mod oauth;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use bytes::Bytes;
use reqwest::{Method, cookie::Jar, redirect};
use serde::Serialize;

use crate::{
    error::CoreError,
    model::{Auth, Body, FieldKind, HttpSettings, RawLang, Request},
    vars::{Masker, Resolver},
};

pub type Headers = Vec<(String, String)>;

/// Petición lista para enviar. Contiene valores reales (secretos incluidos): no se serializa.
#[derive(Clone)]
pub struct Prepared {
    pub method: Method,
    pub url: url::Url,
    pub headers: Headers,
    pub body: PreparedBody,
    pub timeout: Duration,
    pub client: ClientKey,
    /// Auth digest: credenciales para contestar al reto 401 (segundo intento de `send`).
    pub digest: Option<digest::Credentials>,
}

#[derive(Clone)]
pub enum PreparedBody {
    None,
    Bytes(Vec<u8>),
    Multipart(Vec<PreparedPart>),
}

#[derive(Clone)]
pub struct PreparedPart {
    pub name: String,
    pub value: Vec<u8>,
    pub file_name: Option<String>,
}

/// Opciones que exigen un `reqwest::Client` distinto.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientKey {
    pub verify_tls: bool,
    pub follow_redirects: bool,
    pub max_redirects: usize,
    pub proxy: Option<String>,
}

/// Lo que se envió, con secretos enmascarados: apto para UI, historial y MCP.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ResolvedRequest {
    pub method: String,
    pub url: String,
    pub headers: Headers,
    /// Texto del cuerpo (o un resumen para multipart/binario).
    pub body: Option<String>,
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    pub verify_tls: bool,
    /// Variables que quedaron como `{{x}}`.
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResponseCookie {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: bool,
    pub http_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub headers: Headers,
    #[serde(serialize_with = "lossy_text")]
    pub body: Bytes,
    pub ms: u64,
    pub size: u64,
    pub cookies: Vec<ResponseCookie>,
    /// Cabecera `Authorization` del segundo intento digest (con el `response`), si lo hubo.
    #[serde(skip)]
    pub digest_authorization: Option<String>,
}

fn lossy_text<S: serde::Serializer>(body: &Bytes, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&String::from_utf8_lossy(body))
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }
}

fn has_header(headers: &Headers, name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

fn set_header(headers: &mut Headers, name: &str, value: String) {
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    headers.push((name.to_owned(), value));
}

/// Construye la petición real y su versión enmascarada.
/// `auth` debe ser la efectiva (`auth::effective_auth`); `Inherit` aquí equivale a `none`.
pub fn build_request(
    request: &Request,
    auth: &Auth,
    resolver: &Resolver,
    http: &HttpSettings,
) -> Result<(Prepared, ResolvedRequest, Masker)> {
    let mut masker = resolver.masker().clone();
    let mut unresolved: Vec<String> = Vec::new();
    let resolve = |text: &str, unresolved: &mut Vec<String>| {
        let resolved = resolver.resolve(text);
        for name in resolved.unresolved {
            if !unresolved.contains(&name) {
                unresolved.push(name);
            }
        }
        resolved.text
    };

    let method_text = request.method.trim().to_uppercase();
    let method = Method::from_bytes(method_text.as_bytes())
        .map_err(|_| CoreError::InvalidRequest(format!("invalid method {method_text:?}")))?;

    let mut url_text = resolve(request.url.trim(), &mut unresolved);
    if !url_text.contains("://") {
        // Igual que Postman: sin esquema se asume http.
        url_text.insert_str(0, "http://");
    }
    let hint = |unresolved: &[String]| {
        if unresolved.is_empty() {
            String::new()
        } else {
            format!(" (unresolved: {})", unresolved.join(", "))
        }
    };
    let mut url = url::Url::parse(&url_text).map_err(|error| {
        CoreError::InvalidRequest(format!(
            "invalid url {}: {error}{}",
            masker.mask(&url_text),
            hint(&unresolved)
        ))
    })?;
    // `{` es válido en un host para el estándar URL, pero aquí siempre es una variable sin valor.
    if url.host_str().is_some_and(|host| host.contains('{')) {
        return Err(CoreError::InvalidRequest(format!(
            "invalid url {}: host has unresolved variables{}",
            masker.mask(&url_text),
            hint(&unresolved)
        ))
        .into());
    }
    let params: Vec<(String, String)> = request
        .params
        .iter()
        .filter(|p| p.enabled && !p.key.is_empty())
        .map(|p| {
            (
                resolve(&p.key, &mut unresolved),
                resolve(&p.value, &mut unresolved),
            )
        })
        .collect();
    if !params.is_empty() {
        // Como Postman: los valores van tal cual se escriben (un `%3D` ya codificado no se vuelve
        // a codificar); `url` solo escapa lo que no puede ir en una query (espacio, comillas, `#`…).
        let added: Vec<String> = params
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    k.clone()
                } else {
                    format!("{k}={v}")
                }
            })
            .collect();
        let query = match url.query().filter(|q| !q.is_empty()) {
            Some(existing) => format!("{existing}&{}", added.join("&")),
            None => added.join("&"),
        };
        url.set_query(Some(&query));
    }

    let mut headers: Headers = request
        .headers
        .iter()
        .filter(|h| h.enabled && !h.key.trim().is_empty())
        .map(|h| {
            (
                resolve(h.key.trim(), &mut unresolved),
                resolve(&h.value, &mut unresolved),
            )
        })
        .collect();

    // La auth del panel Auth gana a una cabecera manual con el mismo nombre.
    let applied = auth::apply(auth, resolver, &mut masker, &mut unresolved);
    let digest = match auth {
        Auth::Digest { username, password } => Some(digest::Credentials {
            username: resolve(username, &mut unresolved),
            password: resolve(password, &mut unresolved),
        }),
        _ => None,
    };
    if let Some((name, value)) = applied.header {
        set_header(&mut headers, &name, value);
    }
    if let Some((name, value)) = applied.query {
        url.query_pairs_mut().append_pair(&name, &value);
    }

    let default_type = |headers: &mut Headers, value: &str| {
        if !has_header(headers, "content-type") {
            headers.push(("Content-Type".into(), value.into()));
        }
    };
    let (body, preview) = match &request.body {
        Body::None => (PreparedBody::None, None),
        Body::Raw { lang, text } => {
            let text = resolve(text, &mut unresolved);
            let content_type = match lang {
                RawLang::Json => "application/json",
                RawLang::Text => "text/plain",
                RawLang::Xml => "application/xml",
                RawLang::Html => "text/html",
                RawLang::Javascript => "application/javascript",
            };
            default_type(&mut headers, content_type);
            (PreparedBody::Bytes(text.clone().into_bytes()), Some(text))
        }
        Body::Urlencoded { fields } => {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            for field in fields.iter().filter(|f| f.enabled && !f.key.is_empty()) {
                form.append_pair(
                    &resolve(&field.key, &mut unresolved),
                    &resolve(&field.value, &mut unresolved),
                );
            }
            let text = form.finish();
            default_type(&mut headers, "application/x-www-form-urlencoded");
            (PreparedBody::Bytes(text.clone().into_bytes()), Some(text))
        }
        Body::Multipart { fields } => {
            // reqwest pone el Content-Type con su boundary.
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
            let mut parts = Vec::new();
            let mut summary = Vec::new();
            for field in fields.iter().filter(|f| f.enabled && !f.key.is_empty()) {
                let name = resolve(&field.key, &mut unresolved);
                match field.kind {
                    FieldKind::Text => {
                        let value = resolve(&field.value, &mut unresolved);
                        summary.push(format!("{name}={value}"));
                        parts.push(PreparedPart {
                            name,
                            value: value.into_bytes(),
                            file_name: None,
                        });
                    }
                    FieldKind::File => {
                        let path = resolve(&field.path, &mut unresolved);
                        let value = std::fs::read(&path).map_err(|e| {
                            CoreError::InvalidRequest(format!("cannot read file {path}: {e}"))
                        })?;
                        let file_name = std::path::Path::new(&path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned());
                        summary.push(format!("{name}=@{path}"));
                        parts.push(PreparedPart {
                            name,
                            value,
                            file_name,
                        });
                    }
                }
            }
            (
                PreparedBody::Multipart(parts),
                Some(format!("multipart: {}", summary.join(", "))),
            )
        }
        Body::Binary { path } => {
            let path = resolve(path, &mut unresolved);
            let bytes = std::fs::read(&path)
                .map_err(|e| CoreError::InvalidRequest(format!("cannot read file {path}: {e}")))?;
            default_type(&mut headers, "application/octet-stream");
            let preview = format!("<binary {path}, {} bytes>", bytes.len());
            (PreparedBody::Bytes(bytes), Some(preview))
        }
        Body::Graphql { query, variables } => {
            let query = resolve(query, &mut unresolved);
            let variables = resolve(variables, &mut unresolved);
            let variables: serde_json::Value = if variables.trim().is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_str(&variables).map_err(|e| {
                    CoreError::InvalidRequest(format!("graphql variables are not JSON: {e}"))
                })?
            };
            let text = serde_json::json!({"query": query, "variables": variables}).to_string();
            default_type(&mut headers, "application/json");
            (PreparedBody::Bytes(text.clone().into_bytes()), Some(text))
        }
    };
    if !has_header(&headers, "user-agent") && !http.user_agent.is_empty() {
        headers.push(("User-Agent".into(), http.user_agent.clone()));
    }

    let timeout_ms = request.settings.timeout_ms.unwrap_or(http.timeout_ms);
    let follow_redirects = request
        .settings
        .follow_redirects
        .unwrap_or(http.follow_redirects);
    let verify_tls = request.settings.verify_tls.unwrap_or(http.verify_tls);
    let resolved = ResolvedRequest {
        method: method.to_string(),
        url: masker.mask(url.as_str()),
        headers: headers
            .iter()
            .map(|(k, v)| (k.clone(), masker.mask(v)))
            .collect(),
        body: preview.map(|text| masker.mask(&text)),
        timeout_ms,
        follow_redirects,
        verify_tls,
        unresolved,
    };
    let prepared = Prepared {
        method,
        url,
        headers,
        body,
        timeout: Duration::from_millis(timeout_ms),
        client: ClientKey {
            verify_tls,
            follow_redirects,
            max_redirects: http.max_redirects,
            proxy: http.proxy.clone().filter(|p| !p.trim().is_empty()),
        },
        digest,
    };
    Ok((prepared, resolved, masker))
}

/// Un `reqwest::Client` por combinación de opciones, todos con el mismo tarro de cookies.
pub struct Clients {
    cache: Mutex<HashMap<ClientKey, reqwest::Client>>,
    jar: Arc<Jar>,
}

impl Default for Clients {
    fn default() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            jar: Arc::new(Jar::default()),
        }
    }
}

impl Clients {
    pub fn get(&self, key: &ClientKey) -> Result<reqwest::Client> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(client) = cache.get(key) {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder()
            .cookie_provider(self.jar.clone())
            .gzip(true)
            .brotli(true)
            .redirect(if key.follow_redirects {
                redirect::Policy::limited(key.max_redirects)
            } else {
                redirect::Policy::none()
            })
            .tls_danger_accept_invalid_certs(!key.verify_tls);
        builder = match &key.proxy {
            Some(proxy) => {
                builder.proxy(reqwest::Proxy::all(proxy).context("invalid proxy in settings")?)
            }
            // Sin proxy en settings no se usa ninguno (tampoco HTTP_PROXY del entorno).
            None => builder.no_proxy(),
        };
        let client = builder.build().context("cannot build HTTP client")?;
        cache.insert(key.clone(), client.clone());
        Ok(client)
    }
}

/// Envía la petición y lee el cuerpo completo. Con auth digest, un 401 con reto digest se
/// contesta con un segundo intento firmado (`Response::digest_authorization`).
pub async fn send(
    client: &reqwest::Client,
    prepared: &Prepared,
) -> Result<Response, reqwest::Error> {
    let first = send_once(client, prepared, None).await?;
    let Some(credentials) = &prepared.digest else {
        return Ok(first);
    };
    if first.status != 401 {
        return Ok(first);
    }
    let Some(challenge) = digest::challenge(&first.headers) else {
        return Ok(first);
    };
    let mut uri = prepared.url.path().to_owned();
    if let Some(query) = prepared.url.query() {
        uri.push('?');
        uri.push_str(query);
    }
    let header = digest::authorization(
        &challenge,
        credentials,
        prepared.method.as_str(),
        &uri,
        &digest::cnonce(),
    );
    let mut second = send_once(client, prepared, Some(&header)).await?;
    second.ms += first.ms;
    second.digest_authorization = Some(header);
    Ok(second)
}

async fn send_once(
    client: &reqwest::Client,
    prepared: &Prepared,
    authorization: Option<&str>,
) -> Result<Response, reqwest::Error> {
    let mut builder = client
        .request(prepared.method.clone(), prepared.url.clone())
        .timeout(prepared.timeout);
    for (name, value) in &prepared.headers {
        if authorization.is_some() && name.eq_ignore_ascii_case("authorization") {
            continue;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    if let Some(value) = authorization {
        builder = builder.header("Authorization", value);
    }
    builder = match &prepared.body {
        PreparedBody::None => builder,
        PreparedBody::Bytes(bytes) => builder.body(bytes.clone()),
        PreparedBody::Multipart(parts) => {
            let mut form = reqwest::multipart::Form::new();
            for part in parts {
                let mut piece = reqwest::multipart::Part::bytes(part.value.clone());
                if let Some(file_name) = &part.file_name {
                    piece = piece.file_name(file_name.clone());
                }
                form = form.part(part.name.clone(), piece);
            }
            builder.multipart(form)
        }
    };
    let started = Instant::now();
    let response = builder.send().await?;
    let status = response.status();
    let headers: Headers = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let cookies = response
        .cookies()
        .map(|c| ResponseCookie {
            name: c.name().to_owned(),
            value: c.value().to_owned(),
            domain: c.domain().map(str::to_owned),
            path: c.path().map(str::to_owned),
            secure: c.secure(),
            http_only: c.http_only(),
        })
        .collect();
    let body = response.bytes().await?;
    Ok(Response {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or_default().to_owned(),
        headers,
        size: body.len() as u64,
        body,
        ms: started.elapsed().as_millis() as u64,
        cookies,
        digest_authorization: None,
    })
}

/// Tipo corto de error de red (regla 7 de 8.3) y mensaje con su cadena de causas.
pub fn describe_error(error: &reqwest::Error) -> (&'static str, String) {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_redirect() {
        "redirect"
    } else if error.is_body() {
        "body"
    } else if error.is_decode() {
        "decode"
    } else {
        "request"
    };
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    (kind, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{KeyValue, MultipartField, Variable},
        vars::Scope,
    };

    fn resolver() -> Resolver {
        Resolver::new().with_variables(
            Scope::Environment,
            &[
                Variable::new("base", "http://127.0.0.1:9"),
                Variable::secret("token", "s3cr3t"),
            ],
        )
    }

    fn build(request: &Request) -> (Prepared, ResolvedRequest, Masker) {
        build_request(
            request,
            &request.auth.clone(),
            &resolver(),
            &HttpSettings::default(),
        )
        .unwrap()
    }

    #[test]
    fn url_params_headers_and_masking() {
        let mut request = Request::new("r", "get", "{{base}}/ping?a=1&token={{token}}");
        request.params = vec![
            KeyValue::new("b", "x y"),
            KeyValue {
                enabled: false,
                ..KeyValue::new("c", "3")
            },
            // Ya codificado: no se codifica dos veces (como Postman).
            KeyValue::new("enc", "a%3Db"),
            KeyValue::new("flag", ""),
        ];
        request.headers = vec![
            KeyValue::new("X-Token", "{{token}}"),
            KeyValue {
                enabled: false,
                ..KeyValue::new("X-Off", "1")
            },
            KeyValue::new(" ", "ignored"),
        ];
        let (prepared, resolved, _) = build(&request);
        assert_eq!(prepared.method, Method::GET);
        assert_eq!(
            prepared.url.as_str(),
            "http://127.0.0.1:9/ping?a=1&token=s3cr3t&b=x%20y&enc=a%3Db&flag"
        );
        assert_eq!(
            resolved.url,
            "http://127.0.0.1:9/ping?a=1&token=***&b=x%20y&enc=a%3Db&flag"
        );
        assert_eq!(resolved.headers[0], ("X-Token".into(), "***".into()));
        assert_eq!(resolved.headers[1].0, "User-Agent");
        assert_eq!(resolved.headers.len(), 2);
        assert_eq!(resolved.timeout_ms, 30_000);
        assert!(resolved.follow_redirects && resolved.verify_tls);
    }

    #[test]
    fn missing_scheme_defaults_to_http_and_bad_urls_explain_unresolved() {
        let (prepared, ..) = build(&Request::new("r", "GET", "example.com/a"));
        assert_eq!(prepared.url.as_str(), "http://example.com/a");
        let error = build_request(
            &Request::new("r", "GET", "{{nope}}/a"),
            &Auth::None,
            &resolver(),
            &HttpSettings::default(),
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("unresolved: nope"), "{error}");
        let error = build_request(
            &Request::new("r", "BAD METHOD", "http://x"),
            &Auth::None,
            &resolver(),
            &HttpSettings::default(),
        )
        .err()
        .unwrap();
        assert_eq!(
            error.downcast_ref::<CoreError>().unwrap().code(),
            "invalid_request"
        );
    }

    #[test]
    fn auth_header_replaces_manual_one_and_apikey_query() {
        let mut request = Request::new("r", "POST", "{{base}}/x");
        request.headers = vec![KeyValue::new("authorization", "manual")];
        request.auth = Auth::Bearer {
            token: "{{token}}".into(),
        };
        let (prepared, resolved, _) = build(&request);
        assert_eq!(
            prepared.headers[0],
            ("Authorization".into(), "Bearer s3cr3t".into())
        );
        assert_eq!(
            resolved.headers[0],
            ("Authorization".into(), "Bearer ***".into())
        );
        assert_eq!(prepared.headers.len(), 2);
        request.auth = Auth::ApiKey {
            key: "k".into(),
            value: "{{token}}".into(),
            location: crate::model::ApiKeyIn::Query,
        };
        let (prepared, resolved, _) = build(&request);
        assert!(prepared.url.as_str().ends_with("?k=s3cr3t"));
        assert!(resolved.url.ends_with("?k=***"));
    }

    #[test]
    fn bodies_and_default_content_types() {
        let content_type = |p: &Prepared| {
            p.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.clone())
        };
        let mut request = Request::new("r", "POST", "{{base}}/x");
        for (lang, ct) in [
            (RawLang::Json, "application/json"),
            (RawLang::Text, "text/plain"),
            (RawLang::Xml, "application/xml"),
            (RawLang::Html, "text/html"),
            (RawLang::Javascript, "application/javascript"),
        ] {
            request.body = Body::Raw {
                lang,
                text: r#"{"t":"{{token}}"}"#.into(),
            };
            let (prepared, resolved, _) = build(&request);
            assert_eq!(content_type(&prepared).as_deref(), Some(ct));
            assert_eq!(resolved.body.as_deref(), Some(r#"{"t":"***"}"#));
        }
        request.headers = vec![KeyValue::new("Content-Type", "application/vnd+json")];
        let (prepared, ..) = build(&request);
        assert_eq!(
            content_type(&prepared).as_deref(),
            Some("application/vnd+json")
        );
        request.headers.clear();

        request.body = Body::Urlencoded {
            fields: vec![KeyValue::new("a", "1 2"), KeyValue::new("", "x")],
        };
        let (prepared, resolved, _) = build(&request);
        assert_eq!(resolved.body.as_deref(), Some("a=1+2"));
        assert_eq!(
            content_type(&prepared).as_deref(),
            Some("application/x-www-form-urlencoded")
        );

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.bin");
        std::fs::write(&file, b"\x00\x01").unwrap();
        let path = file.to_string_lossy().into_owned();
        request.body = Body::Binary { path: path.clone() };
        let (prepared, resolved, _) = build(&request);
        assert!(matches!(&prepared.body, PreparedBody::Bytes(b) if b == b"\x00\x01"));
        assert_eq!(
            content_type(&prepared).as_deref(),
            Some("application/octet-stream")
        );
        assert!(resolved.body.unwrap().contains("2 bytes"));

        request.headers = vec![KeyValue::new("Content-Type", "multipart/form-data")];
        request.body = Body::Multipart {
            fields: vec![
                MultipartField {
                    key: "t".into(),
                    value: "{{token}}".into(),
                    kind: FieldKind::Text,
                    path: String::new(),
                    enabled: true,
                },
                MultipartField {
                    key: "f".into(),
                    value: String::new(),
                    kind: FieldKind::File,
                    path: path.clone(),
                    enabled: true,
                },
                MultipartField {
                    key: "off".into(),
                    value: "x".into(),
                    kind: FieldKind::Text,
                    path: String::new(),
                    enabled: false,
                },
            ],
        };
        let (prepared, resolved, _) = build(&request);
        let PreparedBody::Multipart(parts) = &prepared.body else {
            panic!()
        };
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].file_name.as_deref(), Some("a.bin"));
        assert_eq!(content_type(&prepared), None);
        assert!(resolved.body.unwrap().starts_with("multipart: t=***, f=@"));

        request.headers.clear();
        request.body = Body::Graphql {
            query: "{a}".into(),
            variables: r#"{"x":1}"#.into(),
        };
        let (_, resolved, _) = build(&request);
        assert_eq!(
            resolved.body.as_deref(),
            Some(r#"{"query":"{a}","variables":{"x":1}}"#)
        );
        request.body = Body::Graphql {
            query: "{a}".into(),
            variables: String::new(),
        };
        assert!(
            build(&request)
                .1
                .body
                .unwrap()
                .contains("\"variables\":null")
        );
        request.body = Body::Graphql {
            query: "{a}".into(),
            variables: "nope".into(),
        };
        assert!(
            build_request(&request, &Auth::None, &resolver(), &HttpSettings::default()).is_err()
        );
        request.body = Body::Binary {
            path: "Z:/does/not/exist.bin".into(),
        };
        assert!(
            build_request(&request, &Auth::None, &resolver(), &HttpSettings::default()).is_err()
        );
    }

    #[test]
    fn request_settings_override_globals() {
        let mut request = Request::new("r", "GET", "http://x");
        request.settings.timeout_ms = Some(5);
        request.settings.follow_redirects = Some(false);
        request.settings.verify_tls = Some(false);
        let http = HttpSettings {
            proxy: Some("http://127.0.0.1:3128".into()),
            ..HttpSettings::default()
        };
        let (prepared, resolved, _) =
            build_request(&request, &Auth::None, &resolver(), &http).unwrap();
        assert_eq!(prepared.timeout, Duration::from_millis(5));
        assert!(!resolved.follow_redirects && !resolved.verify_tls);
        assert_eq!(
            prepared.client.proxy.as_deref(),
            Some("http://127.0.0.1:3128")
        );
        let clients = Clients::default();
        clients.get(&prepared.client).unwrap();
        clients.get(&prepared.client).unwrap();
        assert_eq!(clients.cache.lock().unwrap().len(), 1);
        let bad = ClientKey {
            proxy: Some("::not a proxy::".into()),
            ..prepared.client
        };
        assert!(clients.get(&bad).is_err());
    }
}
