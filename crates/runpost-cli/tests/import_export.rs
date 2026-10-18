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
