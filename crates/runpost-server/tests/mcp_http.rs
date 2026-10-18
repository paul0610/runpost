//! Aceptación del sprint 6 por HTTP real: un agente conectado a `/mcp` de la app ejecuta un
//! `DELETE` marcado `require_approval`; la app (WebSocket) muestra el aviso, y el usuario aprueba
//! (se ejecuta y aparece en el panel) o deniega (el modelo recibe `error: policy`).

use std::{sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode};
use runpost_core::{
    TOKEN_HEADER, Workspace,
    model::{AgentPolicy, Collection, Environment, Item, Request, Variable},
    secrets::MemoryKeyring,
};
use runpost_server::{ServerHandle, ServerOptions};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no WebSocket event in 10 s")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// Siguiente evento de ese tipo (salta `workspace_changed`, `agent_client`…).
async fn next_of(socket: &mut Socket, kind: &str) -> Value {
    loop {
        let event = next(socket).await;
        if event["type"] == kind {
            return event;
        }
    }
}

/// La UI de la app: WebSocket con el token como subprotocolo.
async fn viewer(server: &ServerHandle) -> Socket {
    let url = format!("ws://127.0.0.1:{}/ws", server.port);
    let mut request = url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("runpost, {}", server.token.expose())
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(next(&mut socket).await["type"], "hello");
    socket
}

/// Cliente MCP Streamable HTTP mínimo, como el de Claude Code.
struct Mcp {
    client: Client,
    url: String,
    token: String,
    session: Option<String>,
    next_id: u64,
}

impl Mcp {
    fn request(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(&self.url)
            .header(TOKEN_HEADER, &self.token)
            .header("Accept", "application/json, text/event-stream")
            .json(body);
        if let Some(session) = &self.session {
            request = request
                .header("mcp-session-id", session)
                .header("mcp-protocol-version", "2025-06-18");
        }
        request
    }

