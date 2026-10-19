//! API REST de la sección 7 (para la UI). JSON en ambas direcciones; errores
//! `{"error":{"code":"…","message":"…"}}`.

use std::{collections::VecDeque, path::PathBuf};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use runpost_core::{
    CLI_BIN, CoreError, Executor, ImportReport, NetworkError, RequestDraft, RunContext, Source,
    codegen::Lang,
    executor::snippet,
    export::postman_v21::{export_collection, export_environment},
    import::Format,
    import::postman_v21::PostmanKind,
    model::{Collection, Environment, Globals, Item, Position, Request as SavedRequest, Settings},
    storage::{HistoryEntry, fs::write_atomic},
    vars::Vars,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::AppState;

/// Cuántas respuestas recientes (sin enmascarar) se guardan para "Guardar a archivo".
const RECENT_BODIES: usize = 20;
/// Límite de subida para importar colecciones grandes.
const IMPORT_LIMIT: usize = 50 * 1024 * 1024;

pub(crate) type ApiResult = Result<Response, ApiError>;

pub(crate) struct ApiError {
    status: StatusCode,
    body: Value,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: json!({"error": {"code": code, "message": message.into()}}),
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        if let Some(network) = error.downcast_ref::<NetworkError>() {
            return Self {
                status: StatusCode::BAD_GATEWAY,
                body: json!({"error": {"code": network.kind, "message": network.message,
                                       "url": network.url, "history_id": network.history_id}}),
            };
        }
        if let Some(core) = error.downcast_ref::<CoreError>() {
            let status = match core {
                CoreError::NotFound { .. } => StatusCode::NOT_FOUND,
                CoreError::Ambiguous { .. } => StatusCode::CONFLICT,
                CoreError::UnsupportedSchema { .. } => StatusCode::UNPROCESSABLE_ENTITY,
                CoreError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
                CoreError::Keyring(_) => StatusCode::SERVICE_UNAVAILABLE,
            };
            return Self::new(status, &core.code(), core.to_string());
        }
        tracing::warn!("api error: {error:#}");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            format!("{error:#}"),
        )
    }
}

impl From<CoreError> for ApiError {
    fn from(error: CoreError) -> Self {
        anyhow::Error::from(error).into()
    }
}

/// `Json<T>` con errores en el formato de la API (axum responde texto plano por defecto).
pub(crate) struct Body<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> FromRequest<S> for Body<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Body(value)),
            Err(rejection) => Err(ApiError::new(
                rejection.status(),
                "invalid_body",
                rejection.body_text(),
            )),
        }
    }
}

pub(crate) fn ok<T: serde::Serialize>(value: T) -> ApiResult {
    Ok(Json(value).into_response())
}

fn created<T: serde::Serialize>(value: T) -> ApiResult {
    Ok((StatusCode::CREATED, Json(value)).into_response())
}

fn no_content() -> ApiResult {
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `\\?\D:\x` (ruta canónica de Windows) → `D:\x`, para mostrarla y para configs.
pub fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
        _ => text,
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(|| async { Json(json!({"ok": true})) }))
        .route("/settings", get(get_settings).put(put_settings))
        .route(
            "/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/collections/{id}",
            get(get_collection)
                .put(put_collection)
                .delete(delete_collection),
        )
        .route("/collections/{id}/items", post(create_item))
        .route(
            "/collections/{id}/items/{item}",
            axum::routing::patch(patch_item).delete(delete_item),
        )
        .route("/collections/{id}/items/{item}/move", post(move_item))
        .route(
            "/collections/{id}/items/{item}/duplicate",
            post(duplicate_item),
        )
        .route(
            "/environments",
            get(list_environments).post(create_environment),
        )
        .route(
            "/environments/{id}",
            get(get_environment)
                .put(put_environment)
                .delete(delete_environment),
        )
        .route("/globals", get(get_globals).put(put_globals))
        .route("/send", post(send))
        .route("/codegen/{lang}", post(codegen))
        .route("/history", get(list_history).delete(clear_history))
        .route("/history/{id}", get(get_history))
        .route(
            "/import",
            post(import).layer(DefaultBodyLimit::max(IMPORT_LIMIT)),
        )
        .route("/export/collections/{id}", get(export))
        .route("/export/environments/{id}", get(export_env))
        .route("/runner", post(crate::runner::start))
        .route(
            "/runner/{id}",
            get(crate::runner::status).delete(crate::runner::cancel),
        )
        .route("/runner/{id}/report", get(crate::runner::download))
        .route("/files/save", post(save_file))
        .route("/agent", get(agent_state))
        .route("/agent/approvals/{id}", post(decide_approval))
        .route("/agent/config", get(agent_config))
}

