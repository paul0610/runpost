//! Tarro de cookies del motor (fase 4, gestor de cookies). Es el `cookie_store` que usa el `Jar` de
//! reqwest, más lo que el `Jar` no deja hacer: listar, añadir, editar y borrar. Vive en memoria
//! mientras dura el proceso: las cookies de sesión son credenciales y no se escriben en el
//! workspace.

use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use anyhow::Result;
use bytes::Bytes;
use reqwest::header::HeaderValue;
use serde::Serialize;

use crate::error::CoreError;

#[derive(Default)]
pub struct Cookies(RwLock<cookie_store::CookieStore>);

/// Una cookie para la UI ("Gestionar cookies").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CookieInfo {
    pub domain: String,
    /// Sin atributo `Domain`: vale solo para ese host, no para sus subdominios.
    pub host_only: bool,
    pub path: String,
    pub name: String,
    pub value: String,
    /// Segundos Unix; `None` = cookie de sesión.
    pub expires: Option<i64>,
    pub secure: bool,
    pub http_only: bool,
    /// La cookie como `Set-Cookie`, para editarla.
    pub raw: String,
}

fn read(
    lock: &RwLock<cookie_store::CookieStore>,
) -> RwLockReadGuard<'_, cookie_store::CookieStore> {
    lock.read().unwrap_or_else(|poison| poison.into_inner())
}

fn write(
    lock: &RwLock<cookie_store::CookieStore>,
) -> RwLockWriteGuard<'_, cookie_store::CookieStore> {
    lock.write().unwrap_or_else(|poison| poison.into_inner())
}

fn invalid(message: impl Into<String>) -> anyhow::Error {
    CoreError::InvalidRequest(message.into()).into()
}

impl reqwest::cookie::CookieStore for Cookies {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, url: &url::Url) {
        let cookies = cookie_headers
            .filter_map(|value| std::str::from_utf8(value.as_bytes()).ok())
            .filter_map(|text| cookie::Cookie::parse(text.to_owned()).ok());
        write(&self.0).store_response_cookies(cookies, url);
    }

    fn cookies(&self, url: &url::Url) -> Option<HeaderValue> {
        let text = read(&self.0)
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if text.is_empty() {
            return None;
        }
        HeaderValue::from_maybe_shared(Bytes::from(text)).ok()
    }
}

/// `api.example.com` sin esquema, ruta ni puerto; un punto delante se ignora.
fn clean_domain(domain: &str) -> Result<String> {
    let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    if domain.is_empty()
        || domain
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | ':' | '?' | '#' | '@'))
    {
        return Err(invalid(format!(
            "invalid cookie domain {domain:?}: write only the host, e.g. api.example.com"
        )));
    }
    Ok(domain)
}

impl Cookies {
    /// Las cookies vigentes, por dominio, ruta y nombre.
    pub fn list(&self) -> Vec<CookieInfo> {
        let store = read(&self.0);
        let mut out: Vec<CookieInfo> = store
            .iter_unexpired()
            .map(|cookie| {
                let (domain, host_only) = match &cookie.domain {
                    cookie_store::CookieDomain::HostOnly(host) => (host.clone(), true),
                    cookie_store::CookieDomain::Suffix(suffix) => (suffix.clone(), false),
                    _ => (String::new(), true),
                };
                let expires = match &cookie.expires {
                    cookie_store::CookieExpiration::AtUtc(at) => Some(at.unix_timestamp()),
                    cookie_store::CookieExpiration::SessionEnd => None,
                };
                CookieInfo {
                    domain,
                    host_only,
                    path: String::from(cookie.path.as_ref()),
                    name: cookie.name().to_owned(),
                    value: cookie.value().to_owned(),
                    expires,
                    secure: cookie.secure().unwrap_or(false),
                    http_only: cookie.http_only().unwrap_or(false),
                    raw: cookie.to_string(),
                }
            })
            .collect();
        out.sort_by(|a, b| (&a.domain, &a.path, &a.name).cmp(&(&b.domain, &b.path, &b.name)));
        out
    }

    /// Añade o sustituye una cookie para `domain` a partir de su texto `Set-Cookie`
    /// (`sid=abc; Path=/; HttpOnly`). Con `Domain=` vale también para los subdominios; con una
    /// fecha pasada, la borra.
    pub fn set(&self, domain: &str, raw: &str) -> Result<()> {
        let domain = clean_domain(domain)?;
        let cookie = cookie::Cookie::parse(raw.trim().to_owned())
            .map_err(|error| invalid(format!("invalid cookie: {error}")))?;
        if cookie.name().is_empty() {
            return Err(invalid("invalid cookie: it has no name"));
        }
        let path = cookie.path().filter(|p| p.starts_with('/')).unwrap_or("/");
        let scheme = if cookie.secure().unwrap_or(false) {
            "https"
        } else {
            "http"
        };
        let url = url::Url::parse(&format!("{scheme}://{domain}{path}"))
            .map_err(|error| invalid(format!("invalid cookie domain {domain:?}: {error}")))?;
        match write(&self.0).insert_raw(&cookie, &url) {
            Ok(_) => Ok(()),
            Err(error) => Err(invalid(format!("the cookie was not stored: {error}"))),
        }
    }

