//! Diff de respuestas (sección 19.7): compara dos respuestas (estado y cuerpo JSON estructural)
//! ignorando campos volátiles, para que el modelo pida solo "qué cambió".

use serde::Serialize;
use serde_json::Value;

/// Máximo de cambios devueltos; el resto se cuenta en `more`.
pub const MAX_CHANGES: usize = 50;
/// Valores más largos se resumen para no gastar tokens.
const MAX_VALUE_CHARS: usize = 200;

/// Lado de la comparación: lo que se guarda de una respuesta.
#[derive(Debug, Clone, Copy)]
pub struct Side<'a> {
    pub status: Option<u16>,
    pub content_type: &'a str,
    pub body: &'a [u8],
}

/// Un cambio: `from` falta si el campo es nuevo y `to` falta si desapareció.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Diff {
    pub diff: Vec<Change>,
    /// Cambios que no caben en `MAX_CHANGES`.
    #[serde(skip_serializing_if = "is_zero")]
    pub more: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Campos a ignorar: un nombre (`timestamp`) vale a cualquier profundidad; una ruta (`$.a[*].b`)
/// solo ahí (`[*]` = cualquier posición).
struct Ignore<'a>(&'a [String]);

impl Ignore<'_> {
    fn skips(&self, key: &str, path: &str) -> bool {
        let generic = generic_path(path);
        self.0.iter().any(|rule| {
            if rule.starts_with('$') {
                rule == path || *rule == generic
            } else {
                rule == key
            }
        })
    }
}

/// `$.a[3].b` → `$.a[*].b`.
fn generic_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        out.push(ch);
        if ch == '[' && chars.peek().is_some_and(char::is_ascii_digit) {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            out.push('*');
        }
    }
    out
}

fn child(path: &str, key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !key.starts_with(|c: char| c.is_ascii_digit())
    {
        format!("{path}.{key}")
    } else {
        format!("{path}[{}]", serde_json::to_string(key).unwrap_or_default())
    }
}

fn brief(value: &Value) -> Value {
    let text = value.to_string();
    if text.chars().count() <= MAX_VALUE_CHARS {
        value.clone()
    } else {
        let head: String = text.chars().take(MAX_VALUE_CHARS).collect();
        Value::String(format!("{head}… ({} chars)", text.chars().count()))
    }
}

struct Collector {
    changes: Vec<Change>,
    more: usize,
}

impl Collector {
    fn push(&mut self, path: String, from: Option<&Value>, to: Option<&Value>) {
        if self.changes.len() < MAX_CHANGES {
            self.changes.push(Change {
                path,
                from: from.map(brief),
                to: to.map(brief),
            });
        } else {
            self.more += 1;
        }
    }
}

fn walk(path: &str, old: &Value, new: &Value, ignore: &Ignore, out: &mut Collector) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, va) in a {
                let at = child(path, key);
                if ignore.skips(key, &at) {
                    continue;
                }
                match b.get(key) {
                    Some(vb) => walk(&at, va, vb, ignore, out),
                    None => out.push(at, Some(va), None),
                }
            }
            for (key, vb) in b.iter().filter(|(k, _)| !a.contains_key(*k)) {
                let at = child(path, key);
                if !ignore.skips(key, &at) {
                    out.push(at, None, Some(vb));
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for index in 0..a.len().max(b.len()) {
                let at = format!("{path}[{index}]");
                match (a.get(index), b.get(index)) {
                    (Some(va), Some(vb)) => walk(&at, va, vb, ignore, out),
                    (va, vb) => out.push(at, va, vb),
                }
            }
        }
        _ if old != new => out.push(path.to_owned(), Some(old), Some(new)),
        _ => {}
    }
}

fn parse_json(side: &Side) -> Option<Value> {
    let looks_json = side.content_type.to_ascii_lowercase().contains("json")
        || side
            .body
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .is_some_and(|b| *b == b'{' || *b == b'[');
    looks_json
        .then(|| serde_json::from_slice(side.body).ok())
        .flatten()
}

/// Primera línea distinta de dos textos (para cuerpos que no son JSON).
fn first_difference(old: &str, new: &str) -> (usize, String, String) {
    let mut a = old.lines();
    let mut b = new.lines();
    let mut line = 1;
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) if x == y => line += 1,
            (x, y) => {
                return (
                    line,
                    x.unwrap_or_default().to_owned(),
                    y.unwrap_or_default().to_owned(),
                );
            }
        }
    }
}

