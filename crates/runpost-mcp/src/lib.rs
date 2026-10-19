//! MCP por stdio (stdout pertenece exclusivamente al transporte JSON-RPC de rmcp) y por
//! Streamable HTTP desde la app (sección 8 y sprint 6), con las mismas tools.
use std::{
    fs::OpenOptions,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::ToolCallContext, wrapper::Parameters},
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ResultType, ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::{
        stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use runpost_core::{APP_SLUG, Workspace};
use serde_json::Value;
use tracing_subscriber::fmt::writer::MakeWriterExt;

pub mod agent;
pub mod bridge;
pub mod params;
pub mod tools;

use agent::{AgentHub, Caller};
use tools::{Reply, Tools};

const INSTRUCTIONS: &str = "HTTP client. Start with ws_overview; refer to saved requests by id or path. \
Variables and secrets resolve server-side (secrets show as ***). Save tokens: select (JSONPath), \
get_response with range instead of re-running.";

#[derive(Clone)]
pub struct RunPostMcp {
    tools: Tools,
    /// Solo por HTTP desde la app: panel Agente y aprobaciones.
    hub: Option<Arc<AgentHub>>,
    tool_router: ToolRouter<Self>,
}

/// Cliente y sesión de la petición MCP (por HTTP, `Mcp-Session-Id`).
fn caller_of(context: &RequestContext<RoleServer>) -> Caller {
    let info = context.client_info();
    let session = context
        .extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.headers.get("mcp-session-id"))
        .and_then(|value| value.to_str().ok());
    Caller::new(
        info.as_ref().map_or("", |i| i.name.as_str()),
        info.as_ref().map_or("", |i| i.version.as_str()),
        session,
    )
}

fn caller_in(context: &RequestContext<RoleServer>) -> Option<Caller> {
    context.extensions.get::<Caller>().cloned()
}

/// Una línea de log por llamada: nombre, resultado, bytes y ms. Nunca el contenido.
fn reply(name: &str, started: Instant, result: Reply) -> Result<String, String> {
    let (ok, text) = match result {
        Ok(value) => (true, value.to_string()),
        Err(value) => (false, value.to_string()),
    };
    tracing::info!(
        tool = name,
        ok,
        bytes = text.len(),
        ms = started.elapsed().as_millis() as u64
    );
    if ok { Ok(text) } else { Err(text) }
}

#[tool_router]
impl RunPostMcp {
    pub fn new(workspace: Workspace) -> Self {
        Self::with_tools(Tools::new(workspace), None)
    }

    /// Para HTTP: las mismas `Tools` (caché, límite por minuto) en todas las sesiones, y el hub.
    pub fn with_tools(tools: Tools, hub: Option<Arc<AgentHub>>) -> Self {
        let mut tool_router = Self::tool_router();
        for route in tool_router.map.values_mut() {
            let mut schema = Value::Object(route.attr.input_schema.as_ref().clone());
            compact_schema(&mut schema);
            if let Value::Object(map) = schema {
                route.attr.input_schema = map.into();
            }
        }
        Self {
            tools,
            hub,
            tool_router,
        }
    }

    #[tool(description = "Collections (with request counts) and environments. Call first.")]
    async fn ws_overview(&self) -> Result<String, String> {
        reply("ws_overview", Instant::now(), self.tools.ws_overview())
    }

