//! `runpost import` / `runpost export` de punta a punta con el binario real.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use runpost_core::secrets::KEYRING_ENV;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn runpost(workspace: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
        .env(KEYRING_ENV, "memory")
        .arg("--workspace")
        .arg(workspace)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn import_export_reimport_round_trip() {
    let first = tempfile::tempdir().unwrap();
    let collection = fixture("Acme-Gateway-Scopes.postman_collection.json");
    let output = runpost(first.path(), &["import", collection.to_str().unwrap()]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("3 carpetas y 10 peticiones"), "{stdout}");
    assert!(stdout.contains("collections/acme-gateway-scopes.collection.json"));
    let native = first
        .path()
        .join("collections/acme-gateway-scopes.collection.json");
    assert!(native.is_file());

    let env = fixture("Acme-DESA-fake.postman_environment.json");
    let output = runpost(
        first.path(),
        &["import", env.to_str().unwrap(), "--kind", "environment"],
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("11 variables")
    );

    let exported = first.path().join("export.postman_collection.json");
    runpost(
        first.path(),
        &[
            "export",
            "acme-gateway-scopes",
            "--format",
            "postman_v21",
            "--strip-secrets",
            "-o",
            exported.to_str().unwrap(),
        ],
    );
    let stdout = runpost(
        first.path(),
        &["export", "Acme Gateway - Validación de scopes"],
    )
    .stdout;
    assert_eq!(
        serde_json::from_slice::<Value>(&stdout).unwrap(),
        read_json(&exported)
    );

    let second = tempfile::tempdir().unwrap();
    runpost(second.path(), &["import", exported.to_str().unwrap()]);
    let reimported = second.path().join("collections/export.collection.json");
    assert_eq!(read_json(&reimported), read_json(&native));
}

#[test]
fn import_bruno_folders_and_insomnia_files() {
    let workspace = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let bruno = source.path().join("acme-bruno");
    std::fs::create_dir_all(bruno.join("Accounts")).unwrap();
    std::fs::create_dir_all(bruno.join("environments")).unwrap();
    std::fs::write(
        bruno.join("bruno.json"),
        r#"{"version": "1", "name": "Acme Bruno", "type": "collection"}"#,
    )
    .unwrap();
    std::fs::write(
        bruno.join("Accounts/Sign.bru"),
        "meta {\n  name: Sign\n  seq: 1\n}\n\npost {\n  url: {{apiUrl}}/v1/usd/accounts/acc_1/sign\n  body: json\n  auth: inherit\n}\n\nbody:json {\n  {}\n}\n\nscript:pre-request {\n  bru.setVar(\"a\", 1);\n}\n",
    )
    .unwrap();
    std::fs::write(
        bruno.join("environments/Local.bru"),
        "vars {\n  apiUrl: http://localhost:3080\n}\nvars:secret [\n  accessToken\n]\n",
    )
    .unwrap();
    let output = runpost(workspace.path(), &["import", bruno.to_str().unwrap()]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("colección importada [bruno]: \"Acme Bruno\""),
        "{stdout}"
    );
    assert!(stdout.contains("con 1 carpetas y 1 peticiones"), "{stdout}");
    assert!(stdout.contains("entorno importado [bruno]: \"Acme Bruno - Local\""));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Bruno scripts kept as comments"),
        "{stderr}"
    );

    // Reimportar la carpeta la sustituye.
    let output = runpost(workspace.path(), &["import", bruno.to_str().unwrap()]);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("colección reemplazada [bruno]")
    );

    let insomnia = source.path().join("acme.insomnia.yaml");
    std::fs::write(
        &insomnia,
        "type: collection.insomnia.rest/5.0\nname: Acme Insomnia\nmeta:\n  id: wrk_cli\ncollection:\n  - url: \"{{ _.apiUrl }}/v1/ping\"\n    name: Ping\n    meta:\n      id: req_cli\n    method: GET\n",
    )
    .unwrap();
    let output = runpost(workspace.path(), &["import", insomnia.to_str().unwrap()]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("colección importada [insomnia]: \"Acme Insomnia\""),
        "{stdout}"
    );
}

#[test]
fn export_unknown_collection_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
        .env(KEYRING_ENV, "memory")
        .arg("--workspace")
        .arg(dir.path())
        .args(["export", "nope"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("collection not found: nope"));
}