// ---- settings ----

async fn get_settings(State(state): State<AppState>) -> ApiResult {
    ok(state.workspace.settings())
}

async fn put_settings(State(state): State<AppState>, Body(settings): Body<Settings>) -> ApiResult {
    state.workspace.save_settings(&settings)?;
    ok(settings)
}

// ---- colecciones ----

async fn list_collections(State(state): State<AppState>) -> ApiResult {
    ok(state.workspace.collections()?)
}

#[derive(Deserialize)]
struct CreateCollection {
    name: Option<String>,
    /// Colección completa (duplicar/pegar): recibe ids nuevos.
    collection: Option<Collection>,
}

async fn create_collection(
    State(state): State<AppState>,
    Body(body): Body<CreateCollection>,
) -> ApiResult {
    let collection = match (body.collection, body.name) {
        (Some(mut collection), name) => {
            collection.fresh_ids();
            if let Some(name) = name {
                collection.name = name;
            }
            collection
        }
        (None, Some(name)) if !name.trim().is_empty() => Collection::new(name.trim()),
        _ => return Err(ApiError::bad_request("name is required")),
    };
    state.workspace.save_collection(&collection)?;
    created(collection)
}

async fn get_collection(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    ok(state.workspace.load_collection(&id)?)
}

async fn put_collection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(mut collection): Body<Collection>,
) -> ApiResult {
    state.workspace.load_collection(&id)?;
    collection.id = id;
    state.workspace.save_collection(&collection)?;
    ok(collection)
}

async fn delete_collection(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    state.workspace.delete_collection(&id)?;
    no_content()
}

// ---- árbol ----

#[derive(Deserialize, Default)]
struct Placement {
    parent_id: Option<String>,
    before: Option<String>,
    index: Option<usize>,
}

impl Placement {
    fn position(&self) -> Position {
        Position {
            parent: self.parent_id.clone().filter(|p| !p.is_empty()),
            before: self.before.clone(),
            index: self.index,
        }
    }
}

#[derive(Deserialize)]
struct CreateItem {
    #[serde(flatten)]
    placement: Placement,
    item: Item,
}

async fn create_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<CreateItem>,
) -> ApiResult {
    let mut collection = state.workspace.load_collection(&id)?;
    let item_id = collection.insert_item(&body.placement.position(), body.item)?;
    state.workspace.save_collection(&collection)?;
    let item = collection.find_item(&item_id).cloned();
    created(json!({"id": item_id, "item": item}))
}

/// Mezcla superficial: cambia los campos dados salvo `id`, `type` e `items`.
async fn patch_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(String, String)>,
    Body(patch): Body<Map<String, Value>>,
) -> ApiResult {
    let mut collection = state.workspace.load_collection(&id)?;
    let item = collection
        .find_item_mut(&item_id)
        .ok_or_else(|| CoreError::not_found("item", &item_id))?;
    let mut value = serde_json::to_value(&*item).map_err(anyhow::Error::from)?;
    for (key, field) in patch {
        if !matches!(key.as_str(), "id" | "type" | "items") {
            value[key] = field;
        }
    }
    *item = serde_json::from_value(value)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_body", e.to_string()))?;
    let updated = item.clone();
    state.workspace.save_collection(&collection)?;
    ok(updated)
}

async fn delete_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(String, String)>,
) -> ApiResult {
    let mut collection = state.workspace.load_collection(&id)?;
    collection
        .remove_item(&item_id)
        .ok_or_else(|| CoreError::not_found("item", &item_id))?;
    state.workspace.save_collection(&collection)?;
    no_content()
}

#[derive(Deserialize, Default)]
struct MoveItem {
    #[serde(flatten)]
    placement: Placement,
    /// Colección destino si es distinta (arrastrar entre colecciones).
    collection_id: Option<String>,
}

