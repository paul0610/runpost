//! Lógica de las tools (sección 8.2). Cada una devuelve `Ok(json)` o `Err(json de error)`;
//! `lib.rs` solo las expone por rmcp. Todo lo que sale pasa por enmascarado.

use std::{
    collections::VecDeque,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use runpost_core::{
    CoreError, Executor, RequestDraft, RequestSelector, RunContext, RunResult, Source, Workspace,
    budget::{self, Budget, RunDetail, Shaped},
    diff::{self, Side},
    import::postman_v21::PostmanKind,
    model::{
        Auth, Body, Collection, Environment, Folder, Item, KeyValue, RawLang, Request, SecretStore,
        Variable, new_id,
    },
    policy::{self, RateLimiter, Violation},
    runner::{self, Guard, RunnerOptions},
    storage::Snapshot,
    vars::{MASK, Masker},
};
use serde_json::{Map, Value, json};

use crate::{
    agent::{AgentCall, AgentHub, Approval, Caller, Decision},
    params::{self, Pairs},
};

pub type Reply = Result<Value, Value>;

/// Respuesta reciente para `get_response` sin releer SQLite. Ya enmascarada.
#[derive(Debug, Clone)]
struct Cached {
    id: String,
    /// Petición guardada de la que sale (para su referencia, 19.7).
    request_id: Option<String>,
    status: Option<u16>,
    ct: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[derive(Clone)]
pub struct Tools {
    ws: Workspace,
    executor: Arc<Executor>,
    limiter: Arc<RateLimiter>,
    cache: Arc<Mutex<VecDeque<Cached>>>,
    /// Por HTTP desde la app: a quién pedir aprobación (`require_approval`).
    hub: Option<Arc<AgentHub>>,
}

/// Respuesta al modelo según lo que decidió el usuario (o la falta de app).
fn decision_error(decision: Decision, violation: &Violation) -> Value {
    let hint = match decision {
        Decision::Denied => "the user denied it in the app",
        Decision::Expired => "no answer in 60 s; ask the user before retrying",
        Decision::Cancelled => "cancelled while waiting for the user",
        Decision::Approved | Decision::Unavailable => return violation.to_json(),
    };
    json!({"error": "policy", "rule": violation.rule, "hint": hint})
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        out
    }
}

fn error(code: &str, hint: &str) -> Value {
    json!({"error": code, "hint": hint})
}

fn core_error(error: &anyhow::Error, mask: &Masker) -> Value {
    budget::shape_error(error, mask)
}

fn pairs(map: &Option<Pairs>) -> Vec<KeyValue> {
    map.iter()
        .flatten()
        .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
        .collect()
}

/// `body` de MCP: texto (JSON si se puede parsear) o JSON.
fn body_from(value: &Value) -> Body {
    match value {
        Value::Null => Body::None,
        Value::String(text) => Body::Raw {
            lang: if serde_json::from_str::<Value>(text).is_ok() {
                RawLang::Json
            } else {
                RawLang::Text
            },
            text: text.clone(),
        },
        other => Body::Raw {
            lang: RawLang::Json,
            text: other.to_string(),
        },
    }
}

fn auth_from(value: &Value) -> Result<Auth, Value> {
    serde_json::from_value(value.clone()).map_err(|e| {
        json!({"error": "invalid_auth", "msg": e.to_string(),
               "hint": "types: none,inherit,bearer{token},basic{username,password},apikey{key,value,in}"})
    })
}

/// Busca o crea la carpeta `a/b/c` dentro de `items`.
fn folder_items<'a>(items: &'a mut Vec<Item>, path: Option<&str>) -> &'a mut Vec<Item> {
    let mut current = items;
    for name in path
        .into_iter()
        .flat_map(|p| p.split('/'))
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        let index = match current
            .iter()
            .position(|item| matches!(item, Item::Folder(f) if f.name == name))
        {
            Some(index) => index,
            None => {
                current.push(Item::Folder(Folder::new(name)));
                current.len() - 1
            }
        };
        let Item::Folder(folder) = &mut current[index] else {
            unreachable!()
        };
        current = &mut folder.items;
    }
    current
}

