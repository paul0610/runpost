//! Comando curl → petición (sección 11, fase 4), como "Import → Raw text" de Postman.
//!
//! Entiende las comillas de bash (`'…'`, `"…"`, `$'…'`), las continuaciones de línea de bash (`\`),
//! cmd (`^`) y PowerShell (`` ` ``), el escape `^` de "Copy as cURL (cmd)" de los navegadores y las
//! opciones habituales. Lo que no afecta a la petición (`-s`, `-v`, `-o`…) se ignora.

use anyhow::{Result, bail};

use crate::model::{
    Auth, Body, FieldKind, KeyValue, MultipartField, RawLang, Request, RequestSettings,
};

use super::split_query;

/// ¿Parece un comando curl?
pub fn looks_like(text: &str) -> bool {
    let first = text.split_whitespace().next().unwrap_or_default();
    matches!(
        first.trim_matches(|c| c == '"' || c == '\''),
        "curl" | "curl.exe"
    )
}

/// Opciones con valor (formas cortas y largas).
const WITH_VALUE: &[&str] = &[
    "-X",
    "--request",
    "-H",
    "--header",
    "-d",
    "--data",
    "--data-ascii",
    "--data-binary",
    "--data-raw",
    "--data-urlencode",
    "--json",
    "-F",
    "--form",
    "--form-string",
    "-u",
    "--user",
    "--url",
    "-A",
    "--user-agent",
    "-e",
    "--referer",
    "-b",
    "--cookie",
    "-m",
    "--max-time",
    "-o",
    "--output",
    "--connect-timeout",
    "-x",
    "--proxy",
    "-c",
    "--cookie-jar",
    "-w",
    "--write-out",
    "--cacert",
    "--cert",
    "--key",
    "-E",
    "-T",
    "--upload-file",
    "-r",
    "--range",
    "--retry",
    "-y",
    "-Y",
    "--resolve",
    "--interface",
    "--limit-rate",
];

/// Letras cortas sin valor que pueden ir juntas (`-sSL`).
const FLAG_LETTERS: &str = "sSLkvifGIgN#0123";

