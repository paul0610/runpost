//! Archivos de datos del runner (`pm.iterationData`): CSV con cabecera o JSON (array de objetos),
//! como el Collection Runner de Postman.

use anyhow::{Result, bail};
use serde_json::Value;

use crate::vars::Vars;

/// Lee el archivo según la extensión; sin `.csv` ni `.json`, mira si el texto empieza por `[`.
pub fn parse(name: &str, text: &str) -> Result<Vec<Vars>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".json") || (!lower.ends_with(".csv") && text.trim_start().starts_with('['))
    {
        parse_json(text)
    } else {
        parse_csv(text)
    }
}

/// Array de objetos. Los valores que no son texto se guardan como JSON (`1`, `true`, `{"a":1}`).
pub fn parse_json(text: &str) -> Result<Vec<Vars>> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| anyhow::anyhow!("invalid JSON data file: {e}"))?;
    let Value::Array(rows) = value else {
        bail!("the JSON data file must be an array of objects");
    };
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| match row {
            Value::Object(map) => Ok(map
                .into_iter()
                .map(|(key, value)| {
                    let text = match value {
                        Value::String(text) => text,
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    (key, text)
                })
                .collect()),
            _ => bail!("row {} of the JSON data file is not an object", index + 1),
        })
        .collect()
}

/// CSV (RFC 4180): primera fila de cabeceras, comillas dobles con `""` como escape, saltos de
/// línea dentro de comillas, CRLF o LF. Las filas vacías se ignoran; los campos que faltan quedan
/// vacíos y los que sobran se descartan.
pub fn parse_csv(text: &str) -> Result<Vec<Vars>> {
    let records = records(text)?;
    let mut records = records
        .into_iter()
        .filter(|r| !(r.len() == 1 && r[0].is_empty()));
    let Some(header) = records.next() else {
        return Ok(Vec::new());
    };
    let header: Vec<String> = header.into_iter().map(|h| h.trim().to_owned()).collect();
    if header.iter().all(String::is_empty) {
        bail!("the CSV data file needs a header row");
    }
    Ok(records
        .map(|record| {
            header
                .iter()
                .enumerate()
                .filter(|(_, key)| !key.is_empty())
                .map(|(i, key)| (key.clone(), record.get(i).cloned().unwrap_or_default()))
                .collect()
        })
        .collect())
}

fn records(text: &str) -> Result<Vec<Vec<String>>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut line = 1;
    while let Some(ch) = chars.next() {
        if quoted {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    field.push(ch);
                }
                _ => field.push(ch),
            }
            continue;
        }
        match ch {
            '"' if field.is_empty() => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                line += 1;
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(ch),
        }
    }
    if quoted {
        bail!("unterminated quote in the CSV data file (line {line})");
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(data: &[Vars]) -> Vec<Vec<(&str, &str)>> {
        data.iter()
            .map(|row| row.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect())
            .collect()
    }

    #[test]
    fn csv_with_quotes_newlines_bom_and_crlf() {
        let text = "\u{feff}scopeLabel,token,note\r\nview,\"v2,abc\",\"say \"\"hi\"\"\"\r\n\r\nspend,x,\"two\nlines\"\nshort\n";
        let data = parse("tokens-matriz.csv", text).unwrap();
        assert_eq!(
            rows(&data),
            vec![
                vec![
                    ("note", "say \"hi\""),
                    ("scopeLabel", "view"),
                    ("token", "v2,abc")
                ],
                vec![
                    ("note", "two\nlines"),
                    ("scopeLabel", "spend"),
                    ("token", "x")
                ],
                vec![("note", ""), ("scopeLabel", "short"), ("token", "")],
            ]
        );
        assert!(parse("x.csv", "a,b\n\"open").is_err());
        assert!(parse("x.csv", "").unwrap().is_empty());
    }

    #[test]
    fn json_array_of_objects() {
        let data = parse(
            "data.json",
            r#"[{"a":"x","n":1,"b":true,"o":{"k":1},"z":null}]"#,
        )
        .unwrap();
        assert_eq!(
            rows(&data),
            vec![vec![
                ("a", "x"),
                ("b", "true"),
                ("n", "1"),
                ("o", r#"{"k":1}"#),
                ("z", "")
            ]]
        );
        assert!(parse("data.json", r#"{"a":1}"#).is_err());
        assert!(parse("data.json", r#"[1]"#).is_err());
        // Sin extensión conocida decide el contenido.
        assert_eq!(parse("pegado", "[{\"a\":\"1\"}]").unwrap().len(), 1);
        assert_eq!(parse("pegado", "a\n1\n2").unwrap().len(), 2);
    }
}
