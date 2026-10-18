//! Scripts `pre` y `test` en QuickJS (sección 10): subconjunto `pm.*` compatible con Postman.
//!
//! Cada script corre en un runtime nuevo (contexto limpio) con 5 s y 32 MB. Lo que comparten los
//! scripts de una cadena (colección → carpetas → petición) son las variables y las cabeceras de la
//! petición, que pasan de uno a otro como JSON. `prelude.js` implementa `pm.*` en JavaScript sobre
//! unas pocas funciones nativas (hash, base64 y `pm.sendRequest`). Los errores de un script nunca
//! se propagan: se convierten en un test fallido `<script error>`.

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use base64::{
    Engine, alphabet,
    engine::{
        DecodePaddingMode,
        general_purpose::{GeneralPurpose, GeneralPurposeConfig, STANDARD},
    },
};
use rquickjs::{
    Coerced, Context, Ctx, Exception, Promise, Runtime, Value, context::EvalOptions, prelude::Func,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::{
    engine::{self, ClientKey, Headers, Prepared, PreparedBody},
    executor::TestResult,
    model::{Environment, KeyValue, Variable},
    vars::Vars,
};

/// Tiempo máximo de cada script, incluidos sus `setTimeout` y `pm.sendRequest`.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Memoria máxima del runtime de cada script.
pub const MEMORY_LIMIT: usize = 32 * 1024 * 1024;
/// Nombre del test que representa un error del propio script (sección 10).
pub const SCRIPT_ERROR: &str = "<script error>";

const PRELUDE: &str = include_str!("prelude.js");

/// `atob`/`base64.decode` aceptan el relleno `=` o su ausencia, como los navegadores.
const FORGIVING: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    #[serde(rename = "prerequest")]
    Pre,
    #[serde(rename = "test")]
    Test,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Pre => "pre-request script",
            Phase::Test => "test script",
        }
    }
}

/// Un script de la cadena y de quién es (para los mensajes de error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSource {
    /// `collection "X"`, `folder "Y"` o `request "Z"`.
    pub owner: String,
    pub code: String,
}

/// `pm.info`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptInfo {
    pub request_name: String,
    pub request_id: String,
    /// Empieza en 0, como en Postman.
    pub iteration: u32,
    pub iteration_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderItem {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub disabled: bool,
}

impl From<&KeyValue> for HeaderItem {
    fn from(kv: &KeyValue) -> Self {
        Self {
            key: kv.key.clone(),
            value: kv.value.clone(),
            disabled: !kv.enabled,
        }
    }
}

impl From<HeaderItem> for KeyValue {
    fn from(item: HeaderItem) -> Self {
        KeyValue {
            key: item.key,
            value: item.value,
            enabled: !item.disabled,
        }
    }
}

/// `pm.request`: en `pre` la petición sin resolver (como Postman); en `test`, la enviada.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ScriptRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<HeaderItem>,
    /// Forma de Postman: `{mode, raw | urlencoded | formdata | graphql | file}`.
    pub body: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub key: String,
    pub value: String,
}

/// `pm.response`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResponse {
    pub code: u16,
    pub status: String,
    pub headers: Vec<Pair>,
    pub body: String,
    pub response_time: u64,
    pub size: u64,
}

impl From<&engine::Response> for ScriptResponse {
    fn from(response: &engine::Response) -> Self {
        Self {
            code: response.status,
            status: response.status_text.clone(),
            headers: pairs_from(&response.headers),
            body: String::from_utf8_lossy(&response.body).into_owned(),
            response_time: response.ms,
            size: response.size,
        }
    }
}

