//! Secretos en el llavero (sección 19.4) con un llavero en memoria: las pruebas nunca tocan el
//! llavero real del sistema.

use std::sync::Arc;

use runpost_core::{
    Executor, RunContext, Source, Workspace,
    export::postman_v21::export_environment,
    model::{
        Auth, Collection, Environment, Globals, Item, Request, Scripts, SecretStore, Variable,
    },
    secrets::{Keyring, MemoryKeyring, account},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

const TOKEN: &str = "fake-keyring-token-0000";

fn keyring_secret(key: &str, value: &str) -> Variable {
    Variable {
        store: Some(SecretStore::Keyring),
        ..Variable::secret(key, value)
    }
}

fn open(keyring: &Arc<MemoryKeyring>) -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open_with_keyring(dir.path(), keyring.clone()).unwrap();
    (dir, ws)
}

fn env_file(ws: &Workspace) -> String {
    let dir = ws.root().join("environments");
    let entry = std::fs::read_dir(dir).unwrap().next().unwrap().unwrap();
    std::fs::read_to_string(entry.path()).unwrap()
}

fn var<'a>(env: &'a Environment, key: &str) -> &'a Variable {
    env.variables.iter().find(|v| v.key == key).unwrap()
}

/// Aceptación del sprint 6: con `accessToken` en el llavero, el archivo del entorno no contiene el
/// valor y la petición igual lleva el bearer correcto.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keyring_token_stays_out_of_the_file_and_reaches_the_server() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\":true}"))
        .expect(1)
        .mount(&server)
        .await;
    let keyring = Arc::new(MemoryKeyring::new());
    let (_dir, ws) = open(&keyring);
    let mut env = Environment::new("Acme DESA (fake)");
    env.variables = vec![
        Variable::new("apiUrl", server.uri()),
        keyring_secret("accessToken", TOKEN),
    ];
    ws.save_environment(&env)?;

    let text = env_file(&ws);
    assert!(!text.contains(TOKEN), "{text}");
    let on_disk: Environment = serde_json::from_str(&text)?;
    assert_eq!(var(&on_disk, "accessToken").value, None);
    assert_eq!(
        var(&on_disk, "accessToken").store,
        Some(SecretStore::Keyring)
    );
    assert_eq!(
        keyring.get(&account(&env.id, "accessToken"))?.as_deref(),
        Some(TOKEN)
    );
    // Al cargar, el valor vuelve del llavero (también desde otro `Workspace` sobre la misma carpeta).
    let reopened = Workspace::open_with_keyring(ws.root(), keyring.clone())?;
    let loaded = reopened.find_environment("Acme DESA (fake)")?;
    assert_eq!(var(&loaded, "accessToken").value.as_deref(), Some(TOKEN));

    let mut collection = Collection::new("Acme");
    collection.auth = Some(Auth::Bearer {
        token: "{{accessToken}}".into(),
    });
    let request = Request::new("user me", "GET", "{{apiUrl}}/v1/me");
    collection
        .items
        .push(Item::Request(Box::new(request.clone())));
    ws.save_collection(&collection)?;
    let mut ctx = RunContext::new(Source::Mcp);
    ctx.env_id = Some(env.id.clone());
    let result = Executor::new().run(&ws, &request, ctx).await?;
    server.verify().await;
    assert_eq!(result.response.status, 200);
    let entry = ws.history().get(&result.history_id)?.unwrap();
    assert!(!format!("{entry:?}").contains(TOKEN));
    assert!(!env_file(&ws).contains(TOKEN));
    Ok(())
}

#[test]
fn move_to_keyring_and_back_to_the_file() -> anyhow::Result<()> {
    let keyring = Arc::new(MemoryKeyring::new());
    let (_dir, ws) = open(&keyring);
    let mut env = Environment::new("desa");
    env.variables = vec![Variable::secret("accessToken", TOKEN)];
    ws.save_environment(&env)?;
    assert!(env_file(&ws).contains(TOKEN));
    let key = account(&env.id, "accessToken");

    // "Mover al llavero": la UI marca `store: keyring` y guarda.
    let mut loaded = ws.find_environment("desa")?;
    loaded.variables[0].store = Some(SecretStore::Keyring);
    ws.save_environment(&loaded)?;
    assert!(!env_file(&ws).contains(TOKEN));
    assert_eq!(keyring.get(&key)?.as_deref(), Some(TOKEN));

    // Guardar lo cargado (con el valor en memoria) no duplica nada en el archivo.
    let again = ws.find_environment("desa")?;
    ws.save_environment(&again)?;
    assert!(!env_file(&ws).contains(TOKEN));

    // Desmarcar "Secreto": vuelve al archivo y sale del llavero.
    let mut plain = ws.find_environment("desa")?;
    plain.variables[0].secret = false;
    ws.save_environment(&plain)?;
    let on_disk: Environment = serde_json::from_str(&env_file(&ws))?;
    assert_eq!(on_disk.variables[0].value.as_deref(), Some(TOKEN));
    assert_eq!(on_disk.variables[0].store, None);
    assert_eq!(keyring.get(&key)?, None);

    // Borrar el entorno borra sus secretos del llavero.
    let mut back = ws.find_environment("desa")?;
    back.variables[0].secret = true;
    back.variables[0].store = Some(SecretStore::Keyring);
    ws.save_environment(&back)?;
    assert!(keyring.get(&key)?.is_some());
    ws.delete_environment(&env.id)?;
    assert_eq!(keyring.get(&key)?, None);
    Ok(())
}