fn upsert_into(collection: &mut Collection, item: &params::UpsertItem) -> Result<String, Value> {
    let items = folder_items(&mut collection.items, item.folder.as_deref());
    let index = match items
        .iter()
        .position(|i| matches!(i, Item::Request(r) if r.name == item.name))
    {
        Some(index) => index,
        None => {
            let Some(url) = &item.url else {
                return Err(error(
                    "invalid_request",
                    "url is required for a new request",
                ));
            };
            let method = item.method.clone().unwrap_or_else(|| "GET".into());
            items.push(Item::Request(Box::new(Request::new(
                item.name.clone(),
                method,
                url.clone(),
            ))));
            items.len() - 1
        }
    };
    let Item::Request(request) = &mut items[index] else {
        unreachable!()
    };
    if let Some(method) = &item.method {
        request.method = method.to_uppercase();
    }
    if let Some(url) = &item.url {
        request.url = url.clone();
    }
    if item.headers.is_some() {
        request.headers = pairs(&item.headers);
    }
    if let Some(body) = &item.body {
        request.body = body_from(body);
    }
    if let Some(auth) = &item.auth {
        request.auth = auth_from(auth)?;
    }
    if let Some(scripts) = &item.scripts {
        if let Some(pre) = &scripts.pre {
            request.scripts.pre = pre.clone();
        }
        if let Some(test) = &scripts.test {
            request.scripts.test = test.clone();
        }
    }
    Ok(request.id.clone())
}

impl Tools {
    pub fn new(ws: Workspace) -> Self {
        Self {
            ws,
            executor: Arc::new(Executor::new()),
            limiter: Arc::new(RateLimiter::new()),
            cache: Arc::new(Mutex::new(VecDeque::new())),
            hub: None,
        }
    }

    /// Las peticiones protegidas se preguntan en la app en vez de rechazarse.
    pub fn with_hub(mut self, hub: Arc<AgentHub>) -> Self {
        self.hub = Some(hub);
        self
    }

    /// `require_approval` por HTTP con la app abierta: aviso en la app y espera de hasta 60 s.
    /// Sin hub (stdio) o sin ventana abierta, el error de política de siempre.
    async fn approve(
        &self,
        approval: Approval,
        violation: &Violation,
        caller: Option<&Caller>,
    ) -> Result<(), Value> {
        let (Some(hub), Some(caller)) = (&self.hub, caller) else {
            return Err(violation.to_json());
        };
        let approval = Approval {
            client: caller.client.clone(),
            ..approval
        };
        match hub.ask(approval).await {
            Decision::Approved => Ok(()),
            decision => Err(decision_error(decision, violation)),
        }
    }

    fn environment_name(&self, selector: Option<&str>) -> Option<String> {
        self.ws
            .active_environment(selector)
            .ok()
            .flatten()
            .map(|e| e.name)
    }

