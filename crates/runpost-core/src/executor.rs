//! Ejecución de peticiones guardadas o ad hoc, con scripts e historial (secciones 6 y 10).

use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    codegen,
    engine::{self, ClientKey, Clients, ResolvedRequest, Response},
    error::NetworkError,
    model::{
        Auth, Body, Collection, FieldKind, Folder, HttpSettings, KeyValue, Request,
        RequestSettings, Scripts, new_id,
    },
    script::{
        self, ChainOutput, HeaderItem, Phase, ScriptHost, ScriptInfo, ScriptRequest,
        ScriptResponse, ScriptSource, ScriptState, VarScope,
    },
    storage::{HistoryEntry, Source, Workspace},
    vars::{Masker, Resolver, Scope, Vars},
};

/// Longitud máxima de un mensaje de error de red (regla 7 de 8.3).
const MAX_ERROR_CHARS: usize = 300;

#[derive(Debug, Clone, Default)]
pub struct RunContext {
    /// Id o nombre del entorno; `None` = `settings.active_environment`.
    pub env_id: Option<String>,
    /// Overrides (MCP `vars`) y, en el runner, las variables locales de `pm.variables.set`.
    pub overrides: Vars,
    /// Datos de la iteración del runner; segundo en el orden de 5.7.
    pub iteration: Vars,
    /// `pm.info.iteration` (desde 0) y `pm.info.iterationCount`.
    pub iteration_index: u32,
    pub iteration_count: u32,
    pub source: Source,
    pub record_history: bool,
}

impl RunContext {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            record_history: true,
            iteration_count: 1,
            ..Self::default()
        }
    }
}

/// Resultado de `pm.test`. Un error del propio script llega como `{"name":"<script error>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    #[serde(default)]
    pub msg: Option<String>,
    /// `pm.test.skip` o `pm.test` sin función: no cuenta como pasado ni como fallido.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

/// Variable modificada por un script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VarChange {
    pub scope: Scope,
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub response: Response,
    /// Lo enviado, ya enmascarado.
    pub resolved: ResolvedRequest,
    pub tests: Vec<TestResult>,
    pub vars_changed: Vec<VarChange>,
    /// Id en el historial (`response_id` en MCP). Vacío si `record_history` era `false`.
    pub history_id: String,
    pub console: Vec<String>,
    /// Un script `pre` llamó a `pm.execution.skipRequest()`: no se envió nada.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
    /// Secretos usados por esta ejecución, para enmascarar la salida (TokenBudget, sprint 2).
    #[serde(skip)]
    pub masker: Masker,
    /// Variables locales al terminar (`pm.variables.set`); el runner las pasa a la siguiente.
    #[serde(skip)]
    pub locals: Vars,
}

/// Petición no guardada (UI sin guardar, `send` de MCP). Con `collection_id`, hereda
/// variables, auth y scripts de esa colección.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDraft {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub params: Vec<KeyValue>,
    #[serde(default)]
    pub headers: Vec<KeyValue>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub settings: RequestSettings,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub collection_id: Option<String>,
}

impl RequestDraft {
    /// Petición sin id ni nombre (no está guardada).
    pub fn into_request(self) -> Request {
        Request {
            id: String::new(),
            name: String::new(),
            method: self.method,
            url: self.url,
            params: self.params,
            headers: self.headers,
            auth: self.auth,
            body: self.body,
            scripts: self.scripts,
            settings: self.settings,
            description: String::new(),
        }
    }
}