async fn move_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(String, String)>,
    Body(body): Body<MoveItem>,
) -> ApiResult {
    let position = body.placement.position();
    let mut source = state.workspace.load_collection(&id)?;
    match body.collection_id.filter(|target| *target != id) {
        None => {
            source.move_item(&item_id, &position)?;
            state.workspace.save_collection(&source)?;
        }
        Some(target_id) => {
            let mut target = state.workspace.load_collection(&target_id)?;
            let item = source
                .remove_item(&item_id)
                .ok_or_else(|| CoreError::not_found("item", &item_id))?;
            target.insert_item(&position, item)?;
            // Primero el destino: si algo falla, el elemento no se pierde.
            state.workspace.save_collection(&target)?;
            state.workspace.save_collection(&source)?;
        }
    }
    no_content()
}

#[derive(Deserialize, Default)]
struct Duplicate {
    name: Option<String>,
}

async fn duplicate_item(
    State(state): State<AppState>,
    Path((id, item_id)): Path<(String, String)>,
    Body(body): Body<Duplicate>,
) -> ApiResult {
    let mut collection = state.workspace.load_collection(&id)?;
    let new_id = collection.duplicate_item(&item_id, body.name.as_deref())?;
    state.workspace.save_collection(&collection)?;
    created(json!({"id": new_id}))
}

// ---- entornos y globales ----

async fn list_environments(State(state): State<AppState>) -> ApiResult {
    ok(state.workspace.environments()?)
}

#[derive(Deserialize)]
struct CreateEnvironment {
    name: Option<String>,
    environment: Option<Environment>,
}

async fn create_environment(
    State(state): State<AppState>,
    Body(body): Body<CreateEnvironment>,
) -> ApiResult {
    let env = match (body.environment, body.name) {
        (Some(mut env), name) => {
            env.id = runpost_core::model::new_id("env");
            if let Some(name) = name {
                env.name = name;
            }
            env
        }
        (None, Some(name)) if !name.trim().is_empty() => Environment::new(name.trim()),
        _ => return Err(ApiError::bad_request("name is required")),
    };
    state.workspace.save_environment(&env)?;
    created(env)
}

async fn get_environment(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    ok(state.workspace.find_environment(&id)?)
}

async fn put_environment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(mut env): Body<Environment>,
) -> ApiResult {
    let existing = state.workspace.find_environment(&id)?;
    env.id = existing.id;
    state.workspace.save_environment(&env)?;
    ok(env)
}

async fn delete_environment(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let env = state.workspace.find_environment(&id)?;
    state.workspace.delete_environment(&env.id)?;
    no_content()
}

async fn get_globals(State(state): State<AppState>) -> ApiResult {
    ok(state.workspace.globals()?)
}

async fn put_globals(State(state): State<AppState>, Body(globals): Body<Globals>) -> ApiResult {
    state.workspace.save_globals(&globals)?;
    ok(globals)
}

// ---- enviar ----

#[derive(Deserialize)]
#[serde(untagged)]
enum SendTarget {
    ById { request_id: String },
    Saved(Box<SavedRequest>),
    Draft(Box<RequestDraft>),
}

#[derive(Deserialize)]
struct SendBody {
    request: SendTarget,
    env_id: Option<String>,
    #[serde(default)]
    overrides: Vars,
}

/// Cuerpo como texto si es UTF-8; si no, base64 con `body_encoding`.
fn body_json(bytes: &[u8]) -> (Value, &'static str) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (json!(text), "utf8"),
        Err(_) => (json!(STANDARD.encode(bytes)), "base64"),
    }
}

