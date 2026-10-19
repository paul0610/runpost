//! Snippets de código (sección 18, fase 4): curl, fetch, PowerShell y Python requests.
//!
//! Las variables normales se sustituyen; los secretos, las dinámicas y las desconocidas quedan como
//! `{{nombre}}`: el snippet puede acabar en un chat o un ticket y nunca lleva un secreto. Basic se
//! codifica al ejecutar el snippet (no dentro de él) y los archivos se citan por ruta, sin leerlos.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    model::{ApiKeyIn, Auth, Body, FieldKind, RawLang, Request},
    vars::Resolver,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Curl,
    Fetch,
    Powershell,
    Python,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::Curl, Lang::Fetch, Lang::Powershell, Lang::Python];

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "curl" => Some(Self::Curl),
            "fetch" | "js" | "javascript" => Some(Self::Fetch),
            "powershell" | "pwsh" => Some(Self::Powershell),
            "python" | "py" | "requests" => Some(Self::Python),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Curl => "curl",
            Self::Fetch => "fetch",
            Self::Powershell => "powershell",
            Self::Python => "python",
        }
    }
}

/// La petición tal como la escribe un snippet.
#[derive(Debug, Default)]
struct Snip {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    /// Basic: cada lenguaje lo codifica a su manera al ejecutarse.
    basic: Option<(String, String)>,
    body: SnipBody,
}

#[derive(Debug, Default)]
enum SnipBody {
    #[default]
    None,
    /// `json` si el texto es JSON válido: algunos lenguajes lo escriben como objeto.
    Text {
        text: String,
        json: Option<Value>,
    },
    Form(Vec<(String, String)>),
    Multipart(Vec<Part>),
    File(String),
}

#[derive(Debug)]
struct Part {
    name: String,
    /// Valor de texto o ruta del archivo.
    value: String,
    file: bool,
}

/// `auth` debe ser la efectiva (`engine::auth::effective_auth`).
pub fn render(lang: Lang, request: &Request, auth: &Auth, resolver: &Resolver) -> String {
    let snip = prepare(request, auth, resolver);
    match lang {
        Lang::Curl => curl(&snip),
        Lang::Fetch => fetch(&snip),
        Lang::Powershell => powershell(&snip),
        Lang::Python => python(&snip),
    }
}

fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    headers.push((name.to_owned(), value));
}

fn default_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name)) {
        headers.push((name.to_owned(), value.to_owned()));
    }
}