fn pairs_from(headers: &Headers) -> Vec<Pair> {
    headers
        .iter()
        .map(|(key, value)| Pair {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VarScope {
    Environment,
    Globals,
    Collection,
    /// `pm.variables.set`: vive lo que dura la ejecución (en el runner, toda la ejecución).
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
    Set,
    Unset,
}

/// Un `set`/`unset` hecho por un script; el ejecutor los repite sobre el disco al terminar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VarOp {
    pub scope: VarScope,
    pub op: OpKind,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// Variables que leen y cambian los scripts de una ejecución.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptState {
    pub environment: Option<Environment>,
    pub globals: Vec<Variable>,
    pub collection: Vec<Variable>,
    pub iteration: Vars,
    pub local: Vars,
    /// Operaciones aplicadas, en orden (para guardarlas y para `vars_changed`).
    pub ops: Vec<VarOp>,
}

impl ScriptState {
    /// Aplica la operación en memoria. Sin entorno activo, `pm.environment.set` no hace nada
    /// (igual que Postman con "No Environment") y se devuelve `false`.
    pub fn apply(&mut self, op: VarOp) -> bool {
        match op.scope {
            VarScope::Environment => match &mut self.environment {
                Some(env) => apply_to(&mut env.variables, &op),
                None => return false,
            },
            VarScope::Globals => apply_to(&mut self.globals, &op),
            VarScope::Collection => apply_to(&mut self.collection, &op),
            VarScope::Local => match op.op {
                OpKind::Set => {
                    self.local
                        .insert(op.key.clone(), op.value.clone().unwrap_or_default());
                }
                OpKind::Unset => {
                    self.local.remove(&op.key);
                }
            },
        }
        self.ops.push(op);
        true
    }

    fn input_vars(&self) -> Json {
        json!({
            "environment": self.environment.as_ref().map(|env| json!({"name": env.name, "values": visible(&env.variables)})),
            "globals": visible(&self.globals),
            "collection": visible(&self.collection),
            "iteration": self.iteration,
            "local": self.local,
        })
    }
}

/// Variables que un script puede leer: habilitadas y con valor.
fn visible(vars: &[Variable]) -> Vec<Pair> {
    vars.iter()
        .filter(|v| v.enabled)
        .filter_map(|v| {
            v.value.as_ref().map(|value| Pair {
                key: v.key.clone(),
                value: value.clone(),
            })
        })
        .collect()
}

/// Aplica una operación a una lista de variables del modelo, conservando `secret` y `store`.
pub fn apply_to(vars: &mut Vec<Variable>, op: &VarOp) {
    match op.op {
        OpKind::Set => {
            let value = op.value.clone().unwrap_or_default();
            match vars.iter_mut().find(|v| v.key == op.key) {
                Some(var) => {
                    var.value = Some(value);
                    var.enabled = true;
                }
                None => vars.push(Variable::new(op.key.clone(), value)),
            }
        }
        OpKind::Unset => vars.retain(|v| v.key != op.key),
    }
}

/// Lo que necesitan las funciones nativas: el runtime de tokio (para `pm.sendRequest`) y el
/// cliente HTTP con los ajustes del workspace.
#[derive(Clone)]
pub struct ScriptHost {
    pub runtime: tokio::runtime::Handle,
    pub client: reqwest::Client,
    pub timeout: Duration,
    pub user_agent: String,
}

/// Resultado de la cadena de scripts de una fase.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChainOutput {
    pub tests: Vec<TestResult>,
    pub console: Vec<String>,
    /// Cabeceras de la petición si un script `pre` las cambió.
    pub headers: Option<Vec<KeyValue>>,
    /// `pm.execution.skipRequest()`.
    pub skip: bool,
}

/// Ejecuta en orden los scripts de una fase. Bloquea: llamar desde `spawn_blocking`.
pub fn run_chain(
    phase: Phase,
    scripts: &[ScriptSource],
    info: &ScriptInfo,
    request: &ScriptRequest,
    response: Option<&ScriptResponse>,
    state: &mut ScriptState,
    host: &ScriptHost,
) -> ChainOutput {
    let mut out = ChainOutput::default();
    let mut request = request.clone();
    for script in scripts.iter().filter(|s| !s.code.trim().is_empty()) {
        let label = format!("{} of {}", phase.label(), script.owner);
        let input = json!({
            "phase": phase,
            "info": info,
            "vars": state.input_vars(),
            "request": request,
            "response": response,
        });
        let result = run_script(&label, &script.code, &input, host);
        out.tests.extend(result.tests);
        out.console.extend(result.console);
        for op in result.changes {
            let scope = op.scope;
            if !state.apply(op) && scope == VarScope::Environment {
                out.console
                    .push("[warn] pm.environment: no active environment, value not saved".into());
            }
        }
        if let Some(headers) = result.request_headers {
            out.headers = Some(headers.iter().cloned().map(KeyValue::from).collect());
            request.headers = headers;
        }
        if result.skip {
            out.skip = true;
            break;
        }
    }
    out
}

/// Salida de un script, tal como la devuelve `__rp_finish()`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOutput {
    #[serde(default)]
    pub tests: Vec<TestResult>,
    #[serde(default)]
    pub console: Vec<String>,
    #[serde(default)]
    pub changes: Vec<VarOp>,
    #[serde(default)]
    pub request_headers: Option<Vec<HeaderItem>>,
    #[serde(default)]
    pub skip: bool,
}

