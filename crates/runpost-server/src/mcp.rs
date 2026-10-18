//! MCP por Streamable HTTP servido por la app (sprint 6): `http://127.0.0.1:<puerto>/mcp`, con las
//! mismas tools que stdio y protegido por el mismo token local que `/api` (cabecera
//! `X-RunPost-Token` o `Authorization: Bearer`).
//!
//! El puerto es fijo (7879 por defecto, `settings.mcp.http_port`) para que la configuración del
//! agente se pegue una sola vez. El token sigue siendo de un solo arranque (sección 13): los
//! clientes lo piden en cada conexión con `runpost_mcp mcp-headers` (su `headersHelper`), que lo
//! lee del llavero del sistema (cuenta `session/<puerto>`), donde la app lo deja al arrancar y lo
//! borra al cerrar.

use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use runpost_core::{TOKEN_HEADER, Workspace};
use runpost_mcp::{agent::AgentHub, tools::Tools};
use serde::Serialize;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::SessionToken;

// SPEC-DEVIATION: S6-D1 — el MCP por HTTP tiene su propio puerto fijo (la API de la UI sigue en uno
// aleatorio, sección 7) y, mientras la app está abierta, su token se publica en el llavero del
// sistema para el `headersHelper` de los clientes (la sección 13 lo dejaba solo en memoria).

/// Para el panel Agente: dónde escucha el MCP o por qué no arrancó.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Status {
    pub port: Option<u16>,
    pub url: Option<String>,
    pub error: Option<String>,
}

pub(crate) struct Listener {
    pub port: u16,
    cancel: CancellationToken,
    signal: oneshot::Sender<()>,
    task: JoinHandle<std::io::Result<()>>,
    workspace: Workspace,
    token: SessionToken,
}

impl Listener {
    pub async fn stop(self) {
        self.workspace.clear_session(self.port, self.token.expose());
        // Cierra las sesiones MCP (flujos SSE abiertos) y deja de aceptar conexiones.
        self.cancel.cancel();
        let _ = self.signal.send(());
        let mut task = self.task;
        if tokio::time::timeout(Duration::from_secs(3), &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
    }
}

pub(crate) async fn start(
    port: u16,
    tools: Tools,
    hub: Arc<AgentHub>,
    token: SessionToken,
    workspace: &Workspace,
) -> (Option<Listener>, Status) {
    let failed = |error: String| {
        tracing::warn!(port, "MCP over HTTP not started: {error}");
        (
            None,
            Status {
                error: Some(error),
                ..Status::default()
            },
        )
    };
    let listener = match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).await {
        Ok(listener) => listener,
        Err(error) => return failed(format!("127.0.0.1:{port}: {error}")),
    };
    let port = match listener.local_addr() {
        Ok(address) => address.port(),
        Err(error) => return failed(error.to_string()),
    };
    let cancel = CancellationToken::new();
    let router = Router::new()
        .route_service(
            "/mcp",
            runpost_mcp::http_service(tools, hub, port, cancel.clone()),
        )
        .layer(middleware::from_fn_with_state(token.clone(), authenticate));
    let (signal, stop) = oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stop.await;
            })
            .await
    });
    // `workspace`: el puente stdio (`runpost_mcp mcp`) solo se conecta a la app del mismo workspace.
    let session = serde_json::json!({"port": port, "token": token.expose(),
                                     "workspace": workspace.root().to_string_lossy()})
    .to_string();
    if let Err(error) = workspace.publish_session(port, &session) {
        tracing::warn!("MCP over HTTP: connection data not saved in the keyring: {error}");
    }
    let url = format!("http://127.0.0.1:{port}/mcp");
    tracing::info!(%url, "MCP over HTTP listening");
    (
        Some(Listener {
            port,
            cancel,
            signal,
            task,
            workspace: workspace.clone(),
            token,
        }),
        Status {
            port: Some(port),
            url: Some(url),
            error: None,
        },
    )
}

/// El mismo token que `/api`: `X-RunPost-Token` o `Authorization: Bearer <token>`.
async fn authenticate(State(token): State<SessionToken>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let by_header = text(TOKEN_HEADER).is_some_and(|value| value == token.expose());
    let by_bearer = text(header::AUTHORIZATION.as_str())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|value| value.trim() == token.expose());
    if !by_header && !by_bearer {
        return (
            StatusCode::UNAUTHORIZED,
            "unauthorized: missing or wrong X-RunPost-Token (is RunPost open?)",
        )
            .into_response();
    }
    next.run(request).await
}