async fn send(State(state): State<AppState>, Body(body): Body<SendBody>) -> ApiResult {
    let mut ctx = RunContext::new(Source::Ui);
    ctx.env_id = body.env_id.filter(|e| !e.is_empty());
    ctx.overrides = body.overrides;
    let executor: &Executor = &state.executor;
    let result = match body.request {
        SendTarget::ById { request_id } => {
            let (_, request) = state
                .workspace
                .find_request(&runpost_core::RequestSelector::Id(request_id))?;
            executor.run(&state.workspace, &request, ctx).await?
        }
        SendTarget::Saved(request) => executor.run(&state.workspace, &request, ctx).await?,
        SendTarget::Draft(draft) => executor.send_adhoc(&state.workspace, *draft, ctx).await?,
    };
    if !result.history_id.is_empty() {
        let mut recent = state.recent.lock().unwrap_or_else(|p| p.into_inner());
        recent.push_front((result.history_id.clone(), result.response.body.to_vec()));
        recent.truncate(RECENT_BODIES);
    }
    let mut value = serde_json::to_value(&result).map_err(anyhow::Error::from)?;
    let (body, encoding) = body_json(&result.response.body);
    value["response"]["body"] = body;
    value["response"]["body_encoding"] = json!(encoding);
    ok(value)
}

// ---- snippets (fase 4) ----

#[derive(Deserialize)]
struct CodegenBody {
    request: SendTarget,
    env_id: Option<String>,
}

/// Snippet de curl, fetch, PowerShell o Python; los secretos quedan como `{{nombre}}`.
async fn codegen(
    State(state): State<AppState>,
    Path(lang): Path<String>,
    Body(body): Body<CodegenBody>,
) -> ApiResult {
    let lang = Lang::parse(&lang)
        .ok_or_else(|| ApiError::bad_request(format!("unknown snippet language {lang:?}")))?;
    let env = body.env_id.filter(|e| !e.is_empty());
    let ws = &state.workspace;
    let code = match body.request {
        SendTarget::ById { request_id } => {
            let (_, request) = ws.find_request(&runpost_core::RequestSelector::Id(request_id))?;
            snippet(ws, &request, None, env.as_deref(), lang)?
        }
        SendTarget::Saved(request) => snippet(ws, &request, None, env.as_deref(), lang)?,
        SendTarget::Draft(draft) => {
            let collection = draft.collection_id.clone();
            let request = draft.into_request();
            snippet(ws, &request, collection.as_deref(), env.as_deref(), lang)?
        }
    };
    ok(json!({"lang": lang.as_str(), "code": code}))
}

// ---- historial ----

#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    q: Option<String>,
}

async fn list_history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> ApiResult {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    ok(state
        .workspace
        .history()
        .list(limit, query.offset.unwrap_or(0), query.q.as_deref())?)
}

fn history_json(entry: HistoryEntry) -> Value {
    let body = |bytes: &Option<Vec<u8>>| match bytes {
        Some(bytes) => {
            let (text, encoding) = body_json(bytes);
            json!({"text": text, "encoding": encoding})
        }
        None => Value::Null,
    };
    json!({
        "id": entry.id, "ts": entry.ts, "request_id": entry.request_id, "source": entry.source,
        "method": entry.method, "url": entry.url, "status": entry.status,
        "duration_ms": entry.duration_ms, "size_bytes": entry.size_bytes,
        "req_headers": entry.req_headers, "req_body": body(&entry.req_body),
        "res_headers": entry.res_headers, "res_body": body(&entry.res_body),
        "tests_json": entry.tests_json, "error": entry.error,
    })
}

async fn get_history(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let entry = state
        .workspace
        .history()
        .get(&id)?
        .ok_or_else(|| CoreError::not_found("history", &id))?;
    ok(history_json(entry))
}

async fn clear_history(State(state): State<AppState>) -> ApiResult {
    let deleted = state.workspace.history().clear()?;
    state
        .recent
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clear();
    ok(json!({"deleted": deleted}))
}

// ---- import / export ----

#[derive(Deserialize, Default)]
struct ImportJson {
    format: Option<String>,
    kind: Option<String>,
    path: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    text: Option<String>,
}

fn parse_kind(kind: Option<&str>) -> Result<Option<PostmanKind>, ApiError> {
    kind.filter(|k| !k.is_empty())
        .map(|k| {
            k.parse::<PostmanKind>()
                .map_err(|e| ApiError::bad_request(e.to_string()))
        })
        .transpose()
}

fn check_format(format: Option<&str>) -> Result<Option<Format>, ApiError> {
    match format.filter(|f| !f.is_empty()) {
        None => Ok(None),
        Some(name) => Format::parse(name).map(Some).ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "unsupported_format",
                format!("{name} (postman_v21, openapi, har, insomnia, bruno or curl)"),
            )
        }),
    }
}

