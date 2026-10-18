//! Integración: ejecución real contra `wiremock`, sin red externa.

use std::path::PathBuf;

use runpost_core::{
    Executor, NetworkError, RequestDraft, RunContext, Source, Workspace,
    model::{
        Auth, Body, Collection, Environment, Folder, Item, KeyValue, RawLang, Request, Scripts,
        Variable,
    },
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

// Sin `+`: en una query cruda el servidor lo leería como espacio (igual que con Postman).
const SECRET: &str = "s3cr3t-T0ken/with~chars";

fn workspace() -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    (dir, ws)
}

/// Colección con `{{base}}` y `{{token}}` (secreto) en el entorno activo.
fn setup(ws: &Workspace, base: &str) -> (Collection, Request) {
    let mut env = Environment::new("test");
    env.variables = vec![
        Variable::new("base", base),
        Variable::secret("token", SECRET),
    ];
    ws.save_environment(&env).unwrap();
    let mut settings = ws.settings();
    settings.active_environment = Some(env.id.clone());
    ws.save_settings(&settings).unwrap();

    let mut collection = Collection::new("Mock API");
    collection.auth = Some(Auth::Bearer {
        token: "{{token}}".into(),
    });
    collection.variables = vec![Variable::new("who", "collection")];
    let mut request = Request::new("ping", "POST", "{{base}}/ping?token={{token}}");
    request.body = Body::Raw {
        lang: RawLang::Json,
        text: r#"{"hello":"{{who}}","n":1}"#.into(),
    };
    let mut folder = Folder::new("Folder");
    folder.items.push(Item::Request(Box::new(request.clone())));
    collection.items.push(Item::Folder(folder));
    ws.save_collection(&collection).unwrap();
    (collection, request)
}

