//! API local con autenticación por arranque, Origin y bind IPv4 fijo.
use std::{
    collections::VecDeque,
    fmt,
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use runpost_core::{Executor, TOKEN_HEADER, Workspace};
use runpost_mcp::{agent::AgentHub, tools::Tools};
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{broadcast, oneshot},
    task::JoinHandle,
};
use tower_http::cors::CorsLayer;

mod api;
mod mcp;
mod runner;
mod ws;

pub use api::{agent_config_text, display_path};

#[derive(Clone)]
pub struct SessionToken(String);

impl SessionToken {
    fn generate() -> Result<Self> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).context("cannot generate local authentication token")?;
        let value = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(***)")
    }
}

pub struct Shutdown {
    signal: Option<oneshot::Sender<()>>,
    task: JoinHandle<std::io::Result<()>>,
    mcp: Option<mcp::Listener>,
}

impl Shutdown {
    pub async fn wait(mut self) -> Result<()> {
        if let Some(mcp) = self.mcp.take() {
            mcp.stop().await;
        }
        if let Some(signal) = self.signal.take() {
            let _ = signal.send(());
        }
        self.task
            .await
            .context("local server task failed")?
            .context("local server failed")
    }
}

pub struct ServerHandle {
    pub port: u16,
    pub token: SessionToken,
    /// Puerto del MCP por HTTP si está activo.
    pub mcp_port: Option<u16>,
    pub shutdown: Shutdown,
}

impl ServerHandle {
    pub async fn shutdown(self) -> Result<()> {
        self.shutdown.wait().await
    }
}

#[derive(Default, Clone, Copy)]
pub struct ServerOptions {
    pub dev: bool,
    pub browser: bool,
    /// MCP por HTTP en `127.0.0.1:<puerto>/mcp` (sprint 6): `None` lo apaga y `Some(0)` usa un
    /// puerto libre (pruebas). La app y `serve` pasan `settings.mcp.http_port`.
    pub mcp_port: Option<u16>,
}

#[derive(Clone)]
struct AppState {
    port: u16,
    token: SessionToken,
    origins: Arc<Vec<HeaderValue>>,
    browser: bool,
    workspace: Workspace,
    executor: Arc<Executor>,
    /// Cuerpos reales de las últimas respuestas enviadas desde la UI (para guardarlos a archivo).
    recent: Arc<api::RecentBodies>,
    /// Eventos de la app hacia los WebSocket (runner).
    events: broadcast::Sender<Value>,
    /// Ejecuciones del runner lanzadas desde la UI.
    runs: Arc<runner::Runs>,
    /// Panel Agente: llamadas MCP por HTTP, medidor y aprobaciones.
    agent: Arc<AgentHub>,
    /// Estado del MCP por HTTP para el panel (puerto o por qué no arrancó).
    mcp: Arc<mcp::Status>,
}

pub async fn start(workspace: Workspace, bind_port: Option<u16>) -> Result<ServerHandle> {
    start_with_options(workspace, bind_port, ServerOptions::default()).await
}