    /// Mensajes JSON-RPC de una respuesta, en JSON o como flujo SSE.
    async fn messages(response: reqwest::Response) -> Vec<Value> {
        let text = response.text().await.unwrap();
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            return vec![value];
        }
        text.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|data| !data.is_empty())
            .filter_map(|data| serde_json::from_str(data).ok())
            .collect()
    }

    async fn connect(url: String, token: String) -> Self {
        let mut mcp = Self {
            client: Client::new(),
            url,
            token,
            session: None,
            next_id: 1,
        };
        let response = mcp
            .request(
                &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "claude-code", "version": "2.1.302"}}}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        mcp.session = response
            .headers()
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_owned());
        let init = Self::messages(response).await;
        assert_eq!(init[0]["result"]["serverInfo"]["name"], "runpost");
        let ack = mcp
            .request(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .send()
            .await
            .unwrap();
        assert_eq!(ack.status(), StatusCode::ACCEPTED);
        mcp
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let response = self
            .request(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .send()
            .await
            .unwrap();
        let messages = Self::messages(response).await;
        messages
            .into_iter()
            .find(|m| m["id"] == id)
            .expect("no JSON-RPC response")
    }

    /// `tools/call` → (texto que ve el modelo, isError).
    async fn call(&mut self, tool: &str, arguments: Value) -> (String, bool) {
        let result = self
            .rpc("tools/call", json!({"name": tool, "arguments": arguments}))
            .await;
        (
            result["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            result["result"]["isError"] == json!(true),
        )
    }
}

async fn decide(server: &ServerHandle, id: &str, approve: bool) -> StatusCode {
    Client::new()
        .post(format!(
            "http://127.0.0.1:{}/api/agent/approvals/{id}",
            server.port
        ))
        .header(TOKEN_HEADER, server.token.expose())
        .json(&json!({"approve": approve}))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_with_require_approval_waits_for_the_user_in_the_app() {
    let mock = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/usd/accounts/acc-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"deleted": true})))
        .expect(1)
        .mount(&mock)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open_with_keyring(dir.path(), Arc::new(MemoryKeyring::new())).unwrap();
    let mut env = Environment::new("Acme-DESA (fake)");
    env.variables = vec![
        Variable::new("apiUrl", mock.uri()),
        Variable::new("accountId", "acc-123"),
    ];
    ws.save_environment(&env).unwrap();
    let mut settings = ws.settings();
    settings.active_environment = Some(env.id.clone());
    ws.save_settings(&settings).unwrap();
    let mut collection = Collection::new("Acme Gateway Scopes");
    collection.agent_policy = Some(AgentPolicy {
        require_approval: vec!["DELETE".into()],
        ..AgentPolicy::default()
    });
    let account_url = "{{apiUrl}}/v1/usd/accounts/{{accountId}}";
    let request = Request::new("Borrar cuenta", "DELETE", account_url);
    collection
        .items
        .push(Item::Request(Box::new(request.clone())));
    ws.save_collection(&collection).unwrap();

    let server = runpost_server::start_with_options(
        ws.clone(),
        None,
        ServerOptions {
            mcp_port: Some(0),
            ..ServerOptions::default()
        },
    )
    .await
    .unwrap();
    let mcp_port = server.mcp_port.expect("MCP over HTTP is running");
    let url = format!("http://127.0.0.1:{mcp_port}/mcp");
    // La app deja en el llavero lo que necesita el headersHelper (`runpost_mcp mcp-headers`).
    let session: Value = serde_json::from_str(&ws.session(mcp_port).unwrap().unwrap()).unwrap();
    assert_eq!(
        session,
        json!({"port": mcp_port, "token": server.token.expose(),
               "workspace": ws.root().to_string_lossy()})
    );

    // Mismo token local que /api; un navegador (Origin) no entra aunque lo tenga.
    let client = Client::new();
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "x", "version": "1"}}});
    let status = |r: reqwest::Response| r.status();
    let no_token = client.post(&url).json(&initialize).send().await.unwrap();
    assert_eq!(status(no_token), StatusCode::UNAUTHORIZED);
    let browser = client
        .post(&url)
        .header(TOKEN_HEADER, server.token.expose())
        .header("Origin", "https://evil.example")
        .header("Accept", "application/json, text/event-stream")
        .json(&initialize)
        .send()
        .await
        .unwrap();
    assert_eq!(status(browser), StatusCode::FORBIDDEN);
    let bearer = client
        .post(&url)
        .bearer_auth(server.token.expose())
        .header("Accept", "application/json, text/event-stream")
        .json(&initialize)
        .send()
        .await
        .unwrap();
    assert_eq!(status(bearer), StatusCode::OK);

    let mut mcp = Mcp::connect(url.clone(), server.token.expose().to_owned()).await;
    let tools = mcp.rpc("tools/list", json!({})).await;
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 13);

    // Sin la app mirando (ninguna ventana conectada) no hay a quién preguntar: error inmediato.
    let (text, is_error) = mcp
        .call("run_request", json!({"request": "Borrar cuenta"}))
        .await;
    assert!(is_error);
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"error": "policy", "rule": "require_approval", "hint": "ask the user to approve in the app"})
    );

    // Con la app abierta: aviso con método, URL sin resolver, colección, entorno y regla.
    let mut socket = viewer(&server).await;
    let waiting = tokio::spawn(async move {
        let reply = mcp
            .call("run_request", json!({"request": "Borrar cuenta"}))
            .await;
        (mcp, reply)
    });
    let shown = next_of(&mut socket, "agent_approval").await["approval"].clone();
    assert_eq!(shown["client"], "Claude Code");
    assert_eq!(shown["tool"], "run_request");
    assert_eq!(shown["method"], "DELETE");
    assert_eq!(shown["url"], account_url);
    assert_eq!(shown["request"], "Borrar cuenta");
    assert_eq!(shown["collection"], "Acme Gateway Scopes");
    assert_eq!(shown["environment"], "Acme-DESA (fake)");
    assert_eq!(shown["rule"], "require_approval: DELETE");
    assert_eq!(
        shown["expires"].as_i64().unwrap() - shown["ts"].as_i64().unwrap(),
        60_000
    );
    let id = shown["id"].as_str().unwrap().to_owned();
    let state: Value = client
        .get(format!("http://127.0.0.1:{}/api/agent", server.port))
        .header(TOKEN_HEADER, server.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["pending"][0]["id"], id.as_str());
    assert_eq!(state["mcp"]["url"], url.as_str());
    assert!(!waiting.is_finished(), "the tool must wait for the user");

    // Aprobar: se ejecuta y aparece en el panel.
    assert_eq!(decide(&server, &id, true).await, StatusCode::OK);
    let (mut mcp, (text, is_error)) = waiting.await.unwrap();
    assert!(!is_error, "{text}");
    let result: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(result["status"], 200);
    assert_eq!(result["body"], json!({"deleted": true}));
    assert_eq!(
        next_of(&mut socket, "agent_approval_done").await["decision"],
        "approved"
    );
    let row = next_of(&mut socket, "agent_call").await["call"].clone();
    assert_eq!(row["tool"], "run_request");
    assert_eq!(row["target"], "Borrar cuenta");
    assert_eq!(row["request_id"], request.id.as_str());
    assert_eq!(row["status"], "200");
    assert_eq!(row["client"], "Claude Code");
    // Medidor: `sent` es exactamente lo que recibió el modelo; `raw`, la respuesta completa.
    assert_eq!(row["sent"].as_u64().unwrap() as usize, text.len());
    assert!(row["raw"].as_u64().unwrap() > 0);
    assert_eq!(decide(&server, &id, true).await, StatusCode::NOT_FOUND);

    // Denegar: el modelo recibe `error: policy` y no se envía nada.
    let waiting = tokio::spawn(async move {
        let reply = mcp
            .call("run_request", json!({"request": "Borrar cuenta"}))
            .await;
        (mcp, reply)
    });
    let id = next_of(&mut socket, "agent_approval").await["approval"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(decide(&server, &id, false).await, StatusCode::OK);
    let (_mcp, (text, is_error)) = waiting.await.unwrap();
    assert!(is_error);
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"error": "policy", "rule": "require_approval", "hint": "the user denied it in the app"})
    );
    let row = next_of(&mut socket, "agent_call").await["call"].clone();
    assert_eq!(
        (row["status"].as_str(), row["ok"].as_bool()),
        (Some("policy"), Some(false))
    );

    let state: Value = client
        .get(format!("http://127.0.0.1:{}/api/agent", server.port))
        .header(TOKEN_HEADER, server.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["meter"]["calls"], 3);
    assert_eq!(state["clients"][0]["client"], "Claude Code");
    assert!(state["pending"].as_array().unwrap().is_empty());
    mock.verify().await;

    let _ = socket.close(None).await;
    let _ = socket.send(Message::Close(None)).await;
    server.shutdown().await.unwrap();
    // Al cerrar la app, el token sale del llavero y el puerto queda libre.
    assert_eq!(ws.session(mcp_port).unwrap(), None);
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", mcp_port))
            .await
            .is_err()
    );
}