fn script_error(label: &str, message: &str) -> TestResult {
    TestResult {
        name: SCRIPT_ERROR.into(),
        passed: false,
        msg: Some(format!("{label}: {message}")),
        skipped: false,
    }
}

/// Ejecuta un script con su `input` (fase, variables, petición, respuesta, info). Nunca falla:
/// cualquier error (sintaxis, excepción, tiempo, memoria) queda como test `<script error>`.
pub fn run_script(label: &str, code: &str, input: &Json, host: &ScriptHost) -> ScriptOutput {
    match run_inner(label, code, input, host) {
        Ok(mut output) => {
            for test in output.tests.iter_mut().filter(|t| t.name == SCRIPT_ERROR) {
                if let Some(msg) = &test.msg
                    && !msg.starts_with(label)
                {
                    test.msg = Some(format!("{label}: {msg}"));
                }
            }
            output
        }
        Err(message) => ScriptOutput {
            tests: vec![script_error(label, &message)],
            ..ScriptOutput::default()
        },
    }
}

fn run_inner(
    label: &str,
    code: &str,
    input: &Json,
    host: &ScriptHost,
) -> Result<ScriptOutput, String> {
    let runtime = Runtime::new().map_err(|e| format!("cannot start QuickJS: {e}"))?;
    runtime.set_memory_limit(MEMORY_LIMIT);
    let deadline = Rc::new(Cell::new(Instant::now() + TIMEOUT));
    let interrupted = Rc::new(Cell::new(false));
    {
        let deadline = deadline.clone();
        let interrupted = interrupted.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let late = Instant::now() > deadline.get();
            if late {
                interrupted.set(true);
            }
            late
        })));
    }
    let context = Context::full(&runtime).map_err(|e| format!("cannot start QuickJS: {e}"))?;
    context.with(|ctx| {
        install_natives(&ctx, host, deadline.clone())
            .map_err(|e| format!("cannot prepare the script: {e}"))?;
        ctx.globals()
            .set("__rp_input", input.to_string())
            .map_err(|e| format!("cannot prepare the script: {e}"))?;
        if let Err(error) = ctx.eval::<(), _>(PRELUDE) {
            let detail = describe(&ctx, error);
            return Err(format!("cannot prepare the script: {detail}"));
        }

        let mut errors: Vec<String> = Vec::new();
        let mut options = EvalOptions::default();
        // Como el sandbox de Postman: sin modo estricto y con `await` en el nivel superior.
        options.strict = false;
        options.promise = true;
        options.filename = Some(label.to_owned());
        let promise = match ctx.eval_with_options::<Promise, _>(code, options) {
            Ok(promise) => Some(promise),
            Err(error) => {
                errors.push(describe(&ctx, error));
                None
            }
        };

        // Bucle de eventos: microtareas y luego temporizadores, hasta que no quede nada o se
        // acabe el tiempo.
        let mut timed_out = false;
        loop {
            while ctx.execute_pending_job() {}
            if interrupted.get() || Instant::now() >= deadline.get() {
                timed_out = true;
                break;
            }
            let next = match ctx.eval::<f64, _>("__rp_next_timer()") {
                Ok(next) => next,
                Err(_) => {
                    timed_out = interrupted.get();
                    break;
                }
            };
            if next < 0.0 {
                break;
            }
            let wait = Duration::from_millis(next.ceil() as u64);
            let remaining = deadline.get().saturating_duration_since(Instant::now());
            if wait > remaining {
                std::thread::sleep(remaining);
                timed_out = true;
                break;
            }
            std::thread::sleep(wait);
            if ctx.eval::<(), _>("__rp_run_timers()").is_err() {
                timed_out = interrupted.get();
                break;
            }
        }
        // A partir de aquí solo se recoge el resultado: sin límite de tiempo.
        deadline.set(Instant::now() + Duration::from_secs(3600));

        if timed_out {
            errors.push(format!("script timed out after {} s", TIMEOUT.as_secs()));
        } else if let Some(promise) = promise
            && let Some(Err(error)) = promise.result::<Value>()
        {
            errors.push(describe(&ctx, error));
        }

        let json: String = ctx
            .eval("__rp_finish()")
            .map_err(|e| format!("cannot read the script result: {}", describe(&ctx, e)))?;
        let mut output: ScriptOutput = serde_json::from_str(&json)
            .map_err(|e| format!("cannot read the script result: {e}"))?;
        output
            .tests
            .extend(errors.iter().map(|message| script_error(label, message)));
        Ok(output)
    })
}