#[test]
fn globals_get_an_id_for_their_keyring_secrets() -> anyhow::Result<()> {
    let keyring = Arc::new(MemoryKeyring::new());
    let (_dir, ws) = open(&keyring);
    ws.save_globals(&Globals {
        variables: vec![
            Variable::new("scopeLabel", "x"),
            keyring_secret("apiKey", TOKEN),
        ],
        ..Globals::default()
    })?;
    let text = std::fs::read_to_string(ws.root().join("globals.json"))?;
    assert!(!text.contains(TOKEN), "{text}");
    let globals = ws.globals()?;
    assert!(globals.id.starts_with("glb_"));
    assert_eq!(globals.variables[1].value.as_deref(), Some(TOKEN));
    assert_eq!(
        keyring.get(&account(&globals.id, "apiKey"))?.as_deref(),
        Some(TOKEN)
    );
    // Sin secretos en el llavero, `globals.json` sigue sin `id`.
    let (_dir2, plain) = open(&keyring);
    plain.save_globals(&Globals {
        variables: vec![Variable::secret("t", "v")],
        ..Globals::default()
    })?;
    assert!(plain.globals()?.id.is_empty());
    Ok(())
}

#[test]
fn export_strip_secrets_empties_keyring_secrets_too() -> anyhow::Result<()> {
    let keyring = Arc::new(MemoryKeyring::new());
    let (_dir, ws) = open(&keyring);
    let mut env = Environment::new("desa");
    env.variables = vec![
        keyring_secret("accessToken", TOKEN),
        Variable::secret("accountPassphrase", "fake-passphrase"),
    ];
    ws.save_environment(&env)?;
    let loaded = ws.find_environment("desa")?;
    let full = export_environment(&loaded, false).to_string();
    assert!(full.contains(TOKEN) && full.contains("fake-passphrase"));
    let stripped = export_environment(&loaded, true).to_string();
    assert!(!stripped.contains(TOKEN) && !stripped.contains("fake-passphrase"));
    Ok(())
}

#[test]
fn reimport_keeps_secrets_in_the_keyring() -> anyhow::Result<()> {
    let keyring = Arc::new(MemoryKeyring::new());
    let (_dir, ws) = open(&keyring);
    let postman = serde_json::json!({
        "id": "5c4f0000-0000-4000-8000-000000000001", "name": "Acme DESA (fake)",
        "values": [{"key": "accessToken", "value": TOKEN, "type": "secret", "enabled": true}],
        "_postman_variable_scope": "environment"
    })
    .to_string();
    let report = ws.import_postman(&postman, None, None)?;
    assert!(env_file(&ws).contains(TOKEN));
    let mut env = ws.find_environment(&report.id)?;
    env.variables[0].store = Some(SecretStore::Keyring);
    ws.save_environment(&env)?;
    ws.import_postman(&postman, None, None)?;
    assert!(!env_file(&ws).contains(TOKEN));
    let env = ws.find_environment(&report.id)?;
    assert_eq!(env.variables[0].store, Some(SecretStore::Keyring));
    assert_eq!(env.variables[0].value.as_deref(), Some(TOKEN));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scripts_rotate_a_keyring_secret_in_the_keyring() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let keyring = Arc::new(MemoryKeyring::new());
    let (_dir, ws) = open(&keyring);
    let mut env = Environment::new("desa");
    env.variables = vec![
        Variable::new("base", server.uri()),
        keyring_secret("accessToken", TOKEN),
    ];
    ws.save_environment(&env)?;
    let mut collection = Collection::new("c");
    let mut request = Request::new("r", "GET", "{{base}}/x");
    request.scripts = Scripts {
        pre: String::new(),
        test: "pm.environment.set('accessToken', 'fake-rotated-token');".into(),
    };
    collection
        .items
        .push(Item::Request(Box::new(request.clone())));
    ws.save_collection(&collection)?;
    let mut ctx = RunContext::new(Source::Ui);
    ctx.env_id = Some(env.id.clone());
    Executor::new().run(&ws, &request, ctx).await?;
    let text = env_file(&ws);
    assert!(
        !text.contains("fake-rotated-token") && !text.contains(TOKEN),
        "{text}"
    );
    assert_eq!(
        keyring.get(&account(&env.id, "accessToken"))?.as_deref(),
        Some("fake-rotated-token")
    );
    Ok(())
}