/// Lo que devuelve `/import`: informes, errores por archivo y las peticiones de curl (sin guardar;
/// la app las abre en pestañas, como Postman).
#[derive(Default)]
struct ImportOutcome {
    reports: Vec<ImportReport>,
    errors: Vec<Value>,
    requests: Vec<SavedRequest>,
}

impl ImportOutcome {
    /// Un texto (archivo subido o pegado) en cualquier formato.
    fn text(
        &mut self,
        ws: &runpost_core::Workspace,
        source: &str,
        text: &str,
        format: Option<Format>,
        kind: Option<PostmanKind>,
        slug: Option<&str>,
    ) {
        let is_curl = format == Some(Format::Curl)
            || (format.is_none() && runpost_core::import::curl::looks_like(text.trim_start()));
        let result = if is_curl {
            runpost_core::import::curl::parse(text).map(|request| self.requests.push(request))
        } else {
            ws.import(text, format, kind, slug)
                .map(|reports| self.reports.extend(reports))
        };
        if let Err(error) = result {
            self.errors
                .push(json!({"file": source, "message": format!("{error:#}")}));
        }
    }

    /// Una ruta del disco: archivo o colección de Bruno (carpeta o su `bruno.json`).
    fn path(
        &mut self,
        ws: &runpost_core::Workspace,
        path: &str,
        format: Option<Format>,
        kind: Option<PostmanKind>,
    ) {
        let file = PathBuf::from(path);
        let text = if file.is_file() {
            std::fs::read_to_string(&file).ok()
        } else {
            None
        };
        if let Some(text) = text.filter(|t| runpost_core::import::curl::looks_like(t.trim_start()))
        {
            return self.text(ws, path, &text, Some(Format::Curl), kind, None);
        }
        match ws.import_path(&file, format, kind) {
            Ok(reports) => self.reports.extend(reports),
            Err(error) => self
                .errors
                .push(json!({"file": path, "message": format!("{error:#}")})),
        }
    }
}

/// `multipart/form-data` (campos `file`, `kind`, `format`) o JSON `{path|paths|text, kind, format}`.
/// Acepta varios archivos a la vez (colección y entorno, R1 de docs/UX.md) y, desde la fase 4,
/// OpenAPI 3, HAR, Insomnia, Bruno y comandos curl.
async fn import(State(state): State<AppState>, request: Request) -> ApiResult {
    let is_multipart = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("multipart/form-data"));
    let mut outcome = ImportOutcome::default();
    let ws = &state.workspace;
    if is_multipart {
        let mut multipart = Multipart::from_request(request, &state)
            .await
            .map_err(|e| ApiError::bad_request(e.body_text()))?;
        let mut files: Vec<(String, String)> = Vec::new();
        let (mut kind, mut format) = (None, None);
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| ApiError::bad_request(e.body_text()))?
        {
            let name = field.name().unwrap_or_default().to_owned();
            let file_name = field.file_name().unwrap_or("import.json").to_owned();
            let text = field
                .text()
                .await
                .map_err(|e| ApiError::bad_request(e.body_text()))?;
            match name.as_str() {
                "kind" => kind = Some(text),
                "format" => format = Some(text),
                _ => files.push((file_name, text)),
            }
        }
        let format = check_format(format.as_deref())?;
        let kind = parse_kind(kind.as_deref())?;
        for (file_name, text) in files {
            let stem = runpost_core::storage::import_stem(std::path::Path::new(&file_name));
            outcome.text(ws, &file_name, &text, format, kind, Some(&stem));
        }
    } else {
        let Body(body) = Body::<ImportJson>::from_request(request, &state).await?;
        let format = check_format(body.format.as_deref())?;
        let kind = parse_kind(body.kind.as_deref())?;
        let mut paths = body.paths;
        paths.extend(body.path);
        for path in paths {
            outcome.path(ws, &path, format, kind);
        }
        if let Some(text) = body.text {
            outcome.text(ws, "text", &text, format, kind, None);
        }
    }
    if outcome.reports.is_empty() && outcome.errors.is_empty() && outcome.requests.is_empty() {
        return Err(ApiError::bad_request(
            "nothing to import: send a file, path, paths or text",
        ));
    }
    ok(json!({"reports": outcome.reports, "errors": outcome.errors, "requests": outcome.requests}))
}

