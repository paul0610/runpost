//! Collection Runner (fase 3): colección o carpeta, iteraciones, datos CSV/JSON, retardo,
//! cancelación, eventos de progreso e informe (`RunnerReport`).

pub mod data;
pub mod report;

use std::{sync::Arc, time::Instant};

use anyhow::Result;
use serde::Serialize;
use tokio::sync::mpsc;
pub use tokio_util::sync::CancellationToken;

use crate::{
    error::{CoreError, NetworkError},
    executor::{Executor, RunContext, TestResult},
    model::{Collection, Folder, Item, Request},
    storage::{Source, Workspace},
    vars::Vars,
};

/// Comprobación previa a cada petición (p. ej. `agent_policy` por MCP). `Err` = no se envía.
pub type Guard = Arc<dyn Fn(&Collection, &Request) -> Result<(), String> + Send + Sync>;

#[derive(Clone, Default)]
pub struct RunnerOptions {
    /// Id, ruta (`"A/B"`) o nombre de una carpeta de la colección; `None` = toda la colección.
    pub folder: Option<String>,
    /// Id o nombre del entorno; `None` = el activo.
    pub env: Option<String>,
    /// Sin indicar: una por fila de datos, o 1 sin datos (como Postman).
    pub iterations: Option<u32>,
    /// Filas del archivo de datos (`pm.iterationData`).
    pub data: Vec<Vars>,
    /// Nombre del archivo de datos, para el informe.
    pub data_name: Option<String>,
    /// Pausa entre peticiones.
    pub delay_ms: u64,
    /// Para en el primer fallo (test fallido o error de red).
    pub fail_fast: bool,
    pub source: Source,
    pub record_history: bool,
    /// Variables locales iniciales (overrides); los scripts las amplían con `pm.variables.set`.
    pub overrides: Vars,
    pub cancel: CancellationToken,
    pub guard: Option<Guard>,
}

impl std::fmt::Debug for RunnerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerOptions")
            .field("folder", &self.folder)
            .field("env", &self.env)
            .field("iterations", &self.iterations)
            .field("data_rows", &self.data.len())
            .field("delay_ms", &self.delay_ms)
            .field("fail_fast", &self.fail_fast)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NamedRef {
    pub id: String,
    pub name: String,
}

/// Una petición ejecutada (o omitida) en una iteración.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunnerItem {
    /// Desde 0.
    pub iteration: u32,
    pub id: String,
    pub name: String,
    /// Ruta dentro de la colección: `"Carpeta/Petición"`.
    pub path: String,
    pub method: String,
    /// URL enviada, con secretos enmascarados.
    pub url: String,
    pub status: Option<u16>,
    pub ms: u64,
    pub size: u64,
    pub tests: Vec<TestResult>,
    /// Error de red, de construcción o de política: la petición no tuvo respuesta.
    pub error: Option<String>,
    /// `pm.execution.skipRequest()`.
    pub skipped: bool,
    pub console: Vec<String>,
}

impl RunnerItem {
    pub fn failed_tests(&self) -> impl Iterator<Item = &TestResult> {
        self.tests.iter().filter(|t| !t.passed && !t.skipped)
    }