    /// Fila del panel Agente (19.2) para una llamada; `text` es lo que recibió el modelo.
    pub fn observe(
        &self,
        tool: &str,
        args: &Value,
        text: &str,
        is_error: bool,
        elapsed: Duration,
        caller: &Caller,
    ) -> AgentCall {
        let field = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        let parsed: Option<Value> = serde_json::from_str(text).ok();
        let delivered = text.len() as u64;
        let (raw, sent) = match parsed.as_ref().and_then(|v| v.get("t")) {
            Some(t) => (
                t["raw"].as_u64().unwrap_or(delivered),
                t["sent"].as_u64().unwrap_or(delivered),
            ),
            None => (delivered, delivered),
        };
        let status = match parsed.as_ref() {
            _ if is_error => parsed
                .as_ref()
                .and_then(|v| v["error"].as_str())
                .unwrap_or("error")
                .to_owned(),
            Some(v) if v["status"].is_u64() => v["status"].to_string(),
            Some(v) if v["skipped"] == json!(true) => "skipped".to_owned(),
            Some(v) if tool == "run_collection" => {
                match (v["passed"].as_u64(), v["total"].as_u64()) {
                    (Some(passed), Some(total)) => format!("{passed}/{total}"),
                    _ => format!("{} ✘", v["failed"].as_u64().unwrap_or_default()),
                }
            }
            _ => "ok".to_owned(),
        };
        let keys = || {
            args.get("keys")
                .or_else(|| args.get("vars"))
                .map(|v| match v {
                    Value::Array(items) => items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                    Value::Object(map) => map.keys().cloned().collect::<Vec<_>>().join(", "),
                    _ => String::new(),
                })
                .unwrap_or_default()
        };
        let mut target = match tool {
            "run_request" | "get_request" => field("request").to_owned(),
            "run_collection" if !field("folder").is_empty() => field("folder").to_owned(),
            "run_collection" | "upsert_request" if field("name").is_empty() => {
                field("collection").to_owned()
            }
            "upsert_request" => format!("{}/{}", field("collection"), field("name")),
            "send" => format!("{} {}", field("method").to_uppercase(), field("url")),
            "get_response" => field("response_id").to_owned(),
            "env_get" | "env_set" => keys(),
            "list_requests" => [field("collection"), field("folder"), field("q")]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
            "search" | "history" => field("q").to_owned(),
            "import_postman" => Path::new(field("path"))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let mut extra = |label: &str, value: &str| {
            if !value.is_empty() {
                if !target.is_empty() {
                    target.push_str(" · ");
                }
                target.push_str(label);
                target.push_str(value);
            }
        };
        extra("select ", field("select"));
        extra("diff ", field("diff_against"));
        if tool == "run_collection" {
            extra("", field("output").trim());
            if field("output").trim().is_empty() {
                extra("", "summary");
            }
        }
        let request_id = matches!(tool, "run_request" | "get_request")
            .then(|| {
                self.ws
                    .find_request(&RequestSelector::parse(field("request")))
                    .ok()
                    .map(|(_, request)| request.id)
            })
            .flatten();
        AgentCall {
            tool: tool.to_owned(),
            target: self.workspace_masker().mask(&truncate(&target, 160)),
            request_id,
            status,
            ok: !is_error,
            ms: elapsed.as_millis() as u64,
            raw,
            sent,
            client: caller.client.clone(),
            session: caller.session.clone(),
            ..AgentCall::default()
        }
    }

    /// Todos los secretos conocidos (entornos, colecciones, globales), para errores y listados.
    fn workspace_masker(&self) -> Masker {
        let mut mask = Masker::new();
        let mut add = |vars: &[Variable]| {
            for var in vars.iter().filter(|v| v.secret) {
                if let Some(value) = &var.value {
                    mask.add(value);
                }
            }
        };
        for env in self.ws.environments().unwrap_or_default() {
            add(&env.variables);
        }
        for summary in self.ws.collections().unwrap_or_default() {
            if let Ok(c) = self.ws.load_collection(&summary.id) {
                add(&c.variables);
            }
        }
        if let Ok(globals) = self.ws.globals() {
            add(&globals.variables);
        }
        mask
    }

    fn fail(&self, error: &anyhow::Error) -> Value {
        core_error(error, &self.workspace_masker())
    }

    fn remember(&self, result: &mut RunResult, request_id: Option<&str>) {
        if result.history_id.is_empty() {
            // Sin historial: id solo de caché, para poder usar get_response igualmente.
            result.history_id = new_id("c");
        }
        let limit = self.ws.settings().mcp.cache_responses;
        let entry = Cached {
            id: result.history_id.clone(),
            request_id: request_id.map(str::to_owned),
            status: (!result.skipped).then_some(result.response.status),
            ct: result
                .response
                .content_type()
                .unwrap_or_default()
                .to_owned(),
            headers: result.response.headers.clone(),
            body: result.masker.mask_bytes(&result.response.body),
        };
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.push_front(entry);
        cache.truncate(limit);
    }

    fn shape(
        &self,
        mut result: RunResult,
        request_id: Option<&str>,
        select: Option<String>,
        body_max: Option<usize>,
        headers: Option<bool>,
    ) -> Value {
        self.remember(&mut result, request_id);
        let settings = self.ws.settings().mcp;
        let budget = Budget {
            body_max_chars: body_max.unwrap_or(settings.body_max_chars),
            select,
            headers: headers.unwrap_or(settings.headers_default),
            mask: result.masker.clone(),
        };
        budget::shape_response(&result, &budget)
    }

    pub fn ws_overview(&self) -> Reply {
        let collections = self.ws.collections().map_err(|e| self.fail(&e))?;
        let envs = self.ws.environments().map_err(|e| self.fail(&e))?;
        let active = self
            .ws
            .active_environment(None)
            .ok()
            .flatten()
            .map(|e| e.id);
        Ok(json!({
            "collections": collections.iter().map(|c| json!({"id": c.id, "name": c.name, "requests": c.requests})).collect::<Vec<_>>(),
            "environments": envs.iter().map(|e| json!({"id": e.id, "name": e.name, "active": Some(&e.id) == active.as_ref()})).collect::<Vec<_>>(),
            "active_env": active,
        }))
    }

    pub fn list_requests(&self, p: params::ListRequests) -> Reply {
        let collections: Vec<Collection> = match &p.collection {
            Some(selector) => vec![
                self.ws
                    .find_collection(selector)
                    .map_err(|e| self.fail(&e))?,
            ],
            None => self
                .ws
                .collections()
                .map_err(|e| self.fail(&e))?
                .iter()
                .filter_map(|s| self.ws.load_collection(&s.id).ok())
                .collect(),
        };
        let folder = p.folder.as_deref().map(str::to_lowercase);
        let q = p.q.as_deref().map(str::to_lowercase);
        let mut out = Vec::new();
        for collection in &collections {
            collection.walk_requests(|parents, request| {
                if let Some(folder) = &folder
                    && !parents.iter().any(|f| f.name.to_lowercase() == *folder)
                {
                    return;
                }
                if let Some(q) = &q
                    && !request.name.to_lowercase().contains(q)
                    && !request.url.to_lowercase().contains(q)
                {
                    return;
                }
                let mut path = collection.name.clone();
                for f in parents {
                    path.push('/');
                    path.push_str(&f.name);
                }
                path.push('/');
                path.push_str(&request.name);
                out.push(json!({"id": request.id, "path": path, "m": request.method, "url": request.url}));
            });
        }
        Ok(Value::Array(out))
    }

    pub fn get_request(&self, p: params::GetRequest) -> Reply {
        let (_, request) = self
            .ws
            .find_request(&RequestSelector::parse(&p.request))
            .map_err(|e| self.fail(&e))?;
        let full = serde_json::to_value(&request).unwrap_or_default();
        let fields = p.fields.unwrap_or_else(|| {
            ["method", "url", "params", "headers", "body"]
                .map(String::from)
                .to_vec()
        });
        let mut out = Map::new();
        out.insert("id".into(), json!(request.id));
        for field in fields {
            if let Some(value) = full.get(&field) {
                out.insert(field, value.clone());
            }
        }
        Ok(Value::Object(out))
    }

    pub async fn run_request(&self, p: params::RunRequest, caller: Option<&Caller>) -> Reply {
        let (collection, request) = self
            .ws
            .find_request(&RequestSelector::parse(&p.request))
            .map_err(|e| self.fail(&e))?;
        if let Some(policy) = &collection.agent_policy {
            match policy::check_static(policy, &request.method, &request.id) {
                Ok(()) => {}
                Err(violation) if violation.rule == "require_approval" => {
                    let rule = if violation.detail == request.id {
                        format!("require_approval: {}", request.name)
                    } else {
                        format!("require_approval: {}", violation.detail)
                    };
                    let approval = Approval {
                        tool: "run_request".into(),
                        method: request.method.clone(),
                        url: request.url.clone(),
                        request: request.name.clone(),
                        request_id: Some(request.id.clone()),
                        collection: collection.name.clone(),
                        environment: self.environment_name(p.env.as_deref()),
                        rule,
                        count: 1,
                        ..Approval::default()
                    };
                    self.approve(approval, &violation, caller).await?;
                }
                Err(violation) => return Err(violation.to_json()),
            }
            self.limiter
                .check(&collection.id, policy.max_calls_per_minute)
                .map_err(|v| v.to_json())?;
        }
        let mut ctx = RunContext::new(Source::Mcp);
        ctx.env_id = p.env;
        ctx.overrides = p.vars.unwrap_or_default();
        ctx.record_history = p.save_history.unwrap_or(true);
        let result = self
            .executor
            .run(&self.ws, &request, ctx)
            .await
            .map_err(|e| self.fail(&e))?;
        Ok(self.shape(result, Some(&request.id), p.select, p.body_max, p.headers))
    }

    /// Collection Runner por MCP. La política del agente se aplica a cada petición: las que no
    /// pasan quedan como `<request error>` sin enviarse. Por HTTP con la app abierta, las que
    /// requieren aprobación se preguntan una sola vez, todas juntas, antes de empezar.
    pub async fn run_collection(&self, p: params::RunCollection, caller: Option<&Caller>) -> Reply {
        let detail = match p.output.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
            None | Some("summary") => RunDetail::Summary,
            Some("failures") => RunDetail::Failures,
            Some("full") => RunDetail::Full,
            Some(_) => return Err(error("bad_output", "use summary, failures or full")),
        };
        let (data, data_name) = match &p.data {
            Some(file) => {
                let text = std::fs::read_to_string(file)
                    .map_err(|e| error("data_unreadable", &format!("cannot read {file}: {e}")))?;
                let rows = runner::data::parse(file, &text)
                    .map_err(|e| error("data_invalid", &format!("{e:#}")))?;
                let name = Path::new(file)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                (rows, name)
            }
            None => (Vec::new(), None),
        };
        let approved = self.approve_run(&p, caller).await?;
        let limiter = self.limiter.clone();
        let guard: Guard = Arc::new(move |collection, request| {
            let Some(policy) = &collection.agent_policy else {
                return Ok(());
            };
            match policy::check_static(policy, &request.method, &request.id) {
                Err(v) if !(approved && v.rule == "require_approval") => {
                    return Err(format!("policy: {} ({})", v.rule, v.hint));
                }
                _ => {}
            }
            limiter
                .check(&collection.id, policy.max_calls_per_minute)
                .map_err(|v| format!("policy: {} ({})", v.rule, v.hint))
        });
        let opts = RunnerOptions {
            folder: p.folder,
            env: p.env,
            iterations: p.iterations,
            data,
            data_name,
            source: Source::Mcp,
            guard: Some(guard),
            ..RunnerOptions::default()
        };
        let report = self
            .executor
            .run_collection(&self.ws, &p.collection, opts, None)
            .await
            .map_err(|e| self.fail(&e))?;
        Ok(budget::shape_run(&report, detail))
    }