/// Snippet de código de una petición guardada (aunque traiga cambios sin guardar) o de un borrador
/// de `collection_id`: auth heredada de carpetas y colección, y variables del entorno (`env_id` o el
/// activo), la colección y las globales. Los secretos quedan como `{{nombre}}`.
pub fn snippet(
    ws: &Workspace,
    request: &Request,
    collection_id: Option<&str>,
    env_id: Option<&str>,
    lang: codegen::Lang,
) -> Result<String> {
    let mut owner: Option<Collection> = None;
    if !request.id.is_empty() {
        for summary in ws.collections()? {
            let collection = ws.load_collection(&summary.id)?;
            if collection.parents_of(&request.id).is_some() {
                owner = Some(collection);
                break;
            }
        }
    }
    if owner.is_none()
        && let Some(id) = collection_id
    {
        owner = Some(ws.find_collection(id)?);
    }
    let parents = owner
        .as_ref()
        .and_then(|c| c.parents_of(&request.id))
        .unwrap_or_default();
    let auth = engine::auth::effective_auth(owner.as_ref(), &parents, request);
    let environment = ws.active_environment(env_id)?;
    let globals = ws.globals()?;
    let resolver = Resolver::new()
        .with_variables(
            Scope::Environment,
            environment.as_ref().map_or(&[][..], |e| &e.variables),
        )
        .with_variables(
            Scope::Collection,
            owner.as_ref().map_or(&[][..], |c| &c.variables),
        )
        .with_variables(Scope::Globals, &globals.variables);
    Ok(codegen::render(lang, request, &auth, &resolver))
}

/// Un eslabón de la cadena de scripts: colección → carpetas → petición.
#[derive(Debug, Clone)]
struct Link {
    owner: String,
    scripts: Scripts,
}

fn chain(collection: Option<&Collection>, parents: &[&Folder], request: &Request) -> Vec<Link> {
    let mut links = Vec::new();
    if let Some(collection) = collection {
        links.push(Link {
            owner: format!("collection \"{}\"", collection.name),
            scripts: collection.scripts.clone(),
        });
    }
    for folder in parents {
        links.push(Link {
            owner: format!("folder \"{}\"", folder.name),
            scripts: folder.scripts.clone(),
        });
    }
    let name = if request.name.is_empty() {
        "request".to_owned()
    } else {
        format!("request \"{}\"", request.name)
    };
    links.push(Link {
        owner: name,
        scripts: request.scripts.clone(),
    });
    links
}

fn sources(links: &[Link], pick: impl Fn(&Scripts) -> &String) -> Vec<ScriptSource> {
    links
        .iter()
        .filter(|link| !pick(&link.scripts).trim().is_empty())
        .map(|link| ScriptSource {
            owner: link.owner.clone(),
            code: pick(&link.scripts).clone(),
        })
        .collect()
}

/// `pm.request.body` con la forma de Postman.
fn body_json(body: &Body) -> serde_json::Value {
    match body {
        Body::None => json!({"mode": "none"}),
        Body::Raw { text, .. } => json!({"mode": "raw", "raw": text}),
        Body::Urlencoded { fields } => json!({"mode": "urlencoded", "urlencoded": fields
            .iter().map(|f| json!({"key": f.key, "value": f.value, "disabled": !f.enabled})).collect::<Vec<_>>()}),
        Body::Multipart { fields } => json!({"mode": "formdata", "formdata": fields
            .iter().map(|f| match f.kind {
                FieldKind::Text => json!({"key": f.key, "value": f.value, "type": "text", "disabled": !f.enabled}),
                FieldKind::File => json!({"key": f.key, "src": f.path, "type": "file", "disabled": !f.enabled}),
            }).collect::<Vec<_>>()}),
        Body::Binary { path } => json!({"mode": "file", "file": {"src": path}}),
        Body::Graphql { query, variables } => {
            json!({"mode": "graphql", "graphql": {"query": query, "variables": variables}})
        }
    }
}

fn resolver_for(state: &ScriptState) -> Resolver {
    // Un valor de datos o local para una clave que es secreta en el modelo también es secreto.
    let model = state
        .environment
        .iter()
        .flat_map(|e| e.variables.iter())
        .chain(state.collection.iter())
        .chain(state.globals.iter());
    let secret_keys: std::collections::HashSet<&str> =
        model.filter(|v| v.secret).map(|v| v.key.as_str()).collect();
    let secret = |key: &str| secret_keys.contains(key);
    Resolver::new()
        .with_pairs_where(Scope::Override, &state.local, secret)
        .with_pairs_where(Scope::Iteration, &state.iteration, secret)
        .with_variables(
            Scope::Environment,
            state.environment.as_ref().map_or(&[][..], |e| &e.variables),
        )
        .with_variables(Scope::Collection, &state.collection)
        .with_variables(Scope::Globals, &state.globals)
}