fn prepare(request: &Request, auth: &Auth, resolver: &Resolver) -> Snip {
    let r = |text: &str| resolver.resolve_safe(text);
    let method = match request.method.trim() {
        "" => "GET".to_owned(),
        method => method.to_uppercase(),
    };
    let mut query: Vec<String> = request
        .params
        .iter()
        .filter(|p| p.enabled && !p.key.is_empty())
        .map(|p| match r(&p.value) {
            value if value.is_empty() => r(&p.key),
            value => format!("{}={value}", r(&p.key)),
        })
        .collect();
    let mut headers: Vec<(String, String)> = request
        .headers
        .iter()
        .filter(|h| h.enabled && !h.key.trim().is_empty())
        .map(|h| (r(h.key.trim()), r(&h.value)))
        .collect();
    let mut basic = None;
    match auth {
        Auth::None | Auth::Inherit => {}
        Auth::Bearer { token } => {
            if !token.is_empty() {
                set_header(
                    &mut headers,
                    "Authorization",
                    format!("Bearer {}", r(token)),
                );
            }
        }
        Auth::Basic { username, password } => {
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
            basic = Some((r(username), r(password)));
        }
        Auth::ApiKey {
            key,
            value,
            location,
        } => {
            if !key.is_empty() {
                match location {
                    ApiKeyIn::Header => set_header(&mut headers, &r(key), r(value)),
                    ApiKeyIn::Query => query.push(format!("{}={}", r(key), r(value))),
                }
            }
        }
    }
    let mut url = r(request.url.trim());
    if !query.is_empty() {
        let (base, fragment) = match url.find('#') {
            Some(at) => (url[..at].to_owned(), url[at..].to_owned()),
            None => (url.clone(), String::new()),
        };
        let separator = if base.contains('?') { "&" } else { "?" };
        url = format!("{base}{separator}{}{fragment}", query.join("&"));
    }

    let body = match &request.body {
        Body::None => SnipBody::None,
        Body::Raw { lang, text } => {
            let content_type = match lang {
                RawLang::Json => "application/json",
                RawLang::Text => "text/plain",
                RawLang::Xml => "application/xml",
                RawLang::Html => "text/html",
                RawLang::Javascript => "application/javascript",
            };
            default_header(&mut headers, "Content-Type", content_type);
            let text = r(text);
            let json = match lang {
                RawLang::Json => serde_json::from_str(&text).ok(),
                _ => None,
            };
            SnipBody::Text { text, json }
        }
        Body::Urlencoded { fields } => SnipBody::Form(
            fields
                .iter()
                .filter(|f| f.enabled && !f.key.is_empty())
                .map(|f| (r(&f.key), r(&f.value)))
                .collect(),
        ),
        Body::Multipart { fields } => {
            // La herramienta pone el Content-Type con su boundary.
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("content-type"));
            SnipBody::Multipart(
                fields
                    .iter()
                    .filter(|f| f.enabled && !f.key.is_empty())
                    .map(|f| Part {
                        name: r(&f.key),
                        value: r(match f.kind {
                            FieldKind::Text => &f.value,
                            FieldKind::File => &f.path,
                        }),
                        file: f.kind == FieldKind::File,
                    })
                    .collect(),
            )
        }
        Body::Binary { path } => {
            default_header(&mut headers, "Content-Type", "application/octet-stream");
            SnipBody::File(r(path))
        }
        Body::Graphql { query, variables } => {
            default_header(&mut headers, "Content-Type", "application/json");
            let variables = r(variables);
            let variables = if variables.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&variables).unwrap_or(Value::String(variables))
            };
            let json = serde_json::json!({"query": r(query), "variables": variables});
            SnipBody::Text {
                text: json.to_string(),
                json: Some(json),
            }
        }
    };
    Snip {
        method,
        url,
        headers,
        basic,
        body,
    }
}

// ---- curl (bash) ----

