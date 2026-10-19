//! API REST + WebSocket de la sección 7 contra un servidor real en 127.0.0.1.

use std::{path::PathBuf, time::Duration};

use futures_util::StreamExt;
use reqwest::{Client, Method, StatusCode};
use runpost_core::{TOKEN_HEADER, Workspace};
use runpost_server::ServerHandle;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

struct Api {
    server: ServerHandle,
    client: Client,
    _dir: tempfile::TempDir,
}

impl Api {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = runpost_server::start(Workspace::open(dir.path()).unwrap(), None)
            .await
            .unwrap();
        Self {
            server,
            client: Client::new(),
            _dir: dir,
        }
    }

    /// Con un llavero en memoria: lo que se guarde en él no toca el del sistema.
    async fn start_memory_keyring() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let keyring = std::sync::Arc::new(runpost_core::secrets::MemoryKeyring::new());
        let workspace = Workspace::open_with_keyring(dir.path(), keyring).unwrap();
        let server = runpost_server::start(workspace, None).await.unwrap();
        Self {
            server,
            client: Client::new(),
            _dir: dir,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/api{path}", self.server.port)
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method, self.url(path))
            .header(TOKEN_HEADER, self.server.token.expose());
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        (
            status,
            if text.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).unwrap()
            },
        )
    }

    async fn ok(&self, method: Method, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.call(method, path, body).await;
        assert!(status.is_success(), "{path}: {status} {value}");
        value
    }
}