fn vars_changed(state: &ScriptState) -> Vec<VarChange> {
    let mut changes: Vec<VarChange> = Vec::new();
    for op in &state.ops {
        let scope = match op.scope {
            VarScope::Environment => Scope::Environment,
            VarScope::Globals => Scope::Globals,
            VarScope::Collection => Scope::Collection,
            VarScope::Local => continue,
        };
        if !changes.iter().any(|c| c.scope == scope && c.key == op.key) {
            changes.push(VarChange {
                scope,
                key: op.key.clone(),
            });
        }
    }
    changes
}

/// Repite sobre el disco los `set`/`unset` de los scripts (entorno, globales y colección). Se
/// relee cada archivo justo antes para no pisar cambios hechos mientras tanto.
fn persist(ws: &Workspace, state: &ScriptState, collection_id: Option<&str>) {
    let ops = |scope: VarScope| state.ops.iter().filter(move |op| op.scope == scope);
    let outcome = (|| -> Result<()> {
        if let Some(env) = &state.environment
            && ops(VarScope::Environment).next().is_some()
        {
            let mut stored = ws.find_environment(&env.id)?;
            for op in ops(VarScope::Environment) {
                script::apply_to(&mut stored.variables, op);
            }
            ws.save_environment(&stored)?;
        }
        if ops(VarScope::Globals).next().is_some() {
            let mut globals = ws.globals()?;
            for op in ops(VarScope::Globals) {
                script::apply_to(&mut globals.variables, op);
            }
            ws.save_globals(&globals)?;
        }
        if let Some(id) = collection_id
            && ops(VarScope::Collection).next().is_some()
        {
            let mut collection = ws.load_collection(id)?;
            for op in ops(VarScope::Collection) {
                script::apply_to(&mut collection.variables, op);
            }
            ws.save_collection(&collection)?;
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        tracing::warn!("script variables not saved: {error:#}");
    }
}

fn mask_tests(tests: Vec<TestResult>, masker: &Masker) -> Vec<TestResult> {
    tests
        .into_iter()
        .map(|t| TestResult {
            name: masker.mask(&t.name),
            msg: t.msg.map(|m| masker.mask(&m)),
            ..t
        })
        .collect()
}

#[derive(Default)]
pub struct Executor {
    clients: Clients,
}

impl Executor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ejecuta `req`. Si su id pertenece a una colección del workspace, usa sus variables, la auth
    /// y los scripts heredados de carpetas/colección (aunque `req` traiga cambios sin guardar).
    pub async fn run(&self, ws: &Workspace, req: &Request, ctx: RunContext) -> Result<RunResult> {
        let mut owner: Option<Collection> = None;
        for summary in ws.collections()? {
            let collection = ws.load_collection(&summary.id)?;
            if collection.parents_of(&req.id).is_some() {
                owner = Some(collection);
                break;
            }
        }
        let parents = owner
            .as_ref()
            .and_then(|c| c.parents_of(&req.id))
            .unwrap_or_default();
        self.run_in(ws, owner.as_ref(), &parents, req, ctx).await
    }

    /// Como `run`, con la colección y las carpetas ya localizadas (lo usa el runner).
    pub async fn run_in(
        &self,
        ws: &Workspace,
        collection: Option<&Collection>,
        parents: &[&Folder],
        req: &Request,
        ctx: RunContext,
    ) -> Result<RunResult> {
        let auth = engine::auth::effective_auth(collection, parents, req);
        let links = chain(collection, parents, req);
        self.execute(
            ws,
            req,
            collection,
            &links,
            &auth,
            Some(req.id.clone()),
            ctx,
        )
        .await
    }

    pub async fn send_adhoc(
        &self,
        ws: &Workspace,
        draft: RequestDraft,
        ctx: RunContext,
    ) -> Result<RunResult> {
        let collection = match &draft.collection_id {
            Some(id) => Some(ws.find_collection(id)?),
            None => None,
        };
        let request = draft.into_request();
        let auth = engine::auth::effective_auth(collection.as_ref(), &[], &request);
        let links = chain(collection.as_ref(), &[], &request);
        self.execute(ws, &request, collection.as_ref(), &links, &auth, None, ctx)
            .await
    }

    /// Cliente y ajustes para `pm.sendRequest`.
    fn script_host(&self, http: &HttpSettings) -> Result<ScriptHost> {
        let client = self.clients.get(&ClientKey {
            verify_tls: http.verify_tls,
            follow_redirects: http.follow_redirects,
            max_redirects: http.max_redirects,
            proxy: http.proxy.clone().filter(|p| !p.trim().is_empty()),
        })?;
        Ok(ScriptHost {
            runtime: tokio::runtime::Handle::current(),
            client,
            timeout: Duration::from_millis(http.timeout_ms),
            user_agent: http.user_agent.clone(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_scripts(
        &self,
        phase: Phase,
        scripts: Vec<ScriptSource>,
        info: ScriptInfo,
        request: ScriptRequest,
        response: Option<ScriptResponse>,
        mut state: ScriptState,
        host: ScriptHost,
    ) -> Result<(ChainOutput, ScriptState)> {
        tokio::task::spawn_blocking(move || {
            let out = script::run_chain(
                phase,
                &scripts,
                &info,
                &request,
                response.as_ref(),
                &mut state,
                &host,
            );
            (out, state)
        })
        .await
        .context("script runner stopped")
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        ws: &Workspace,
        request: &Request,
        collection: Option<&Collection>,
        links: &[Link],
        auth: &Auth,
        request_id: Option<String>,
        ctx: RunContext,
    ) -> Result<RunResult> {
        let settings = ws.settings();
        let mut state = ScriptState {
            environment: ws.active_environment(ctx.env_id.as_deref())?,
            globals: ws.globals()?.variables,
            collection: collection.map(|c| c.variables.clone()).unwrap_or_default(),
            iteration: ctx.iteration.clone(),
            local: ctx.overrides.clone(),
            ops: Vec::new(),
        };
        // Secretos de antes de los scripts: un script puede cambiarlos y el valor viejo sigue
        // pudiendo aparecer en su consola.
        let mut masker = resolver_for(&state).masker().clone();
        let collection_id = collection.map(|c| c.id.as_str());
        let info = ScriptInfo {
            request_name: request.name.clone(),
            request_id: request_id.clone().unwrap_or_default(),
            iteration: ctx.iteration_index,
            iteration_count: ctx.iteration_count.max(1),
        };
        let pre = sources(links, |s| &s.pre);
        let test = sources(links, |s| &s.test);
        let host = if pre.is_empty() && test.is_empty() {
            None
        } else {
            Some(self.script_host(&settings.http)?)
        };
        let mut request = request.clone();
        let mut tests: Vec<TestResult> = Vec::new();
        let mut console: Vec<String> = Vec::new();

        // 1. Scripts `pre`, antes de resolver variables (pueden cambiarlas).
        if let Some(host) = host.clone().filter(|_| !pre.is_empty()) {
            let template = ScriptRequest {
                url: request.url.clone(),
                method: request.method.to_uppercase(),
                headers: request.headers.iter().map(HeaderItem::from).collect(),
                body: body_json(&request.body),
            };
            let (out, back) = self
                .run_scripts(Phase::Pre, pre, info.clone(), template, None, state, host)
                .await?;
            state = back;
            tests.extend(out.tests);
            console.extend(out.console);
            if let Some(headers) = out.headers {
                request.headers = headers;
            }
            if out.skip {
                persist(ws, &state, collection_id);
                masker.merge(resolver_for(&state).masker());
                return Ok(RunResult {
                    response: Response::default(),
                    resolved: ResolvedRequest {
                        method: request.method.to_uppercase(),
                        url: masker.mask(&request.url),
                        ..ResolvedRequest::default()
                    },
                    tests: mask_tests(tests, &masker),
                    vars_changed: vars_changed(&state),
                    history_id: String::new(),
                    console: console.iter().map(|line| masker.mask(line)).collect(),
                    skipped: true,
                    masker,
                    locals: state.local,
                });
            }
        }

        // 2. La petición, con las variables tal como las dejaron los scripts.
        let resolver = resolver_for(&state);
        masker.merge(resolver.masker());
        let (prepared, resolved, built_masker) =
            match engine::build_request(&request, auth, &resolver, &settings.http) {
                Ok(built) => built,
                Err(error) => {
                    persist(ws, &state, collection_id);
                    return Err(error);
                }
            };
        masker.merge(&built_masker);
        let client = self.clients.get(&prepared.client)?;
        let started = chrono::Utc::now().timestamp_millis();
        let outcome = engine::send(&client, &prepared).await;

        let history_id = new_id("h");
        let mut entry = HistoryEntry {
            id: history_id.clone(),
            ts: started,
            request_id: request_id.filter(|id| !id.is_empty()),
            source: ctx.source,
            method: resolved.method.clone(),
            url: resolved.url.clone(),
            req_headers: resolved.headers.clone(),
            req_body: resolved.body.clone().map(String::into_bytes),
            ..HistoryEntry::default()
        };
        let response = match outcome {
            Ok(response) => response,
            Err(error) => {
                let (kind, message) = engine::describe_error(&error);
                let message: String = masker
                    .mask(&message)
                    .chars()
                    .take(MAX_ERROR_CHARS)
                    .collect();
                entry.duration_ms = (chrono::Utc::now().timestamp_millis() - started).max(0) as u64;
                entry.error = Some(format!("{kind}: {message}"));
                persist(ws, &state, collection_id);
                let recorded = ctx.record_history && self.record(ws, &entry);
                return Err(NetworkError {
                    kind,
                    message,
                    url: resolved.url,
                    history_id: recorded.then_some(history_id),
                }
                .into());
            }
        };

        // 3. Scripts `test`, con la petición enviada y la respuesta.
        if let Some(host) = host.filter(|_| !test.is_empty()) {
            let sent = ScriptRequest {
                url: prepared.url.to_string(),
                method: resolved.method.clone(),
                headers: prepared
                    .headers
                    .iter()
                    .map(|(key, value)| HeaderItem {
                        key: key.clone(),
                        value: value.clone(),
                        disabled: false,
                    })
                    .collect(),
                body: body_json(&request.body),
            };
            let (out, back) = self
                .run_scripts(
                    Phase::Test,
                    test,
                    info,
                    sent,
                    Some(ScriptResponse::from(&response)),
                    state,
                    host,
                )
                .await?;
            state = back;
            tests.extend(out.tests);
            console.extend(out.console);
            masker.merge(resolver_for(&state).masker());
        }
        persist(ws, &state, collection_id);
        let tests = mask_tests(tests, &masker);
        let console: Vec<String> = console.iter().map(|line| masker.mask(line)).collect();

        entry.status = Some(response.status);
        entry.duration_ms = response.ms;
        entry.size_bytes = response.size;
        entry.res_headers = response
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), masker.mask(v)))
            .collect();
        entry.res_body = Some(masker.mask_bytes(&response.body));
        if !tests.is_empty() {
            entry.tests_json = serde_json::to_string(&tests).ok();
        }
        let recorded = ctx.record_history && self.record(ws, &entry);
        Ok(RunResult {
            response,
            resolved,
            tests,
            vars_changed: vars_changed(&state),
            history_id: if recorded { history_id } else { String::new() },
            console,
            skipped: false,
            masker,
            locals: state.local,
        })
    }

    fn record(&self, ws: &Workspace, entry: &HistoryEntry) -> bool {
        match ws.history().insert(entry) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!("history not recorded: {error:#}");
                false
            }
        }
    }
}
