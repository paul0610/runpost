//! Autenticación digest (RFC 7616, compatible con RFC 2617 y RFC 2069): del reto
//! `WWW-Authenticate: Digest …` de un 401 sale la cabecera `Authorization: Digest …` del segundo
//! intento. MD5, SHA-256 y SHA-512-256, con y sin `-sess`, y `qop=auth`; `auth-int` (firmar el
//! cuerpo) no se soporta.

use aws_lc_rs::digest;
use md5::{Digest as _, Md5};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Algorithm {
    Md5,
    Sha256,
    Sha512_256,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha256 => "SHA-256",
            Self::Sha512_256 => "SHA-512-256",
        }
    }

    fn hash(self, data: &str) -> String {
        let bytes: Vec<u8> = match self {
            Self::Md5 => Md5::digest(data.as_bytes()).to_vec(),
            Self::Sha256 => digest::digest(&digest::SHA256, data.as_bytes())
                .as_ref()
                .to_vec(),
            Self::Sha512_256 => digest::digest(&digest::SHA512_256, data.as_bytes())
                .as_ref()
                .to_vec(),
        };
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// Reto digest de un servidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub algorithm: Algorithm,
    /// `-sess`: el HA1 lleva el nonce y el cnonce.
    pub session: bool,
    /// El servidor ofrece `qop=auth`; sin `qop`, cálculo de RFC 2069.
    pub qop: bool,
    pub userhash: bool,
}

/// Usuario y contraseña ya resueltos.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// `Digest realm="a", nonce="b", Basic realm="c"` → `[("digest", [..]), ("basic", [..])]`. Una
/// cabecera puede traer varios esquemas; las comas dentro de comillas no separan.
fn challenges(value: &str) -> Vec<(String, Vec<(String, String)>)> {
    let chars: Vec<char> = value.chars().collect();
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != ',' {
            i += 1;
        }
        let token: String = chars[start..i].iter().collect();
        if token.is_empty() {
            i += 1;
            continue;
        }
        let mut next = i;
        while next < chars.len() && chars[next].is_whitespace() {
            next += 1;
        }
        if next < chars.len() && chars[next] == '=' {
            i = next + 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let mut value = String::new();
            if i < chars.len() && chars[i] == '"' {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    value.push(chars[i]);
                    i += 1;
                }
                i += 1;
            } else {
                while i < chars.len() && chars[i] != ',' && !chars[i].is_whitespace() {
                    value.push(chars[i]);
                    i += 1;
                }
            }
            if let Some((_, params)) = out.last_mut() {
                params.push((token.to_ascii_lowercase(), value));
            }
        } else {
            out.push((token.to_ascii_lowercase(), Vec::new()));
            i = next;
        }
    }
    out
}

fn to_challenge(params: &[(String, String)]) -> Option<Challenge> {
    let get = |key: &str| {
        params
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    let nonce = get("nonce")?.to_owned();
    let (algorithm, session) = match get("algorithm")
        .unwrap_or("MD5")
        .to_ascii_uppercase()
        .as_str()
    {
        "MD5" => (Algorithm::Md5, false),
        "MD5-SESS" => (Algorithm::Md5, true),
        "SHA-256" => (Algorithm::Sha256, false),
        "SHA-256-SESS" => (Algorithm::Sha256, true),
        "SHA-512-256" => (Algorithm::Sha512_256, false),
        "SHA-512-256-SESS" => (Algorithm::Sha512_256, true),
        _ => return None,
    };
    let qop = match get("qop") {
        None => false,
        // Solo `auth-int` exigiría firmar el cuerpo: ese reto no se acepta.
        Some(qop)
            if qop
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("auth")) =>
        {
            true
        }
        Some(_) => return None,
    };
    Some(Challenge {
        realm: get("realm").unwrap_or_default().to_owned(),
        nonce,
        opaque: get("opaque").map(str::to_owned),
        algorithm,
        session,
        qop,
        userhash: get("userhash").is_some_and(|v| v.eq_ignore_ascii_case("true")),
    })
}

/// El reto digest más fuerte que se entiende entre las cabeceras `WWW-Authenticate`.
pub fn challenge(headers: &[(String, String)]) -> Option<Challenge> {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("www-authenticate"))
        .flat_map(|(_, value)| challenges(value))
        .filter(|(scheme, _)| scheme == "digest")
        .filter_map(|(_, params)| to_challenge(&params))
        .max_by_key(|c| c.algorithm)
}