/// `Nombre: mensaje` de la excepción pendiente (o del error de rquickjs si no es una excepción).
fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if !matches!(error, rquickjs::Error::Exception) {
        return error.to_string();
    }
    let value = ctx.catch();
    if let Some(object) = value.as_object() {
        let name = object
            .get::<_, Option<String>>("name")
            .ok()
            .flatten()
            .unwrap_or_else(|| "Error".into());
        let message = object
            .get::<_, Option<String>>("message")
            .ok()
            .flatten()
            .unwrap_or_default();
        return if message.is_empty() {
            name
        } else {
            format!("{name}: {message}")
        };
    }
    let text = value
        .get::<Coerced<String>>()
        .map(|c| c.0)
        .unwrap_or_else(|_| "unknown error".into());
    format!("Error: {text}")
}

fn install_natives<'js>(
    ctx: &Ctx<'js>,
    host: &ScriptHost,
    deadline: Rc<Cell<Instant>>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    globals.set(
        "__rp_digest",
        Func::new(
            |ctx: Ctx<'js>, algorithm: String, data: String, encoding: String| {
                digest(&algorithm, data.as_bytes(), &encoding)
                    .map_err(|message| Exception::throw_type(&ctx, &message))
            },
        ),
    )?;
    globals.set(
        "__rp_hmac",
        Func::new(
            |ctx: Ctx<'js>, algorithm: String, key: String, data: String, encoding: String| {
                hmac(&algorithm, key.as_bytes(), data.as_bytes(), &encoding)
                    .map_err(|message| Exception::throw_type(&ctx, &message))
            },
        ),
    )?;
    globals.set(
        "__rp_base64",
        Func::new(|ctx: Ctx<'js>, mode: String, text: String| {
            base64_native(&mode, &text).map_err(|message| Exception::throw_type(&ctx, &message))
        }),
    )?;
    let host = host.clone();
    globals.set(
        "__rp_send",
        Func::new(move |options: String| send_native(&host, deadline.get(), &options)),
    )?;
    Ok(())
}

fn encode(bytes: &[u8], encoding: &str) -> Result<String, String> {
    match encoding {
        "hex" | "" => Ok(bytes.iter().map(|b| format!("{b:02x}")).collect()),
        "base64" => Ok(STANDARD.encode(bytes)),
        other => Err(format!(
            "unknown encoding {other:?} (use \"hex\" or \"base64\")"
        )),
    }
}

fn digest(algorithm: &str, data: &[u8], encoding: &str) -> Result<String, String> {
    use aws_lc_rs::digest;
    let algorithm: &'static digest::Algorithm = match algorithm.to_ascii_lowercase().as_str() {
        "sha1" => &digest::SHA1_FOR_LEGACY_USE_ONLY,
        "sha256" => &digest::SHA256,
        "sha384" => &digest::SHA384,
        "sha512" => &digest::SHA512,
        other => {
            return Err(format!(
                "unknown hash {other:?} (use sha1, sha256, sha384 or sha512)"
            ));
        }
    };
    encode(digest::digest(algorithm, data).as_ref(), encoding)
}

fn hmac(algorithm: &str, key: &[u8], data: &[u8], encoding: &str) -> Result<String, String> {
    use aws_lc_rs::hmac;
    let algorithm = match algorithm.to_ascii_lowercase().as_str() {
        "sha1" => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        "sha256" => hmac::HMAC_SHA256,
        "sha384" => hmac::HMAC_SHA384,
        "sha512" => hmac::HMAC_SHA512,
        other => {
            return Err(format!(
                "unknown HMAC {other:?} (use sha1, sha256, sha384 or sha512)"
            ));
        }
    };
    let tag = hmac::sign(&hmac::Key::new(algorithm, key), data);
    encode(tag.as_ref(), encoding)
}

