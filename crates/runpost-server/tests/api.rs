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
            Some(json!({"format": "openapi", "text": "{}"})),
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
