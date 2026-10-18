//! Benchmark de docs/BENCH.md: sesión guiada de 15 llamadas MCP sobre la colección Acme
//! contra wiremock. Se ejecuta a mano y escribe una tabla Markdown en stdout:
//!
//! cargo test -p runpost-cli --test mcp_bench -- --ignored --nocapture

mod common;

use common::{ENV, McpClient, acme_mock, acme_workspace};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn guided_acme_session() {
    let server = acme_mock().await;
    let dir = acme_workspace();
    let mut mcp = McpClient::start(dir.path());
    let tools = mcp.request("tools/list", json!({})).to_string();
    let mut session_id = String::new();

    let calls: Vec<(&str, Value)> = vec![
        ("ws_overview", json!({})),
        (
            "list_requests",
            json!({"collection": "acme-gateway-scopes"}),
        ),
        (
            "env_get",
            json!({"keys": ["gatewayUrl", "accessToken", "currency", "accountId"]}),
        ),
        (
            "env_set",
            json!({"env": ENV, "vars": {"gatewayUrl": server.uri(), "apiUrl": server.uri()}}),
        ),
        ("run_request", json!({"request": "00.2 Gateway - ping"})),
        (
            "run_request",
            json!({"request": "00.1 API", "select": "$.session.scope"}),
        ),
        ("run_request", json!({"request": "#5 POST keys/local"})),
        ("run_request", json!({"request": "#6 POST encrypt"})),
        ("run_request", json!({"request": "#7 POST sign (sonda"})),
        ("run_request", json!({"request": "#5 keys/local SIN token"})),
        ("run_request", json!({"request": "#7 sign token INVÁLIDO"})),
        (
            "get_response",
            json!({"response_id": "SESSION", "range": [0, 200]}),
        ),
        ("history", json!({"limit": 5})),
        (
            "get_request",
            json!({"request": "#7b POST sign", "fields": ["url", "headers"]}),
        ),
        ("search", json!({"q": "sign"})),
    ];

    println!("| # | tool | argumentos | respuesta (chars) | ≈ tokens | crudo HTTP (bytes) |");
    println!("|---|------|------------|------------------:|---------:|-------------------:|");
    let (mut total_chars, mut total_raw) = (0usize, 0usize);
    for (index, (tool, mut args)) in calls.into_iter().enumerate() {
        if args["response_id"] == "SESSION" {
            args["response_id"] = json!(session_id);
        }
        let (text, is_error) = mcp.call(tool, args.clone());
        assert!(!is_error, "{tool}: {text}");
        let value: Value = serde_json::from_str(&text).unwrap();
        if args["select"] == "$.session.scope" {
            session_id = value["response_id"].as_str().unwrap().to_owned();
        }
        let raw = value["t"]["raw"].as_u64().map(|r| r as usize);
        total_chars += text.len();
        total_raw += raw.unwrap_or(0);
        let mut shown = args.to_string().replace(&server.uri(), "<mock>");
        if shown.len() > 60 {
            shown = format!("{}…", shown.chars().take(60).collect::<String>());
        }
        println!(
            "| {} | `{tool}` | `{}` | {} | {} | {} |",
            index + 1,
            shown.replace('|', "\\|"),
            text.len(),
            text.len() / 4,
            raw.map_or("—".into(), |r| r.to_string())
        );
    }
    println!();
    println!(
        "tools/list: {} chars ≈ {} tokens",
        tools.len(),
        tools.len() / 4
    );
    println!(
        "15 respuestas: {total_chars} chars ≈ {} tokens",
        total_chars / 4
    );
    println!(
        "crudo HTTP de las 7 ejecuciones: {total_raw} bytes ≈ {} tokens",
        total_raw / 4
    );
    mcp.finish();
}