fn base64_native(mode: &str, text: &str) -> Result<String, String> {
    let compact = || {
        text.chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect::<String>()
    };
    match mode {
        "encode" => Ok(STANDARD.encode(text.as_bytes())),
        "decode" => FORGIVING
            .decode(compact())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|e| format!("invalid base64: {e}")),
        "btoa" => {
            let mut bytes = Vec::with_capacity(text.len());
            for ch in text.chars() {
                let code = u32::from(ch);
                if code > 0xFF {
                    return Err(
                        "InvalidCharacterError: btoa only accepts Latin-1 characters (use runpost.crypto.base64 for UTF-8)"
                            .into(),
                    );
                }
                bytes.push(code as u8);
            }
            Ok(STANDARD.encode(bytes))
        }
        "atob" => FORGIVING
            .decode(compact())
            .map(|bytes| bytes.iter().map(|&b| char::from(b)).collect())
            .map_err(|_| "InvalidCharacterError: the string is not valid base64".into()),
        other => Err(format!("unknown base64 mode {other:?}")),
    }
}

#[derive(Deserialize)]
struct SendOptions {
    url: String,
    method: String,
    #[serde(default)]
    header: Vec<Pair>,
    #[serde(default)]
    body: Option<SendBody>,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
enum SendBody {
    Raw { raw: String },
    Urlencoded { urlencoded: Vec<Pair> },
}

/// `pm.sendRequest`: sin resolver `{{variables}}` (Postman tampoco lo hace). Devuelve JSON con
/// `response` o `error`.
fn send_native(host: &ScriptHost, deadline: Instant, options: &str) -> String {
    let outcome = (|| -> Result<Json, String> {
        let options: SendOptions = serde_json::from_str(options)
            .map_err(|e| format!("invalid pm.sendRequest options: {e}"))?;
        let mut url_text = options.url.trim().to_owned();
        if !url_text.contains("://") {
            url_text.insert_str(0, "http://");
        }
        let url = url::Url::parse(&url_text).map_err(|e| format!("invalid url {url_text}: {e}"))?;
        let method = reqwest::Method::from_bytes(options.method.trim().to_uppercase().as_bytes())
            .map_err(|_| format!("invalid method {:?}", options.method))?;
        let mut headers: Headers = options
            .header
            .into_iter()
            .filter(|h| !h.key.trim().is_empty())
            .map(|h| (h.key, h.value))
            .collect();
        let has = |headers: &Headers, name: &str| {
            headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
        };
        let body = match options.body {
            None => PreparedBody::None,
            Some(SendBody::Raw { raw }) => PreparedBody::Bytes(raw.into_bytes()),
            Some(SendBody::Urlencoded { urlencoded }) => {
                let mut form = url::form_urlencoded::Serializer::new(String::new());
                for pair in &urlencoded {
                    form.append_pair(&pair.key, &pair.value);
                }
                if !has(&headers, "content-type") {
                    headers.push((
                        "Content-Type".into(),
                        "application/x-www-form-urlencoded".into(),
                    ));
                }
                PreparedBody::Bytes(form.finish().into_bytes())
            }
        };
        if !has(&headers, "user-agent") && !host.user_agent.is_empty() {
            headers.push(("User-Agent".into(), host.user_agent.clone()));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("script timed out after {} s", TIMEOUT.as_secs()));
        }
        let prepared = Prepared {
            method,
            url,
            headers,
            body,
            timeout: host.timeout.min(remaining),
            // `engine::send` usa el cliente que se le pasa; la clave no interviene.
            client: ClientKey {
                verify_tls: true,
                follow_redirects: true,
                max_redirects: 10,
                proxy: None,
            },
        };
        let response = host
            .runtime
            .block_on(engine::send(&host.client, &prepared))
            .map_err(|e| {
                let (kind, message) = engine::describe_error(&e);
                format!("{kind}: {message}")
            })?;
        Ok(json!({"response": ScriptResponse::from(&response)}))
    })();
    match outcome {
        Ok(value) => value.to_string(),
        Err(message) => json!({"error": message}).to_string(),
    }
}

#[cfg(test)]
mod tests;