/// Compara `old` (la referencia) con `new`. Un diff vacío significa "nada relevante cambió".
pub fn diff_responses(old: Side, new: Side, ignore: &[String]) -> Diff {
    let mut out = Collector {
        changes: Vec::new(),
        more: 0,
    };
    if old.status != new.status {
        out.push(
            "status".into(),
            Some(&Value::from(old.status)),
            Some(&Value::from(new.status)),
        );
    }
    match (parse_json(&old), parse_json(&new)) {
        (Some(a), Some(b)) => walk("$", &a, &b, &Ignore(ignore), &mut out),
        _ if old.body != new.body => {
            let a = String::from_utf8_lossy(old.body);
            let b = String::from_utf8_lossy(new.body);
            let (line, x, y) = first_difference(&a, &b);
            out.push(
                format!("body:{line}"),
                Some(&Value::String(x)),
                Some(&Value::String(y)),
            );
        }
        _ => {}
    }
    Diff {
        diff: out.changes,
        more: out.more,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn json_side(body: &[u8]) -> Side<'_> {
        Side {
            status: Some(200),
            content_type: "application/json",
            body,
        }
    }

    fn run(old: Value, new: Value, ignore: &[&str]) -> Value {
        let (a, b) = (old.to_string(), new.to_string());
        let ignore: Vec<String> = ignore.iter().map(|s| s.to_string()).collect();
        serde_json::to_value(diff_responses(
            json_side(a.as_bytes()),
            json_side(b.as_bytes()),
            &ignore,
        ))
        .unwrap()
    }

    #[test]
    fn volatile_timestamp_gives_an_empty_diff() {
        let old = json!({"status": "ok", "timestamp": 1, "data": {"timestamp": "a"}});
        let new = json!({"status": "ok", "timestamp": 2, "data": {"timestamp": "b"}});
        assert_eq!(
            run(old.clone(), new.clone(), &["timestamp"]),
            json!({"diff": []})
        );
        assert_eq!(
            run(old, new, &[]),
            json!({"diff": [{"path": "$.timestamp", "from": 1, "to": 2},
                             {"path": "$.data.timestamp", "from": "a", "to": "b"}]})
        );
    }

    #[test]
    fn added_removed_changed_and_paths() {
        let old = json!({"a": 1, "gone": true, "list": [1, 2, 3], "weird key": 1,
                         "items": [{"id": 1, "at": "x"}, {"id": 2, "at": "y"}]});
        let new = json!({"a": 2, "new": null, "list": [1, 5], "weird key": 2,
                         "items": [{"id": 1, "at": "z"}, {"id": 2, "at": "w"}]});
        assert_eq!(
            run(old, new, &["$.items[*].at"]),
            json!({"diff": [
                {"path": "$.a", "from": 1, "to": 2},
                {"path": "$.gone", "from": true},
                {"path": "$.list[1]", "from": 2, "to": 5},
                {"path": "$.list[2]", "from": 3},
                {"path": "$[\"weird key\"]", "from": 1, "to": 2},
                {"path": "$.new", "to": null}
            ]})
        );
    }

    #[test]
    fn status_text_bodies_and_limits() {
        let diff = diff_responses(
            Side {
                status: Some(200),
                content_type: "text/plain",
                body: b"same\nold line",
            },
            Side {
                status: Some(500),
                content_type: "text/plain",
                body: b"same\nnew line",
            },
            &[],
        );
        assert_eq!(
            serde_json::to_value(diff).unwrap(),
            json!({"diff": [{"path": "status", "from": 200, "to": 500},
                             {"path": "body:2", "from": "old line", "to": "new line"}]})
        );
        let many_old = json!(
            (0..80)
                .map(|i| (format!("k{i}"), json!(i)))
                .collect::<serde_json::Map<_, _>>()
        );
        let many_new = json!(
            (0..80)
                .map(|i| (format!("k{i}"), json!(i + 1)))
                .collect::<serde_json::Map<_, _>>()
        );
        let diff = run(many_old, many_new, &[]);
        assert_eq!(diff["diff"].as_array().unwrap().len(), MAX_CHANGES);
        assert_eq!(diff["more"], 30);
        let long = run(json!({"x": "a".repeat(500)}), json!({"x": "b"}), &[]);
        assert!(
            long["diff"][0]["from"]
                .as_str()
                .unwrap()
                .ends_with("… (502 chars)")
        );
    }
}