    #[tool(
        description = "List saved requests: id, path, method, unresolved url. Filter by collection, folder or q."
    )]
    async fn list_requests(
        &self,
        Parameters(p): Parameters<params::ListRequests>,
    ) -> Result<String, String> {
        reply("list_requests", Instant::now(), self.tools.list_requests(p))
    }

    #[tool(
        description = "Saved request in native format, only the requested fields; or as a curl, fetch, PowerShell or Python snippet."
    )]
    async fn get_request(
        &self,
        Parameters(p): Parameters<params::GetRequest>,
    ) -> Result<String, String> {
        reply("get_request", Instant::now(), self.tools.get_request(p))
    }

    #[tool(
        description = "Run a saved request. Compact result: status, ms, body (minified, truncated), response_id. Secrets masked."
    )]
    async fn run_request(
        &self,
        Parameters(p): Parameters<params::RunRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let caller = caller_in(&context);
        reply(
            "run_request",
            started,
            self.tools.run_request(p, caller.as_ref()).await,
        )
    }

    #[tool(
        description = "Send an ad hoc request ({{vars}} allowed). Optional save_as stores it in a collection."
    )]
    async fn send(&self, Parameters(p): Parameters<params::Send>) -> Result<String, String> {
        let started = Instant::now();
        reply("send", started, self.tools.send(p).await)
    }

    #[tool(
        description = "Read a previous response by response_id without re-running: select, char range, headers, or diff_against \"ref\"/another response_id."
    )]
    async fn get_response(
        &self,
        Parameters(p): Parameters<params::GetResponse>,
    ) -> Result<String, String> {
        reply("get_response", Instant::now(), self.tools.get_response(p))
    }

    // SPEC-DEVIATION: S5-D1 — la 13.ª tool: 8.1 pide ≤ 12, pero el catálogo de 8.2 incluye
    // `run_collection` y el sprint 5 la exige; `tools/list` sigue por debajo de 2 500 tokens.
    #[tool(
        description = "Run a collection or folder with its pre-request and test scripts. output: summary (default), failures or full."
    )]
    async fn run_collection(
        &self,
        Parameters(p): Parameters<params::RunCollection>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let caller = caller_in(&context);
        reply(
            "run_collection",
            started,
            self.tools.run_collection(p, caller.as_ref()).await,
        )
    }

    #[tool(description = "Environment variables (default: active). Secrets as ***.")]
    async fn env_get(&self, Parameters(p): Parameters<params::EnvGet>) -> Result<String, String> {
        reply("env_get", Instant::now(), self.tools.env_get(p))
    }

    #[tool(description = "Set environment variables; secret lists keys to mark secret.")]
    async fn env_set(&self, Parameters(p): Parameters<params::EnvSet>) -> Result<String, String> {
        reply("env_set", Instant::now(), self.tools.env_set(p))
    }

    #[tool(
        description = "Create or update requests by collection+folder+name. Use requests:[…] to build a whole collection in one call."
    )]
    async fn upsert_request(
        &self,
        Parameters(p): Parameters<params::Upsert>,
    ) -> Result<String, String> {
        reply(
            "upsert_request",
            Instant::now(),
            self.tools.upsert_request(p),
        )
    }

    #[tool(
        description = "Import a Postman v2.1 collection or environment from a path or pasted text."
    )]
    async fn import_postman(
        &self,
        Parameters(p): Parameters<params::ImportPostman>,
    ) -> Result<String, String> {
        reply(
            "import_postman",
            Instant::now(),
            self.tools.import_postman(p),
        )
    }

    #[tool(
        description = "Recent executions, newest first: id, ts, method, masked url, status, ms."
    )]
    async fn history(&self, Parameters(p): Parameters<params::History>) -> Result<String, String> {
        reply("history", Instant::now(), self.tools.history(p))
    }

    #[tool(description = "Find saved requests whose name, url, headers or body contain q.")]
    async fn search(&self, Parameters(p): Parameters<params::Search>) -> Result<String, String> {
        reply("search", Instant::now(), self.tools.search(p))
    }
}

#[tool_handler]
impl ServerHandler for RunPostMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(APP_SLUG, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    /// Como el que genera `#[tool_handler]`, más el panel Agente cuando hay hub (HTTP): quién
    /// llama, qué tool, resultado, ms y bytes crudos frente a enviados.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        mut context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(hub) = self.hub.clone() else {
            let call = ToolCallContext::new(self, request, context);
            return self.tool_router.call(call).await;
        };
        let caller = caller_of(&context);
        hub.seen(&caller, true);
        context.extensions.insert(caller.clone());
        let tool = request.name.to_string();
        let args = Value::Object(request.arguments.clone().unwrap_or_default());
        let started = Instant::now();
        let call = ToolCallContext::new(self, request, context);
        let result = self.tool_router.call(call).await;
        let (text, is_error) = match &result {
            Ok(CallToolResponse::Complete(done)) => (
                done.content
                    .first()
                    .and_then(|c| c.as_text())
                    .map(|t| t.text.clone())
                    .unwrap_or_default(),
                done.is_error.unwrap_or(false),
            ),
            Ok(_) => (String::new(), false),
            Err(error) => (error.message.to_string(), true),
        };
        hub.record(
            self.tools
                .observe(&tool, &args, &text, is_error, started.elapsed(), &caller),
        );
        result
    }

    /// Igual que el generado; con hub, `tools/list` (lo primero al conectar) marca al cliente
    /// como conectado en el panel.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if let Some(hub) = &self.hub {
            hub.seen(&caller_of(&context), false);
        }
        let supports_cache_hints = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
            ttl_ms: supports_cache_hints.then_some(0),
            cache_scope: supports_cache_hints.then_some(CacheScope::Public),
        })
    }
}