    // SPEC-DEVIATION: S6-D4 — una sola aprobación, antes de empezar, para todas las peticiones
    // `require_approval` del run (8.3 solo describe la espera para una petición).
    /// Aprobación previa de un `run_collection` con peticiones `require_approval`. `Ok(true)` si
    /// el usuario aprobó; `Ok(false)` si no hacía falta o no hay a quién preguntar (stdio).
    async fn approve_run(
        &self,
        p: &params::RunCollection,
        caller: Option<&Caller>,
    ) -> Result<bool, Value> {
        if self.hub.is_none() || caller.is_none() {
            return Ok(false);
        }
        let Ok(collection) = self.ws.find_collection(&p.collection) else {
            return Ok(false);
        };
        let Some(policy) = &collection.agent_policy else {
            return Ok(false);
        };
        let scope = match p.folder.as_deref().filter(|f| !f.trim().is_empty()) {
            Some(selector) => match runner::find_folder(&collection, selector) {
                Ok(folder) => Some(folder.id.clone()),
                Err(_) => return Ok(false),
            },
            None => None,
        };
        let mut protected = Vec::new();
        collection.walk_requests(|parents, request| {
            let inside = scope
                .as_ref()
                .is_none_or(|id| parents.iter().any(|f| &f.id == id));
            if inside
                && let Err(v) = policy::check_static(policy, &request.method, &request.id)
                && v.rule == "require_approval"
            {
                protected.push((request.clone(), v));
            }
        });
        let Some((first, violation)) = protected.first().cloned() else {
            return Ok(false);
        };
        let approval = Approval {
            tool: "run_collection".into(),
            method: first.method.clone(),
            url: first.url.clone(),
            request: first.name.clone(),
            request_id: Some(first.id.clone()),
            collection: collection.name.clone(),
            environment: self.environment_name(p.env.as_deref()),
            rule: format!("require_approval: {}", violation.detail),
            count: protected.len(),
            ..Approval::default()
        };
        self.approve(approval, &violation, caller)
            .await
            .map(|()| true)
    }