    /// Borra una cookie; `false` si no estaba.
    pub fn remove(&self, domain: &str, path: &str, name: &str) -> bool {
        write(&self.0)
            .remove(domain.trim().trim_start_matches('.'), path, name)
            .is_some()
    }

    /// Borra las cookies de `domain` (o todas) y dice cuántas vigentes había.
    pub fn clear(&self, domain: Option<&str>) -> usize {
        let mut store = write(&self.0);
        let Some(domain) = domain else {
            let count = store.iter_unexpired().count();
            store.clear();
            return count;
        };
        let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        let targets: Vec<(String, String, String, bool)> = store
            .iter_any()
            .filter_map(|cookie| {
                let stored = match &cookie.domain {
                    cookie_store::CookieDomain::HostOnly(host)
                    | cookie_store::CookieDomain::Suffix(host) => host.clone(),
                    _ => return None,
                };
                (stored == domain).then(|| {
                    (
                        stored,
                        String::from(cookie.path.as_ref()),
                        cookie.name().to_owned(),
                        !cookie.is_expired(),
                    )
                })
            })
            .collect();
        for (domain, path, name, _) in &targets {
            store.remove(domain, path, name);
        }
        targets.iter().filter(|(.., live)| *live).count()
    }
}

#[cfg(test)]
mod tests {
    use reqwest::cookie::CookieStore as _;

    use super::*;

    /// La cabecera `Cookie` para `url`, ordenada (el orden de envío no está fijado).
    fn sent(jar: &Cookies, url: &url::Url) -> Vec<String> {
        let mut pairs: Vec<String> = jar
            .cookies(url)
            .map(|value| {
                value
                    .to_str()
                    .unwrap()
                    .split("; ")
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        pairs.sort();
        pairs
    }

    #[test]
    fn stores_lists_edits_and_removes() {
        let jar = Cookies::default();
        let url = url::Url::parse("https://api.acme.test/v1/session").unwrap();
        let headers = [
            HeaderValue::from_static("sid=abc; Path=/; HttpOnly; Secure"),
            HeaderValue::from_static("theme=dark; Domain=acme.test; Path=/; Max-Age=3600"),
            // Sin `Path`, la ruta por defecto es la del recurso (RFC 6265 5.1.4).
            HeaderValue::from_static("draft=1"),
        ];
        jar.set_cookies(&mut headers.iter(), &url);
        assert_eq!(sent(&jar, &url), ["draft=1", "sid=abc", "theme=dark"]);
        let other = url::Url::parse("http://www.acme.test/").unwrap();
        assert_eq!(sent(&jar, &other), ["theme=dark"]);

        let list = jar.list();
        assert_eq!(list.len(), 3);
        let draft = list.iter().find(|c| c.name == "draft").unwrap();
        assert_eq!(draft.path, "/v1");
        let sid = list.iter().find(|c| c.name == "sid").unwrap();
        assert!(sid.host_only && sid.secure && sid.http_only);
        assert_eq!(
            (sid.domain.as_str(), sid.path.as_str()),
            ("api.acme.test", "/")
        );
        assert_eq!(sid.expires, None);
        let theme = list.iter().find(|c| c.name == "theme").unwrap();
        assert!(!theme.host_only);
        assert!(theme.expires.unwrap() > chrono::Utc::now().timestamp());

        // Editar desde el gestor: mismo nombre y ruta, valor nuevo.
        jar.set("api.acme.test", "sid=xyz; Path=/; HttpOnly; Secure")
            .unwrap();
        assert_eq!(sent(&jar, &url), ["draft=1", "sid=xyz", "theme=dark"]);
        jar.set(".acme.test", "lang=es; Domain=acme.test").unwrap();
        assert!(jar.list().iter().any(|c| c.name == "lang" && !c.host_only));
        // Una fecha pasada la borra.
        jar.set(
            "api.acme.test",
            "sid=gone; Path=/; Secure; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        )
        .unwrap();
        assert!(!jar.list().iter().any(|c| c.name == "sid"));

        for (domain, raw) in [
            ("", "a=1"),
            ("https://api.acme.test", "a=1"),
            ("api.acme.test", "=1"),
            ("api.acme.test", "no equals sign"),
        ] {
            assert!(jar.set(domain, raw).is_err(), "{domain} {raw}");
        }

        assert!(jar.remove("acme.test", "/", "theme"));
        assert!(!jar.remove("acme.test", "/", "theme"));
        assert_eq!(jar.clear(Some("ACME.test")), 1);
        jar.set("x.test", "a=1").unwrap();
        assert_eq!(jar.clear(None), 2);
        assert!(jar.list().is_empty());
    }
}
