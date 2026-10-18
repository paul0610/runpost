//! Aceptación S5 del CLI: `runpost run` sobre la colección Acme contra wiremock.

mod common;

use std::{collections::BTreeMap, path::Path, process::Command};

use common::{ENV, FAKE_TOKEN, acme_mock, acme_workspace};
use quick_xml::{Reader, events::Event};
use runpost_core::{Workspace, secrets::KEYRING_ENV};
use serde_json::Value;

fn point_environment_to(dir: &Path, base: &str) {
    let ws = Workspace::open(dir).unwrap();
    let mut env = ws.find_environment(ENV).unwrap();
    for var in &mut env.variables {
        if var.key == "gatewayUrl" || var.key == "apiUrl" {
            var.value = Some(base.to_owned());
        }
    }
    ws.save_environment(&env).unwrap();
}

fn runpost(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
        .env(KEYRING_ENV, "memory")
        .arg("--workspace")
        .arg(dir)
        .arg("run")
        .args(args)
        .output()
        .unwrap()
}

/// Recorre el XML entero (etiquetas equilibradas, atributos bien formados) y cuenta etiquetas.
fn parse_xml(xml: &str) -> BTreeMap<String, usize> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut counts = BTreeMap::new();
    let mut open = 0i32;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(tag)) | Ok(Event::Empty(tag)) => {
                for attribute in tag.attributes().with_checks(true) {
                    attribute.expect("well-formed attribute");
                }
                let name = tag.name().into_inner().to_owned();
                *counts.entry(name).or_insert(0) += 1;
            }
            Ok(Event::End(_)) => open -= 1,
            Ok(_) => {}
            Err(error) => panic!("invalid XML at byte {}: {error}", reader.buffer_position()),
        }
    }
    assert!(open <= 0);
    counts
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn junit_is_valid_xml_and_exit_code_is_one_when_a_test_fails() {
    let server = acme_mock().await;
    let dir = acme_workspace();
    point_environment_to(dir.path(), &server.uri());

    // El comando de la aceptación del sprint 5, tal cual.
    let output = runpost(
        dir.path(),
        &["Acme Gateway Scopes", "--env", "fake", "--out", "junit"],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let xml = String::from_utf8(output.stdout).unwrap();
    let counts = parse_xml(&xml);
    assert_eq!(counts.get("testsuites"), Some(&1));
    assert_eq!(counts.get("testsuite"), Some(&10));
    assert_eq!(counts.get("failure"), Some(&3));
    assert_eq!(counts.get("skipped"), Some(&1));
    assert!(xml.contains("failures=\"3\" errors=\"0\""), "{xml}");
    assert!(!xml.contains(FAKE_TOKEN));

    // A archivo, sin nada en stdout.
    let file = dir.path().join("informe.xml");
    let output = runpost(
        dir.path(),
        &[
            "Acme Gateway Scopes",
            "--env",
            "fake",
            "--out",
            "junit",
            "-o",
            file.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    parse_xml(&std::fs::read_to_string(&file).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_and_pretty_exit_zero_when_everything_passes() {
    let server = acme_mock().await;
    let dir = acme_workspace();
    point_environment_to(dir.path(), &server.uri());

    let output = runpost(
        dir.path(),
        &[
            "acme-gateway-scopes",
            "--folder",
            "00 - Diagnóstico",
            "--out",
            "json",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["totals"]["tests"], 3);
    assert_eq!(report["totals"]["failed"], 0);
    assert_eq!(report["folder"]["name"], "00 - Diagnóstico");
    let console = report["items"][0]["console"][0].as_str().unwrap();
    assert!(console.starts_with("[fake-scope] scopes: "), "{console}");

    let output = runpost(
        dir.path(),
        &["Acme Gateway Scopes", "--folder", "00", "--iterations", "2"],
    );
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with(
            "Acme Gateway - Validación de scopes / 00 - Diagnóstico · entorno Acme DESA (fake) · 2 iteraciones\n"
        ),
        "{text}"
    );
    assert!(text.contains("\nIteración 2\n"));
    assert!(text.contains("↷ GET  00 - Diagnóstico/00.2 Gateway - ping  omitida"));
}

#[test]
fn bad_selectors_and_data_exit_two() {
    let dir = acme_workspace();
    let output = runpost(
        dir.path(),
        &["Acme Gateway Scopes", "--folder", "no existe"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "runpost: folder not found: no existe"
    );
    let bad = dir.path().join("datos.json");
    std::fs::write(&bad, "{\"no\": \"array\"}").unwrap();
    let output = runpost(
        dir.path(),
        &["Acme Gateway Scopes", "--data", bad.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("must be an array of objects")
    );
}
