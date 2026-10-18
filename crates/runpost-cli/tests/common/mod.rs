//! Cliente JSON-RPC mínimo sobre el stdio de `runpost mcp` y mocks de Acme para wiremock.
#![allow(dead_code)]

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use runpost_core::{Workspace, secrets::KEYRING_ENV};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, path_regex},
};

pub const FAKE_TOKEN: &str = "fake-access-token-not-real-0000";
pub const ENV: &str = "Acme DESA (fake)";

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Workspace temporal con la colección Acme y el entorno falso (activo).
pub fn acme_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    ws.import_postman_file(
        &fixture("Acme-Gateway-Scopes.postman_collection.json"),
        None,
    )
    .unwrap();
    let env = ws
        .import_postman_file(&fixture("Acme-DESA-fake.postman_environment.json"), None)
        .unwrap();
    let mut settings = ws.settings();
    settings.active_environment = Some(env.id);
    ws.save_settings(&settings).unwrap();
    dir
}

/// Respuestas simuladas de Acme Gateway. El endpoint de sesión devuelve eco del token
/// para comprobar que el enmascarado lo tapa.
pub async fn acme_mock() -> MockServer {
    let server = MockServer::start().await;
    let bearer = format!("Bearer {FAKE_TOKEN}");
    Mock::given(method("GET"))
        .and(path("/v1/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "gateway is ok!"})))
        .mount(&server)
        .await;
    let scopes = [
        "accounts_read",
        "accounts_sign",
        "reports_read",
        "settings_read",
    ];
    Mock::given(method("GET"))
        .and(path("/v1/session"))
        .and(header("authorization", bearer.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"session": {
            "id": "sess_0000000000000001", "client": "acme", "user": "user_0000000000000001",
            "scope": scopes, "created": "2026-10-17T10:00:00.000Z", "expires": "2026-10-17T18:00:00.000Z",
            "origin": "test.acme.example", "accessToken": FAKE_TOKEN,
            "label": "fake-scope", "organization": "org_0000000000000001",
            "ipRestrict": [], "limits": {"usd": {"perDay": "100000", "perRequest": "10000"}},
            "mfa": {"required": false, "verifiedAt": "2026-10-17T10:00:00.000Z"}
        }})))
        .mount(&server)
        .await;
    // Con token válido: 200; sin token o con el inválido: 401 (como Gateway).
    Mock::given(method("POST"))
        .and(path_regex(r"^/v1/(.+/)?(keys/local|encrypt|accounts/.+/sign)$"))
        .and(header("authorization", bearer.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "pub": "pub-fake-000000000000000000000000000000000000000000000000000",
            "encrypted": "{\"iv\":\"fake\",\"v\":1,\"iter\":10000,\"ks\":256,\"ts\":64,\"mode\":\"ccm\",\"cipher\":\"aes\",\"ct\":\"ZmFrZQ==\"}"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/v1/"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "unauthorized", "name": "Unauthorized", "requestId": "ck0fake0request0id"
        })))
        .mount(&server)
        .await;
    server
}

pub struct McpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    /// Todo lo que el servidor escribió en stdout.
    pub transcript: String,
}

impl McpClient {
    pub fn start(workspace: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_runpost_mcp"))
            .env(KEYRING_ENV, "memory")
            .args(["mcp", "--workspace"])
            .arg(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
            transcript: String::new(),
        };
        client.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        );
        client.send_line(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        client
    }

    fn send_line(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    /// Envía una petición y devuelve su `result` (la línea cruda queda en `transcript`).
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send_line(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "server closed stdout"
            );
            self.transcript.push_str(&line);
            let message: Value = serde_json::from_str(&line).expect("stdout carries only JSON-RPC");
            if message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
        }
    }

    /// `tools/call` → (texto devuelto por la tool, is_error).
    pub fn call(&mut self, tool: &str, arguments: Value) -> (String, bool) {
        let result = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        (text, result["isError"] == json!(true))
    }

    pub fn call_json(&mut self, tool: &str, arguments: Value) -> Value {
        let (text, is_error) = self.call(tool, arguments);
        assert!(!is_error, "{tool}: {text}");
        serde_json::from_str(&text).unwrap()
    }

    pub fn finish(mut self) -> String {
        drop(self.stdin.take());
        assert!(self.child.wait().unwrap().success());
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut self.stdout, &mut rest).unwrap();
        assert!(rest.is_empty(), "extra stdout after shutdown: {rest}");
        std::mem::take(&mut self.transcript)
    }
}