pub fn parse(text: &str) -> Result<Request> {
    let words = tokenize(&unescape_cmd(&join_lines(text)))?;
    let mut words = words.into_iter();
    match words.next().as_deref() {
        Some("curl" | "curl.exe") => {}
        _ => bail!("not a curl command"),
    }
    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<KeyValue> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut binary_file: Option<String> = None;
    let mut json = false;
    let mut form: Vec<MultipartField> = Vec::new();
    let mut auth = Auth::Inherit;
    let mut settings = RequestSettings::default();
    let mut get = false;
    let mut head = false;

    let mut pending: Vec<String> = words.collect();
    pending.reverse();
    while let Some(word) = pending.pop() {
        // Valor pegado: `-XPOST`, `-H'Accept: x'`, `--data=a`.
        let (option, attached) = split_option(&word);
        let Some(option) = option else {
            if url.is_none() {
                url = Some(word);
            }
            continue;
        };
        if !WITH_VALUE.contains(&option.as_str()) {
            match option.as_str() {
                "-G" | "--get" => get = true,
                "-I" | "--head" => head = true,
                "-k" | "--insecure" => settings.verify_tls = Some(false),
                "-L" | "--location" => settings.follow_redirects = Some(true),
                short if short.len() > 2 && !short.starts_with("--") => {
                    // `-sSL`: cada letra es una opción sin valor; si alguna lleva valor, el resto lo es.
                    let letters: Vec<char> = short[1..].chars().collect();
                    for (at, letter) in letters.iter().enumerate() {
                        let single = format!("-{letter}");
                        if WITH_VALUE.contains(&single.as_str()) {
                            let rest: String = letters[at + 1..].iter().collect();
                            if !rest.is_empty() {
                                pending.push(rest);
                            }
                            pending.push(single);
                            break;
                        } else if FLAG_LETTERS.contains(*letter) {
                            // Al fondo: no se cuela entre una opción con valor y su valor.
                            pending.insert(0, single);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let value = match attached {
            Some(value) => value,
            None => pending.pop().unwrap_or_default(),
        };
        match option.as_str() {
            "-X" | "--request" => method = Some(value.to_uppercase()),
            "-H" | "--header" => {
                if let Some((name, value)) = value.split_once(':') {
                    headers.push(KeyValue::new(name.trim(), value.trim()));
                }
            }
            "-d" | "--data" | "--data-ascii" | "--data-raw" => data.push(value),
            "--data-binary" => match value.strip_prefix('@') {
                Some(path) => binary_file = Some(path.to_owned()),
                None => data.push(value),
            },
            "--data-urlencode" => data.push(match value.split_once('=') {
                Some((name, raw)) => format!("{name}={}", encode(raw)),
                None => encode(&value),
            }),
            "--json" => {
                json = true;
                data.push(value);
            }
            "-F" | "--form" | "--form-string" => {
                let (name, raw) = value.split_once('=').unwrap_or((&value, ""));
                let file = option != "--form-string" && raw.starts_with('@');
                let raw = raw.trim_start_matches('@');
                // `@archivo;type=…`: el tipo no se guarda (lo pone la herramienta).
                let raw = raw.split(";type=").next().unwrap_or(raw);
                let raw = raw.trim_matches('"').to_owned();
                form.push(MultipartField {
                    key: name.to_owned(),
                    value: if file { String::new() } else { raw.clone() },
                    kind: if file {
                        FieldKind::File
                    } else {
                        FieldKind::Text
                    },
                    path: if file { raw } else { String::new() },
                    enabled: true,
                });
            }
            "-u" | "--user" => {
                let (username, password) = value.split_once(':').unwrap_or((&value, ""));
                auth = Auth::Basic {
                    username: username.to_owned(),
                    password: password.to_owned(),
                };
            }
            "--url" => url = Some(value),
            "-A" | "--user-agent" => headers.push(KeyValue::new("User-Agent", value)),
            "-e" | "--referer" => headers.push(KeyValue::new("Referer", value)),
            "-b" | "--cookie" if value.contains('=') => {
                headers.push(KeyValue::new("Cookie", value));
            }
            "-m" | "--max-time" => {
                if let Ok(seconds) = value.parse::<f64>() {
                    settings.timeout_ms = Some((seconds * 1000.0) as u64);
                }
            }
            _ => {}
        }
    }

    let Some(url) = url.filter(|u| !u.is_empty()) else {
        bail!("the curl command has no URL");
    };
    if json {
        for (name, value) in [
            ("Content-Type", "application/json"),
            ("Accept", "application/json"),
        ] {
            if !headers.iter().any(|h| h.key.eq_ignore_ascii_case(name)) {
                headers.push(KeyValue::new(name, value));
            }
        }
    }
    let content_type = headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase())
        .unwrap_or_default();

    let mut full_url = url;
    let joined = data.join("&");
    let mut body = Body::None;
    if !form.is_empty() {
        headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
        body = Body::Multipart { fields: form };
    } else if let Some(path) = binary_file {
        body = Body::Binary { path };
    } else if !data.is_empty() && get {
        let separator = if full_url.contains('?') { '&' } else { '?' };
        full_url = format!("{full_url}{separator}{joined}");
    } else if !data.is_empty() {
        let trimmed = joined.trim_start();
        let looks_json = trimmed.starts_with('{') || trimmed.starts_with('[');
        let is_form = content_type.contains("x-www-form-urlencoded")
            || (content_type.is_empty()
                && !looks_json
                && joined.contains('=')
                && !trimmed.starts_with('<'));
        body = if is_form {
            headers.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
            Body::Urlencoded {
                fields: joined
                    .split('&')
                    .filter(|pair| !pair.is_empty())
                    .map(|pair| {
                        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                        KeyValue::new(decode(key), decode(value))
                    })
                    .collect(),
            }
        } else {
            let lang = if content_type.contains("json") || json || looks_json {
                RawLang::Json
            } else if content_type.contains("xml") || trimmed.starts_with('<') {
                RawLang::Xml
            } else if content_type.contains("html") {
                RawLang::Html
            } else {
                RawLang::Text
            };
            Body::Raw { lang, text: joined }
        };
    }

    let method = match method {
        Some(method) => method,
        None if head => "HEAD".into(),
        None if !matches!(body, Body::None) => "POST".into(),
        None => "GET".into(),
    };
    let (url, params) = split_query(&full_url);
    let mut request = Request::new(url.clone(), method, url);
    request.params = params;
    request.headers = headers;
    request.auth = auth;
    request.body = body;
    request.settings = settings;
    Ok(request)
}

/// `--opción`, `--opción=valor`, `-X`, `-XPOST` → (opción, valor pegado).
fn split_option(word: &str) -> (Option<String>, Option<String>) {
    if let Some(long) = word.strip_prefix("--") {
        if long.is_empty() {
            return (None, None);
        }
        return match long.split_once('=') {
            Some((name, value)) if WITH_VALUE.contains(&format!("--{name}").as_str()) => {
                (Some(format!("--{name}")), Some(value.to_owned()))
            }
            _ => (Some(word.to_owned()), None),
        };
    }
    let mut chars = word.chars();
    if chars.next() != Some('-') {
        return (None, None);
    }
    let Some(letter) = chars.next() else {
        return (None, None);
    };
    let short = format!("-{letter}");
    let rest = chars.as_str();
    if !rest.is_empty() && WITH_VALUE.contains(&short.as_str()) {
        return (Some(short), Some(rest.to_owned()));
    }
    (Some(word.to_owned()), None)
}

/// Une las líneas continuadas con `\`, `^` (cmd) o `` ` `` (PowerShell).
fn join_lines(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out = String::with_capacity(text.len());
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        match trimmed.chars().last() {
            Some('\\' | '^' | '`') if lines.peek().is_some() => {
                out.push_str(&trimmed[..trimmed.len() - 1]);
                out.push(' ');
            }
            _ => {
                out.push_str(line);
                if lines.peek().is_some() {
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// "Copy as cURL (cmd)": `^` escapa el carácter siguiente y `^\^"` es una comilla dentro de `"…"`.
fn unescape_cmd(text: &str) -> String {
    if !text.contains("^\"") && !text.contains("^&") && !text.contains("^%") {
        return text.to_owned();
    }
    let text = text.replace("^\\^\"", "\\\"");
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '^' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Palabras de shell: comillas simples literales, dobles con `\"` `\\` `\$` `` \` ``, `$'…'` con
/// escapes de C, y `\` fuera de comillas.
fn tokenize(text: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => current.push(ch),
                        None => bail!("unclosed single quote"),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.peek() {
                            Some('"' | '\\' | '$' | '`') => {
                                current.push(chars.next().unwrap_or_default())
                            }
                            Some('\n') => {
                                chars.next();
                            }
                            _ => current.push('\\'),
                        },
                        Some(ch) => current.push(ch),
                        None => bail!("unclosed double quote"),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => current.push('\n'),
                            Some('t') => current.push('\t'),
                            Some('r') => current.push('\r'),
                            Some('x') => {
                                let hex: String = (0..2).filter_map(|_| chars.next()).collect();
                                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                                    current.push(byte as char);
                                }
                            }
                            Some(other) => current.push(other),
                            None => bail!("unclosed $'…' quote"),
                        },
                        Some(ch) => current.push(ch),
                        None => bail!("unclosed $'…' quote"),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

/// Codificación de `--data-urlencode`.
fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// `a%20b+c` → `a b c` (campos de un cuerpo urlencoded).
fn decode(text: &str) -> String {
    url::form_urlencoded::parse(format!("x={text}").as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| text.to_owned())
}

#[cfg(test)]
mod tests;