#[derive(Deserialize)]
struct ExportQuery {
    format: Option<String>,
    strip_secrets: Option<String>,
    /// Exportar solo esta carpeta como colección.
    folder: Option<String>,
}

fn truthy(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "true" | "yes"))
}

/// La colección entera o una carpeta suya como colección propia. La carpeta recibe un id derivado
/// del suyo para que reimportarla no sustituya a la colección original.
fn exportable(
    state: &AppState,
    collection_id: &str,
    folder: Option<&str>,
) -> Result<Collection, ApiError> {
    let collection = state.workspace.load_collection(collection_id)?;
    let Some(folder_id) = folder.filter(|f| !f.is_empty()) else {
        return Ok(collection);
    };
    match collection.find_item(folder_id) {
        Some(Item::Folder(folder)) => Ok(Collection {
            id: format!("col_{}", folder.id.trim_start_matches("fld_")),
            name: folder.name.clone(),
            description: folder.description.clone(),
            auth: folder.auth.clone().or(collection.auth.clone()),
            scripts: folder.scripts.clone(),
            items: folder.items.clone(),
            ..collection.clone()
        }),
        Some(Item::Request(_)) => Err(ApiError::bad_request(format!(
            "{folder_id} is not a folder"
        ))),
        None => Err(CoreError::not_found("item", folder_id).into()),
    }
}

fn attachment(name: &str, kind: &str, body: Value) -> ApiResult {
    let file = format!(
        "{}.postman_{kind}.json",
        runpost_core::storage::fs::slugify(name)
    );
    Ok((
        [(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{file}\""),
        )],
        Json(body),
    )
        .into_response())
}

async fn export(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ExportQuery>,
) -> ApiResult {
    check_format(query.format.as_deref())?;
    let collection = exportable(&state, &id, query.folder.as_deref())?;
    let exported = export_collection(&collection, truthy(query.strip_secrets.as_deref()));
    attachment(&collection.name, "collection", exported)
}

async fn export_env(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ExportQuery>,
) -> ApiResult {
    check_format(query.format.as_deref())?;
    let env = state.workspace.find_environment(&id)?;
    let exported = export_environment(&env, truthy(query.strip_secrets.as_deref()));
    attachment(&env.name, "environment", exported)
}

#[derive(Deserialize)]
struct ExportSource {
    collection_id: String,
    folder_id: Option<String>,
    #[serde(default)]
    strip_secrets: bool,
}

#[derive(Deserialize)]
struct ExportEnvSource {
    environment_id: String,
    #[serde(default)]
    strip_secrets: bool,
}

/// Escribe en `path` (elegido con el diálogo nativo) solo contenido que el servidor ya tiene:
/// el cuerpo de una respuesta reciente o el export de una colección, carpeta o entorno.
/// No acepta bytes arbitrarios.
#[derive(Deserialize)]
struct SaveFile {
    path: String,
    history_id: Option<String>,
    export: Option<ExportSource>,
    export_environment: Option<ExportEnvSource>,
    runner_report: Option<RunnerReportSource>,
}

#[derive(Deserialize)]
struct RunnerReportSource {
    run_id: String,
    format: Option<String>,
}

fn pretty(value: &Value) -> Result<Vec<u8>, ApiError> {
    let mut text = serde_json::to_string_pretty(value).map_err(anyhow::Error::from)?;
    text.push('\n');
    Ok(text.into_bytes())
}