/// `cnonce` nuevo: 16 bytes aleatorios en hex.
pub fn cnonce() -> String {
    let mut bytes = [0u8; 16];
    // Sin aleatoriedad del sistema, un valor fijo sigue siendo válido (solo pierde entropía).
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn quote(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Cabecera `Authorization` para `method` y `uri` (la ruta con su query, tal cual se envía).
pub fn authorization(
    challenge: &Challenge,
    credentials: &Credentials,
    method: &str,
    uri: &str,
    cnonce: &str,
) -> String {
    let algorithm = challenge.algorithm;
    let h = |data: &str| algorithm.hash(data);
    let nonce = &challenge.nonce;
    let mut ha1 = h(&format!(
        "{}:{}:{}",
        credentials.username, challenge.realm, credentials.password
    ));
    if challenge.session {
        ha1 = h(&format!("{ha1}:{nonce}:{cnonce}"));
    }
    let ha2 = h(&format!("{method}:{uri}"));
    let nc = "00000001";
    let response = if challenge.qop {
        h(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"))
    } else {
        h(&format!("{ha1}:{nonce}:{ha2}"))
    };
    let username = if challenge.userhash {
        h(&format!("{}:{}", credentials.username, challenge.realm))
    } else {
        credentials.username.clone()
    };
    let name = if challenge.session {
        format!("{}-sess", algorithm.name())
    } else {
        algorithm.name().to_owned()
    };
    let mut header = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", algorithm={name}, response=\"{response}\"",
        quote(&username),
        quote(&challenge.realm),
        quote(nonce),
        quote(uri),
    );
    if challenge.qop {
        header.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(", opaque=\"{}\"", quote(opaque)));
    }
    if challenge.userhash {
        header.push_str(", userhash=true");
    }
    header
}

/// La cabecera para el historial y la UI: sin el `response` (sale de la contraseña).
pub fn masked(header: &str) -> String {
    match header.find("response=\"") {
        Some(at) => {
            let start = at + "response=\"".len();
            let end = header[start..]
                .find('"')
                .map_or(header.len(), |end| start + end);
            format!("{}***{}", &header[..start], &header[end..])
        }
        None => header.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(value: &str) -> Vec<(String, String)> {
        vec![("WWW-Authenticate".into(), value.into())]
    }

    #[test]
    fn rfc_2617_example() {
        let challenge = challenge(&header(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        ))
        .unwrap();
        assert_eq!(challenge.algorithm, Algorithm::Md5);
        assert!(challenge.qop && !challenge.session);
        let credentials = Credentials {
            username: "Mufasa".into(),
            password: "Circle Of Life".into(),
        };
        let value = authorization(
            &challenge,
            &credentials,
            "GET",
            "/dir/index.html",
            "0a4f113b",
        );
        assert!(
            value.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{value}"
        );
        assert!(value.contains(r#"qop=auth, nc=00000001, cnonce="0a4f113b""#));
        assert!(value.ends_with(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));
        assert_eq!(
            masked(&value),
            value.replace("6629fae49393a05397450978507c4ef1", "***")
        );
    }

    #[test]
    fn rfc_7616_prefers_sha_256_and_matches_both_examples() {
        let common = r#"realm="http-auth@example.org", qop="auth, auth-int", nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
        let headers = vec![
            (
                "WWW-Authenticate".to_owned(),
                format!("Digest {common}, algorithm=MD5"),
            ),
            (
                "www-authenticate".to_owned(),
                format!("Digest {common}, algorithm=SHA-256"),
            ),
            (
                "WWW-Authenticate".to_owned(),
                "Basic realm=\"x\"".to_owned(),
            ),
        ];
        let strongest = challenge(&headers).unwrap();
        assert_eq!(strongest.algorithm, Algorithm::Sha256);
        let credentials = Credentials {
            username: "Mufasa".into(),
            password: "Circle of Life".into(),
        };
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        let sha = authorization(&strongest, &credentials, "GET", "/dir/index.html", cnonce);
        assert!(
            sha.contains(
                r#"response="753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1""#
            ),
            "{sha}"
        );
        assert!(sha.contains("algorithm=SHA-256"));
        let md5 = Challenge {
            algorithm: Algorithm::Md5,
            ..strongest
        };
        let value = authorization(&md5, &credentials, "GET", "/dir/index.html", cnonce);
        assert!(
            value.contains(r#"response="8ca523f5e9506fed4657c9700eebdbec""#),
            "{value}"
        );
    }

    #[test]
    fn parsing_edge_cases() {
        // Varios esquemas en una cabecera, comas entre comillas y comillas escapadas.
        let parsed = challenge(&header(
            r#"Basic realm="a, b", Digest realm="x \"y\"", nonce=abc, algorithm=MD5-sess, userhash=true"#,
        ))
        .unwrap();
        assert_eq!(parsed.realm, r#"x "y""#);
        assert_eq!(parsed.nonce, "abc");
        assert!(parsed.session && parsed.userhash && !parsed.qop);
        let value = authorization(
            &parsed,
            &Credentials {
                username: "u".into(),
                password: "p".into(),
            },
            "POST",
            "/a?b=1",
            "c",
        );
        assert!(value.contains(r#"realm="x \"y\"""#));
        assert!(value.contains("algorithm=MD5-sess"));
        assert!(value.ends_with("userhash=true"));
        assert!(!value.contains("qop="));
        assert!(!value.contains(r#"username="u""#));

        assert_eq!(challenge(&header("Digest realm=\"x\"")), None, "no nonce");
        assert_eq!(
            challenge(&header("Digest nonce=\"n\", algorithm=SHA-1")),
            None,
            "unknown algorithm"
        );
        assert_eq!(
            challenge(&header("Digest nonce=\"n\", qop=\"auth-int\"")),
            None,
            "auth-int only"
        );
        assert_eq!(challenge(&header("Bearer realm=\"x\"")), None);
        assert_eq!(cnonce().len(), 32);
        assert_ne!(cnonce(), cnonce());
    }
}
