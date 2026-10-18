//! Aceptación S1: importar la colección Acme real (fixture) y su entorno falso.

use std::path::PathBuf;

use runpost_core::{
    Workspace,
    export::postman_v21::{export_collection, export_environment},
    import::postman_v21::{PostmanKind, import_collection, import_environment},
    model::{Auth, Body, Item, RawLang, SecretStore},
};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

const COLLECTION: &str = "Acme-Gateway-Scopes.postman_collection.json";
const ENVIRONMENT: &str = "Acme-DESA-fake.postman_environment.json";

fn original() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture(COLLECTION)).unwrap()).unwrap()
}

/// Scripts del Postman original, por nombre de petición: (pre, test).
fn postman_scripts() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for folder in original()["item"].as_array().unwrap() {
        for item in folder["item"].as_array().unwrap() {
            let mut pre = String::new();
            let mut test = String::new();
            for event in item["event"].as_array().into_iter().flatten() {
                let code = event["script"]["exec"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| l.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("\n");
                match event["listen"].as_str().unwrap() {
                    "prerequest" => pre = code,
                    "test" => test = code,
                    _ => {}
                }
            }
            out.push((item["name"].as_str().unwrap().to_owned(), pre, test));
        }
    }
    out
}

#[test]
fn imports_acme_collection_into_workspace() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let ws = Workspace::open(dir.path())?;
    let report = ws.import_postman_file(&fixture(COLLECTION), None)?;
    assert_eq!(report.kind, PostmanKind::Collection);
    assert_eq!((report.folders, report.requests), (3, 10));
    assert_eq!(
        report.file,
        "collections/acme-gateway-scopes.collection.json"
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(!report.replaced);
    assert!(ws.root().join(&report.file).is_file());

    let collection = ws.load_collection(&report.id)?;
    assert_eq!(collection.id, "col_000000000000400080000000000000c1");
    assert_eq!(collection.name, "Acme Gateway - Validación de scopes");
    let folders: Vec<_> = collection
        .items
        .iter()
        .map(|i| i.name().to_owned())
        .collect();
    assert_eq!(
        folders,
        [
            "00 - Diagnóstico",
            "01 - Matriz de scopes (token bajo prueba)",
            "02 - Controles negativos (sin token / token inválido)"
        ]
    );

    // Scripts pre/test preservados línea a línea.
    let mut imported = Vec::new();
    collection.walk_requests(|_, r| {
        imported.push((
            r.name.clone(),
            r.scripts.pre.clone(),
            r.scripts.test.clone(),
        ))
    });
    assert_eq!(imported, postman_scripts());
    assert_eq!(
        imported
            .iter()
            .filter(|(_, pre, _)| !pre.is_empty())
            .count(),
        6
    );
    assert!(imported.iter().all(|(_, _, test)| !test.is_empty()));

    let (_, ping) = ws.find_request(&"00.2 Gateway - ping".into())?;
    assert_eq!(ping.url, "{{gatewayUrl}}/v1/ping");
    assert_eq!(ping.auth, Auth::None);
    assert!(ping.scripts.pre.contains("pm.execution.skipRequest()"));
    let (_, sign) = ws.find_request(
        &"Acme Gateway - Validación de scopes/01 - Matriz de scopes (token bajo prueba)/#7b".into(),
    )?;
    assert_eq!(sign.method, "POST");
    let Body::Raw { lang, text } = &sign.body else {
        panic!("raw body")
    };
    assert_eq!(*lang, RawLang::Json);
    assert!(text.contains("{{draftJson}}"));
    assert_eq!(sign.headers[1].value, "Bearer {{accessToken}}");

    // El archivo en disco es el formato nativo legible.
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(ws.root().join(&report.file))?)?;
    assert_eq!(on_disk["schema"], 1);
    assert_eq!(on_disk["items"][0]["type"], "folder");
    Ok(())
}

#[test]
fn export_and_reimport_gives_the_same_tree() -> anyhow::Result<()> {
    let first = import_collection(&std::fs::read_to_string(fixture(COLLECTION))?)?;
    let exported = export_collection(&first.collection, true);
    let again = import_collection(&exported.to_string())?;
    assert_eq!(again.collection, first.collection);
    // Y una segunda vuelta produce exactamente el mismo JSON Postman.
    assert_eq!(export_collection(&again.collection, true), exported);
    assert_eq!(
        exported["info"]["_postman_id"],
        original()["info"]["_postman_id"]
    );
    Ok(())
}

#[test]
fn reimport_replaces_and_keeps_ids() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let ws = Workspace::open(dir.path())?;
    let first = ws.import_postman_file(&fixture(COLLECTION), Some(PostmanKind::Collection))?;
    let before = ws.load_collection(&first.id)?;
    let second = ws.import_postman_file(&fixture(COLLECTION), None)?;
    assert!(second.replaced);
    assert_eq!(second.file, first.file);
    assert_eq!(ws.collections()?.len(), 1);
    assert_eq!(ws.load_collection(&first.id)?, before);
    Ok(())
}

#[test]
fn imports_fake_environment_with_secrets() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let ws = Workspace::open(dir.path())?;
    let report = ws.import_postman_file(&fixture(ENVIRONMENT), None)?;
    assert_eq!(report.kind, PostmanKind::Environment);
    assert_eq!(report.file, "environments/acme-desa-fake.env.json");
    assert_eq!(report.variables, 11);
    let env = ws.find_environment("Acme DESA (fake)")?;
    let token = env
        .variables
        .iter()
        .find(|v| v.key == "accessToken")
        .unwrap();
    assert!(token.secret);
    assert_eq!(token.store, Some(SecretStore::File));
    let secrets: Vec<_> = env
        .variables
        .iter()
        .filter(|v| v.secret)
        .map(|v| v.key.as_str())
        .collect();
    assert_eq!(
        secrets,
        ["accessToken", "userEncryptedKey", "accountPassphrase"]
    );

    let text = std::fs::read_to_string(fixture(ENVIRONMENT))?;
    let stripped = export_environment(&import_environment(&text)?, true);
    assert!(!stripped.to_string().contains("fake-access-token"));
    assert_eq!(
        import_environment(&export_environment(&env, false).to_string())?,
        env
    );
    Ok(())
}

#[test]
fn every_folder_and_request_has_a_stable_unique_id() -> anyhow::Result<()> {
    let first = import_collection(&std::fs::read_to_string(fixture(COLLECTION))?)?.collection;
    let mut ids = Vec::new();
    for item in &first.items {
        ids.push(item.id().to_owned());
        if let Item::Folder(folder) = item {
            ids.extend(folder.items.iter().map(|i| i.id().to_owned()));
        }
    }
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 13);
    Ok(())
}