async fn save_file(State(state): State<AppState>, Body(body): Body<SaveFile>) -> ApiResult {
    let bytes = match (
        &body.history_id,
        &body.export,
        &body.export_environment,
        &body.runner_report,
    ) {
        (Some(id), None, None, None) => {
            let recent = state
                .recent
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .find(|(hid, _)| hid == id)
                .map(|(_, bytes)| bytes.clone());
            match recent {
                Some(bytes) => bytes,
                None => state
                    .workspace
                    .history()
                    .get(id)?
                    .ok_or_else(|| CoreError::not_found("history", id))?
                    .res_body
                    .unwrap_or_default(),
            }
        }
        (None, Some(export), None, None) => {
            let collection =
                exportable(&state, &export.collection_id, export.folder_id.as_deref())?;
            pretty(&export_collection(&collection, export.strip_secrets))?
        }
        (None, None, Some(export), None) => {
            let env = state.workspace.find_environment(&export.environment_id)?;
            pretty(&export_environment(&env, export.strip_secrets))?
        }
        (None, None, None, Some(source)) => {
            let report = state.runs.report(&source.run_id)?;
            crate::runner::render(&report, source.format.as_deref().unwrap_or("json"))?.2
        }
        _ => {
            return Err(ApiError::bad_request(
                "pass one of history_id, export, export_environment or runner_report",
            ));
        }
    };
    let path = PathBuf::from(&body.path);
    if !path.is_absolute() {
        return Err(ApiError::bad_request("path must be absolute"));
    }
    write_atomic(&path, &bytes)?;
    ok(json!({"ok": true, "path": display_path(&path), "bytes": bytes.len()}))
}

// ---- agente ----

/// Panel Agente: llamadas, medidor, clientes, aprobaciones pendientes y estado del MCP por HTTP.
async fn agent_state(State(state): State<AppState>) -> ApiResult {
    let mut snapshot = state.agent.snapshot();
    snapshot["mcp"] = json!(*state.mcp);
    ok(snapshot)
}

#[derive(Deserialize)]
struct Decision {
    approve: bool,
}

/// Aprobar / Denegar desde el aviso de la app.
async fn decide_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Body(decision): Body<Decision>,
) -> ApiResult {
    if state.agent.decide(&id, decision.approve) {
        ok(json!({"ok": true}))
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "approval_not_found",
            "the request expired or was already decided",
        ))
    }
}

#[derive(Deserialize)]
struct AgentQuery {
    client: String,
}

/// `headersHelper` / `http_headers_helper`: el cliente pide el token de la app en cada conexión.
fn helper_command(exe: &str, port: u16) -> String {
    let exe = if exe.contains(' ') {
        format!("\"{exe}\"")
    } else {
        exe.to_owned()
    };
    format!("{exe} mcp-headers --port {port}")
}

/// Texto de configuración MCP para pegar en el cliente (R1 de docs/UX.md): Claude Code y Codex
/// por HTTP cuando la app sirve `/mcp` (sus llamadas se ven en el panel Agente); Claude Desktop,
/// y cualquiera si el MCP por HTTP está apagado, por stdio. Devuelve (texto, destino, transporte).
pub fn agent_config_text(
    client: &str,
    exe: &str,
    workspace: &str,
    mcp_port: Option<u16>,
) -> Option<(String, &'static str, &'static str)> {
    let env_key = runpost_core::WORKSPACE_ENV;
    let slug = runpost_core::APP_SLUG;
    let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let url = mcp_port.map(|port| (format!("http://127.0.0.1:{port}/mcp"), port));
    Some(match (client, url) {
        ("claude-code", Some((url, port))) => {
            let config = json!({"mcpServers": {slug: {
                "type": "http", "url": url, "headersHelper": helper_command(exe, port)}}});
            (
                serde_json::to_string_pretty(&config).unwrap_or_default(),
                ".mcp.json (raíz del proyecto)",
                "http",
            )
        }
        ("codex", Some((url, port))) => (
            // Codex corta una tool a los 60 s: la espera de una aprobación necesita algo más.
            format!(
                "[mcp_servers.{slug}]\nurl = {}\nhttp_headers_helper = {}\ntool_timeout_sec = 90\n",
                quote(&url),
                quote(&helper_command(exe, port))
            ),
            "~/.codex/config.toml",
            "http",
        ),
        ("claude-code" | "claude-desktop", _) => {
            let config = json!({"mcpServers": {slug: {
                "command": exe, "args": ["mcp"], "env": {env_key: workspace}}}});
            let target = if client == "claude-code" {
                ".mcp.json (raíz del proyecto) · o: claude mcp add"
            } else {
                "%APPDATA%\\Claude\\claude_desktop_config.json"
            };
            (
                serde_json::to_string_pretty(&config).unwrap_or_default(),
                target,
                "stdio",
            )
        }
        ("codex", None) => (
            format!(
                "[mcp_servers.{slug}]\ncommand = {}\nargs = [\"mcp\"]\nenv = {{ {env_key} = {} }}\n",
                quote(exe),
                quote(workspace)
            ),
            "~/.codex/config.toml",
            "stdio",
        ),
        _ => return None,
    })
}

