//! Aceptación S2 por subproceso: `runpost mcp` real, JSON-RPC por stdio, Acme contra wiremock.

mod common;

use common::{ENV, FAKE_TOKEN, McpClient, acme_mock, acme_workspace};
use runpost_core::{
    Workspace,
    model::{AgentPolicy, Collection},
};
use serde_json::{Value, json};

fn compact(text: &str) {
    assert!(
        !text.contains('\n') && !text.contains(": ") && !text.contains(", "),
        "not compact: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acme_over_stdio_is_compact_and_never_leaks_the_secret() {
    let server = acme_mock().await;
    let dir = acme_workspace();
    let mut mcp = McpClient::start(dir.path());

    let tools = mcp.request("tools/list", json!({}));
    let tools_chars = tools.to_string().len();
    assert_eq!(tools["tools"].as_array().unwrap().len(), 13);
    assert!(
        tools_chars / 4 < 2500,
        "tools/list ≈ {} tokens",
        tools_chars / 4
    );

    let overview = mcp.call_json("ws_overview", json!({}));
    assert_eq!(overview["collections"][0]["requests"], 10);
    assert_eq!(overview["environments"][0]["active"], true);

    let changed = mcp.call_json(
        "env_set",
        json!({"env": ENV, "vars": {"gatewayUrl": server.uri(), "apiUrl": server.uri()}}),
    );
    assert_eq!(
        changed,
        json!({"ok": true, "changed": ["apiUrl", "gatewayUrl"]})
    );

    // Aceptación: "00.2 Gateway - ping" → {"status":200,...} en < 300 bytes.
    let (ping, is_error) = mcp.call("run_request", json!({"request": "00.2 Gateway - ping"}));
    assert!(!is_error, "{ping}");
    compact(&ping);
    assert!(ping.starts_with(r#"{"status":200,"#), "{ping}");
    assert!(ping.len() < 300, "{} bytes: {ping}", ping.len());

    // La sesión devuelve el token en el cuerpo: el modelo solo ve ***.
    let session = mcp.call_json(
        "run_request",
        json!({"request": "00.1 API", "select": "$.session.scope"}),
    );
    assert_eq!(
        session["body"],
        json!([
            "accounts_read",
            "accounts_sign",
            "reports_read",
            "settings_read"
        ])
    );
    // Objetivo de la sección 14 (< 60 tokens) para la respuesta en sí. Desde el sprint 5 la colección
    // Acme trae scripts: sus `tests` y `console` (regla 5 de 8.3) van aparte y se miden aparte.
    let mut response_only = session.clone();
    let scripts = json!({
        "tests": response_only.as_object_mut().unwrap().remove("tests"),
        "console": response_only.as_object_mut().unwrap().remove("console"),
    });
    assert!(
        response_only.to_string().len() / 4 < 60,
        "select to one field: {response_only}"
    );
    assert_eq!(scripts["tests"], json!({"passed": 2, "failed": 0}));
    assert!(scripts.to_string().len() / 4 < 80, "scripts: {scripts}");
    let full = mcp.call_json(
        "get_response",
        json!({"response_id": session["response_id"], "select": "$.session.accessToken"}),
    );
    assert_eq!(full["body"], "***");
    let range = mcp.call_json(
        "get_response",
        json!({"response_id": session["response_id"], "range": [0, 20]}),
    );
    assert_eq!(range["body"], r#"{"session":{"id":"se"#);
    assert!(range["total_chars"].as_u64().unwrap() > 300);
    let headers = mcp.call_json(
        "get_response",
        json!({"response_id": session["response_id"], "headers": true}),
    );
    assert_eq!(headers["headers"]["content-type"], "application/json");

    let ok = mcp.call_json(
        "run_request",
        json!({"request": "#6 POST encrypt", "headers": true}),
    );
    assert_eq!(ok["status"], 200);
    let denied = mcp.call_json("run_request", json!({"request": "#7 sign token INVÁLIDO"}));
    assert_eq!(denied["status"], 401);

    let env = mcp.call_json("env_get", json!({"keys": ["accessToken", "currency"]}));
    assert_eq!(
        env,
        json!({"name": ENV, "vars": {"accessToken": "***", "currency": "usd"}})
    );

    let history = mcp.call_json("history", json!({"limit": 3}));
    assert_eq!(history.as_array().unwrap().len(), 3);
    assert_eq!(history[0]["status"], 401);
    let only_ok = mcp.call_json("history", json!({"status": 200}));
    assert_eq!(only_ok.as_array().unwrap().len(), 3);

    let listed = mcp.call_json("list_requests", json!({"folder": "00 - Diagnóstico"}));
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[1]["url"], "{{gatewayUrl}}/v1/ping");
    let request = mcp.call_json(
        "get_request",
        json!({"request": listed[1]["id"], "fields": ["url", "scripts"]}),
    );
    assert!(
        request["scripts"]["pre"]
            .as_str()
            .unwrap()
            .contains("skipRequest")
    );
    assert!(request.get("method").is_none());
    let found = mcp.call_json("search", json!({"q": "sign"}));
    assert_eq!(found.as_array().unwrap().len(), 4);

    let (missing, is_error) = mcp.call("run_request", json!({"request": "nope"}));
    assert!(is_error);
    assert_eq!(
        serde_json::from_str::<Value>(&missing).unwrap()["hint"],
        "list_requests shows ids and paths"
    );

    let transcript = mcp.finish();
    assert!(
        !transcript.contains(FAKE_TOKEN),
        "secret leaked to the model"
    );
    for line in transcript.lines() {
        serde_json::from_str::<Value>(line).expect("only JSON-RPC lines on stdout");
    }
    server.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_upsert_import_and_policy() {
    let server = acme_mock().await;
    let dir = acme_workspace();
    let mut mcp = McpClient::start(dir.path());
    mcp.call_json(
        "env_set",
        json!({"env": ENV, "vars": {"gatewayUrl": server.uri()}}),
    );

    let sent = mcp.call_json(
        "send",
        json!({"method": "POST", "url": "{{gatewayUrl}}/v1/encrypt",
               "headers": {"Authorization": "Bearer {{accessToken}}"}, "body": {"input": "x"},
               "save_as": {"collection": "Scratch", "folder": "a/b", "name": "enc"}}),
    );
    assert_eq!(sent["status"], 200);
    let saved = sent["saved_id"].as_str().unwrap().to_owned();
    assert!(saved.starts_with("req_"));
    let again = mcp.call_json("run_request", json!({"request": "Scratch/a/b/enc"}));
    assert_eq!(again["status"], 200);

    let batch = mcp.call_json(
        "upsert_request",
        json!({"collection": "Scratch", "requests": [
            {"name": "ping", "method": "GET", "url": "{{gatewayUrl}}/v1/ping"},
            {"folder": "a/b", "name": "enc", "scripts": {"test": "pm.test('x', () => {})"}}
        ]}),
    );
    assert_eq!(
        batch["ids"][1],
        saved.as_str(),
        "existing request updated in place"
    );
    let (bad, is_error) = mcp.call(
        "upsert_request",
        json!({"collection": "Scratch", "name": "no-url"}),
    );
    assert!(is_error && bad.contains("url is required"));

    let text = std::fs::read_to_string(common::fixture(
        "Acme-Gateway-Scopes.postman_collection.json",
    ))
    .unwrap();
    let imported = mcp.call_json("import_postman", json!({"text": text}));
    assert_eq!(imported["requests"], 10);

    // Política: solo GET permitido y como mucho 1 llamada/minuto.
    let ws = Workspace::open(dir.path()).unwrap();
    let mut scratch: Collection = ws.find_collection("Scratch").unwrap();
    scratch.agent_policy = Some(AgentPolicy {
        allow_methods: Some(vec!["GET".into()]),
        require_approval: vec![],
        max_calls_per_minute: Some(1),
    });
    ws.save_collection(&scratch).unwrap();
    let (blocked, is_error) = mcp.call("run_request", json!({"request": "Scratch/a/b/enc"}));
    assert!(is_error);
    assert_eq!(
        serde_json::from_str::<Value>(&blocked).unwrap(),
        json!({"error": "policy", "rule": "allow_methods", "hint": "ask the user to approve in the app"})
    );
    assert_eq!(
        mcp.call_json("run_request", json!({"request": "Scratch/ping"}))["status"],
        200
    );
    let (limited, _) = mcp.call("run_request", json!({"request": "Scratch/ping"}));
    assert!(
        limited.contains(r#""rule":"max_calls_per_minute""#),
        "{limited}"
    );
    scratch.agent_policy = Some(AgentPolicy {
        require_approval: vec!["GET".into()],
        ..AgentPolicy::default()
    });
    ws.save_collection(&scratch).unwrap();
    let (approval, _) = mcp.call("run_request", json!({"request": "Scratch/ping"}));
    assert!(
        approval.contains(r#""rule":"require_approval""#),
        "{approval}"
    );

    let transcript = mcp.finish();
    assert!(!transcript.contains(FAKE_TOKEN));
}

/// Aceptación S5: `run_collection(..., output: "failures")` devuelve solo los tests fallidos.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_collection_summary_and_only_failures() {
    let server = acme_mock().await;
    let dir = acme_workspace();
    let mut mcp = McpClient::start(dir.path());
    mcp.call_json(
        "env_set",
        json!({"env": ENV, "vars": {"gatewayUrl": server.uri(), "apiUrl": server.uri()}}),
    );

    let (text, is_error) = mcp.call(
        "run_collection",
        json!({"collection": "Acme Gateway Scopes"}),
    );
    assert!(!is_error, "{text}");
    let summary: Value = serde_json::from_str(&text).unwrap();
    // JSON minificado (los mensajes de los tests sí llevan ": " y ", ").
    assert_eq!(text, summary.to_string());
    // Con este mock, #7 no firma ni falla en Gateway ("INCONCLUSO") y los controles "SIN token"
    // de #5 y #6 reciben 401: tres tests fallan, como fallarían en Postman.
    assert_eq!(
        (
            &summary["total"],
            &summary["passed"],
            &summary["failed"],
            &summary["requests"],
            &summary["skipped"]
        ),
        (&json!(10), &json!(7), &json!(3), &json!(10), &json!(1))
    );
    assert_eq!(summary["failures"].as_array().unwrap().len(), 3);

    let (text, is_error) = mcp.call(
        "run_collection",
        json!({"collection": "Acme Gateway Scopes", "output": "failures"}),
    );
    assert!(!is_error, "{text}");
    let failures: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(text, failures.to_string());
    let keys: Vec<&String> = failures.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["failed", "failures", "t"]);
    assert_eq!(failures["failed"], 3);
    let tests: Vec<&str> = failures["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["test"].as_str().unwrap())
        .collect();
    assert_eq!(
        tests,
        [
            "#7 sign con [fake-scope] → INCONCLUSO (200: )",
            "#5 SIN token → 200 (confirma que no requiere scope)",
            "#6 SIN token → 200 (confirma que no requiere scope)",
        ]
    );
    assert_eq!(
        failures["failures"][1]["msg"],
        "AssertionError: expected response to have status code 200 but got 401"
    );

    let (text, is_error) = mcp.call(
        "run_collection",
        json!({"collection": "Acme Gateway Scopes", "folder": "00", "output": "full"}),
    );
    assert!(!is_error, "{text}");
    let full: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(full["items"].as_array().unwrap().len(), 2);
    assert_eq!(
        full["items"][1]["tests"][0],
        json!({"name": "Gateway responde (ping)", "ok": true})
    );

    let (text, is_error) = mcp.call("run_collection", json!({"collection": "nope"}));
    assert!(is_error);
    assert!(text.contains("collection_not_found"), "{text}");

    let transcript = mcp.finish();
    assert!(!transcript.contains(FAKE_TOKEN));
}

/// Sprint 6: `env_set` con `secret` guarda el valor en el llavero (en las pruebas, el de memoria del
/// proceso MCP); el archivo del entorno solo lleva la referencia y la petición sigue autenticada.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn env_set_secret_goes_to_the_keyring_and_still_authenticates() {
    let server = acme_mock().await;
    let dir = acme_workspace();
    let env_file = || {
        let entry = std::fs::read_dir(dir.path().join("environments"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        std::fs::read_to_string(entry.path()).unwrap()
    };
    assert!(env_file().contains(FAKE_TOKEN));
    let mut mcp = McpClient::start(dir.path());
    mcp.call_json(
        "env_set",
        json!({"env": ENV, "vars": {"gatewayUrl": server.uri(), "apiUrl": server.uri()}}),
    );
    let changed = mcp.call_json(
        "env_set",
        json!({"env": ENV, "vars": {"accessToken": FAKE_TOKEN}, "secret": ["accessToken"]}),
    );
    assert_eq!(changed, json!({"ok": true, "changed": ["accessToken"]}));
    let text = env_file();
    assert!(!text.contains(FAKE_TOKEN), "{text}");
    assert!(text.contains(r#""store": "keyring""#), "{text}");

    let session = mcp.call_json(
        "run_request",
        json!({"request": "00.1 API", "select": "$.session.label"}),
    );
    assert_eq!(session["status"], 200, "{session}");
    let vars = mcp.call_json("env_get", json!({"keys": ["accessToken"]}));
    assert_eq!(vars["vars"]["accessToken"], "***");
    assert!(!mcp.transcript.contains(FAKE_TOKEN));
}

/// Sprint 6 (19.7): dos ejecuciones con `timestamp` distinto dan diff vacío porque es volátil por
/// defecto; otro campo sí aparece salvo que vaya en `ignore`; la referencia se guarda con `save_ref`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_response_diff_ignores_volatile_fields() {
    use runpost_core::model::{Item, Request};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let server = MockServer::start().await;
    for (timestamp, nonce) in [(1_760_000_000, "a"), (1_760_000_042, "b")] {
        Mock::given(method("GET"))
            .and(path("/clock"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status": "ok", "timestamp": timestamp, "nonce": nonce})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    let mut clock = Collection::new("Clock");
    let request = Request::new("now", "GET", format!("{}/clock", server.uri()));
    clock.items.push(Item::Request(Box::new(request)));
    ws.save_collection(&clock).unwrap();
    let mut mcp = McpClient::start(dir.path());

    let first = mcp.call_json("run_request", json!({"request": "Clock/now"}));
    let second = mcp.call_json("run_request", json!({"request": "Clock/now"}));
    assert_eq!(second["body"]["timestamp"], 1_760_000_042);
    let (r1, r2) = (&first["response_id"], &second["response_id"]);

    let diff = mcp.call_json(
        "get_response",
        json!({"response_id": r2, "diff_against": r1}),
    );
    assert_eq!(
        diff,
        json!({"diff": [{"path": "$.nonce", "from": "a", "to": "b"}]})
    );
    let same = mcp.call_json(
        "get_response",
        json!({"response_id": r2, "diff_against": r1, "ignore": ["nonce"]}),
    );
    assert_eq!(same, json!({"diff": []}));

    let missing = mcp.call(
        "get_response",
        json!({"response_id": r2, "diff_against": "ref"}),
    );
    assert!(
        missing.1 && missing.0.contains("ref_not_found"),
        "{missing:?}"
    );
    let saved = mcp.call_json("get_response", json!({"response_id": r1, "save_ref": true}));
    assert_eq!(saved, json!({"ref_saved": true}));
    let against_ref = mcp.call_json(
        "get_response",
        json!({"response_id": r2, "diff_against": "ref", "ignore": ["$.nonce"]}),
    );
    assert_eq!(against_ref, json!({"diff": []}));
}

/// `get_request` con `snippet` (fase 4): código listo para copiar, con las variables resueltas y el
/// token secreto como `{{accessToken}}`, nunca su valor.
#[test]
fn get_request_snippet_keeps_the_secret_as_a_variable() {
    let dir = acme_workspace();
    let mut mcp = McpClient::start(dir.path());
    let reply = mcp.call_json(
        "get_request",
        json!({"request": "#7 POST sign (sonda", "snippet": "python"}),
    );
    let code = reply["snippet"].as_str().unwrap();
    assert!(
        code.contains(r#"url = "http://127.0.0.1:3080/v1/usd/accounts/acc_0123456789abcdef/sign""#),
        "{code}"
    );
    assert!(
        code.contains(r#""Authorization": "Bearer {{accessToken}}""#),
        "{code}"
    );
    let (bad, is_error) = mcp.call(
        "get_request",
        json!({"request": "00.2 Gateway - ping", "snippet": "cobol"}),
    );
    assert!(is_error && bad.contains("invalid_request"), "{bad}");
    let transcript = mcp.finish();
    assert!(!transcript.contains(FAKE_TOKEN));
}

#[test]
fn get_request_hides_literal_credentials_in_auth() {
    let dir = acme_workspace();
    let mut mcp = McpClient::start(dir.path());
    mcp.call_json(
        "upsert_request",
        json!({"collection": "Scratch", "requests": [
            {"name": "digest", "method": "GET", "url": "http://127.0.0.1:9/a",
             "auth": {"type": "digest", "username": "acme", "password": "fake-literal-pass"}},
            {"name": "oauth", "method": "GET", "url": "http://127.0.0.1:9/b",
             "auth": {"type": "oauth2_cc", "token_url": "http://127.0.0.1:9/token",
                      "client_id": "acme", "client_secret": "{{clientSecret}}", "scope": ""}}
        ]}),
    );
    let digest = mcp.call_json(
        "get_request",
        json!({"request": "Scratch/digest", "fields": ["auth"]}),
    );
    assert_eq!(digest["auth"]["username"], "acme");
    assert_eq!(digest["auth"]["password"], "***");
    let oauth = mcp.call_json(
        "get_request",
        json!({"request": "Scratch/oauth", "fields": ["auth"]}),
    );
    assert_eq!(oauth["auth"]["type"], "oauth2_cc");
    assert_eq!(oauth["auth"]["client_secret"], "{{clientSecret}}");
    let transcript = mcp.finish();
    assert!(!transcript.contains("fake-literal-pass"));
}
