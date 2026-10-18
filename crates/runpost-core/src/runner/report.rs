//! Salidas del informe del runner: JUnit XML (CI), texto para la terminal y HTML autónomo.

use std::fmt::Write as _;

use super::{RunnerItem, RunnerReport};

fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 no admite la mayoría de caracteres de control.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

fn seconds(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// JUnit como el reporter de newman: una `testsuite` por petición e iteración y un `testcase` por
/// `pm.test`. Un error de red es `<error>`; una petición omitida, un caso `<skipped/>`.
pub fn to_junit(report: &RunnerReport) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<testsuites name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" time=\"{}\">",
        xml(&report.collection.name),
        report.totals.tests + report.totals.errors + report.totals.skipped,
        report.totals.failed,
        report.totals.errors,
        seconds(report.ms)
    );
    for item in &report.items {
        let suite = if report.iterations > 1 {
            format!("{} (iteración {})", item.path, item.iteration + 1)
        } else {
            item.path.clone()
        };
        let classname = format!("{}.{}", report.collection.name, item.path.replace('/', "."));
        let cases = item.tests.len().max(1);
        let failures = item.failed_tests().count();
        let skipped = item.tests.iter().filter(|t| t.skipped).count() + usize::from(item.skipped);
        let _ = writeln!(
            out,
            "  <testsuite name=\"{}\" id=\"{}\" timestamp=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\">",
            xml(&suite),
            xml(&item.id),
            xml(&report.started_at),
            cases,
            failures,
            usize::from(item.error.is_some()),
            skipped,
            seconds(item.ms)
        );
        if let Some(error) = &item.error {
            let _ = writeln!(
                out,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\">\n      <error type=\"RequestError\" message=\"{}\">{}</error>\n    </testcase>",
                xml(&format!("{} {}", item.method, item.url)),
                xml(&classname),
                seconds(item.ms),
                xml(error),
                xml(error)
            );
        } else if item.skipped && item.tests.is_empty() {
            let _ = writeln!(
                out,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"0.000\">\n      <skipped message=\"pm.execution.skipRequest()\"/>\n    </testcase>",
                xml(&item.name),
                xml(&classname)
            );
        }
        for test in &item.tests {
            let _ = write!(
                out,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\"",
                xml(&test.name),
                xml(&classname),
                seconds(item.ms)
            );
            if test.skipped {
                out.push_str(">\n      <skipped/>\n    </testcase>\n");
            } else if test.passed {
                out.push_str("/>\n");
            } else {
                let msg = test.msg.clone().unwrap_or_default();
                let kind = msg
                    .split(':')
                    .next()
                    .filter(|k| !k.contains(' '))
                    .unwrap_or("AssertionError");
                let _ = writeln!(
                    out,
                    ">\n      <failure type=\"{}\" message=\"{}\">{}</failure>\n    </testcase>",
                    xml(kind),
                    xml(&msg),
                    xml(&msg)
                );
            }
        }
        if item.error.is_none() && !item.skipped && item.tests.is_empty() {
            // Sin tests: un caso que solo dice que hubo respuesta, para que el XML no quede vacío.
            let _ = writeln!(
                out,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\"/>",
                xml(&format!(
                    "{} {} → {}",
                    item.method,
                    item.name,
                    item.status.unwrap_or(0)
                )),
                xml(&classname),
                seconds(item.ms)
            );
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn item_status(item: &RunnerItem) -> String {
    if item.skipped {
        "omitida".into()
    } else if let Some(error) = &item.error {
        error.clone()
    } else {
        format!("{} · {} ms", item.status.unwrap_or(0), item.ms)
    }
}

/// Texto para la terminal (`runpost run --out pretty`).
pub fn to_pretty(report: &RunnerReport) -> String {
    let mut out = String::new();
    let scope = match &report.folder {
        Some(folder) => format!("{} / {}", report.collection.name, folder.name),
        None => report.collection.name.clone(),
    };
    let env = report
        .environment
        .as_deref()
        .map(|e| format!(" · entorno {e}"))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "{scope}{env} · {}",
        plural(report.iterations as usize, "iteración", "iteraciones")
    );
    let mut iteration = None;
    for item in &report.items {
        if report.iterations > 1 && iteration != Some(item.iteration) {
            iteration = Some(item.iteration);
            let _ = writeln!(out, "\nIteración {}", item.iteration + 1);
        }
        let mark = if item.skipped {
            "↷"
        } else if item.failed() {
            "✘"
        } else {
            "✔"
        };
        let _ = writeln!(
            out,
            "  {mark} {}  {}  {}",
            item.method,
            item.path,
            item_status(item)
        );
        for test in &item.tests {
            let mark = if test.skipped {
                "-"
            } else if test.passed {
                "✔"
            } else {
                "✘"
            };
            let _ = writeln!(out, "      {mark} {}", test.name);
            if !test.passed
                && !test.skipped
                && let Some(msg) = &test.msg
            {
                let _ = writeln!(out, "          {msg}");
            }
        }
    }
    let t = &report.totals;
    let mut requests = plural(t.requests, "petición", "peticiones");
    let mut extras = Vec::new();
    if t.skipped > 0 {
        extras.push(plural(t.skipped, "omitida", "omitidas"));
    }
    if t.errors > 0 {
        extras.push(plural(t.errors, "sin respuesta", "sin respuesta"));
    }
    if !extras.is_empty() {
        requests = format!("{requests} ({})", extras.join(", "));
    }
    let _ = writeln!(
        out,
        "\n{requests} · tests: {} ✔ {} ✘ · {} ms{}",
        t.passed,
        t.failed,
        report.ms,
        if report.cancelled {
            " · cancelado"
        } else {
            ""
        }
    );
    out
}

fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Informe HTML autónomo (sin scripts ni recursos externos) para "Exportar informe".
pub fn to_html(report: &RunnerReport) -> String {
    let t = &report.totals;
    let title = match &report.folder {
        Some(folder) => format!("{} / {}", report.collection.name, folder.name),
        None => report.collection.name.clone(),
    };
    let mut rows = String::new();
    for item in &report.items {
        let (class, status) = if item.skipped {
            ("skip", "omitida".to_owned())
        } else if let Some(error) = &item.error {
            ("fail", html(error))
        } else {
            (
                if item.failed() { "fail" } else { "ok" },
                item.status.unwrap_or(0).to_string(),
            )
        };
        let passed = item.tests.iter().filter(|t| t.passed && !t.skipped).count();
        let counted = item.tests.iter().filter(|t| !t.skipped).count();
        let mut tests = String::new();
        for test in &item.tests {
            let (mark, cls) = if test.skipped {
                ("–", "skip")
            } else if test.passed {
                ("✔", "ok")
            } else {
                ("✘", "fail")
            };
            let _ = write!(tests, "<li class=\"{cls}\">{mark} {}", html(&test.name));
            if !test.passed
                && let Some(msg) = &test.msg
            {
                let _ = write!(tests, "<div class=\"msg\">{}</div>", html(msg));
            }
            tests.push_str("</li>");
        }
        let _ = write!(
            rows,
            "<tr class=\"{class}\"><td>{}</td><td><b>{}</b> {}<div class=\"url\">{}</div>{}</td><td>{status}</td><td>{}</td><td>{}</td></tr>",
            item.iteration + 1,
            html(&item.method),
            html(&item.path),
            html(&item.url),
            if tests.is_empty() {
                String::new()
            } else {
                format!("<ul>{tests}</ul>")
            },
            item.ms,
            if counted == 0 {
                "—".to_owned()
            } else {
                format!("{passed}/{counted}")
            }
        );
    }
    format!(
        r#"<!doctype html>
<html lang="es">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Runner · {title}</title>
<style>
  body {{ font: 14px/1.45 "Segoe UI", system-ui, sans-serif; margin: 24px; color: #1b1e23; background: #fff; }}
  h1 {{ font-size: 18px; margin: 0 0 4px; }}
  .meta {{ color: #646b78; margin-bottom: 16px; }}
  .totals span {{ display: inline-block; margin-right: 16px; }}
  table {{ border-collapse: collapse; width: 100%; margin-top: 16px; }}
  th, td {{ text-align: left; padding: 6px 8px; border-bottom: 1px solid #e5e7eb; vertical-align: top; }}
  th {{ color: #646b78; font-weight: normal; font-size: 12px; text-transform: uppercase; }}
  .url {{ font: 12px Consolas, monospace; color: #646b78; word-break: break-all; }}
  ul {{ margin: 6px 0 0; padding-left: 0; list-style: none; }}
  .ok {{ color: #107b38; }} .fail {{ color: #ce2323; }} .skip {{ color: #646b78; }}
  tr.ok, tr.fail, tr.skip {{ color: inherit; }}
  tr.fail td:nth-child(3) {{ color: #ce2323; }} tr.ok td:nth-child(3) {{ color: #107b38; }}
  .msg {{ font: 12px Consolas, monospace; color: #1b1e23; margin-left: 18px; white-space: pre-wrap; }}
</style>
</head>
<body>
<h1>Runner · {title}</h1>
<div class="meta">{env}{iterations} · {started} · {ms} ms{cancelled}</div>
<div class="totals"><span>Peticiones: <b>{requests}</b></span><span class="ok">Tests ✔ <b>{passed}</b></span><span class="fail">Tests ✘ <b>{failed}</b></span><span>Sin respuesta: <b>{errors}</b></span><span>Omitidas: <b>{skipped}</b></span></div>
<table>
<thead><tr><th>#</th><th>Petición</th><th>Estado</th><th>ms</th><th>Tests</th></tr></thead>
<tbody>{rows}</tbody>
</table>
</body>
</html>
"#,
        title = html(&title),
        env = report
            .environment
            .as_deref()
            .map(|e| format!("Entorno {} · ", html(e)))
            .unwrap_or_default(),
        iterations = plural(report.iterations as usize, "iteración", "iteraciones"),
        started = html(&report.started_at),
        ms = report.ms,
        cancelled = if report.cancelled {
            " · cancelado"
        } else {
            ""
        },
        requests = t.requests,
        passed = t.passed,
        failed = t.failed,
        errors = t.errors,
        skipped = t.skipped,
    )
}