#[tokio::test]
async fn bearer_secret_reaches_mock_and_history_is_masked() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ping"))
        .and(header("authorization", format!("Bearer {SECRET}").as_str()))
        .and(query_param("token", SECRET))
        .and(body_json(json!({"hello": "collection", "n": 1})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "sid=abc; Path=/; HttpOnly")
                .set_body_json(json!({"pong": true, "echo": SECRET})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    let (_, request) = setup(&ws, &server.uri());

    let (_, saved) = ws.find_request(&"Mock API/Folder/ping".into())?;
    assert_eq!(saved.id, request.id);
    let result = Executor::new()
        .run(&ws, &saved, RunContext::new(Source::Cli))
        .await?;
    server.verify().await;

    assert_eq!(result.response.status, 200);
    assert_eq!(result.response.content_type(), Some("application/json"));
    assert_eq!(result.response.cookies[0].name, "sid");
    assert!(result.response.cookies[0].http_only);
    assert!(result.tests.is_empty() && result.vars_changed.is_empty());
    // Lo resuelto que ve la UI/MCP va enmascarado.
    assert!(
        result.resolved.url.ends_with("/ping?token=***"),
        "{}",
        result.resolved.url
    );
    assert!(
        result
            .resolved
            .headers
            .contains(&("Authorization".into(), "Bearer ***".into()))
    );
    assert_eq!(
        result
            .masker
            .mask(&String::from_utf8_lossy(&result.response.body)),
        r#"{"pong":true,"echo":"***"}"#
    );

    let entry = ws.history().get(&result.history_id)?.expect("history row");
    assert_eq!(entry.url, format!("{}/ping?token=***", server.uri()));
    assert_eq!(entry.method, "POST");
    assert_eq!(entry.status, Some(200));
    assert_eq!(entry.source, Source::Cli);
    assert_eq!(entry.request_id.as_deref(), Some(request.id.as_str()));
    assert!(
        entry
            .req_headers
            .contains(&("Authorization".into(), "Bearer ***".into()))
    );
    assert_eq!(
        entry.req_body.as_deref(),
        Some(br#"{"hello":"collection","n":1}"#.as_slice())
    );
    assert_eq!(
        entry.res_body.as_deref(),
        Some(br#"{"pong":true,"echo":"***"}"#.as_slice())
    );

    // El secreto no aparece en ninguna parte de la base de datos (ni codificado).
    drop(ws);
    let encoded: String = url::form_urlencoded::byte_serialize(SECRET.as_bytes()).collect();
    for file in ["history.db", "history.db-wal"] {
        let path = _dir.path().join(file);
        if let Ok(bytes) = std::fs::read(&path) {
            for needle in [SECRET, encoded.as_str()] {
                assert!(
                    !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                    "{needle} leaked into {file}"
                );
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn overrides_beat_environment_and_history_can_be_skipped() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/items"))
        .and(header("x-who", "override"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    setup(&ws, &server.uri());
    let mut ctx = RunContext::new(Source::Mcp);
    ctx.overrides.insert("who".into(), "override".into());
    ctx.record_history = false;
    let draft = RequestDraft {
        method: "GET".into(),
        url: "{{base}}/items".into(),
        headers: vec![KeyValue::new("X-Who", "{{who}}")],
        auth: Auth::None,
        ..RequestDraft::default()
    };
    let result = Executor::new().send_adhoc(&ws, draft, ctx).await?;
    assert_eq!(result.response.status, 204);
    assert!(result.history_id.is_empty());
    assert!(ws.history().list(10, 0, None)?.is_empty());
    Ok(())
}

#[tokio::test]
async fn adhoc_with_collection_inherits_auth_and_variables() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(header("authorization", format!("Bearer {SECRET}").as_str()))
        .and(query_param("who", "collection"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    let (collection, _) = setup(&ws, &server.uri());
    let draft = RequestDraft {
        method: "get".into(),
        url: "{{base}}/me".into(),
        params: vec![KeyValue::new("who", "{{who}}")],
        collection_id: Some(collection.id.clone()),
        ..RequestDraft::default()
    };
    let result = Executor::new()
        .send_adhoc(&ws, draft, RunContext::new(Source::Ui))
        .await?;
    assert_eq!(&result.response.body[..], b"ok");
    let row = &ws.history().list(1, 0, None)?[0];
    assert_eq!(
        (row.source, row.request_id.as_deref(), row.status),
        (Source::Ui, None, Some(200))
    );
    Ok(())
}

#[tokio::test]
async fn network_error_is_short_masked_and_recorded() -> anyhow::Result<()> {
    // Puerto cerrado: se obtiene uno libre y se suelta antes de usarlo.
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let (_dir, ws) = workspace();
    setup(&ws, &format!("http://127.0.0.1:{port}"));
    let (_, request) = ws.find_request(&"ping".into())?;
    let error = Executor::new()
        .run(&ws, &request, RunContext::new(Source::Cli))
        .await
        .unwrap_err();
    let network = error.downcast_ref::<NetworkError>().expect("network error");
    assert_eq!(network.kind, "connect");
    assert!(network.url.ends_with("token=***"), "{}", network.url);
    assert!(!network.message.contains(SECRET) && network.message.chars().count() <= 300);
    let entry = ws
        .history()
        .get(network.history_id.as_deref().unwrap())?
        .unwrap();
    assert_eq!(entry.status, None);
    assert!(entry.error.unwrap().starts_with("connect: "));
    Ok(())
}

#[tokio::test]
async fn acme_ping_runs_against_mock_with_fake_environment() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "gateway is ok!"})))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    ws.import_postman_file(
        &fixtures.join("Acme-Gateway-Scopes.postman_collection.json"),
        None,
    )?;
    ws.import_postman_file(
        &fixtures.join("Acme-DESA-fake.postman_environment.json"),
        None,
    )?;
    let mut ctx = RunContext::new(Source::Cli);
    ctx.env_id = Some("Acme DESA (fake)".into());
    ctx.overrides.insert("gatewayUrl".into(), server.uri());
    let (_, ping) = ws.find_request(&"00.2 Gateway - ping".into())?;
    let result = Executor::new().run(&ws, &ping, ctx).await?;
    assert_eq!(result.response.status, 200);
    // Sus scripts de la colección real: el `pre` no salta en la iteración 0 y el test pasa.
    assert!(!result.skipped);
    assert_eq!(result.tests.len(), 1);
    assert_eq!(result.tests[0].name, "Gateway responde (ping)");
    assert!(result.tests[0].passed);
    Ok(())
}

#[tokio::test]
async fn unknown_environment_is_a_typed_error() {
    let (_dir, ws) = workspace();
    let mut ctx = RunContext::new(Source::Cli);
    ctx.env_id = Some("nope".into());
    let draft = RequestDraft {
        method: "GET".into(),
        url: "http://127.0.0.1:9".into(),
        ..RequestDraft::default()
    };
    let error = Executor::new()
        .send_adhoc(&ws, draft, ctx)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<runpost_core::CoreError>()
            .unwrap()
            .code(),
        "env_not_found"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scripts_change_variables_on_disk_and_tests_reach_history() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ping"))
        .and(header("x-pre", "from script"))
        .and(body_json(json!({"hello": "script", "n": 1})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"echo": SECRET})))
        .expect(1)
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    let (mut collection, mut request) = setup(&ws, &server.uri());
    collection.scripts = Scripts {
        pre: "pm.collectionVariables.set('who', 'script');".into(),
        test: String::new(),
    };
    request.scripts = Scripts {
        pre: r#"
            pm.environment.set('tokenLength', pm.environment.get('token').length);
            pm.request.headers.add({ key: 'X-Pre', value: 'from script' });
            console.log('token is', pm.environment.get('token'));
        "#
        .into(),
        test: r#"
            pm.test('status', () => pm.response.to.have.status(200));
            pm.test('echo', () => pm.expect(pm.response.json().echo).to.equal('nope'));
            pm.globals.set('lastStatus', pm.response.code);
            pm.environment.set('token', 'rotated-secret-value');
            console.log('new token', pm.environment.get('token'));
        "#
        .into(),
    };
    if let Item::Folder(folder) = &mut collection.items[0] {
        folder.items[0] = Item::Request(Box::new(request.clone()));
    }
    ws.save_collection(&collection)?;

    let result = Executor::new()
        .run(&ws, &request, RunContext::new(Source::Ui))
        .await?;
    server.verify().await;
    assert_eq!(result.response.status, 200);
    let tests: Vec<_> = result
        .tests
        .iter()
        .map(|t| {
            (
                t.name.as_str(),
                t.passed,
                t.msg.as_deref().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        tests,
        [
            ("status", true, ""),
            (
                "echo",
                false,
                "AssertionError: expected '***' to equal 'nope'"
            )
        ]
    );
    // La consola nunca lleva secretos, ni el viejo ni el nuevo.
    assert_eq!(result.console, ["token is ***", "new token ***"]);
    let changed: Vec<_> = result.vars_changed.iter().map(|v| v.key.as_str()).collect();
    assert_eq!(changed, ["who", "tokenLength", "lastStatus", "token"]);

    // Los cambios quedan en disco, con el flag `secret` intacto.
    let env = ws.active_environment(None)?.unwrap();
    let get = |key: &str| {
        env.variables
            .iter()
            .find(|v| v.key == key)
            .cloned()
            .unwrap()
    };
    assert_eq!(get("tokenLength").value.as_deref(), Some("23"));
    assert_eq!(get("token").value.as_deref(), Some("rotated-secret-value"));
    assert!(get("token").secret);
    assert_eq!(ws.globals()?.variables[0].value.as_deref(), Some("200"));
    let stored = ws.load_collection(&collection.id)?;
    assert_eq!(stored.variables[0].value.as_deref(), Some("script"));

    // El historial guarda los tests, enmascarados.
    let entry = ws.history().get(&result.history_id)?.unwrap();
    let tests_json = entry.tests_json.unwrap();
    assert!(
        tests_json.contains("expected '***' to equal 'nope'"),
        "{tests_json}"
    );
    assert!(!tests_json.contains(SECRET));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skip_request_sends_nothing() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    let (_, mut request) = setup(&ws, &server.uri());
    request.scripts.pre = "pm.test('before skip', () => {}); pm.execution.skipRequest();".into();
    request.scripts.test = "pm.test('never', () => {});".into();
    let result = Executor::new()
        .run(&ws, &request, RunContext::new(Source::Cli))
        .await?;
    server.verify().await;
    assert!(result.skipped);
    assert!(result.history_id.is_empty());
    assert_eq!(result.response.status, 0);
    assert_eq!(result.tests.len(), 1);
    assert_eq!(result.resolved.url, "{{base}}/ping?token={{token}}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsaved_drafts_run_their_own_scripts() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    let draft = RequestDraft {
        method: "GET".into(),
        url: server.uri(),
        scripts: Scripts {
            pre: String::new(),
            test: "pm.test('204', () => pm.response.to.have.status(204)); undefinedThing;".into(),
        },
        ..RequestDraft::default()
    };
    let result = Executor::new()
        .send_adhoc(&ws, draft, RunContext::new(Source::Ui))
        .await?;
    let names: Vec<_> = result
        .tests
        .iter()
        .map(|t| (t.name.as_str(), t.passed))
        .collect();
    assert_eq!(names, [("204", true), ("<script error>", false)]);
    assert_eq!(
        result.tests[1].msg.as_deref(),
        Some("test script of request: ReferenceError: undefinedThing is not defined")
    );
    Ok(())
}