/// Comillas simples de shell: `it's` → `'it'\''s'`.
fn sh(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn curl(s: &Snip) -> String {
    let has_data = !matches!(s.body, SnipBody::None);
    let mut lines = vec![if s.method == "GET" && !has_data {
        format!("curl --location {}", sh(&s.url))
    } else {
        format!("curl --location --request {} {}", s.method, sh(&s.url))
    }];
    lines.extend(
        s.headers
            .iter()
            .map(|(k, v)| format!("--header {}", sh(&format!("{k}: {v}")))),
    );
    if let Some((user, pass)) = &s.basic {
        lines.push(format!("--user {}", sh(&format!("{user}:{pass}"))));
    }
    match &s.body {
        SnipBody::None => {}
        SnipBody::Text { text, .. } => lines.push(format!("--data-raw {}", sh(text))),
        SnipBody::Form(fields) => lines.extend(
            fields
                .iter()
                .map(|(k, v)| format!("--data-urlencode {}", sh(&format!("{k}={v}")))),
        ),
        SnipBody::Multipart(parts) => lines.extend(parts.iter().map(|part| {
            let value = if part.file {
                format!("@\"{}\"", part.value)
            } else {
                format!("\"{}\"", part.value.replace('"', "\\\""))
            };
            format!("--form {}", sh(&format!("{}={value}", part.name)))
        })),
        SnipBody::File(path) => lines.push(format!("--data-binary {}", sh(&format!("@{path}")))),
    }
    lines.join(" \\\n")
}

// ---- JavaScript fetch ----

/// Literal de cadena de JavaScript o Python (comillas dobles, escapes de JSON).
fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn fetch(s: &Snip) -> String {
    let mut out = String::new();
    let has_headers = !s.headers.is_empty() || s.basic.is_some();
    if has_headers {
        out.push_str("const myHeaders = new Headers();\n");
        for (k, v) in &s.headers {
            out.push_str(&format!(
                "myHeaders.append({}, {});\n",
                quoted(k),
                quoted(v)
            ));
        }
        if let Some((user, pass)) = &s.basic {
            out.push_str(&format!(
                "myHeaders.append(\"Authorization\", \"Basic \" + btoa({}));\n",
                quoted(&format!("{user}:{pass}"))
            ));
        }
        out.push('\n');
    }
    let body = match &s.body {
        SnipBody::None => None,
        SnipBody::Text {
            json: Some(json), ..
        } => {
            out.push_str(&format!(
                "const raw = JSON.stringify({});\n\n",
                pretty_json(json)
            ));
            Some("raw")
        }
        SnipBody::Text { text, .. } => {
            out.push_str(&format!("const raw = {};\n\n", quoted(text)));
            Some("raw")
        }
        SnipBody::Form(fields) => {
            out.push_str("const urlencoded = new URLSearchParams();\n");
            for (k, v) in fields {
                out.push_str(&format!(
                    "urlencoded.append({}, {});\n",
                    quoted(k),
                    quoted(v)
                ));
            }
            out.push('\n');
            Some("urlencoded")
        }
        SnipBody::Multipart(parts) => {
            out.push_str("const formdata = new FormData();\n");
            for part in parts {
                if part.file {
                    out.push_str(&format!(
                        "formdata.append({}, fileInput.files[0], {});\n",
                        quoted(&part.name),
                        quoted(&part.value)
                    ));
                } else {
                    out.push_str(&format!(
                        "formdata.append({}, {});\n",
                        quoted(&part.name),
                        quoted(&part.value)
                    ));
                }
            }
            out.push('\n');
            Some("formdata")
        }
        SnipBody::File(path) => {
            out.push_str(&format!(
                "const file = \"<file contents here>\"; // {}\n\n",
                path
            ));
            Some("file")
        }
    };
    let mut options = vec![format!("  method: {}", quoted(&s.method))];
    if has_headers {
        options.push("  headers: myHeaders".into());
    }
    if let Some(body) = body {
        options.push(format!("  body: {body}"));
    }
    options.push("  redirect: \"follow\"".into());
    out.push_str(&format!(
        "const requestOptions = {{\n{}\n}};\n\n",
        options.join(",\n")
    ));
    out.push_str(&format!(
        "fetch({}, requestOptions)\n  .then((response) => response.text())\n  .then((result) => console.log(result))\n  .catch((error) => console.error(error));",
        quoted(&s.url)
    ));
    out
}

// ---- PowerShell (Invoke-RestMethod) ----

/// Cadena literal de PowerShell (no expande `$` ni acentos graves).
fn ps(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Texto largo como here-string literal, salvo que contenga su propio cierre.
fn ps_block(text: &str) -> String {
    if text.contains("\n'@") || text.starts_with("'@") {
        ps(text)
    } else {
        format!("@'\n{text}\n'@")
    }
}

fn powershell(s: &Snip) -> String {
    let mut out = String::new();
    let has_headers = !s.headers.is_empty() || s.basic.is_some();
    if has_headers {
        out.push_str(
            "$headers = New-Object \"System.Collections.Generic.Dictionary[[String],[String]]\"\n",
        );
        for (k, v) in &s.headers {
            out.push_str(&format!("$headers.Add({}, {})\n", ps(k), ps(v)));
        }
        if let Some((user, pass)) = &s.basic {
            out.push_str(&format!(
                "$headers.Add('Authorization', 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes({})))\n",
                ps(&format!("{user}:{pass}"))
            ));
        }
        out.push('\n');
    }
    let mut call = format!(
        "$response = Invoke-RestMethod {} -Method {}",
        ps(&s.url),
        ps(&s.method)
    );
    if has_headers {
        call.push_str(" -Headers $headers");
    }
    match &s.body {
        SnipBody::None => {}
        SnipBody::Text {
            json: Some(json), ..
        } => {
            out.push_str(&format!("$body = {}\n\n", ps_block(&pretty_json(json))));
            call.push_str(" -Body $body");
        }
        SnipBody::Text { text, .. } => {
            out.push_str(&format!("$body = {}\n\n", ps_block(text)));
            call.push_str(" -Body $body");
        }
        SnipBody::Form(fields) => {
            out.push_str("$body = @{\n");
            for (k, v) in fields {
                out.push_str(&format!("    {} = {}\n", ps(k), ps(v)));
            }
            out.push_str("}\n\n");
            call.push_str(" -Body $body");
        }
        SnipBody::Multipart(parts) => {
            out.push_str("# -Form needs PowerShell 7 or later\n$form = @{\n");
            for part in parts {
                let value = if part.file {
                    format!("Get-Item -Path {}", ps(&part.value))
                } else {
                    ps(&part.value)
                };
                out.push_str(&format!("    {} = {}\n", ps(&part.name), value));
            }
            out.push_str("}\n\n");
            call.push_str(" -Form $form");
        }
        SnipBody::File(path) => call.push_str(&format!(" -InFile {}", ps(path))),
    }
    out.push_str(&call);
    out.push_str("\n$response | ConvertTo-Json");
    out
}

// ---- Python requests ----

/// JSON → literal de Python (`True`, `False`, `None`), sangrado como `json.dumps(indent=2)`.
fn py_value(value: &Value, indent: usize) -> String {
    let pad = "  ".repeat(indent + 1);
    let close = "  ".repeat(indent);
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => quoted(text),
        Value::Array(items) if items.is_empty() => "[]".into(),
        Value::Array(items) => {
            let inner: Vec<String> = items
                .iter()
                .map(|item| format!("{pad}{}", py_value(item, indent + 1)))
                .collect();
            format!("[\n{}\n{close}]", inner.join(",\n"))
        }
        Value::Object(map) if map.is_empty() => "{}".into(),
        Value::Object(map) => {
            let inner: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{pad}{}: {}", quoted(k), py_value(v, indent + 1)))
                .collect();
            format!("{{\n{}\n{close}}}", inner.join(",\n"))
        }
    }
}

fn python(s: &Snip) -> String {
    let json_import = matches!(&s.body, SnipBody::Text { json: Some(_), .. });
    let mut out = String::from("import requests\n");
    if json_import {
        out.push_str("import json\n");
    }
    out.push_str(&format!("\nurl = {}\n\n", quoted(&s.url)));
    let mut args = Vec::new();
    match &s.body {
        SnipBody::None => {}
        SnipBody::Text {
            json: Some(json), ..
        } => {
            out.push_str(&format!("payload = json.dumps({})\n", py_value(json, 0)));
            args.push("data=payload");
        }
        SnipBody::Text { text, .. } => {
            out.push_str(&format!("payload = {}\n", quoted(text)));
            args.push("data=payload");
        }
        SnipBody::Form(fields) => {
            let pairs: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("  ({}, {})", quoted(k), quoted(v)))
                .collect();
            out.push_str(&format!("payload = [\n{}\n]\n", pairs.join(",\n")));
            args.push("data=payload");
        }
        SnipBody::Multipart(parts) => {
            let texts: Vec<String> = parts
                .iter()
                .filter(|p| !p.file)
                .map(|p| format!("  ({}, {})", quoted(&p.name), quoted(&p.value)))
                .collect();
            let files: Vec<String> = parts
                .iter()
                .filter(|p| p.file)
                .map(|p| {
                    let name = std::path::Path::new(&p.value)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.value.clone());
                    format!(
                        "  ({}, ({}, open({}, \"rb\")))",
                        quoted(&p.name),
                        quoted(&name),
                        quoted(&p.value)
                    )
                })
                .collect();
            if !texts.is_empty() {
                out.push_str(&format!("payload = [\n{}\n]\n", texts.join(",\n")));
                args.push("data=payload");
            }
            if !files.is_empty() {
                out.push_str(&format!("files = [\n{}\n]\n", files.join(",\n")));
                args.push("files=files");
            }
        }
        SnipBody::File(path) => {
            out.push_str(&format!("payload = open({}, \"rb\")\n", quoted(path)));
            args.push("data=payload");
        }
    }
    if !s.headers.is_empty() {
        let pairs: Vec<String> = s
            .headers
            .iter()
            .map(|(k, v)| format!("  {}: {}", quoted(k), quoted(v)))
            .collect();
        out.push_str(&format!("headers = {{\n{}\n}}\n", pairs.join(",\n")));
        args.insert(0, "headers=headers");
    }
    if let Some((user, pass)) = &s.basic {
        out.push_str(&format!("auth = ({}, {})\n", quoted(user), quoted(pass)));
        args.push("auth=auth");
    }
    let mut call = format!("requests.request({}, url", quoted(&s.method));
    for arg in &args {
        call.push_str(", ");
        call.push_str(arg);
    }
    out.push_str(&format!("\nresponse = {call})\n\nprint(response.text)"));
    out
}

#[cfg(test)]
mod tests;