#[tokio::test]
async fn collections_and_tree_operations() {
    let api = Api::start().await;
    let collection = api
        .ok(Method::POST, "/collections", Some(json!({"name": "Demo"})))
        .await;
    let id = collection["id"].as_str().unwrap().to_owned();
    assert_eq!(
        api.ok(Method::GET, "/collections", None).await[0]["name"],
        "Demo"
    );

    let folder = api
        .ok(
            Method::POST,
            &format!("/collections/{id}/items"),
            Some(json!({"item": {"type": "folder", "id": "", "name": "F"}})),
        )
        .await;
    let folder_id = folder["id"].as_str().unwrap().to_owned();
    let request = api
        .ok(
            Method::POST,
            &format!("/collections/{id}/items"),
            Some(
                json!({"parent_id": folder_id, "item": {"type": "request", "id": "", "name": "get",
                        "method": "GET", "url": "https://httpbin.org/get"}}),
            ),
        )
        .await;
    let request_id = request["id"].as_str().unwrap().to_owned();
    assert!(request_id.starts_with("req_"));

    // PATCH = mezcla superficial (renombrar, guardar la petición entera).
    let patched = api
        .ok(
            Method::PATCH,
            &format!("/collections/{id}/items/{request_id}"),
            Some(json!({"name": "renamed", "headers": [{"key": "X-A", "value": "1"}], "id": "ignored"})),
        )
        .await;
    assert_eq!(
        (patched["name"].as_str(), patched["id"].as_str()),
        (Some("renamed"), Some(request_id.as_str()))
    );

    let duplicated = api
        .ok(
            Method::POST,
            &format!("/collections/{id}/items/{request_id}/duplicate"),
            Some(json!({"name": "copia"})),
        )
        .await;
    let copy_id = duplicated["id"].as_str().unwrap().to_owned();
    api.ok(
        Method::POST,
        &format!("/collections/{id}/items/{copy_id}/move"),
        Some(json!({"parent_id": null})),
    )
    .await;
    let full = api
        .ok(Method::GET, &format!("/collections/{id}"), None)
        .await;
    assert_eq!(full["items"][0]["items"][0]["name"], "renamed");
    assert_eq!(full["items"][1]["name"], "copia");

    let (status, cycle) = api
        .call(
            Method::POST,
            &format!("/collections/{id}/items/{folder_id}/move"),
            Some(json!({"parent_id": folder_id})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(cycle["error"]["code"], "invalid_request");

    // Mover a otra colección.
    let other = api
        .ok(Method::POST, "/collections", Some(json!({"name": "Other"})))
        .await;
    let other_id = other["id"].as_str().unwrap();
    api.ok(
        Method::POST,
        &format!("/collections/{id}/items/{copy_id}/move"),
        Some(json!({"collection_id": other_id})),
    )
    .await;
    assert_eq!(
        api.ok(Method::GET, &format!("/collections/{other_id}"), None)
            .await["items"][0]["name"],
        "copia"
    );

    let (status, _) = api
        .call(
            Method::DELETE,
            &format!("/collections/{id}/items/{request_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, missing) = api
        .call(
            Method::DELETE,
            &format!("/collections/{id}/items/{request_id}"),
            None,
        )
        .await;
    assert_eq!(
        (status, missing["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("item_not_found"))
    );

    let mut whole = api
        .ok(Method::GET, &format!("/collections/{id}"), None)
        .await;
    whole["description"] = json!("docs");
    assert_eq!(
        api.ok(Method::PUT, &format!("/collections/{id}"), Some(whole))
            .await["description"],
        "docs"
    );
    let dup = api
        .ok(
            Method::POST,
            "/collections",
            Some(json!({"collection": full, "name": "Demo 2"})),
        )
        .await;
    assert_ne!(dup["id"], json!(id));
    assert_ne!(dup["items"][0]["id"], json!(folder_id));

    let (status, _) = api
        .call(Method::DELETE, &format!("/collections/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, error) = api
        .call(Method::GET, &format!("/collections/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"]["code"], "collection_not_found");
    let (status, bad) = api
        .call(Method::POST, "/collections", Some(json!({"name": " "})))
        .await;
    assert_eq!(
        (status, bad["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_request"))
    );
    let (status, invalid) = api
        .call(Method::POST, "/collections", Some(json!([1])))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid["error"]["code"], "invalid_body");
    api.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn settings_environments_globals() {
    let api = Api::start().await;
    let mut settings = api.ok(Method::GET, "/settings", None).await;
    assert_eq!(settings["theme"], "system");
    settings["theme"] = json!("dark");
    api.ok(Method::PUT, "/settings", Some(settings)).await;
    assert_eq!(
        api.ok(Method::GET, "/settings", None).await["theme"],
        "dark"
    );

    let env = api
        .ok(Method::POST, "/environments", Some(json!({"name": "DESA"})))
        .await;
    let env_id = env["id"].as_str().unwrap().to_owned();
    let mut env = env;
    env["variables"] =
        json!([{"key": "base", "value": "http://x"}, {"key": "t", "value": "s", "secret": true}]);
    api.ok(Method::PUT, &format!("/environments/{env_id}"), Some(env))
        .await;
    let by_name = api.ok(Method::GET, "/environments/DESA", None).await;
    assert_eq!(by_name["variables"][1]["secret"], true);
    assert_eq!(
        api.ok(Method::GET, "/environments", None)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let (status, _) = api
        .call(Method::DELETE, &format!("/environments/{env_id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let globals = api
        .ok(
            Method::PUT,
            "/globals",
            Some(json!({"schema": 1, "variables": [{"key": "g", "value": "1"}]})),
        )
        .await;
    assert_eq!(globals["variables"][0]["key"], "g");
    assert_eq!(api.ok(Method::GET, "/globals", None).await, globals);
    api.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_history_and_save_to_file() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/get"))
        .and(header("x-demo", "1"))
        .and(header("authorization", "Bearer s3cr3t"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok": true, "echo": "s3cr3t"})),
        )
        .mount(&mock)
        .await;
    let api = Api::start().await;
    let env = api
        .ok(Method::POST, "/environments", Some(json!({"environment": {"schema": 1, "id": "", "name": "E", "variables": [
            {"key": "base", "value": mock.uri()}, {"key": "token", "value": "s3cr3t", "secret": true}]}})))
        .await;
    let draft = json!({"method": "GET", "url": "{{base}}/get", "headers": [{"key": "X-Demo", "value": "1"}],
                       "auth": {"type": "bearer", "token": "{{token}}"}});
    let result = api
        .ok(
            Method::POST,
            "/send",
            Some(json!({"request": draft, "env_id": env["id"]})),
        )
        .await;
    assert_eq!(result["response"]["status"], 200);
    assert_eq!(result["response"]["body_encoding"], "utf8");
    assert!(
        result["response"]["body"]
            .as_str()
            .unwrap()
            .contains("\"ok\":true")
    );
    assert!(
        result["resolved"]["headers"]
            .to_string()
            .contains("Bearer ***")
    );
    let history_id = result["history_id"].as_str().unwrap().to_owned();

    // Guardar el cuerpo real (sin enmascarar) a un archivo elegido por el usuario.
    let out = api._dir.path().join("body.json");
    let saved = api
        .ok(
            Method::POST,
            "/files/save",
            Some(json!({"path": out, "history_id": history_id})),
        )
        .await;
    assert_eq!(saved["ok"], true);
    assert!(std::fs::read_to_string(&out).unwrap().contains("s3cr3t"));
    let (status, _) = api
        .call(
            Method::POST,
            "/files/save",
            Some(json!({"path": "relative.txt", "history_id": history_id})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // El historial guarda la versión enmascarada.
    let rows = api.ok(Method::GET, "/history?limit=5", None).await;
    assert_eq!(rows[0]["id"], json!(history_id));
    let entry = api
        .ok(Method::GET, &format!("/history/{history_id}"), None)
        .await;
    assert_eq!(entry["res_body"]["text"], r#"{"ok":true,"echo":"***"}"#);
    assert_eq!(entry["source"], "ui");

    // Una petición guardada por id y un error de red tipado.
    let collection = api
        .ok(Method::POST, "/collections", Some(json!({"name": "C"})))
        .await;
    let item = api
        .ok(
            Method::POST,
            &format!("/collections/{}/items", collection["id"].as_str().unwrap()),
            Some(json!({"item": {"type": "request", "id": "", "name": "down", "method": "GET", "url": "http://127.0.0.1:9/x"}})),
        )
        .await;
    let (status, error) = api
        .call(
            Method::POST,
            "/send",
            Some(json!({"request": {"request_id": item["id"]}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(error["error"]["code"], "connect");
    assert!(
        error["error"]["history_id"]
            .as_str()
            .unwrap()
            .starts_with("h_")
    );

    let cleared = api.ok(Method::DELETE, "/history", None).await;
    assert_eq!(cleared["deleted"], 2);
    api.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn import_multipart_and_paths_export_and_agent_config() {
    let api = Api::start().await;
    let collection = fixture("Acme-Gateway-Scopes.postman_collection.json");
    let env = fixture("Acme-DESA-fake.postman_environment.json");
    let both = api
        .ok(
            Method::POST,
            "/import",
            Some(json!({"paths": [collection, env]})),
        )
        .await;
    assert_eq!(both["reports"].as_array().unwrap().len(), 2);
    assert_eq!(both["reports"][0]["requests"], 10);
    assert_eq!(both["reports"][1]["kind"], "environment");

    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(std::fs::read(&collection).unwrap())
                .file_name("Acme-Gateway-Scopes.postman_collection.json"),
        )
        .part(
            "file",
            reqwest::multipart::Part::text("{not json").file_name("broken.json"),
        );
    let response = api
        .client
        .post(api.url("/import"))
        .header(TOKEN_HEADER, api.server.token.expose())
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let multipart: Value = response.json().await.unwrap();
    assert_eq!(multipart["reports"][0]["replaced"], true);
    assert_eq!(multipart["errors"][0]["file"], "broken.json");
    let (status, error) = api
        .call(
            Method::POST,
            "/import",
            Some(json!({"format": "raml", "text": "{}"})),
        )
        .await;
    assert_eq!(
        (status, error["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("unsupported_format"))
    );

    let id = both["reports"][0]["id"].as_str().unwrap();
    let response = api
        .client
        .get(api.url(&format!(
            "/export/collections/{id}?format=postman_v21&strip_secrets=1"
        )))
        .header(TOKEN_HEADER, api.server.token.expose())
        .send()
        .await
        .unwrap();
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains(".postman_collection.json")
    );
    let exported: Value = response.json().await.unwrap();
    assert_eq!(
        exported["info"]["schema"],
        "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
    );
    let out = api._dir.path().join("export.json");
    api.ok(
        Method::POST,
        "/files/save",
        Some(json!({"path": out, "export": {"collection_id": id, "strip_secrets": true}})),
    )
    .await;
    assert!(
        std::fs::read_to_string(&out)
            .unwrap()
            .contains("\"_postman_id\"")
    );

    let config = api
        .ok(Method::GET, "/agent/config?client=claude-code", None)
        .await;
    assert!(config["text"].as_str().unwrap().contains("\"mcpServers\""));
    assert!(config["exe"].as_str().unwrap().contains("runpost_mcp"));
    let (status, _) = api
        .call(Method::GET, "/agent/config?client=nope", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    api.server.shutdown().await.unwrap();
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next(socket: &mut Socket) -> Value {
    let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .expect("ws timeout");
    match message.unwrap().unwrap() {
        Message::Text(text) => serde_json::from_str::<Value>(&text).unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn websocket_requires_token_and_reports_workspace_changes() {
    let api = Api::start().await;
    let url = format!("ws://127.0.0.1:{}/ws", api.server.port);
    assert!(
        tokio_tungstenite::connect_async(url.as_str())
            .await
            .is_err(),
        "no token → rejected"
    );

    let mut request = url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("runpost, {}", api.server.token.expose())
            .parse()
            .unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.headers()["sec-websocket-protocol"], "runpost");
    assert_eq!(next(&mut socket).await["type"], "hello");

    // Un cambio hecho "por fuera" (como runpost mcp) llega como workspace_changed.
    let ws = Workspace::open(api._dir.path()).unwrap();
    ws.save_collection(&runpost_core::model::Collection::new("From MCP"))
        .unwrap();
    let event = next(&mut socket).await;
    assert_eq!(
        event,
        json!({"kind": "collection", "slug": "from-mcp", "type": "workspace_changed"})
    );

    let mut hostile = url.as_str().into_client_request().unwrap();
    hostile.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("runpost, {}", api.server.token.expose())
            .parse()
            .unwrap(),
    );
    hostile
        .headers_mut()
        .insert("Origin", "https://evil.example".parse().unwrap());
    assert!(
        tokio_tungstenite::connect_async(hostile).await.is_err(),
        "foreign origin → rejected"
    );
    api.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn export_environment_and_folder_and_save_them() {
    let api = Api::start().await;
    let imported = api
        .ok(
            Method::POST,
            "/import",
            Some(json!({"paths": [fixture("Acme-Gateway-Scopes.postman_collection.json"), fixture("Acme-DESA-fake.postman_environment.json")]})),
        )
        .await;
    let collection_id = imported["reports"][0]["id"].as_str().unwrap().to_owned();
    let env_id = imported["reports"][1]["id"].as_str().unwrap().to_owned();

    // Entorno: con y sin secretos.
    let full = api
        .ok(Method::GET, &format!("/export/environments/{env_id}"), None)
        .await;
    assert!(full.to_string().contains("fake-access-token"));
    let stripped = api
        .ok(
            Method::GET,
            &format!("/export/environments/{env_id}?strip_secrets=1"),
            None,
        )
        .await;
    assert!(!stripped.to_string().contains("fake-access-token"));
    assert_eq!(stripped["values"].as_array().unwrap().len(), 11);

    // Carpeta como colección propia (id distinto: reimportarla no pisa la original).
    let collection = api
        .ok(Method::GET, &format!("/collections/{collection_id}"), None)
        .await;
    let folder_id = collection["items"][1]["id"].as_str().unwrap().to_owned();
    let folder = api
        .ok(
            Method::GET,
            &format!("/export/collections/{collection_id}?folder={folder_id}"),
            None,
        )
        .await;
    assert_eq!(
        folder["info"]["name"],
        "01 - Matriz de scopes (token bajo prueba)"
    );
    assert_eq!(folder["item"].as_array().unwrap().len(), 4);
    assert_ne!(
        folder["info"]["_postman_id"],
        json!("00000000-0000-4000-8000-0000000000c1")
    );
    let request_id = collection["items"][1]["items"][0]["id"].as_str().unwrap();
    let (status, error) = api
        .call(
            Method::GET,
            &format!("/export/collections/{collection_id}?folder={request_id}"),
            None,
        )
        .await;
    assert_eq!(
        (status, error["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_request"))
    );

    // Guardar a archivo los tres tipos de export.
    let dir = api._dir.path();
    let env_file = dir.join("env.json");
    api.ok(
        Method::POST,
        "/files/save",
        Some(json!({"path": env_file, "export_environment": {"environment_id": env_id, "strip_secrets": true}})),
    )
    .await;
    assert!(
        !std::fs::read_to_string(&env_file)
            .unwrap()
            .contains("fake-access-token")
    );
    let folder_file = dir.join("folder.json");
    api.ok(
        Method::POST,
        "/files/save",
        Some(json!({"path": folder_file, "export": {"collection_id": collection_id, "folder_id": folder_id}})),
    )
    .await;
    assert!(
        std::fs::read_to_string(&folder_file)
            .unwrap()
            .contains("#5 POST keys/local")
    );
    let (status, _) = api
        .call(
            Method::POST,
            "/files/save",
            Some(json!({"path": dir.join("x.json"), "history_id": "h", "export_environment": {"environment_id": env_id}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    api.server.shutdown().await.unwrap();
}

async fn socket(api: &Api) -> Socket {
    let url = format!("ws://127.0.0.1:{}/ws", api.server.port);
    let mut request = url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("runpost, {}", api.server.token.expose())
            .parse()
            .unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(next(&mut socket).await["type"], "hello");
    socket
}

/// Eventos del runner de `run_id` hasta `runner_done` (incluido); ignora el resto.
async fn runner_events(socket: &mut Socket, run_id: &str) -> Vec<Value> {
    let mut events = Vec::new();
    loop {
        let event = next(socket).await;
        if event["run_id"] != run_id {
            continue;
        }
        let done = event["type"] == "runner_done";
        events.push(event);
        if done {
            return events;
        }
    }
}

/// Snippets (fase 4): variables del entorno activo, auth heredada de la colección y secretos como
/// `{{nombre}}`, para una petición guardada y para un borrador.
#[tokio::test]
async fn codegen_snippets_resolve_variables_and_keep_secrets() {
    let api = Api::start().await;
    let mut env = api
        .ok(Method::POST, "/environments", Some(json!({"name": "DEV"})))
        .await;
    let env_id = env["id"].as_str().unwrap().to_owned();
    env["variables"] = json!([{"key": "base", "value": "https://api.test"},
                              {"key": "token", "value": "s3cr3t-T0ken", "secret": true}]);
    api.ok(Method::PUT, &format!("/environments/{env_id}"), Some(env))
        .await;
    let mut settings = api.ok(Method::GET, "/settings", None).await;
    settings["active_environment"] = json!(env_id);
    api.ok(Method::PUT, "/settings", Some(settings)).await;

    let collection_id = collection_with(&api, "Snip", &[("Items", "{{base}}/items".into())]).await;
    let mut collection = api
        .ok(Method::GET, &format!("/collections/{collection_id}"), None)
        .await;
    collection["auth"] = json!({"type": "bearer", "token": "{{token}}"});
    let request_id = collection["items"][0]["id"].as_str().unwrap().to_owned();
    api.ok(
        Method::PUT,
        &format!("/collections/{collection_id}"),
        Some(collection),
    )
    .await;

    let saved = api
        .ok(
            Method::POST,
            "/codegen/python",
            Some(json!({"request": {"request_id": request_id}})),
        )
        .await;
    assert_eq!(saved["lang"], "python");
    let code = saved["code"].as_str().unwrap();
    assert!(code.contains(r#"url = "https://api.test/items""#), "{code}");
    assert!(
        code.contains(r#""Authorization": "Bearer {{token}}""#),
        "{code}"
    );
    assert!(!code.contains("s3cr3t"));

    let draft = api
        .ok(
            Method::POST,
            "/codegen/curl",
            Some(json!({"request": {"method": "post", "url": "{{base}}/x",
                "body": {"mode": "raw", "lang": "json", "text": "{}"},
                "collection_id": collection_id}})),
        )
        .await;
    assert_eq!(
        draft["code"],
        "curl --location --request POST 'https://api.test/x' \\\n--header 'Authorization: Bearer {{token}}' \\\n--header 'Content-Type: application/json' \\\n--data-raw '{}'"
    );

    let (status, error) = api
        .call(
            Method::POST,
            "/codegen/go",
            Some(json!({"request": {"request_id": "x"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    api.server.shutdown().await.unwrap();
}

async fn collection_with(api: &Api, name: &str, requests: &[(&str, String)]) -> String {
    let collection = api
        .ok(Method::POST, "/collections", Some(json!({"name": name})))
        .await;
    let id = collection["id"].as_str().unwrap().to_owned();
    for (request, url) in requests {
        api.ok(
            Method::POST,
            &format!("/collections/{id}/items"),
            Some(json!({"item": {
                "type": "request", "id": "", "name": request, "method": "GET", "url": url,
                "scripts": {"pre": "", "test": "pm.test('200', () => pm.response.to.have.status(200)); pm.test('row', () => pm.expect(pm.iterationData.get('x')).to.exist);"}
            }})),
        )
        .await;
    }
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_runs_in_background_with_progress_report_and_cancel() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    let api = Api::start().await;
    let mut socket = socket(&api).await;
    let id = collection_with(
        &api,
        "Run",
        &[
            ("a", format!("{}/a", mock.uri())),
            ("b", format!("{}/b", mock.uri())),
        ],
    )
    .await;

    let started = api
        .ok(
            Method::POST,
            "/runner",
            Some(json!({"collection_id": id, "data": {"name": "d.csv", "text": "x\n1\n2\n"}})),
        )
        .await;
    let run_id = started["run_id"].as_str().unwrap().to_owned();
    let events = runner_events(&mut socket, &run_id).await;
    assert_eq!(events[0]["type"], "runner_progress");
    assert_eq!(events[0]["event"], "started");
    assert_eq!(events[0]["total"], 4);
    assert_eq!(events.len(), 6, "{events:#?}");
    assert_eq!(events[4]["item"]["name"], "b");
    assert_eq!(events[4]["item"]["iteration"], 1);
    let done = events.last().unwrap();
    assert_eq!(done["totals"]["passed"], 8);
    assert_eq!(done["totals"]["failed"], 0);

    let state = api
        .ok(Method::GET, &format!("/runner/{run_id}"), None)
        .await;
    assert_eq!(state["status"], "done");
    assert_eq!(state["report"]["items"].as_array().unwrap().len(), 4);
    assert_eq!(state["report"]["data"], json!({"name": "d.csv", "rows": 2}));

    let html = api
        .client
        .get(api.url(&format!("/runner/{run_id}/report?format=html")))
        .header(TOKEN_HEADER, api.server.token.expose())
        .send()
        .await
        .unwrap();
    assert_eq!(html.headers()["content-type"], "text/html; charset=utf-8");
    assert_eq!(
        html.headers()["content-disposition"],
        "attachment; filename=\"runner-run.html\""
    );
    assert!(
        html.text()
            .await
            .unwrap()
            .contains("<title>Runner · Run</title>")
    );

    let file = api._dir.path().join("informe.json");
    api.ok(
        Method::POST,
        "/files/save",
        Some(json!({"path": file, "runner_report": {"run_id": run_id, "format": "json"}})),
    )
    .await;
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(saved["totals"]["tests"], 8);

    // Cancelar a mitad: la petición lenta se abandona y el informe queda marcado.
    let slow = collection_with(
        &api,
        "Slow",
        &[
            ("slow", format!("{}/slow", mock.uri())),
            ("never", format!("{}/a", mock.uri())),
        ],
    )
    .await;
    let started = api
        .ok(
            Method::POST,
            "/runner",
            Some(json!({"collection_id": slow})),
        )
        .await;
    let run_id = started["run_id"].as_str().unwrap().to_owned();
    tokio::time::sleep(Duration::from_millis(300)).await;
    api.ok(Method::DELETE, &format!("/runner/{run_id}"), None)
        .await;
    let events = runner_events(&mut socket, &run_id).await;
    assert_eq!(events.last().unwrap()["cancelled"], true);
    let state = api
        .ok(Method::GET, &format!("/runner/{run_id}"), None)
        .await;
    assert_eq!(state["report"]["items"], json!([]));

    // Errores de selección y de datos, antes de arrancar.
    let (status, error) = api
        .call(
            Method::POST,
            "/runner",
            Some(json!({"collection_id": "nope"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["error"]["code"], "collection_not_found");
    let (status, _) = api
        .call(
            Method::POST,
            "/runner",
            Some(json!({"collection_id": id, "data": {"name": "d.json", "text": "{}"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = api
        .call(Method::GET, "/runner/run_missing/report", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    api.server.shutdown().await.unwrap();
}

/// Fase 4: OpenAPI (YAML), HAR y curl por `/import`; un curl vuelve como petición sin guardar.
#[tokio::test]
async fn import_openapi_har_and_curl() {
    let api = Api::start().await;
    let openapi = "openapi: 3.0.3\ninfo:\n  title: Mini\nservers:\n  - url: https://mini.test\npaths:\n  /ping:\n    get:\n      summary: Ping\n      responses:\n        '200': { description: ok }\n";
    let result = api
        .ok(Method::POST, "/import", Some(json!({"text": openapi})))
        .await;
    assert_eq!(result["reports"][0]["format"], "openapi");
    assert_eq!(result["reports"][0]["name"], "Mini");
    assert_eq!(result["reports"][0]["requests"], 1);

    let har = json!({"log": {"version": "1.2", "entries": [
        {"request": {"method": "GET", "url": "https://h.test/a?b=1", "headers": []}}]}});
    let result = api
        .ok(
            Method::POST,
            "/import",
            Some(json!({"text": har.to_string()})),
        )
        .await;
    assert_eq!(result["reports"][0]["format"], "har");

    let curl = api
        .ok(
            Method::POST,
            "/import",
            Some(json!({"text": "curl -X POST https://h.test/x -d 'a=1'"})),
        )
        .await;
    assert_eq!(curl["requests"][0]["method"], "POST");
    assert_eq!(curl["requests"][0]["url"], "https://h.test/x");
    assert_eq!(curl["requests"][0]["body"]["mode"], "urlencoded");
    assert!(curl["reports"].as_array().unwrap().is_empty());

    let unknown = api
        .ok(Method::POST, "/import", Some(json!({"text": "hello"})))
        .await;
    assert!(
        unknown["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("unknown format"),
        "{unknown}"
    );
    assert_eq!(
        api.ok(Method::GET, "/collections", None)
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );
    api.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn import_insomnia_and_bruno() {
    let api = Api::start().await;
    let insomnia_v4 = json!({
        "_type": "export", "__export_format": 4,
        "resources": [
            {"_id": "wrk_1", "parentId": null, "_type": "workspace", "name": "Acme Insomnia", "scope": "collection"},
            {"_id": "env_b", "parentId": "wrk_1", "_type": "environment", "name": "Base", "data": {"apiUrl": "http://localhost:3080"}},
            {"_id": "env_d", "parentId": "env_b", "_type": "environment", "name": "DESA", "data": {"apiUrl": "https://desa.example.test"}},
            {"_id": "req_1", "parentId": "wrk_1", "_type": "request", "name": "Ping", "method": "GET",
             "url": "{{ _.apiUrl }}/v1/ping", "authentication": {}}
        ]
    });
    let result = api
        .ok(
            Method::POST,
            "/import",
            Some(json!({"text": insomnia_v4.to_string()})),
        )
        .await;
    let reports = result["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2, "{result}");
    assert_eq!(reports[0]["format"], "insomnia");
    assert_eq!(reports[0]["kind"], "collection");
    assert_eq!(reports[0]["requests"], 1);
    assert_eq!(reports[1]["kind"], "environment");
    assert_eq!(reports[1]["name"], "Acme Insomnia - DESA");
    let id = reports[0]["id"].as_str().unwrap();
    let collection = api
        .ok(Method::GET, &format!("/collections/{id}"), None)
        .await;
    assert_eq!(collection["items"][0]["url"], "{{apiUrl}}/v1/ping");

    let insomnia_v5 = "type: collection.insomnia.rest/5.0\nname: Acme v5\nmeta:\n  id: wrk_5\ncollection:\n  - url: \"{{ _.apiUrl }}/v1/session\"\n    name: Session\n    meta:\n      id: req_5\n    method: GET\n";
    let result = api
        .ok(Method::POST, "/import", Some(json!({"text": insomnia_v5})))
        .await;
    assert_eq!(result["reports"][0]["format"], "insomnia");
    assert_eq!(result["reports"][0]["name"], "Acme v5");

    let bruno_json = json!({
        "version": "1", "name": "Acme Bruno",
        "items": [{"type": "http", "name": "Ping", "seq": 1, "request": {
            "url": "{{apiUrl}}/v1/ping", "method": "GET", "auth": {"mode": "inherit"},
            "assertions": [{"name": "res.status", "value": "eq 200", "enabled": true}]}}],
        "environments": [{"name": "Local", "variables": [
            {"name": "apiUrl", "value": "http://localhost:3080", "enabled": true, "secret": false}]}]
    });
    let result = api
        .ok(
            Method::POST,
            "/import",
            Some(json!({"text": bruno_json.to_string()})),
        )
        .await;
    assert_eq!(result["reports"][0]["format"], "bruno");
    assert_eq!(result["reports"][1]["name"], "Acme Bruno - Local");

    // Carpeta de Bruno por ruta (la app de escritorio manda la ruta del `bruno.json`).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("bruno.json"),
        r#"{"version": "1", "name": "Acme Folder", "type": "collection"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Ping.bru"),
        "meta {\n  name: Ping\n  seq: 1\n}\n\nget {\n  url: {{apiUrl}}/v1/ping\n  auth: inherit\n}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join(".env"), "TOKEN=fake-not-read\n").unwrap();
    let path = dir.path().join("bruno.json").to_string_lossy().into_owned();
    let result = api
        .ok(Method::POST, "/import", Some(json!({"path": path})))
        .await;
    assert_eq!(result["reports"][0]["format"], "bruno", "{result}");
    assert_eq!(result["reports"][0]["name"], "Acme Folder");
    assert_eq!(result["reports"][0]["replaced"], false);
    // Reimportar la misma carpeta sustituye la colección.
    let again = api
        .ok(Method::POST, "/import", Some(json!({"path": path})))
        .await;
    assert_eq!(again["reports"][0]["replaced"], true);
    assert_eq!(again["reports"][0]["id"], result["reports"][0]["id"]);
    api.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn oauth2_tokens_from_the_api() {
    let api = Api::start_memory_keyring().await;
    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(wiremock::matchers::body_string_contains(
            "grant_type=client_credentials",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "fake-cc-token", "token_type": "Bearer", "expires_in": 600}),
        ))
        .mount(&idp)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(wiremock::matchers::body_string_contains(
            "grant_type=authorization_code",
        ))
        .and(wiremock::matchers::body_string_contains("code=fake-code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "fake-user-token", "refresh_token": "fake-refresh",
            "token_type": "Bearer", "expires_in": 600
        })))
        .mount(&idp)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", "Bearer fake-user-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"me": "acme"})))
        .mount(&idp)
        .await;
    let draft =
        |auth: Value| json!({"method": "GET", "url": format!("{}/v1/me", idp.uri()), "auth": auth});

    // Client credentials: estado, pedir, olvidar. Nunca sale el token.
    let cc = draft(
        json!({"type": "oauth2_cc", "token_url": format!("{}/oauth/token", idp.uri()),
                          "client_id": "acme", "client_secret": "fake-secret", "scope": ""}),
    );
    let call = |action: &str| json!({"request": cc.clone(), "action": action});
    assert_eq!(
        api.ok(Method::POST, "/oauth2", Some(call("status"))).await["token"],
        Value::Null
    );
    let got = api.ok(Method::POST, "/oauth2", Some(call("get"))).await;
    assert_eq!(got["token"]["token_type"], "Bearer", "{got}");
    assert!(got["token"]["expires_at"].is_number());
    assert!(!got.to_string().contains("fake-cc-token"));
    assert_eq!(
        api.ok(Method::POST, "/oauth2", Some(call("status"))).await["token"]["refresh"],
        false
    );
    assert_eq!(
        api.ok(Method::POST, "/oauth2", Some(call("clear"))).await["token"],
        Value::Null
    );
    let (status, error) = api
        .call(
            Method::POST,
            "/oauth2",
            Some(json!({"request": draft(json!({"type": "none"}))})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    // Authorization code: la API devuelve la URL (sin abrir el navegador en la prueba), el
    // "navegador" vuelve al callback local y `flows/{id}` da el resultado.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let code = draft(
        json!({"type": "oauth2_code", "auth_url": format!("{}/authorize", idp.uri()),
                            "token_url": format!("{}/oauth/token", idp.uri()), "client_id": "acme-app",
                            "client_secret": "", "scope": "openid",
                            "redirect_uri": format!("http://127.0.0.1:{port}/callback")}),
    );
    let started = api
        .ok(
            Method::POST,
            "/oauth2",
            Some(json!({"request": code.clone(), "action": "get", "open": false})),
        )
        .await;
    let flow = started["flow"].as_str().unwrap().to_owned();
    let url = reqwest::Url::parse(started["authorization_url"].as_str().unwrap()).unwrap();
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    let waiting = {
        let client = api.client.clone();
        let url = api.url(&format!("/oauth2/flows/{flow}"));
        let token = api.server.token.expose().to_owned();
        tokio::spawn(async move {
            let response = client
                .get(url)
                .header(TOKEN_HEADER, token)
                .send()
                .await
                .unwrap();
            (response.status(), response.json::<Value>().await.unwrap())
        })
    };
    let page = reqwest::get(format!(
        "http://127.0.0.1:{port}/callback?code=fake-code&state={state}"
    ))
    .await
    .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let (status, done) = waiting.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["token"]["refresh"], true);

    // El envío usa el token sin que lo vea nadie.
    let sent = api
        .ok(Method::POST, "/send", Some(json!({"request": code})))
        .await;
    assert_eq!(sent["response"]["status"], 200, "{sent}");
    assert!(!sent.to_string().contains("fake-user-token"));
    api.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn cookie_manager() {
    let api = Api::start().await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/login"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("set-cookie", "sid=fake-session; Path=/; HttpOnly"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .and(wiremock::matchers::header_regex(
            "cookie",
            "sid=fake-session",
        ))
        .and(wiremock::matchers::header_regex("cookie", "theme=dark"))
        .respond_with(ResponseTemplate::new(200).set_body_string("me"))
        .expect(1)
        .mount(&server)
        .await;
    let send = |path: &str| json!({"request": {"method": "GET", "url": format!("{}{path}", server.uri())}});
    api.ok(Method::POST, "/send", Some(send("/login"))).await;

    let listed = api.ok(Method::GET, "/cookies", None).await;
    let cookies = listed["cookies"].as_array().unwrap();
    assert_eq!(cookies.len(), 1, "{listed}");
    assert_eq!(cookies[0]["domain"], "127.0.0.1");
    assert_eq!(cookies[0]["name"], "sid");
    assert_eq!(cookies[0]["http_only"], true);
    assert_eq!(cookies[0]["host_only"], true);

    // Añadida a mano desde el gestor: viaja con la siguiente petición.
    let added = api
        .ok(
            Method::PUT,
            "/cookies",
            Some(json!({"domain": "127.0.0.1", "cookie": "theme=dark; Path=/"})),
        )
        .await;
    assert_eq!(added["cookies"].as_array().unwrap().len(), 2);
    let me = api.ok(Method::POST, "/send", Some(send("/me"))).await;
    assert_eq!(me["response"]["status"], 200, "{me}");
    server.verify().await;

    let (status, bad) = api
        .call(
            Method::PUT,
            "/cookies",
            Some(json!({"domain": "https://127.0.0.1", "cookie": "a=1"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");

    let removed = api
        .ok(
            Method::DELETE,
            "/cookies?domain=127.0.0.1&path=/&name=sid",
            None,
        )
        .await;
    assert_eq!(removed["removed"], 1);
    assert_eq!(removed["cookies"].as_array().unwrap().len(), 1);
    let (status, _) = api.call(Method::DELETE, "/cookies?path=/", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let cleared = api.ok(Method::DELETE, "/cookies", None).await;
    assert_eq!(cleared["removed"], 1);
    assert!(cleared["cookies"].as_array().unwrap().is_empty());
    api.server.shutdown().await.unwrap();
}