/// Servicio Streamable HTTP para montar en `/mcp` (la app lo protege con el token local). Solo
/// acepta `Host` de loopback con ese puerto y rechaza cualquier petición con `Origin` (un
/// navegador): los clientes MCP no lo envían.
/// `cancel` cierra las sesiones abiertas (los flujos SSE) al apagar la app.
pub fn http_service(
    tools: Tools,
    hub: Arc<AgentHub>,
    port: u16,
    cancel: tokio_util::sync::CancellationToken,
) -> StreamableHttpService<RunPostMcp, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
        .enforce_origin_validation()
        .with_sse_keep_alive(Some(Duration::from_secs(15)))
        .with_cancellation_token(cancel);
    StreamableHttpService::new(
        move || Ok(RunPostMcp::with_tools(tools.clone(), Some(hub.clone()))),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

/// Quita del esquema lo que no ayuda al modelo y cuesta tokens (8.1 punto 6):
/// `$schema`, `title`, `format`, `minimum: 0` de enteros sin signo y el `null` de los opcionales.
pub fn compact_schema(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            map.remove("$schema");
            map.remove("title");
            map.remove("format");
            if map.get("minimum") == Some(&Value::from(0)) {
                map.remove("minimum");
            }
            if map.get("default") == Some(&Value::Null) {
                map.remove("default");
            }
            if let Some(Value::Array(types)) = map.get("type") {
                let non_null: Vec<Value> = types.iter().filter(|t| *t != "null").cloned().collect();
                if non_null.len() == 1 {
                    map.insert("type".into(), non_null[0].clone());
                }
            }
            // Option<Struct> → anyOf [{$ref}, {type:null}] → {$ref}
            if let Some(Value::Array(variants)) = map.get("anyOf") {
                let non_null: Vec<Value> = variants
                    .iter()
                    .filter(|v| v.get("type") != Some(&Value::from("null")))
                    .cloned()
                    .collect();
                if non_null.len() == 1 && variants.len() == 2 {
                    map.remove("anyOf");
                    if let Value::Object(inner) = &non_null[0] {
                        for (k, v) in inner {
                            map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
            }
            for value in map.values_mut() {
                compact_schema(value);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(compact_schema),
        _ => {}
    }
}

pub async fn serve(workspace: Workspace) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(workspace.root().join("logs/mcp.log"))
        .context("cannot open MCP log")?;
    // Filtro fijo: no activar dumps de protocolo/payloads desde RUST_LOG.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(std::io::stderr.and(Mutex::new(file)))
        .try_init()
        .map_err(|_| anyhow::anyhow!("cannot initialize MCP logging"))?;
    let app = bridge::find_app(&workspace);
    let server = RunPostMcp::new(workspace);
    tracing::info!(
        "MCP stdio started; tools={}",
        server.tool_router.list_all().len()
    );
    match app {
        // La app está abierta sobre este workspace: sus llamadas se ven en el panel Agente.
        Some(app) => {
            tracing::info!("MCP stdio bridged to the RunPost app at {}", app.url);
            let service = bridge::Bridge::new(app, server)
                .serve(stdio())
                .await
                .context("MCP initialization failed")?;
            service.waiting().await.context("MCP service failed")?;
        }
        None => {
            let service = server
                .serve(stdio())
                .await
                .context("MCP initialization failed")?;
            service.waiting().await.context("MCP service failed")?;
        }
    }
    tracing::info!("MCP stdio stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertises_thirteen_short_tools() {
        let dir = tempfile::tempdir().unwrap();
        let server = RunPostMcp::new(Workspace::open(dir.path()).unwrap());
        assert!(server.get_info().capabilities.tools.is_some());
        let tools = server.tool_router.list_all();
        // SPEC-DEVIATION: S5-D1 — 8.1 pide ≤ 12 tools pero el catálogo de 8.2 (y el sprint 5) suma
        // `run_collection` como la 13.ª; el límite que importa, < 2 500 tokens, se sigue cumpliendo.
        assert_eq!(tools.len(), 13);
        for tool in &tools {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(
                description.len() <= 200,
                "{}: {} chars",
                tool.name,
                description.len()
            );
            let schema = Value::Object(tool.input_schema.as_ref().clone()).to_string();
            assert!(
                !schema.contains("$schema") && !schema.contains("\"null\""),
                "{}: {schema}",
                tool.name
            );
        }
        let list = serde_json::to_string(&tools).unwrap();
        assert!(
            list.len() / 4 < 2500,
            "tools/list ≈ {} tokens",
            list.len() / 4
        );
    }

    #[test]
    fn compacts_optional_types() {
        let mut schema = serde_json::json!({"$schema": "x", "title": "T", "type": "object", "properties": {
            "a": {"type": ["string", "null"]}, "n": {"type": ["integer", "null"], "format": "uint", "minimum": 0},
            "s": {"anyOf": [{"$ref": "#/$defs/S"}, {"type": "null"}], "description": "d"}}});
        compact_schema(&mut schema);
        assert_eq!(
            schema,
            serde_json::json!({"type": "object", "properties": {
            "a": {"type": "string"}, "n": {"type": "integer"}, "s": {"description": "d", "$ref": "#/$defs/S"}}})
        );
    }
}

#[cfg(test)]
mod dump {
    #[test]
    #[ignore]
    fn dump_tools_list() {
        let dir = tempfile::tempdir().unwrap();
        let server = super::RunPostMcp::new(runpost_core::Workspace::open(dir.path()).unwrap());
        let list = serde_json::to_string(&server.tool_router.list_all()).unwrap();
        println!("CHARS={} TOKENS~{}\n{list}", list.len(), list.len() / 4);
    }
}