    /// Falló un test o no hubo respuesta.
    pub fn failed(&self) -> bool {
        self.error.is_some() || self.failed_tests().next().is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RunnerTotals {
    pub requests: usize,
    pub skipped: usize,
    pub errors: usize,
    pub tests: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataInfo {
    pub name: Option<String>,
    pub rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunnerReport {
    pub collection: NamedRef,
    pub folder: Option<NamedRef>,
    pub environment: Option<String>,
    pub iterations: u32,
    pub data: Option<DataInfo>,
    /// RFC 3339 (UTC).
    pub started_at: String,
    pub ms: u64,
    pub cancelled: bool,
    pub totals: RunnerTotals,
    pub items: Vec<RunnerItem>,
}

impl RunnerReport {
    /// Algún test falló o alguna petición se quedó sin respuesta (código de salida 1 en el CLI).
    pub fn failed(&self) -> bool {
        self.totals.failed > 0 || self.totals.errors > 0
    }
}

fn totals(items: &[RunnerItem]) -> RunnerTotals {
    let mut totals = RunnerTotals {
        requests: items.len(),
        ..RunnerTotals::default()
    };
    for item in items {
        totals.skipped += usize::from(item.skipped);
        totals.errors += usize::from(item.error.is_some());
        for test in item.tests.iter().filter(|t| !t.skipped) {
            totals.tests += 1;
            if test.passed {
                totals.passed += 1;
            } else {
                totals.failed += 1;
            }
        }
    }
    totals
}

/// Progreso para la UI (WebSocket) y el CLI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RunnerEvent {
    Started {
        /// Peticiones que se van a intentar (todas las iteraciones).
        total: usize,
        iterations: u32,
    },
    Item {
        done: usize,
        total: usize,
        item: Box<RunnerItem>,
    },
}

/// Carpeta de la colección por id, ruta (`"A/B"`), nombre o prefijo único del nombre.
pub fn find_folder<'a>(collection: &'a Collection, selector: &str) -> Result<&'a Folder> {
    fn walk<'a>(items: &'a [Item], prefix: &str, out: &mut Vec<(String, &'a Folder)>) {
        for item in items {
            if let Item::Folder(folder) = item {
                let path = if prefix.is_empty() {
                    folder.name.clone()
                } else {
                    format!("{prefix}/{}", folder.name)
                };
                out.push((path.clone(), folder));
                walk(&folder.items, &path, out);
            }
        }
    }
    let mut folders = Vec::new();
    walk(&collection.items, "", &mut folders);
    let wanted = selector.trim();
    let lower = wanted.to_lowercase();
    type Tier<'t> = &'t dyn Fn(&(String, &Folder)) -> bool;
    let tiers: [Tier; 5] = [
        &|(_, f)| f.id == wanted,
        &|(p, f)| p == wanted || f.name == wanted,
        &|(p, f)| p.to_lowercase() == lower || f.name.to_lowercase() == lower,
        &|(_, f)| f.name.to_lowercase().starts_with(&lower),
        &|(p, _)| p.to_lowercase().starts_with(&lower),
    ];
    let mut ambiguous = 0;
    for tier in tiers {
        let hits: Vec<_> = folders.iter().filter(|f| tier(f)).collect();
        match hits.as_slice() {
            [(_, folder)] => return Ok(folder),
            [] => {}
            many => ambiguous = ambiguous.max(many.len()),
        }
    }
    if ambiguous > 0 {
        Err(CoreError::Ambiguous {
            kind: "folder",
            selector: wanted.into(),
            candidates: ambiguous,
        }
        .into())
    } else {
        Err(CoreError::not_found("folder", wanted).into())
    }
}

/// Peticiones a ejecutar, en el orden del árbol, con sus carpetas padre y su ruta.
fn plan<'a>(
    collection: &'a Collection,
    folder: Option<&Folder>,
) -> Vec<(Vec<&'a Folder>, &'a Request, String)> {
    let mut out = Vec::new();
    collection.walk_requests(|parents, request| {
        if let Some(folder) = folder
            && !parents.iter().any(|p| p.id == folder.id)
        {
            return;
        }
        let mut path: Vec<&str> = parents.iter().map(|p| p.name.as_str()).collect();
        path.push(&request.name);
        out.push((parents.to_vec(), request, path.join("/")));
    });
    out
}