async fn agent_config(State(state): State<AppState>, Query(query): Query<AgentQuery>) -> ApiResult {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.parent()
                .map(|dir| dir.join(format!("{CLI_BIN}{}", std::env::consts::EXE_SUFFIX)))
        })
        .unwrap_or_else(|| PathBuf::from(CLI_BIN));
    let exe_text = display_path(&exe);
    let workspace = display_path(state.workspace.root());
    let (text, target, transport) =
        agent_config_text(&query.client, &exe_text, &workspace, state.mcp.port)
            .ok_or_else(|| ApiError::bad_request("client: claude-code | codex | claude-desktop"))?;
    ok(
        json!({"client": query.client, "text": text, "target": target, "transport": transport,
              "exe": exe_text, "exists": exe.is_file()}),
    )
}

pub(crate) type RecentBodies = std::sync::Mutex<VecDeque<(String, Vec<u8>)>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_strips_verbatim_prefix() {
        assert_eq!(display_path(std::path::Path::new(r"\\?\D:\ws")), r"D:\ws");
        assert_eq!(
            display_path(std::path::Path::new(r"\\?\UNC\srv\share")),
            r"\\?\UNC\srv\share"
        );
        assert_eq!(display_path(std::path::Path::new("/home/x")), "/home/x");
    }

    #[test]
    fn agent_configs_for_each_client() {
        let exe = r"D:\r\runpost_mcp.exe";
        let (json_text, target, transport) =
            agent_config_text("claude-code", exe, r"D:\r\workspace", None).unwrap();
        let value: Value = serde_json::from_str(&json_text).unwrap();
        assert_eq!(value["mcpServers"]["runpost"]["command"], exe);
        assert_eq!(
            value["mcpServers"]["runpost"]["env"]["RUNPOST_WORKSPACE"],
            r"D:\r\workspace"
        );
        assert!(target.contains(".mcp.json"));
        assert_eq!(transport, "stdio");
        let (toml_text, _, _) = agent_config_text("codex", exe, r"D:\r\ws", None).unwrap();
        assert!(
            toml_text.contains(r#"command = "D:\\r\\runpost_mcp.exe""#),
            "{toml_text}"
        );
        assert!(toml_text.contains(r#"env = { RUNPOST_WORKSPACE = "D:\\r\\ws" }"#));
        assert!(agent_config_text("other", "x", "y", None).is_none());

        // Con la app sirviendo /mcp: HTTP con el helper que pide el token en cada conexión.
        let port = Some(7879);
        let (json_text, _, transport) =
            agent_config_text("claude-code", exe, r"D:\r\workspace", port).unwrap();
        assert_eq!(transport, "http");
        let value: Value = serde_json::from_str(&json_text).unwrap();
        assert_eq!(
            value["mcpServers"]["runpost"],
            json!({"type": "http", "url": "http://127.0.0.1:7879/mcp",
                   "headersHelper": r"D:\r\runpost_mcp.exe mcp-headers --port 7879"})
        );
        let (toml_text, _, _) = agent_config_text("codex", exe, r"D:\r\ws", port).unwrap();
        assert!(toml_text.contains(r#"url = "http://127.0.0.1:7879/mcp""#));
        assert!(
            toml_text.contains(
                r#"http_headers_helper = "D:\\r\\runpost_mcp.exe mcp-headers --port 7879""#
            )
        );
        assert!(toml_text.contains("tool_timeout_sec = 90"));
        let (_, _, desktop) = agent_config_text("claude-desktop", exe, "w", port).unwrap();
        assert_eq!(desktop, "stdio");
        assert_eq!(
            helper_command(r"C:\Program Files\x.exe", 7979),
            r#""C:\Program Files\x.exe" mcp-headers --port 7979"#
        );
    }
}