/// Sprint 7: el puente stdio (`runpost_mcp mcp`, el del plugin de Claude Code) encuentra la app abierta
/// sobre su mismo workspace, le reenvía las llamadas (salen en el panel Agente con el nombre del
/// cliente real) y, si la app se cierra, sigue atendiendo en local.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stdio_bridge_forwards_to_the_open_app_and_falls_back() {
    use rmcp::{
        ServiceExt,
        model::{CallToolRequestParams, CallToolResponse, ClientConfig, Implementation},
    };
    use runpost_mcp::{
        RunPostMcp,
        bridge::{Bridge, find_app},
    };

    let keyring = Arc::new(MemoryKeyring::new());
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open_with_keyring(dir.path(), keyring.clone()).unwrap();
    let server = runpost_server::start_with_options(
        ws.clone(),
        None,
        ServerOptions {
            mcp_port: Some(0),
            ..ServerOptions::default()
        },
    )
    .await
    .unwrap();
    let port = server.mcp_port.unwrap();
    let mut settings = ws.settings();
    settings.mcp.http_port = Some(port);
    ws.save_settings(&settings).unwrap();
    let app = find_app(&ws).expect("the open app is found");
    assert_eq!(app.url, format!("http://127.0.0.1:{port}/mcp"));
    // Otro workspace con el mismo puerto no es "su" app.
    let other_dir = tempfile::tempdir().unwrap();
    let other = Workspace::open_with_keyring(other_dir.path(), keyring.clone()).unwrap();
    other.save_settings(&settings).unwrap();
    assert_eq!(find_app(&other), None);

    let (client_io, bridge_io) = tokio::io::duplex(1 << 16);
    let bridge = Bridge::new(app, RunPostMcp::new(ws.clone()));
    let serving = tokio::spawn(async move {
        let service = bridge.serve(bridge_io).await.unwrap();
        let _ = service.waiting().await;
    });
    let mut info = ClientConfig::default();
    info.client_info = Implementation::new("claude-code", "2.1.302");
    let client = info.serve(client_io).await.unwrap();
    assert_eq!(client.list_all_tools().await.unwrap().len(), 13);
    let text = |response: CallToolResponse| match response {
        CallToolResponse::Complete(result) => result.content[0].as_text().unwrap().text.clone(),
        other => panic!("{other:?}"),
    };
    let overview = text(
        client
            .call_tool_once(CallToolRequestParams::new("ws_overview"))
            .await
            .unwrap(),
    );
    assert!(overview.contains("collections"), "{overview}");
    let state: Value = Client::new()
        .get(format!("http://127.0.0.1:{}/api/agent", server.port))
        .header(TOKEN_HEADER, server.token.expose())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(state["calls"][0]["tool"], "ws_overview");
    assert_eq!(state["calls"][0]["client"], "Claude Code");

    // La app se cierra: la siguiente llamada se atiende en local, sin error para el modelo.
    server.shutdown().await.unwrap();
    let local = text(
        client
            .call_tool_once(CallToolRequestParams::new("ws_overview"))
            .await
            .unwrap(),
    );
    assert!(local.contains("collections"), "{local}");
    client.cancel().await.unwrap();
    serving.abort();
}