    pub async fn send(&self, p: params::Send) -> Reply {
        let auth = match &p.auth {
            Some(auth) => auth_from(auth)?,
            None => Auth::Inherit,
        };
        let mut saved_id = None;
        let mut collection_id = None;
        if let Some(save) = &p.save_as {
            let item = params::UpsertItem {
                folder: save.folder.clone(),
                name: save.name.clone(),
                method: Some(p.method.clone()),
                url: Some(p.url.clone()),
                headers: p.headers.clone(),
                body: p.body.clone(),
                auth: p.auth.clone(),
                scripts: None,
            };
            let (id, col) = self.upsert_items(&save.collection, &[item])?;
            saved_id = id.into_iter().next();
            collection_id = Some(col);
        }
        let draft = RequestDraft {
            method: p.method,
            url: p.url,
            headers: pairs(&p.headers),
            body: p.body.as_ref().map(body_from).unwrap_or_default(),
            auth,
            collection_id,
            ..RequestDraft::default()
        };
        let mut ctx = RunContext::new(Source::Mcp);
        ctx.env_id = p.env;
        let add_saved = |mut value: Value| {
            if let (Some(id), Value::Object(map)) = (&saved_id, &mut value) {
                map.insert("saved_id".into(), json!(id));
            }
            value
        };
        match self.executor.send_adhoc(&self.ws, draft, ctx).await {
            Ok(result) => Ok(add_saved(self.shape(
                result,
                saved_id.as_deref(),
                p.select,
                p.body_max,
                None,
            ))),
            Err(e) => Err(add_saved(self.fail(&e))),
        }
    }

