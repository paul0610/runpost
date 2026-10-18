//! Puente stdio → app (sprint 7). Con la app abierta sobre el mismo workspace, `runpost_mcp mcp`
//! reenvía cada llamada a su MCP por HTTP: así un cliente configurado por stdio (por ejemplo el plugin
//! de Claude Code) también aparece en el panel Agente y pasa por sus aprobaciones. Si la app no está,
//! o se cierra entre dos llamadas, las atiende él mismo, como siempre. Si se cierra con una llamada
//! en curso, no la repite: la app pudo ejecutarla ya.
// SPEC-DEVIATION: S7-D2: el plugin registra solo stdio y este puente lleva sus llamadas a la app.

use std::{
    net::{Ipv4Addr, SocketAddr, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler, ServiceError, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ClientConfig, ContentBlock,
        ListToolsResult, PaginatedRequestParams, ServerConfig,
    },
    service::{RequestContext, RunningService},
    transport::{
        StreamableHttpClientTransport, common::client_side_sse::FixedInterval,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use runpost_core::Workspace;
use tokio::sync::Mutex;

use crate::RunPostMcp;

/// Lo que recibe el modelo si la app deja de responder con su llamada en curso.
const INTERRUPTED: &str = r#"{"error":"app_unavailable","hint":"the RunPost app stopped answering mid-call; it may have run, check history before retrying. Next calls run without the app."}"#;

/// La app que sirve MCP por HTTP sobre este workspace.
#[derive(Clone, PartialEq, Eq)]
pub struct AppSession {
    pub url: String,
    pub token: String,
}

/// Sin el token: nunca en logs.
impl std::fmt::Debug for AppSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppSession")
            .field("url", &self.url)
            .field("token", &"***")
            .finish()
    }
}

/// Busca la app abierta: el puerto de `settings.mcp.http_port`, su entrada en el llavero (la deja la
/// app al arrancar), que sea el mismo workspace y que de verdad escuche.
pub fn find_app(workspace: &Workspace) -> Option<AppSession> {
    let port = workspace.settings().mcp.http_port?;
    let text = workspace.session(port).ok().flatten()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if value["workspace"].as_str() != Some(&*workspace.root().to_string_lossy()) {
        return None;
    }
    let token = value["token"].as_str()?.to_owned();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&address, Duration::from_millis(300)).ok()?;
    Some(AppSession {
        url: format!("http://127.0.0.1:{port}/mcp"),
        token,
    })
}

/// Por qué no salió bien un reenvío.
enum Failure {
    /// La llamada no llegó a la app (no escucha, o no se pudo abrir la sesión): se atiende en local.
    Unreachable(String),
    /// La app pudo recibirla y ejecutarla: repetirla en local podría enviar dos veces una petición.
    Interrupted(String),
    /// La app la rechazó con un error de protocolo (tool desconocida, parámetros): se devuelve tal cual.
    Rejected(ErrorData),
}

pub struct Bridge {
    app: AppSession,
    local: RunPostMcp,
    client: Mutex<Option<RunningService<RoleClient, ClientConfig>>>,
    /// La app dejó de responder: el resto de la sesión se atiende en local.
    down: AtomicBool,
}

impl Bridge {
    pub fn new(app: AppSession, local: RunPostMcp) -> Self {
        Self {
            app,
            local,
            client: Mutex::new(None),
            down: AtomicBool::new(false),
        }
    }

    pub fn app(&self) -> &AppSession {
        &self.app
    }

    /// ¿Sigue escuchando la app? Mirarlo antes de enviar separa "se cerró entre dos llamadas" (se
    /// puede atender en local) de "se cerró con la llamada en curso".
    async fn listening(&self) -> bool {
        let Some(address) = self
            .app
            .url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .and_then(|authority| authority.parse::<SocketAddr>().ok())
        else {
            return false;
        };
        let connect = tokio::net::TcpStream::connect(address);
        matches!(
            tokio::time::timeout(Duration::from_millis(300), connect).await,
            Ok(Ok(_))
        )
    }

    /// Reenvía la llamada a la app. La primera abre la conexión presentándose con el nombre del
    /// cliente real (el panel dice "Claude Code", no "runpost").
    async fn forward(
        &self,
        request: CallToolRequestParams,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, Failure> {
        if !self.listening().await {
            return Err(Failure::Unreachable("not listening".into()));
        }
        let peer = {
            let mut client = self.client.lock().await;
            // Una sesión ya cerrada se abre de nuevo: esta llamada todavía no ha salido.
            if client
                .as_ref()
                .is_some_and(|running| running.is_closed() || running.is_transport_closed())
            {
                *client = None;
            }
            if client.is_none() {
                let mut info = ClientConfig::default();
                if let Some(original) = context.client_info() {
                    info.client_info = original;
                }
                let mut config =
                    StreamableHttpClientTransportConfig::with_uri(self.app.url.clone())
                        .auth_header(self.app.token.clone());
                // rmcp reintenta sin límite reabrir un stream cortado: con la app cerrada, la llamada
                // se quedaría esperando para siempre.
                let mut retry = FixedInterval::default();
                retry.max_times = Some(2);
                retry.duration = Duration::from_millis(500);
                config.retry_config = Arc::new(retry);
                let transport = StreamableHttpClientTransport::from_config(config);
                let running = info
                    .serve(transport)
                    .await
                    .map_err(|e| Failure::Unreachable(e.to_string()))?;
                *client = Some(running);
            }
            client
                .as_ref()
                .map(|running| running.peer().clone())
                .ok_or_else(|| Failure::Unreachable("no client".into()))?
        };
        peer.call_tool_once(request)
            .await
            .map_err(|error| match error {
                ServiceError::McpError(error) => Failure::Rejected(error),
                other => Failure::Interrupted(other.to_string()),
            })
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        self.local.get_info()
    }

    /// Las mismas tools que en local: no hace falta preguntar a la app.
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.local.list_tools(request, context).await
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !self.down.load(Ordering::Relaxed) {
            match self.forward(request.clone(), &context).await {
                Ok(response) => return Ok(response),
                Err(Failure::Rejected(error)) => return Err(error),
                Err(Failure::Unreachable(error)) => {
                    tracing::warn!("RunPost app unreachable, serving locally: {error}");
                    self.down.store(true, Ordering::Relaxed);
                }
                Err(Failure::Interrupted(error)) => {
                    tracing::warn!("RunPost app stopped answering mid-call: {error}");
                    self.down.store(true, Ordering::Relaxed);
                    return Ok(CallToolResult::error(vec![ContentBlock::text(INTERRUPTED)]).into());
                }
            }
        }
        self.local.call_tool(request, context).await
    }
}