pub async fn start_with_options(
    workspace: Workspace,
    bind_port: Option<u16>,
    options: ServerOptions,
) -> Result<ServerHandle> {
    let listener = TcpListener::bind(SocketAddrV4::new(
        Ipv4Addr::LOCALHOST,
        bind_port.unwrap_or(0),
    ))
    .await
    .context("cannot bind local server on 127.0.0.1")?;
    let port = listener.local_addr()?.port();
    let token = SessionToken::generate()?;
    let mut origins = vec![
        "tauri://localhost".to_owned(),
        "http://tauri.localhost".to_owned(),
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    if options.dev {
        origins.push("http://localhost:5173".to_owned());
    }
    let origins: Vec<HeaderValue> = origins
        .into_iter()
        .map(|s| s.parse())
        .collect::<Result<_, _>>()?;
    let cors = CorsLayer::new()
        .allow_origin(origins.clone())
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::HeaderName::from_bytes(TOKEN_HEADER.as_bytes())?,
        ]);
    let events = broadcast::channel(1024).0;
    let agent = Arc::new(AgentHub::new(events.clone()));
    let executor = Arc::new(Executor::new());
    let (mcp_listener, mcp_status) = match options.mcp_port {
        Some(mcp_port) => {
            mcp::start(
                mcp_port,
                Tools::new(workspace.clone())
                    .with_executor(executor.clone())
                    .with_hub(agent.clone()),
                agent.clone(),
                token.clone(),
                &workspace,
            )
            .await
        }
        None => (None, mcp::Status::default()),
    };
    let mcp_port = mcp_listener.as_ref().map(|listener| listener.port);
    let state = AppState {
        port,
        token: token.clone(),
        origins: Arc::new(origins),
        browser: options.browser,
        workspace,
        executor,
        recent: Arc::new(Mutex::new(VecDeque::new())),
        events,
        runs: Arc::new(runner::Runs::default()),
        agent,
        mcp: Arc::new(mcp_status),
    };
    let api = api::routes()
        .fallback(|| async { api_error(StatusCode::NOT_FOUND, "not_found") })
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let socket = Router::new()
        .route("/ws", get(ws::handler))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let router = Router::new()
        .nest("/api", api)
        .merge(socket)
        .fallback(frontend)
        .layer(cors)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            validate_origin,
        ))
        .with_state(state);
    let (signal, stop) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stop.await;
            })
            .await
    });
    tracing::info!(port, "local server listening on 127.0.0.1");
    Ok(ServerHandle {
        port,
        token,
        mcp_port,
        shutdown: Shutdown {
            signal: Some(signal),
            task,
            mcp: mcp_listener,
        },
    })
}

fn api_error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"error":{"code":code,"message":code}}))).into_response()
}

/// Token en `X-RunPost-Token` o, para el WebSocket del navegador, como subprotocolo.
async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let by_header = headers
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == state.token.expose());
    let by_protocol = request.uri().path() == "/ws"
        && headers
            .get(header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.split(',').any(|p| p.trim() == state.token.expose()));
    if !by_header && !by_protocol {
        return api_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    next.run(request).await
}

async fn validate_origin(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if let Some(origin) = request.headers().get(header::ORIGIN)
        && !state.origins.contains(origin)
    {
        return api_error(StatusCode::FORBIDDEN, "origin_not_allowed");
    }
    // También evita DNS rebinding: el Host debe ser el loopback del puerto activo.
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !host.is_some_and(|value| {
        value == format!("127.0.0.1:{}", state.port) || value == format!("localhost:{}", state.port)
    }) {
        return api_error(StatusCode::FORBIDDEN, "host_not_allowed");
    }
    next.run(request).await
}

#[cfg(feature = "embed-ui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../ui/dist"]
struct Assets;