    /// Respuesta por id: primero la caché, después el historial.
    fn lookup(&self, response_id: &str) -> Result<Cached, Value> {
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|c| c.id == response_id)
            .cloned();
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let entry = self
            .ws
            .history()
            .get(response_id)
            .map_err(|e| self.fail(&e))?
            .ok_or_else(|| {
                error(
                    "response_not_found",
                    "use the response_id from run_request or history",
                )
            })?;
        let ct = entry
            .res_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        Ok(Cached {
            id: entry.id,
            request_id: entry.request_id,
            status: entry.status,
            ct,
            headers: entry.res_headers,
            body: entry.res_body.unwrap_or_default(),
        })
    }

    /// `diff_against` y `save_ref` (19.7): qué cambió respecto a la referencia o a otra respuesta.
    fn compare(&self, current: &Cached, p: &params::GetResponse) -> Reply {
        let no_request = || {
            error(
                "no_saved_request",
                "only responses of saved requests (run_request) have a reference",
            )
        };
        let mut out = Map::new();
        if let Some(against) = p.diff_against.as_deref() {
            let (status, ct, body) = if against == "ref" {
                let request_id = current.request_id.as_deref().ok_or_else(no_request)?;
                let snapshot = self
                    .ws
                    .history()
                    .snapshot(request_id)
                    .map_err(|e| self.fail(&e))?
                    .ok_or_else(|| {
                        error(
                            "ref_not_found",
                            "save one first: get_response with save_ref:true",
                        )
                    })?;
                (snapshot.status, snapshot.content_type, snapshot.body)
            } else {
                let other = self.lookup(against)?;
                (other.status, other.ct, other.body)
            };
            let mut ignore = self.ws.settings().diff.ignore;
            ignore.extend(p.ignore.iter().flatten().cloned());
            let found = diff::diff_responses(
                Side {
                    status,
                    content_type: &ct,
                    body: &body,
                },
                Side {
                    status: current.status,
                    content_type: &current.ct,
                    body: &current.body,
                },
                &ignore,
            );
            if let Value::Object(map) = serde_json::to_value(found).unwrap_or_default() {
                out = map;
            }
        }
        if p.save_ref.unwrap_or(false) {
            let request_id = current.request_id.clone().ok_or_else(no_request)?;
            self.ws
                .history()
                .save_snapshot(&Snapshot {
                    request_id,
                    history_id: current.id.clone(),
                    ts: now_ms(),
                    status: current.status,
                    content_type: current.ct.clone(),
                    body: current.body.clone(),
                })
                .map_err(|e| self.fail(&e))?;
            out.insert("ref_saved".into(), json!(true));
        }
        Ok(Value::Object(out))
    }

    pub fn get_response(&self, p: params::GetResponse) -> Reply {
        let cached = self.lookup(&p.response_id)?;
        if p.diff_against.is_some() || p.save_ref.unwrap_or(false) {
            return self.compare(&cached, &p);
        }
        let mask = self.workspace_masker();
        if p.headers.unwrap_or(false) {
            return Ok(json!({"headers": budget::shape_headers(&cached.headers, &mask)}));
        }
        let shaped = budget::shape_body(&cached.body, &cached.ct, p.select.as_deref(), &mask)
            .map_err(|msg| json!({"error": "select", "msg": msg}))?;
        let mut out = Map::new();
        match p.range {
            Some([from, to]) => {
                let text = match shaped {
                    Shaped::Json(value) => value.to_string(),
                    Shaped::Text(text) => text,
                };
                let total = text.chars().count();
                let to = to.min(total);
                let from = from.min(to);
                let fragment: String = text.chars().skip(from).take(to - from).collect();
                out.insert("body".into(), json!(fragment));
                out.insert("range".into(), json!([from, to]));
                out.insert("total_chars".into(), json!(total));
            }
            None => budget::put_body(&mut out, shaped, self.ws.settings().mcp.body_max_chars),
        }
        Ok(Value::Object(out))
    }

    fn environment(&self, selector: Option<&str>) -> Result<Environment, Value> {
        match self.ws.active_environment(selector) {
            Ok(Some(env)) => Ok(env),
            Ok(None) => Err(error(
                "no_active_env",
                "pass env; ws_overview lists environments",
            )),
            Err(e) => Err(self.fail(&e)),
        }
    }

    pub fn env_get(&self, p: params::EnvGet) -> Reply {
        let env = self.environment(p.env.as_deref())?;
        let mut vars = Map::new();
        for var in &env.variables {
            if p.keys.as_ref().is_some_and(|keys| !keys.contains(&var.key)) {
                continue;
            }
            let value = if var.secret {
                MASK.to_owned()
            } else {
                var.value.clone().unwrap_or_default()
            };
            vars.insert(var.key.clone(), json!(value));
        }
        Ok(json!({"name": env.name, "vars": vars}))
    }

    pub fn env_set(&self, p: params::EnvSet) -> Reply {
        let mut env = match self.ws.find_environment(&p.env) {
            Ok(env) => env,
            Err(e) if matches!(e.downcast_ref(), Some(CoreError::NotFound { .. })) => {
                Environment::new(p.env.clone())
            }
            Err(e) => return Err(self.fail(&e)),
        };
        // Los secretos van al llavero del sistema (19.4); el archivo solo guarda la referencia.
        let secret_keys = p.secret.unwrap_or_default();
        let in_keyring = |var: &Variable| var.secret && var.store == Some(SecretStore::Keyring);
        let mut changed = Vec::new();
        for (key, value) in &p.vars {
            let secret = secret_keys.contains(key);
            match env.variables.iter_mut().find(|v| &v.key == key) {
                Some(var) => {
                    if var.value.as_deref() != Some(value.as_str()) || (secret && !in_keyring(var))
                    {
                        var.value = Some(value.clone());
                        if secret {
                            var.secret = true;
                            var.store = Some(SecretStore::Keyring);
                        }
                        changed.push(key.clone());
                    }
                }
                None => {
                    let mut var = Variable::new(key.clone(), value.clone());
                    if secret {
                        var.secret = true;
                        var.store = Some(SecretStore::Keyring);
                    }
                    env.variables.push(var);
                    changed.push(key.clone());
                }
            }
        }
        // Marcar como secreto (y llevar al llavero) una variable ya existente sin cambiar su valor.
        for key in &secret_keys {
            if let Some(var) = env
                .variables
                .iter_mut()
                .find(|v| &v.key == key && !in_keyring(v))
            {
                var.secret = true;
                var.store = Some(SecretStore::Keyring);
                if !changed.contains(key) {
                    changed.push(key.clone());
                }
            }
        }
        match self.ws.save_environment(&env) {
            Ok(()) => Ok(json!({"ok": true, "changed": changed})),
            Err(e) if matches!(e.downcast_ref(), Some(CoreError::Keyring(_))) => {
                // Sin llavero (p. ej. Linux sin Secret Service): quedan en el archivo y se avisa.
                for var in env.variables.iter_mut().filter(|v| v.secret) {
                    if secret_keys.contains(&var.key) {
                        var.store = Some(SecretStore::File);
                    }
                }
                self.ws.save_environment(&env).map_err(|e| self.fail(&e))?;
                Ok(json!({"ok": true, "changed": changed, "store": "file",
                          "hint": "OS keyring unavailable: secrets saved in the environment file"}))
            }
            Err(e) => Err(self.fail(&e)),
        }
    }

    /// Crea la colección si no existe y aplica cada item. Devuelve (ids, id de colección).
    fn upsert_items(
        &self,
        collection: &str,
        items: &[params::UpsertItem],
    ) -> Result<(Vec<String>, String), Value> {
        let mut target = match self.ws.find_collection(collection) {
            Ok(c) => c,
            Err(e) if matches!(e.downcast_ref(), Some(CoreError::NotFound { .. })) => {
                Collection::new(collection)
            }
            Err(e) => return Err(self.fail(&e)),
        };
        let ids = items
            .iter()
            .map(|item| upsert_into(&mut target, item))
            .collect::<Result<Vec<_>, _>>()?;
        self.ws
            .save_collection(&target)
            .map_err(|e| self.fail(&e))?;
        Ok((ids, target.id))
    }

    pub fn upsert_request(&self, p: params::Upsert) -> Reply {
        let mut items = p.requests.clone().unwrap_or_default();
        let batch = p.requests.is_some();
        if let Some(name) = &p.name {
            items.insert(
                0,
                params::UpsertItem {
                    folder: p.folder.clone(),
                    name: name.clone(),
                    method: p.method.clone(),
                    url: p.url.clone(),
                    headers: p.headers.clone(),
                    body: p.body.clone(),
                    auth: p.auth.clone(),
                    scripts: p.scripts.clone(),
                },
            );
        }
        if items.is_empty() {
            return Err(error(
                "invalid_request",
                "pass name (+method,url) or requests:[…]",
            ));
        }
        let (ids, _) = self.upsert_items(&p.collection, &items)?;
        Ok(if batch {
            json!({"ids": ids})
        } else {
            json!({"id": ids[0]})
        })
    }

    pub fn import_postman(&self, p: params::ImportPostman) -> Reply {
        let kind = match p.kind.as_deref() {
            Some(kind) => Some(
                kind.parse::<PostmanKind>()
                    .map_err(|e| json!({"error": "invalid_kind", "msg": e.to_string()}))?,
            ),
            None => None,
        };
        let report = match (&p.path, &p.text) {
            (Some(path), _) => self.ws.import_postman_file(Path::new(path), kind),
            (None, Some(text)) => self.ws.import_postman(text, kind, None),
            (None, None) => return Err(error("invalid_request", "pass path or text")),
        }
        .map_err(|e| self.fail(&e))?;
        let mut out = json!({"id": report.id, "name": report.name});
        match report.kind {
            PostmanKind::Collection => out["requests"] = json!(report.requests),
            PostmanKind::Environment => out["vars"] = json!(report.variables),
        }
        if !report.warnings.is_empty() {
            out["warnings"] = json!(report.warnings.iter().take(5).collect::<Vec<_>>());
        }
        Ok(out)
    }

    pub fn history(&self, p: params::History) -> Reply {
        let limit = p.limit.unwrap_or(10).clamp(1, 100);
        let fetch = if p.status.is_some() { 500 } else { limit };
        let rows = self
            .ws
            .history()
            .list(fetch, 0, p.q.as_deref())
            .map_err(|e| self.fail(&e))?;
        Ok(Value::Array(
            rows.into_iter()
                .filter(|row| p.status.is_none() || row.status == p.status)
                .take(limit)
                .map(|row| {
                    let mut out = json!({"id": row.id, "ts": row.ts, "m": row.method, "url": row.url,
                                         "status": row.status, "ms": row.duration_ms});
                    if let Some(error) = row.error {
                        out["err"] = json!(error.split(':').next().unwrap_or_default());
                    }
                    out
                })
                .collect(),
        ))
    }

    pub fn search(&self, p: params::Search) -> Reply {
        let q = p.q.to_lowercase();
        if q.is_empty() {
            return Err(error("invalid_request", "q is empty"));
        }
        let mut out = Vec::new();
        for summary in self.ws.collections().map_err(|e| self.fail(&e))? {
            let Ok(collection) = self.ws.load_collection(&summary.id) else {
                continue;
            };
            for (path, request) in collection.request_paths() {
                let body = match &request.body {
                    Body::Raw { text, .. } => text.clone(),
                    Body::Graphql { query, variables } => format!("{query} {variables}"),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                let headers = request
                    .headers
                    .iter()
                    .map(|h| format!("{}: {}", h.key, h.value))
                    .collect::<Vec<_>>()
                    .join("\n");
                let place = [
                    ("name", &request.name),
                    ("url", &request.url),
                    ("headers", &headers),
                    ("body", &body),
                ]
                .into_iter()
                .find(|(_, text)| text.to_lowercase().contains(&q))
                .map(|(place, _)| place);
                if let Some(place) = place {
                    out.push(json!({"id": request.id, "path": path, "where": place}));
                }
                if out.len() >= 50 {
                    return Ok(Value::Array(out));
                }
            }
        }
        Ok(Value::Array(out))
    }
}
