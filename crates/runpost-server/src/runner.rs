//! Runner por la API (sección 7): `POST /runner` arranca en segundo plano y devuelve `run_id`;
//! el progreso va por el WebSocket (`runner_progress`, `runner_done`), `GET /runner/{id}` da el
//! estado y, al terminar, el informe; `DELETE` cancela; el informe se descarga en JSON o HTML.

use std::{collections::VecDeque, sync::Mutex};

use axum::{
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
};
use runpost_core::{
    CoreError, Source,
    model::new_id,
    runner::{
        self, CancellationToken, RunnerEvent, RunnerItem, RunnerOptions, RunnerReport, report,
    },
    storage::fs::slugify,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    AppState,
    api::{ApiError, ApiResult, Body, ok},
};

/// Ejecuciones que se recuerdan (para consultarlas y exportar su informe).
const KEEP: usize = 10;

#[derive(Default)]
pub(crate) struct Runs(Mutex<VecDeque<Run>>);

struct Run {
    id: String,
    cancel: CancellationToken,
    /// `running` | `done` | `failed`.
    status: &'static str,
    total: usize,
    items: Vec<RunnerItem>,
    report: Option<RunnerReport>,
    error: Option<String>,
}

impl Runs {
    fn with<R>(&self, id: &str, f: impl FnOnce(&mut Run) -> R) -> Option<R> {
        let mut runs = self.0.lock().unwrap_or_else(|p| p.into_inner());
        runs.iter_mut().find(|run| run.id == id).map(f)
    }

    fn insert(&self, run: Run) {
        let mut runs = self.0.lock().unwrap_or_else(|p| p.into_inner());
        runs.push_back(run);
        while runs.len() > KEEP {
            runs.pop_front();
        }
    }

    /// Informe de una ejecución terminada.
    pub(crate) fn report(&self, id: &str) -> Result<RunnerReport, ApiError> {
        self.with(id, |run| run.report.clone())
            .flatten()
            .ok_or_else(|| CoreError::not_found("run", id).into())
    }
}

#[derive(Deserialize)]
pub(crate) struct StartRun {
    collection_id: String,
    folder_id: Option<String>,
    env_id: Option<String>,
    iterations: Option<u32>,
    #[serde(default)]
    delay_ms: u64,
    data: Option<DataSource>,
}

/// Archivo de datos: el texto (navegador) o una ruta absoluta (diálogo nativo).
#[derive(Deserialize)]
struct DataSource {
    name: Option<String>,
    text: Option<String>,
    path: Option<String>,
}

pub(crate) async fn start(State(state): State<AppState>, Body(body): Body<StartRun>) -> ApiResult {
    // Los errores de selección se devuelven aquí, no a mitad de la ejecución.
    let collection = state.workspace.find_collection(&body.collection_id)?;
    if let Some(folder) = body.folder_id.as_deref().filter(|f| !f.is_empty()) {
        runner::find_folder(&collection, folder)?;
    }
    state.workspace.active_environment(body.env_id.as_deref())?;
    let (data, data_name) = match body.data {
        Some(source) => {
            let (text, name) = match (source.text, source.path) {
                (Some(text), _) => (text, source.name),
                (None, Some(path)) => {
                    let file = std::path::Path::new(&path);
                    if !file.is_absolute() {
                        return Err(ApiError::bad_request("data path must be absolute"));
                    }
                    let text = std::fs::read_to_string(file)
                        .map_err(|e| ApiError::bad_request(format!("cannot read {path}: {e}")))?;
                    let name = source
                        .name
                        .or_else(|| file.file_name().map(|n| n.to_string_lossy().into_owned()));
                    (text, name)
                }
                (None, None) => return Err(ApiError::bad_request("data needs text or path")),
            };
            let rows = runner::data::parse(name.as_deref().unwrap_or_default(), &text)
                .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
            (rows, name)
        }
        None => (Vec::new(), None),
    };

    let id = new_id("run");
    let cancel = CancellationToken::new();
    state.runs.insert(Run {
        id: id.clone(),
        cancel: cancel.clone(),
        status: "running",
        total: 0,
        items: Vec::new(),
        report: None,
        error: None,
    });
    let opts = RunnerOptions {
        folder: body.folder_id.filter(|f| !f.is_empty()),
        env: body.env_id,
        iterations: body.iterations,
        data,
        data_name,
        delay_ms: body.delay_ms,
        source: Source::Ui,
        cancel,
        ..RunnerOptions::default()
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<RunnerEvent>();
    let forward = {
        let state = state.clone();
        let id = id.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                state.runs.with(&id, |run| match &event {
                    RunnerEvent::Started { total, .. } => run.total = *total,
                    RunnerEvent::Item { item, .. } => run.items.push((**item).clone()),
                });
                let mut value = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
                value["type"] = json!("runner_progress");
                value["run_id"] = json!(id);
                let _ = state.events.send(value);
            }
        })
    };
    {
        let state = state.clone();
        let id = id.clone();
        let collection_id = collection.id.clone();
        tokio::spawn(async move {
            let outcome = state
                .executor
                .run_collection(&state.workspace, &collection_id, opts, Some(tx))
                .await;
            // El emisor se soltó al terminar: el reenvío acaba en cuanto vacía la cola.
            let _ = forward.await;
            let done = match outcome {
                Ok(report) => {
                    let done = json!({
                        "type": "runner_done", "run_id": id, "totals": report.totals,
                        "cancelled": report.cancelled, "ms": report.ms,
                    });
                    state.runs.with(&id, |run| {
                        run.status = "done";
                        run.report = Some(report);
                    });
                    done
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    state.runs.with(&id, |run| {
                        run.status = "failed";
                        run.error = Some(message.clone());
                    });
                    json!({"type": "runner_done", "run_id": id, "error": message})
                }
            };
            let _ = state.events.send(done);
        });
    }
    ok(json!({"run_id": id}))
}

pub(crate) async fn status(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    let value = state.runs.with(&id, |run| {
        let mut value = json!({
            "run_id": run.id, "status": run.status, "total": run.total,
            "done": run.items.len(), "report": run.report, "error": run.error,
        });
        if run.report.is_none() {
            value["items"] = json!(run.items);
        }
        value
    });
    match value {
        Some(value) => ok(value),
        None => Err(CoreError::not_found("run", &id).into()),
    }
}

pub(crate) async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    match state.runs.with(&id, |run| run.cancel.cancel()) {
        Some(()) => ok(json!({"ok": true})),
        None => Err(CoreError::not_found("run", &id).into()),
    }
}

#[derive(Deserialize)]
pub(crate) struct ReportQuery {
    format: Option<String>,
}

/// `(nombre de archivo, content-type, bytes)` del informe.
pub(crate) fn render(
    report: &RunnerReport,
    format: &str,
) -> Result<(String, &'static str, Vec<u8>), ApiError> {
    let base = format!("runner-{}", slugify(&report.collection.name));
    match format {
        "html" => Ok((
            format!("{base}.html"),
            "text/html; charset=utf-8",
            report::to_html(report).into_bytes(),
        )),
        "json" | "" => {
            let mut text = serde_json::to_string_pretty(report).map_err(anyhow::Error::from)?;
            text.push('\n');
            Ok((
                format!("{base}.json"),
                "application/json",
                text.into_bytes(),
            ))
        }
        other => Err(ApiError::bad_request(format!(
            "unknown report format {other:?} (use json or html)"
        ))),
    }
}

pub(crate) async fn download(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ReportQuery>,
) -> ApiResult {
    let report = state.runs.report(&id)?;
    let (file, content_type, bytes) = render(&report, query.format.as_deref().unwrap_or("json"))?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}