async fn frontend(State(state): State<AppState>, request: Request) -> Response {
    let path = request.uri().path();
    if !state.browser || path == "/api" || path.starts_with("/api/") || path == "/ws" {
        return api_error(StatusCode::NOT_FOUND, "not_found");
    }
    #[cfg(feature = "embed-ui")]
    {
        let path = path.trim_start_matches('/');
        if !path.is_empty()
            && path != "index.html"
            && let Some(asset) = Assets::get(path)
        {
            let mime = if path.ends_with(".js") {
                "text/javascript"
            } else if path.ends_with(".css") {
                "text/css"
            } else if path.ends_with(".svg") {
                "image/svg+xml"
            } else {
                "application/octet-stream"
            };
            return (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                asset.data.into_owned(),
            )
                .into_response();
        }
        let Some(index) = Assets::get("index.html") else {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "ui_missing");
        };
        let config = json!({"port":state.port,"token":state.token.expose(),"mode":"browser","workspace":display_path(state.workspace.root())});
        // JSON dentro de HTML: escapar '<' impide cerrar el script desde una ruta de workspace.
        let config = config
            .to_string()
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026");
        let index = String::from_utf8_lossy(&index.data).replace(
            "<!-- RUNPOST_BACKEND -->",
            &format!("<script>window.__RUNPOST_BACKEND__={config};</script>"),
        );
        ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'self'; connect-src 'self' http://127.0.0.1:* ws://127.0.0.1:*; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'self'")], axum::body::Body::from(index)).into_response()
    }
    #[cfg(not(feature = "embed-ui"))]
    api_error(StatusCode::NOT_FOUND, "ui_not_embedded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn request(port: u16, path: &str, headers: &str, method: &str) -> Result<String> {
        let mut stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
        stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n{headers}\r\n").as_bytes()).await?;
        let mut output = String::new();
        stream.read_to_string(&mut output).await?;
        Ok(output)
    }

    #[tokio::test]
    async fn health_auth_origin_cors_and_shutdown() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let handle = start(Workspace::open(directory.path())?, None).await?;
        assert_ne!(handle.port, 0);
        assert_eq!(handle.token.expose().len(), 64);
        let auth = format!("{TOKEN_HEADER}: {}\r\n", handle.token.expose());
        let missing = request(handle.port, "/api/health", "", "GET").await?;
        assert!(missing.starts_with("HTTP/1.1 401"));
        let wrong = request(
            handle.port,
            "/api/health",
            "X-RunPost-Token: fake\r\n",
            "GET",
        )
        .await?;
        assert!(wrong.starts_with("HTTP/1.1 401"));
        let valid = request(handle.port, "/api/health", &auth, "GET").await?;
        assert!(valid.starts_with("HTTP/1.1 200"));
        assert!(valid.ends_with(r#"{"ok":true}"#));
        for origin in ["https://evil.example", "http://localhost:5173", "null"] {
            let response = request(
                handle.port,
                "/api/health",
                &format!("{auth}Origin: {origin}\r\n"),
                "GET",
            )
            .await?;
            assert!(
                response.starts_with("HTTP/1.1 403"),
                "rejected origin must fail"
            );
        }
        for origin in [
            "tauri://localhost".to_owned(),
            "http://tauri.localhost".to_owned(),
            format!("http://127.0.0.1:{}", handle.port),
        ] {
            let response = request(
                handle.port,
                "/api/health",
                &format!("{auth}Origin: {origin}\r\n"),
                "GET",
            )
            .await?;
            assert!(response.starts_with("HTTP/1.1 200"));
            assert!(response.contains(&format!("access-control-allow-origin: {origin}")));
        }
        let preflight = request(handle.port, "/api/health", "Origin: http://tauri.localhost\r\nAccess-Control-Request-Method: GET\r\nAccess-Control-Request-Headers: x-runpost-token\r\n", "OPTIONS").await?;
        assert!(preflight.starts_with("HTTP/1.1 200"));
        assert!(!format!("{:?}", handle.token).contains(handle.token.expose()));
        let port = handle.port;
        handle.shutdown().await?;
        assert!(
            tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn explicit_dev_origin_and_distinct_tokens() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let ws = Workspace::open(directory.path())?;
        let first = start_with_options(
            ws.clone(),
            None,
            ServerOptions {
                dev: true,
                ..ServerOptions::default()
            },
        )
        .await?;
        let second = start(ws, None).await?;
        assert_ne!(first.token.expose(), second.token.expose());
        let response = request(
            first.port,
            "/api/health",
            &format!(
                "{TOKEN_HEADER}: {}\r\nOrigin: http://localhost:5173\r\n",
                first.token.expose()
            ),
            "GET",
        )
        .await?;
        assert!(response.starts_with("HTTP/1.1 200"));
        first.shutdown().await?;
        second.shutdown().await?;
        Ok(())
    }

    #[cfg(feature = "embed-ui")]
    #[tokio::test]
    async fn browser_injection_is_private_and_api_never_falls_back_to_html() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let server = start_with_options(
            Workspace::open(directory.path())?,
            None,
            ServerOptions {
                browser: true,
                ..ServerOptions::default()
            },
        )
        .await?;
        let html = request(server.port, "/", "", "GET").await?;
        assert!(html.contains("window.__RUNPOST_BACKEND__="));
        assert!(html.contains(server.token.expose()));
        assert!(html.contains("no-store"));
        let bad_origin =
            request(server.port, "/", "Origin: https://evil.example\r\n", "GET").await?;
        assert!(!bad_origin.contains(server.token.expose()));
        let unknown = request(server.port, "/api/missing", "", "GET").await?;
        assert!(!unknown.contains(server.token.expose()));
        server.shutdown().await?;
        Ok(())
    }
}
