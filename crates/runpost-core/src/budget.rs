//! TokenBudget (sección 8.3): lo que ve el modelo por MCP, compacto y sin secretos.

use serde_json::{Map, Value, json};

use crate::{
    error::{CoreError, NetworkError},
    executor::RunResult,
    runner::RunnerReport,
    vars::{MASK, Masker},
};

/// Máximo de fallos de tests listados (regla 4).
const MAX_FAILURES: usize = 10;
/// Máximo de líneas de consola y caracteres por línea (regla 5).
const MAX_CONSOLE_LINES: usize = 5;
const MAX_CONSOLE_CHARS: usize = 200;
/// Máximo de caracteres de un mensaje de error (regla 7).
const MAX_ERROR_CHARS: usize = 300;

#[derive(Debug, Clone)]
pub struct Budget {
    pub body_max_chars: usize,
    /// JSONPath (RFC 9535) aplicado al cuerpo JSON.
    pub select: Option<String>,
    pub headers: bool,
    pub mask: Masker,
}

/// Cuerpo ya filtrado, antes de truncar.
#[derive(Debug, Clone, PartialEq)]
pub enum Shaped {
    Json(Value),
    Text(String),
}

impl Shaped {
    fn into_text(self) -> String {
        match self {
            Shaped::Json(value) => value.to_string(),
            Shaped::Text(text) => text,
        }
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// `"application/json; charset=utf-8"` → `"application/json"`.
pub fn short_content_type(ct: &str) -> String {
    ct.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn is_json(ct: &str) -> bool {
    let ct = short_content_type(ct);
    ct == "application/json" || ct.ends_with("+json") || ct.ends_with("/json")
}

/// Regla 1: JSON → minificado (y `select`); HTML → `<title>` + texto visible; resto → texto.
/// Siempre enmascara antes de parsear. Error si `select` no es un JSONPath válido o el cuerpo no es JSON.
pub fn shape_body(
    body: &[u8],
    ct: &str,
    select: Option<&str>,
    mask: &Masker,
) -> Result<Shaped, String> {
    let Ok(text) = std::str::from_utf8(body) else {
        if select.is_some() {
            return Err("select needs a JSON body".into());
        }
        return Ok(Shaped::Text(format!("<binary {} bytes>", body.len())));
    };
    let text = mask.mask(text);
    let parsed = if is_json(ct) || select.is_some() {
        serde_json::from_str::<Value>(&text).ok()
    } else {
        None
    };
    match (parsed, select) {
        (Some(value), Some(path)) => {
            let path =
                serde_json_path::JsonPath::parse(path).map_err(|e| format!("bad JSONPath: {e}"))?;
            let nodes = path.query(&value).all();
            Ok(Shaped::Json(match nodes.as_slice() {
                [] => Value::Null,
                [one] => (*one).clone(),
                many => Value::Array(many.iter().map(|v| (*v).clone()).collect()),
            }))
        }
        (None, Some(_)) => Err("select needs a JSON body".into()),
        (Some(value), None) => Ok(Shaped::Json(value)),
        (None, None) if short_content_type(ct).contains("html") => {
            Ok(Shaped::Text(html_to_text(&text)))
        }
        (None, None) => Ok(Shaped::Text(text)),
    }
}

/// `<title>` + texto visible, sin etiquetas, scripts ni estilos; espacios colapsados.
pub fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let title = lower.find("<title").and_then(|start| {
        let open_end = start + lower[start..].find('>')? + 1;
        let close = open_end + lower[open_end..].find("</title")?;
        Some(collapse(&decode_entities(&html[open_end..close])))
    });
    let mut text = String::with_capacity(html.len() / 2);
    let mut i = 0;
    while i < html.len() {
        if lower[i..].starts_with('<') {
            // Saltar bloques enteros cuyo contenido no es visible.
            let skip_block = ["script", "style", "head", "noscript", "template"]
                .into_iter()
                .find(|tag| {
                    lower[i + 1..].starts_with(tag)
                        && lower[i + 1 + tag.len()..]
                            .starts_with(|c: char| c == '>' || c.is_whitespace())
                });
            if let Some(tag) = skip_block {
                let close = format!("</{tag}");
                i = match lower[i..].find(&close) {
                    Some(at) => i + at + lower[i + at..].find('>').map_or(close.len(), |g| g + 1),
                    None => html.len(),
                };
            } else {
                i = lower[i..].find('>').map_or(html.len(), |g| i + g + 1);
            }
            text.push(' ');
        } else {
            let next = lower[i..].find('<').map_or(html.len(), |n| i + n);
            text.push_str(&html[i..next]);
            i = next;
        }
    }
    let body = collapse(&decode_entities(&text));
    match title {
        Some(title) if !title.is_empty() => format!("{title} | {body}"),
        _ => body,
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Regla 2: inserta `body` en `out`, truncando a `max` caracteres. JSON truncado va como texto.
pub fn put_body(out: &mut Map<String, Value>, shaped: Shaped, max: usize) {
    let text = match &shaped {
        Shaped::Json(value) => value.to_string(),
        Shaped::Text(text) => text.clone(),
    };
    let total = text.chars().count();
    if total <= max {
        out.insert(
            "body".into(),
            match shaped {
                Shaped::Json(value) => value,
                Shaped::Text(text) => Value::String(text),
            },
        );
    } else {
        out.insert(
            "body".into(),
            Value::String(truncate_chars(&shaped.into_text(), max)),
        );
        out.insert("truncated".into(), Value::Bool(true));
        out.insert("total_chars".into(), json!(total));
    }
}

/// Regla 3: cabeceras sensibles siempre como `***`, el resto pasa por el `Masker`.
pub fn is_sensitive_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
    ) || (name.starts_with("x-") && (name.ends_with("-key") || name.ends_with("-token")))
}

pub fn shape_headers(headers: &[(String, String)], mask: &Masker) -> Value {
    let mut out = Map::new();
    for (name, value) in headers {
        let value = if is_sensitive_header(name) {
            MASK.to_owned()
        } else {
            mask.mask(value)
        };
        match out.get_mut(name) {
            Some(Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(&value);
            }
            _ => {
                out.insert(name.clone(), Value::String(value));
            }
        }
    }
    Value::Object(out)
}

/// Respuesta de `run_request` / `send` (8.3).
pub fn shape_response(r: &RunResult, b: &Budget) -> Value {
    let response = &r.response;
    let mut out = Map::new();
    if r.skipped {
        // `pm.execution.skipRequest()`: no hubo petición, solo lo que dejaron los scripts.
        out.insert("skipped".into(), json!(true));
    } else {
        let ct = response.content_type().unwrap_or_default();
        out.insert("status".into(), json!(response.status));
        out.insert("ms".into(), json!(response.ms));
        out.insert("bytes".into(), json!(response.size));
        if !ct.is_empty() {
            out.insert("ct".into(), json!(short_content_type(ct)));
        }
        if !r.history_id.is_empty() {
            out.insert("response_id".into(), json!(r.history_id));
        }
        match shape_body(&response.body, ct, b.select.as_deref(), &b.mask) {
            Ok(shaped) => put_body(&mut out, shaped, b.body_max_chars),
            Err(error) => {
                out.insert("select_error".into(), json!(error));
                let shaped = shape_body(&response.body, ct, None, &b.mask)
                    .unwrap_or(Shaped::Text(String::new()));
                put_body(&mut out, shaped, b.body_max_chars);
            }
        }
        if b.headers {
            out.insert("headers".into(), shape_headers(&response.headers, &b.mask));
        }
    }
    // Reglas 4 y 5: solo si hubo tests o consola (scripts, sprint 5).
    if !r.tests.is_empty() {
        let failed: Vec<_> = r.tests.iter().filter(|t| !t.passed && !t.skipped).collect();
        let passed = r.tests.iter().filter(|t| t.passed && !t.skipped).count();
        let mut tests = Map::new();
        tests.insert("passed".into(), json!(passed));
        tests.insert("failed".into(), json!(failed.len()));
        if !failed.is_empty() {
            tests.insert(
                "failures".into(),
                Value::Array(
                    failed
                        .iter()
                        .take(MAX_FAILURES)
                        .map(|t| json!({"name": b.mask.mask(&t.name), "msg": b.mask.mask(t.msg.as_deref().unwrap_or_default())}))
                        .collect(),
                ),
            );
        }
        out.insert("tests".into(), Value::Object(tests));
    }
    if !r.vars_changed.is_empty() {
        out.insert(
            "vars_changed".into(),
            json!(r.vars_changed.iter().map(|v| &v.key).collect::<Vec<_>>()),
        );
    }
    if !r.console.is_empty() {
        let lines: Vec<String> = r
            .console
            .iter()
            .take(MAX_CONSOLE_LINES)
            .map(|line| truncate_chars(&b.mask.mask(line), MAX_CONSOLE_CHARS))
            .collect();
        out.insert("console".into(), json!(lines));
    }
    let raw = response.size as usize
        + response
            .headers
            .iter()
            .map(|(k, v)| k.len() + v.len() + 4)
            .sum::<usize>();
    with_meter(out, raw)
}

/// Detalle de `run_collection` por MCP.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunDetail {
    /// Contadores y hasta 20 fallos.
    #[default]
    Summary,
    /// Solo los fallos (tests fallidos y peticiones sin respuesta).
    Failures,
    /// Contadores y cada petición con sus tests.
    Full,
}

/// Máximo de fallos listados en el resumen de `run_collection`.
const MAX_RUN_FAILURES: usize = 20;

/// Salida de `run_collection` (8.2): `{"total","passed","failed","ms","failures":[{"path","test","msg"}]}`.
/// Los nombres y mensajes ya llegan enmascarados del ejecutor; la consola no se incluye.
pub fn shape_run(report: &RunnerReport, detail: RunDetail) -> Value {
    let totals = &report.totals;
    let several = report.iterations > 1;
    let base = |path: &str, iteration: u32| {
        let mut entry = Map::new();
        entry.insert("path".into(), json!(path));
        if several {
            entry.insert("it".into(), json!(iteration + 1));
        }
        entry
    };
    let mut failures: Vec<Value> = Vec::new();
    for item in &report.items {
        if let Some(error) = &item.error {
            let mut entry = base(&item.path, item.iteration);
            entry.insert("test".into(), json!("<request error>"));
            entry.insert("msg".into(), json!(truncate_chars(error, MAX_ERROR_CHARS)));
            failures.push(Value::Object(entry));
        }
        for test in item.failed_tests() {
            let mut entry = base(&item.path, item.iteration);
            entry.insert("test".into(), json!(test.name));
            if let Some(msg) = &test.msg {
                entry.insert("msg".into(), json!(truncate_chars(msg, MAX_ERROR_CHARS)));
            }
            failures.push(Value::Object(entry));
        }
    }
    let mut out = Map::new();
    if detail == RunDetail::Failures {
        out.insert("failed".into(), json!(failures.len()));
        out.insert("failures".into(), Value::Array(failures));
    } else {
        out.insert("total".into(), json!(totals.tests));
        out.insert("passed".into(), json!(totals.passed));
        out.insert("failed".into(), json!(totals.failed));
        out.insert("ms".into(), json!(report.ms));
        out.insert("requests".into(), json!(totals.requests));
        if totals.skipped > 0 {
            out.insert("skipped".into(), json!(totals.skipped));
        }
        if totals.errors > 0 {
            out.insert("errors".into(), json!(totals.errors));
        }
        if report.cancelled {
            out.insert("cancelled".into(), json!(true));
        }
        if detail == RunDetail::Summary {
            if !failures.is_empty() {
                let more = failures.len().saturating_sub(MAX_RUN_FAILURES);
                failures.truncate(MAX_RUN_FAILURES);
                out.insert("failures".into(), Value::Array(failures));
                if more > 0 {
                    out.insert("more_failures".into(), json!(more));
                }
            }
        } else {
            let items: Vec<Value> = report
                .items
                .iter()
                .map(|item| {
                    let mut entry = base(&item.path, item.iteration);
                    if item.skipped {
                        entry.insert("skipped".into(), json!(true));
                    } else if let Some(error) = &item.error {
                        entry.insert(
                            "error".into(),
                            json!(truncate_chars(error, MAX_ERROR_CHARS)),
                        );
                    } else {
                        entry.insert("status".into(), json!(item.status));
                        entry.insert("ms".into(), json!(item.ms));
                    }
                    if !item.tests.is_empty() {
                        let tests: Vec<Value> = item
                            .tests
                            .iter()
                            .map(|test| {
                                let mut t = Map::new();
                                t.insert("name".into(), json!(test.name));
                                if test.skipped {
                                    t.insert("skipped".into(), json!(true));
                                } else {
                                    t.insert("ok".into(), json!(test.passed));
                                    if !test.passed
                                        && let Some(msg) = &test.msg
                                    {
                                        t.insert(
                                            "msg".into(),
                                            json!(truncate_chars(msg, MAX_ERROR_CHARS)),
                                        );
                                    }
                                }
                                Value::Object(t)
                            })
                            .collect();
                        entry.insert("tests".into(), Value::Array(tests));
                    }
                    Value::Object(entry)
                })
                .collect();
            out.insert("items".into(), Value::Array(items));
        }
    }
    let raw = serde_json::to_string(report).map_or(0, |text| text.len());
    with_meter(out, raw)
}

/// Regla 9: añade `"t":{"raw":N,"sent":M}`, donde `sent` incluye el propio campo `t`.
pub fn with_meter(mut out: Map<String, Value>, raw: usize) -> Value {
    let base = Value::Object(out.clone()).to_string().len();
    let mut sent = base;
    for _ in 0..3 {
        let field = format!(r#","t":{{"raw":{raw},"sent":{sent}}}"#).len();
        sent = base + field;
    }
    out.insert("t".into(), json!({"raw": raw, "sent": sent}));
    Value::Object(out)
}

/// Regla 7 y errores del núcleo: `{"error":"connect","msg":"…","url":"…"}` o
/// `{"error":"env_not_found","hint":"…"}`. Todo enmascarado y corto.
pub fn shape_error(error: &anyhow::Error, mask: &Masker) -> Value {
    if let Some(network) = error.downcast_ref::<NetworkError>() {
        let mut out = Map::new();
        out.insert("error".into(), json!(network.kind));
        out.insert(
            "msg".into(),
            json!(truncate_chars(
                &mask.mask(&network.message),
                MAX_ERROR_CHARS
            )),
        );
        out.insert("url".into(), json!(mask.mask(&network.url)));
        if let Some(id) = &network.history_id {
            out.insert("response_id".into(), json!(id));
        }
        return Value::Object(out);
    }
    if let Some(core) = error.downcast_ref::<CoreError>() {
        let mut out = Map::new();
        out.insert("error".into(), json!(core.code()));
        out.insert(
            "msg".into(),
            json!(truncate_chars(
                &mask.mask(&core.to_string()),
                MAX_ERROR_CHARS
            )),
        );
        let hint = match core {
            CoreError::NotFound { kind: "env", .. } => Some("ws_overview lists environments"),
            CoreError::NotFound {
                kind: "request", ..
            } => Some("list_requests shows ids and paths"),
            CoreError::NotFound {
                kind: "collection", ..
            } => Some("ws_overview lists collections"),
            CoreError::Ambiguous { .. } => Some("use the id or the full path"),
            _ => None,
        };
        if let Some(hint) = hint {
            out.insert("hint".into(), json!(hint));
        }
        return Value::Object(out);
    }
    json!({"error": "internal", "msg": truncate_chars(&mask.mask(&format!("{error:#}")), MAX_ERROR_CHARS)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::{ResolvedRequest, Response},
        executor::{TestResult, VarChange},
        vars::Scope,
    };

    fn result(ct: &str, body: &str) -> RunResult {
        let mut masker = Masker::new();
        masker.add("s3cr3t");
        RunResult {
            response: Response {
                status: 200,
                status_text: "OK".into(),
                headers: vec![
                    ("content-type".into(), ct.into()),
                    ("set-cookie".into(), "sid=1".into()),
                    ("x-api-key".into(), "k".into()),
                    ("x-echo".into(), "s3cr3t".into()),
                    ("x-echo".into(), "two".into()),
                ],
                body: body.as_bytes().to_vec().into(),
                ms: 143,
                size: body.len() as u64,
                cookies: Vec::new(),
                digest_authorization: None,
            },
            resolved: ResolvedRequest::default(),
            tests: Vec::new(),
            vars_changed: Vec::new(),
            history_id: "h_1".into(),
            console: Vec::new(),
            skipped: false,
            masker,
            locals: Default::default(),
        }
    }

    fn budget(r: &RunResult) -> Budget {
        Budget {
            body_max_chars: 2000,
            select: None,
            headers: false,
            mask: r.masker.clone(),
        }
    }

    fn tokens(value: &Value) -> usize {
        value.to_string().len() / 4
    }

    #[test]
    fn rule1_json_is_minified_and_key_order_is_stable() {
        let r = result(
            "application/json; charset=utf-8",
            "{\n  \"status\": \"ok\",\n  \"n\": [1, 2]\n}",
        );
        let out = shape_response(&r, &budget(&r));
        let text = out.to_string();
        assert!(
            text.starts_with(r#"{"status":200,"ms":143,"bytes":"#),
            "{text}"
        );
        assert!(
            text.contains(
                r#""ct":"application/json","response_id":"h_1","body":{"status":"ok","n":[1,2]}"#
            ),
            "{text}"
        );
        assert!(
            !text.contains("truncated") && !text.contains("headers") && !text.contains("tests")
        );
    }

    #[test]
    fn rule1_select_one_many_none_and_errors() {
        let r = result(
            "application/json",
            r#"{"a":{"b":1},"items":[{"id":1},{"id":2}]}"#,
        );
        let mut b = budget(&r);
        b.select = Some("$.a.b".into());
        assert_eq!(shape_response(&r, &b)["body"], json!(1));
        b.select = Some("$.items[*].id".into());
        assert_eq!(shape_response(&r, &b)["body"], json!([1, 2]));
        b.select = Some("$.missing".into());
        assert_eq!(shape_response(&r, &b)["body"], Value::Null);
        b.select = Some("$$bad".into());
        let out = shape_response(&r, &b);
        assert!(
            out["select_error"]
                .as_str()
                .unwrap()
                .starts_with("bad JSONPath")
        );
        assert_eq!(out["body"]["a"]["b"], 1);
        let text = result("text/plain", "hello");
        let mut b = budget(&text);
        b.select = Some("$.a".into());
        assert_eq!(
            shape_response(&text, &b)["select_error"],
            "select needs a JSON body"
        );
    }

    #[test]
    fn select_to_one_field_is_under_60_tokens() {
        let big: Vec<Value> = (0..200)
            .map(|i| json!({"id": i, "name": format!("account-{i}"), "currency": "usd"}))
            .collect();
        let r = result(
            "application/json",
            &json!({"accounts": big, "count": 200}).to_string(),
        );
        let mut b = budget(&r);
        b.select = Some("$.count".into());
        let out = shape_response(&r, &b);
        assert!(tokens(&out) < 60, "{} tokens: {out}", tokens(&out));
    }

    #[test]
    fn json_body_parses_even_without_json_content_type_when_selecting() {
        let r = result("text/plain", r#"{"a":1}"#);
        let mut b = budget(&r);
        b.select = Some("$.a".into());
        assert_eq!(shape_response(&r, &b)["body"], 1);
        assert_eq!(shape_response(&r, &budget(&r))["body"], r#"{"a":1}"#);
    }

    #[test]
    fn rule1_html_becomes_title_and_visible_text() {
        let html = "<html><head><title>Hi &amp; bye</title><style>p{}</style></head><body>\
                    <script>var x = '<p>';</script><h1>Hello</h1>\n<p>World&nbsp;!</p></body></html>";
        let r = result("text/html", html);
        assert_eq!(
            shape_response(&r, &budget(&r))["body"],
            "Hi & bye | Hello World !"
        );
        assert_eq!(
            html_to_text("plain <b>bold</b> <SCRIPT type=x>bad()</SCRIPT> end"),
            "plain bold end"
        );
        assert_eq!(html_to_text("<p>unclosed <script>x"), "unclosed");
    }

    #[test]
    fn rule1_binary_bodies_are_summarised() {
        let mut r = result("application/octet-stream", "");
        r.response.body = vec![0xff, 0xfe, 0x00].into();
        assert_eq!(shape_response(&r, &budget(&r))["body"], "<binary 3 bytes>");
        let mut b = budget(&r);
        b.select = Some("$".into());
        assert_eq!(
            shape_response(&r, &b)["select_error"],
            "select needs a JSON body"
        );
    }

    #[test]
    fn rule2_truncates_with_total_chars() {
        let r = result(
            "application/json",
            &json!({"text": "x".repeat(50)}).to_string(),
        );
        let mut b = budget(&r);
        b.body_max_chars = 10;
        let out = shape_response(&r, &b);
        assert_eq!(out["body"], r#"{"text":"x"#);
        assert_eq!(out["truncated"], true);
        assert_eq!(out["total_chars"], 61);
        let r = result("text/plain", "ñandú€€");
        let mut b = budget(&r);
        b.body_max_chars = 3;
        assert_eq!(shape_response(&r, &b)["body"], "ñan");
    }

    #[test]
    fn rule3_headers_only_on_request_and_masked() {
        let r = result("text/plain", "x");
        let mut b = budget(&r);
        b.headers = true;
        let headers = &shape_response(&r, &b)["headers"];
        assert_eq!(headers["set-cookie"], "***");
        assert_eq!(headers["x-api-key"], "***");
        assert_eq!(headers["x-echo"], "***, two");
        assert_eq!(headers["content-type"], "text/plain");
        assert!(is_sensitive_header("X-Auth-Token") && is_sensitive_header("Authorization"));
        assert!(!is_sensitive_header("x-request-id"));
    }

    #[test]
    fn rules4_5_tests_and_console_are_summarised() {
        let mut r = result("text/plain", "x");
        r.tests = (0..15)
            .map(|i| TestResult {
                name: format!("t{i}"),
                passed: i < 2,
                msg: Some("expected s3cr3t".into()),
                skipped: false,
            })
            .collect();
        r.console = (0..8).map(|i| format!("{i} {}", "y".repeat(300))).collect();
        r.vars_changed = vec![VarChange {
            scope: Scope::Environment,
            key: "scopeLabel".into(),
        }];
        let out = shape_response(&r, &budget(&r));
        assert_eq!(out["tests"]["passed"], 2);
        assert_eq!(out["tests"]["failed"], 13);
        assert_eq!(out["tests"]["failures"].as_array().unwrap().len(), 10);
        assert_eq!(out["tests"]["failures"][0]["msg"], "expected ***");
        assert_eq!(out["vars_changed"], json!(["scopeLabel"]));
        let console = out["console"].as_array().unwrap();
        assert_eq!(console.len(), 5);
        assert_eq!(console[0].as_str().unwrap().chars().count(), 200);
        r.tests.truncate(2);
        assert!(
            shape_response(&r, &budget(&r))["tests"]
                .get("failures")
                .is_none()
        );
    }

    #[test]
    fn rule6_secrets_never_leave_even_in_json_strings() {
        let r = result("application/json", r#"{"token":"s3cr3t","n":1}"#);
        let mut b = budget(&r);
        b.headers = true;
        let text = shape_response(&r, &b).to_string();
        assert!(!text.contains("s3cr3t"), "{text}");
        assert!(text.contains(r#""token":"***""#));
    }

    #[test]
    fn rule7_errors_are_short_and_masked() {
        let mut mask = Masker::new();
        mask.add("s3cr3t");
        let network = anyhow::Error::new(NetworkError {
            kind: "connect",
            message: format!("dns error s3cr3t {}", "z".repeat(400)),
            url: "http://h/?t=***".into(),
            history_id: Some("h_9".into()),
        });
        let out = shape_error(&network, &mask);
        assert_eq!(out["error"], "connect");
        assert_eq!(out["msg"].as_str().unwrap().chars().count(), 300);
        assert!(!out.to_string().contains("s3cr3t"));
        assert_eq!(out["response_id"], "h_9");
        let env = anyhow::Error::new(CoreError::not_found("env", "x"));
        assert_eq!(
            shape_error(&env, &mask)["hint"],
            "ws_overview lists environments"
        );
        let amb = anyhow::Error::new(CoreError::Ambiguous {
            kind: "request",
            selector: "p".into(),
            candidates: 2,
        });
        assert_eq!(shape_error(&amb, &mask)["error"], "request_ambiguous");
        let invalid = anyhow::Error::new(CoreError::InvalidRequest("bad".into()));
        assert!(shape_error(&invalid, &mask).get("hint").is_none());
        let other = anyhow::anyhow!("boom s3cr3t");
        assert_eq!(
            shape_error(&other, &mask),
            json!({"error":"internal","msg":"boom ***"})
        );
    }

    #[test]
    fn rule9_meter_counts_itself() {
        let r = result("application/json", r#"{"status":"gateway is ok!"}"#);
        let out = shape_response(&r, &budget(&r));
        assert_eq!(
            out["t"]["sent"].as_u64().unwrap() as usize,
            out.to_string().len()
        );
        assert!(out["t"]["raw"].as_u64().unwrap() > r.response.size);
        assert!(out.to_string().len() < 300, "{out}");
    }

    #[test]
    fn typical_5kb_json_stays_under_700_tokens() {
        let items: Vec<Value> = (0..60)
            .map(|i| json!({"id": format!("{i:032x}"), "label": "account", "n": i}))
            .collect();
        let body = serde_json::to_string_pretty(&json!({"items": items})).unwrap();
        assert!(body.len() > 5000);
        let r = result("application/json", &body);
        let out = shape_response(&r, &budget(&r));
        assert!(tokens(&out) < 700, "{} tokens", tokens(&out));
    }
}