impl Executor {
    /// Ejecuta una colección (o una carpeta) por id, nombre o slug. El progreso se manda por
    /// `progress` si el receptor sigue vivo; cancelar `opts.cancel` para pararlo.
    pub async fn run_collection(
        &self,
        ws: &Workspace,
        collection: &str,
        opts: RunnerOptions,
        progress: Option<mpsc::UnboundedSender<RunnerEvent>>,
    ) -> Result<RunnerReport> {
        let collection = ws.find_collection(collection)?;
        let folder = match opts.folder.as_deref().filter(|f| !f.trim().is_empty()) {
            Some(selector) => Some(find_folder(&collection, selector)?.clone()),
            None => None,
        };
        let environment = ws.active_environment(opts.env.as_deref())?;
        let iterations = opts
            .iterations
            .unwrap_or(if opts.data.is_empty() {
                1
            } else {
                opts.data.len() as u32
            })
            .max(1);
        let steps = plan(&collection, folder.as_ref());
        let total = steps.len() * iterations as usize;
        let started = Instant::now();
        let started_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let send = |event: RunnerEvent| {
            if let Some(progress) = &progress {
                let _ = progress.send(event);
            }
        };
        send(RunnerEvent::Started { total, iterations });

        let mut items: Vec<RunnerItem> = Vec::new();
        let mut locals = opts.overrides.clone();
        let mut cancelled = false;
        'run: for iteration in 0..iterations {
            // Con más iteraciones que filas, se repite la última (como Postman).
            let row = opts
                .data
                .get((iteration as usize).min(opts.data.len().saturating_sub(1)))
                .cloned()
                .unwrap_or_default();
            for (index, (parents, request, path)) in steps.iter().enumerate() {
                if opts.cancel.is_cancelled() {
                    cancelled = true;
                    break 'run;
                }
                if (index > 0 || iteration > 0) && opts.delay_ms > 0 {
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_millis(opts.delay_ms)) => {}
                        _ = opts.cancel.cancelled() => { cancelled = true; break 'run; }
                    }
                }
                let mut item = RunnerItem {
                    iteration,
                    id: request.id.clone(),
                    name: request.name.clone(),
                    path: path.clone(),
                    method: request.method.to_uppercase(),
                    url: request.url.clone(),
                    status: None,
                    ms: 0,
                    size: 0,
                    tests: Vec::new(),
                    error: None,
                    skipped: false,
                    console: Vec::new(),
                };
                // Las variables de colección pueden haber cambiado por un script anterior.
                let current = ws
                    .load_collection(&collection.id)
                    .unwrap_or_else(|_| collection.clone());
                let blocked = opts
                    .guard
                    .as_ref()
                    .and_then(|guard| guard(&current, request).err());
                if let Some(reason) = blocked {
                    item.error = Some(reason);
                } else {
                    let mut ctx = RunContext::new(opts.source);
                    ctx.env_id = environment.as_ref().map(|e| e.id.clone());
                    ctx.overrides = locals.clone();
                    ctx.iteration = row.clone();
                    ctx.iteration_index = iteration;
                    ctx.iteration_count = iterations;
                    ctx.record_history = opts.record_history;
                    let run = self.run_in(ws, Some(&current), parents, request, ctx);
                    let outcome = tokio::select! {
                        outcome = run => outcome,
                        _ = opts.cancel.cancelled() => { cancelled = true; break 'run; }
                    };
                    match outcome {
                        Ok(result) => {
                            locals = result.locals.clone();
                            item.method = result.resolved.method.clone();
                            item.url = result.resolved.url.clone();
                            item.skipped = result.skipped;
                            if !result.skipped {
                                item.status = Some(result.response.status);
                                item.ms = result.response.ms;
                                item.size = result.response.size;
                            }
                            item.tests = result.tests;
                            item.console = result.console;
                        }
                        Err(error) => {
                            if let Some(network) = error.downcast_ref::<NetworkError>() {
                                item.url = network.url.clone();
                                item.error = Some(format!("{}: {}", network.kind, network.message));
                            } else {
                                item.error = Some(format!("{error:#}"));
                            }
                        }
                    }
                }
                let stop = opts.fail_fast && item.failed();
                items.push(item.clone());
                send(RunnerEvent::Item {
                    done: items.len(),
                    total,
                    item: Box::new(item),
                });
                if stop {
                    break 'run;
                }
            }
        }
        Ok(RunnerReport {
            collection: NamedRef {
                id: collection.id.clone(),
                name: collection.name.clone(),
            },
            folder: folder.map(|f| NamedRef {
                id: f.id,
                name: f.name,
            }),
            environment: environment.map(|e| e.name),
            iterations,
            data: (!opts.data.is_empty()).then(|| DataInfo {
                name: opts.data_name.clone(),
                rows: opts.data.len(),
            }),
            started_at,
            ms: started.elapsed().as_millis() as u64,
            cancelled,
            totals: totals(&items),
            items,
        })
    }
}

#[cfg(test)]
mod tests;