/// Si la app deja de responder con una llamada en curso, el puente no la repite en local (un POST
/// saldría dos veces): el modelo recibe `app_unavailable` y lo siguiente se atiende en local.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stdio_bridge_never_sends_a_request_twice() {
    use rmcp::{
        ServiceExt,
        model::{CallToolRequestParams, CallToolResponse, ClientConfig},
    };
    use runpost_mcp::{
        RunPostMcp,
        bridge::{AppSession, Bridge},
    };

    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/pagos"))
        .respond_with(ResponseTemplate::new(201).set_delay(Duration::from_millis(1500)))
        .expect(1)
        .mount(&mock)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open_with_keyring(dir.path(), Arc::new(MemoryKeyring::new())).unwrap();
    let mut collection = Collection::new("Pagos");
    let url = format!("{}/pagos", mock.uri());
    let request = Request::new("Crear pago", "POST", &url);
    collection.items.push(Item::Request(Box::new(request)));
    ws.save_collection(&collection).unwrap();
    let server = runpost_server::start_with_options(
        ws.clone(),
        None,
        ServerOptions {
            mcp_port: Some(0),
            ..ServerOptions::default()
        },
    )
    .await
    .unwrap();
    let app_port = server.mcp_port.unwrap();

    // Un proxy entre el puente y la app. Al abortarlo corta sus conexiones y deja de escuchar: para
    // el puente, la app se cierra con la llamada en curso (la app de verdad sí la ejecuta). Reescribe
    // `Host`, porque la app solo acepta el de su puerto.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    let proxy = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (from, to) = (
            format!("Host: 127.0.0.1:{proxy_port}").to_lowercase(),
            format!("host: 127.0.0.1:{app_port}"),
        );
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (inbound, _) = listener.accept().await.unwrap();
            let (from, to) = (from.clone(), to.clone());
            connections.spawn(async move {
                let Ok(outbound) = tokio::net::TcpStream::connect(("127.0.0.1", app_port)).await
                else {
                    return;
                };
                let (mut client_read, mut client_write) = inbound.into_split();
                let (mut app_read, mut app_write) = outbound.into_split();
                let upstream = async {
                    let mut buffer = vec![0; 64 * 1024];
                    while let Ok(n @ 1..) = client_read.read(&mut buffer).await {
                        let text = String::from_utf8_lossy(&buffer[..n]).replace(&from, &to);
                        if app_write.write_all(text.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                };
                tokio::select! {
                    _ = upstream => {}
                    _ = tokio::io::copy(&mut app_read, &mut client_write) => {}
                }
            });
        }
    });

    let app = AppSession {
        url: format!("http://127.0.0.1:{proxy_port}/mcp"),
        token: server.token.expose().to_owned(),
    };
    let (client_io, bridge_io) = tokio::io::duplex(1 << 16);
    let bridge = Bridge::new(app, RunPostMcp::new(ws.clone()));
    let serving = tokio::spawn(async move {
        let service = bridge.serve(bridge_io).await.unwrap();
        let _ = service.waiting().await;
    });
    let client = ClientConfig::default().serve(client_io).await.unwrap();
    let peer = client.peer().clone();
    let arguments = json!({"request": "Crear pago"})
        .as_object()
        .unwrap()
        .clone();
    let pending = tokio::spawn(async move {
        let call = CallToolRequestParams::new("run_request").with_arguments(arguments);
        peer.call_tool_once(call).await
    });
    // La app ya envió la petición (el mock tarda 1,5 s en contestar) cuando se "cierra".
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    proxy.abort();
    let response = tokio::time::timeout(Duration::from_secs(15), pending)
        .await
        .expect("the call must not hang when the app goes away")
        .unwrap()
        .unwrap();
    let CallToolResponse::Complete(result) = response else {
        panic!("{response:?}");
    };
    assert_eq!(result.is_error, Some(true));
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("\"error\":\"app_unavailable\""), "{text}");

    // Después, en local; y la petición salió una sola vez.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let overview = client
        .call_tool_once(CallToolRequestParams::new("ws_overview"))
        .await
        .unwrap();
    let CallToolResponse::Complete(overview) = overview else {
        panic!("{overview:?}");
    };
    assert!(
        overview.content[0]
            .as_text()
            .unwrap()
            .text
            .contains("Pagos"),
        "{overview:?}"
    );
    mock.verify().await;
    client.cancel().await.unwrap();
    serving.abort();
    server.shutdown().await.unwrap();
}
