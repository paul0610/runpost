use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use runpost_core::secrets::KEYRING_ENV;
use serde_json::{Value, json};

#[test]
fn mcp_handshake_lists_tools_and_clean_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
        .env(KEYRING_ENV, "memory")
        .args(["mcp", "--workspace"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"s0-test","version":"0"}}});
    writeln!(input, "{initialize}").unwrap();
    input.flush().unwrap();
    let line = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("initialize timed out");
    let response: Value = serde_json::from_str(&line).expect("stdout must only contain JSON-RPC");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(response["result"]["serverInfo"]["name"], "runpost");
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    )
    .unwrap();
    input.flush().unwrap();
    let line = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("tools/list timed out");
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 13);
    drop(input);
    let status = child.wait().unwrap();
    assert!(status.success());
    reader.join().unwrap();
    assert!(
        rx.try_recv().is_err(),
        "no extra stdout after JSON-RPC replies"
    );
    let log = std::fs::read_to_string(dir.path().join("logs/mcp.log")).unwrap();
    assert!(log.contains("MCP stdio started"));
}

#[test]
fn run_of_a_missing_collection_exits_two_without_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
        .env(KEYRING_ENV, "memory")
        .arg("--workspace")
        .arg(dir.path())
        .args(["run", "fake"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "runpost: collection not found: fake"
    );
}

/// Sin la app abierta, el `headersHelper` imprime `{}`, sale con 0 y no crea el workspace.
#[test]
fn mcp_headers_without_the_app_prints_an_empty_object() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("not-created");
    let output = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
        .env(KEYRING_ENV, "memory")
        .env("CLAUDE_CODE_MCP_SERVER_URL", "http://127.0.0.1:7979/mcp")
        .args(["mcp-headers", "--workspace"])
        .arg(&missing)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "{}");
    assert!(!missing.exists());
}

/// La cadena real del `headersHelper`: con la app abierta, su token está en el llavero del sistema y
/// `runpost_mcp mcp-headers` lo imprime; al cerrarla, la entrada desaparece. Usa el llavero real, así
/// que se ejecuta a mano: `cargo test -p runpost-cli --test stdio -- --ignored os_keyring`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "uses the real OS keyring"]
async fn os_keyring_mcp_headers_reads_the_token_of_the_open_app() {
    let dir = tempfile::tempdir().unwrap();
    let keyring = std::sync::Arc::new(runpost_core::secrets::OsKeyring::default());
    let ws = runpost_core::Workspace::open_with_keyring(dir.path(), keyring).unwrap();
    let server = runpost_server::start_with_options(
        ws.clone(),
        None,
        runpost_server::ServerOptions {
            mcp_port: Some(0),
            ..runpost_server::ServerOptions::default()
        },
    )
    .await
    .unwrap();
    let port = server.mcp_port.unwrap();
    let headers = || {
        // Como Claude Code: la URL del servidor en CLAUDE_CODE_MCP_SERVER_URL, sin --port.
        let output = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
            .env_remove(KEYRING_ENV)
            .env(
                "CLAUDE_CODE_MCP_SERVER_URL",
                format!("http://127.0.0.1:{port}/mcp"),
            )
            .arg("mcp-headers")
            .output()
            .unwrap();
        assert!(output.status.success());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    assert_eq!(headers(), json!({"X-RunPost-Token": server.token.expose()}));
    server.shutdown().await.unwrap();
    assert_eq!(headers(), json!({}));
    assert_eq!(ws.session(port).unwrap(), None);
}

/// El plugin de Claude Code por stdio con la app abierta: `runpost_mcp mcp` encuentra la app del mismo
/// workspace en el llavero real y le reenvía la llamada, que aparece en el panel Agente. Llavero real:
/// `cargo test -p runpost-cli --test stdio -- --ignored os_keyring`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "uses the real OS keyring"]
async fn os_keyring_stdio_bridges_to_the_open_app() {
    let dir = tempfile::tempdir().unwrap();
    let keyring = std::sync::Arc::new(runpost_core::secrets::OsKeyring::default());
    let ws = runpost_core::Workspace::open_with_keyring(dir.path(), keyring).unwrap();
    let server = runpost_server::start_with_options(
        ws.clone(),
        None,
        runpost_server::ServerOptions {
            mcp_port: Some(0),
            ..runpost_server::ServerOptions::default()
        },
    )
    .await
    .unwrap();
    let mut settings = ws.settings();
    settings.mcp.http_port = server.mcp_port;
    ws.save_settings(&settings).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
        .env_remove(KEYRING_ENV)
        .args(["mcp", "--workspace"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |message: Value| writeln!(stdin, "{message}").unwrap();
    send(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18",
        "capabilities":{},"clientInfo":{"name":"claude-code","version":"2.1.302"}}}),
    );
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    send(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ws_overview","arguments":{}}}),
    );
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["id"], 2, "{line}");

    let state: Value = reqwest_get_agent(server.port, server.token.expose()).await;
    assert_eq!(state["calls"][0]["tool"], "ws_overview");
    assert_eq!(state["calls"][0]["client"], "Claude Code");
    drop(stdin);
    let _ = child.wait();
    server.shutdown().await.unwrap();
    assert_eq!(ws.session(settings.mcp.http_port.unwrap()).unwrap(), None);
}

/// `GET /api/agent` con el token local (sin depender de reqwest en este crate de pruebas).
async fn reqwest_get_agent(port: u16, token: &str) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request = format!(
        "GET /api/agent HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-RunPost-Token: {token}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    serde_json::from_str(body).unwrap_or(Value::Null)
}
