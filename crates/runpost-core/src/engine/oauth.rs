//! OAuth 2.0 (fase 4): client credentials y authorization code con PKCE y callback local
//! (RFC 6749, RFC 7636 y RFC 8252).
//!
//! Con client credentials el token se pide solo al enviar. Con authorization code, `start` prepara
//! el inicio de sesión: escucha el callback en el puerto de `redirect_uri` (solo 127.0.0.1),
//! devuelve la URL de autorización para abrirla en el navegador y canjea el código en segundo
//! plano. Al caducar, el token se renueva con el refresh token si lo hay. Los tokens viven en
//! memoria; los de authorization code también en el llavero del sistema, para no repetir el inicio
//! de sesión al reiniciar. Nunca se escriben en el workspace, el historial ni la salida MCP: quien
//! los usa los registra como secretos (`Resolver::with_secret`).

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use anyhow::Result;
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

use crate::{
    error::CoreError,
    model::{Auth, ClientAuth, OAUTH_REDIRECT_URI},
    secrets::Keyring,
    vars::Resolver,
};

/// Lo que se espera a que el usuario inicie sesión en el navegador.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// Un token que vence en menos de esto ya se renueva.
const EXPIRY_SKEW_MS: i64 = 30_000;
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

fn oauth_error(message: impl Into<String>) -> anyhow::Error {
    CoreError::OAuth(message.into()).into()
}

/// El motivo de un error, sin repetir el prefijo "OAuth 2.0:".
fn reason(error: &anyhow::Error) -> String {
    match error.downcast_ref::<CoreError>() {
        Some(CoreError::OAuth(message)) => message.clone(),
        _ => format!("{error:#}"),
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn random_bytes(len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes)
        .map_err(|error| oauth_error(format!("no secure random numbers: {error}")))?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    ClientCredentials,
    AuthorizationCode,
}

/// Configuración ya resuelta (sin `{{variables}}`).
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub grant: Grant,
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub scope: String,
    pub redirect_uri: String,
    pub client_auth: ClientAuth,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("grant", &self.grant)
            .field("token_url", &self.token_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &"***")
            .finish_non_exhaustive()
    }
}

impl Config {
    /// `None` si `auth` no es OAuth 2.0. También devuelve las variables que quedaron sin valor.
    pub fn resolve(auth: &Auth, resolver: &Resolver) -> Option<(Self, Vec<String>)> {
        let mut missing: Vec<String> = Vec::new();
        let mut r = |text: &str| {
            let resolved = resolver.resolve(text.trim());
            for name in resolved.unresolved {
                if !missing.contains(&name) {
                    missing.push(name);
                }
            }
            resolved.text
        };
        let config = match auth {
            Auth::OAuth2Cc(cc) => Config {
                grant: Grant::ClientCredentials,
                auth_url: String::new(),
                token_url: r(&cc.token_url),
                client_id: r(&cc.client_id),
                client_secret: r(&cc.client_secret),
                scope: r(&cc.scope),
                redirect_uri: String::new(),
                client_auth: cc.client_auth,
            },
            Auth::OAuth2Code(code) => {
                let redirect = r(&code.redirect_uri);
                Config {
                    grant: Grant::AuthorizationCode,
                    auth_url: r(&code.auth_url),
                    token_url: r(&code.token_url),
                    client_id: r(&code.client_id),
                    client_secret: r(&code.client_secret),
                    scope: r(&code.scope),
                    redirect_uri: if redirect.is_empty() {
                        OAUTH_REDIRECT_URI.to_owned()
                    } else {
                        redirect
                    },
                    client_auth: code.client_auth,
                }
            }
            _ => return None,
        };
        Some((config, missing))
    }

    /// Clave de la caché y de la cuenta del llavero (`oauth2/<clave>`). Lleva el secreto del
    /// cliente (con hash): si cambia, se pide otro token.
    pub fn key(&self) -> String {
        use aws_lc_rs::digest;
        let grant = match self.grant {
            Grant::ClientCredentials => "client_credentials",
            Grant::AuthorizationCode => "authorization_code",
        };
        let client_auth = match self.client_auth {
            ClientAuth::Basic => "basic",
            ClientAuth::Body => "body",
        };
        let seed = serde_json::json!([
            grant,
            self.auth_url,
            self.token_url,
            self.client_id,
            self.client_secret,
            self.scope,
            self.redirect_uri,
            client_auth,
        ])
        .to_string();
        hex(&digest::digest(&digest::SHA256, seed.as_bytes()).as_ref()[..16])
    }

    fn check(&self) -> Result<()> {
        let http_url = |text: &str, what: &str| -> Result<()> {
            if text.is_empty() {
                return Err(oauth_error(format!("the {what} is empty")));
            }
            match url::Url::parse(text) {
                Ok(url) if matches!(url.scheme(), "http" | "https") => Ok(()),
                _ => Err(oauth_error(format!("the {what} is not an http(s) URL"))),
            }
        };
        http_url(&self.token_url, "access token URL")?;
        if self.grant == Grant::AuthorizationCode {
            http_url(&self.auth_url, "authorization URL")?;
        }
        if self.client_id.is_empty() {
            return Err(oauth_error("the client ID is empty"));
        }
        Ok(())
    }
}

/// Token de acceso. `Debug` no lo muestra.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: String,
    /// Milisegundos Unix; `None` = el servidor no dijo cuándo caduca.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("token_type", &self.token_type)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Lo que se puede enseñar de un token (nunca el token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenInfo {
    pub token_type: String,
    pub expires_at: Option<i64>,
    pub scope: String,
    /// Hay refresh token: se renovará solo.
    pub refresh: bool,
}

impl Token {
    fn expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|at| at - EXPIRY_SKEW_MS <= now)
    }

    pub fn info(&self) -> TokenInfo {
        TokenInfo {
            token_type: self.token_type.clone(),
            expires_at: self.expires_at,
            scope: self.scope.clone(),
            refresh: self.refresh_token.is_some(),
        }
    }
}

fn form_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// Petición al endpoint de tokens (RFC 6749 4.1.3, 4.4.2 y 6). El cliente va en Basic (con
/// `client_id` y `client_secret` codificados como pide 2.3.1) o en el cuerpo; un cliente público
/// (sin secreto) siempre manda `client_id` en el cuerpo.
async fn token_request(
    config: &Config,
    client: &reqwest::Client,
    mut form: Vec<(&'static str, String)>,
) -> Result<Token> {
    config.check()?;
    let mut request = client
        .post(&config.token_url)
        .timeout(TOKEN_TIMEOUT)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded");
    let public = config.client_secret.is_empty();
    if config.client_auth == ClientAuth::Basic && !public {
        let credentials = format!(
            "{}:{}",
            form_encode(&config.client_id),
            form_encode(&config.client_secret)
        );
        request = request.header(
            "Authorization",
            format!("Basic {}", STANDARD.encode(credentials)),
        );
    } else {
        form.push(("client_id", config.client_id.clone()));
        if !public {
            form.push(("client_secret", config.client_secret.clone()));
        }
    }
    let body = {
        let mut body = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in &form {
            body.append_pair(key, value);
        }
        body.finish()
    };
    let response = request.body(body).send().await.map_err(|error| {
        let (kind, message) = super::describe_error(&error);
        oauth_error(format!("token request failed ({kind}): {message}"))
    })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    // JSON (RFC 6749 5.1) o form-urlencoded (GitHub sin `Accept`).
    let value: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        Value::Object(
            url::form_urlencoded::parse(text.as_bytes())
                .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                .collect(),
        )
    });
    let field = |key: &str| {
        value
            .get(key)
            .and_then(|v| match v {
                Value::String(text) => Some(text.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
    };
    let Some(access_token) = field("access_token").filter(|_| status.is_success()) else {
        let mut message = format!("the token endpoint answered HTTP {}", status.as_u16());
        if let Some(error) = field("error") {
            message.push_str(&format!(": {error}"));
        }
        if let Some(description) = field("error_description") {
            let short: String = description.chars().take(200).collect();
            message.push_str(&format!(" ({short})"));
        }
        return Err(oauth_error(message));
    };
    Ok(Token {
        access_token,
        refresh_token: field("refresh_token"),
        token_type: field("token_type").unwrap_or_else(|| "Bearer".into()),
        expires_at: field("expires_in")
            .and_then(|secs| secs.parse::<i64>().ok())
            .map(|secs| now_ms() + secs.saturating_mul(1000)),
        scope: field("scope").unwrap_or_default(),
    })
}

async fn client_credentials(config: &Config, client: &reqwest::Client) -> Result<Token> {
    let mut form = vec![("grant_type", "client_credentials".to_owned())];
    if !config.scope.is_empty() {
        form.push(("scope", config.scope.clone()));
    }
    token_request(config, client, form).await
}

/// PKCE (RFC 7636): `(code_verifier, code_challenge)` con S256.
fn pkce() -> Result<(String, String)> {
    use aws_lc_rs::digest;
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes(32)?);
    let challenge =
        URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref());
    Ok((verifier, challenge))
}

fn authorization_url(config: &Config, state: &str, challenge: &str) -> Result<String> {
    let mut url = url::Url::parse(&config.auth_url)
        .map_err(|error| oauth_error(format!("invalid authorization URL: {error}")))?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &config.redirect_uri);
        if !config.scope.is_empty() {
            query.append_pair("scope", &config.scope);
        }
        query
            .append_pair("state", state)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
    }
    Ok(url.into())
}

/// Dirección y ruta del callback. Solo HTTP en loopback (RFC 8252 7.3): nada sale del equipo.
fn callback_address(redirect_uri: &str) -> Result<(SocketAddr, String)> {
    let url = url::Url::parse(redirect_uri)
        .map_err(|error| oauth_error(format!("invalid redirect URI: {error}")))?;
    let ip = match url.host() {
        Some(url::Host::Domain(domain)) if domain.eq_ignore_ascii_case("localhost") => {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        }
        Some(url::Host::Ipv4(ip)) if ip.is_loopback() => IpAddr::V4(ip),
        Some(url::Host::Ipv6(ip)) if ip.is_loopback() => IpAddr::V6(ip),
        _ => {
            return Err(oauth_error(
                "the redirect URI must point to this computer, e.g. http://127.0.0.1:7880/callback",
            ));
        }
    };
    if url.scheme() != "http" {
        return Err(oauth_error(
            "the redirect URI must use http:// (RunPost serves the callback on this computer)",
        ));
    }
    let port = url.port().ok_or_else(|| {
        oauth_error("the redirect URI needs a port, e.g. http://127.0.0.1:7880/callback")
    })?;
    Ok((SocketAddr::new(ip, port), url.path().to_owned()))
}

/// Destino (`/ruta?query`) de una petición GET; `None` si no lo es o no llega a tiempo.
async fn read_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let read = async {
        loop {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..n]);
            if buffer.windows(4).any(|w| w == b"\r\n\r\n") || buffer.len() > 8192 {
                break;
            }
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(10), read)
        .await
        .ok()??;
    let head = String::from_utf8_lossy(&buffer);
    let mut parts = head.lines().next()?.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    parts.next().map(str::to_owned)
}

const PAGE_OK: &str = "Signed in to RunPost. You can close this tab.<br>Sesión iniciada en RunPost. Puedes cerrar esta pestaña.";
const PAGE_DENIED: &str = "The sign-in was not completed. Go back to RunPost.<br>No se completó el inicio de sesión. Vuelve a RunPost.";
const PAGE_BAD: &str = "This link is not valid for the current sign-in.<br>Este enlace no vale para el inicio de sesión en curso.";

async fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>RunPost</title></head><body style=\"font-family:system-ui,sans-serif;margin:3em;line-height:1.6\"><h1>RunPost</h1><p>{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Espera el callback con el `state` de este inicio de sesión y devuelve su `code`. Lo que no
/// trae ese `state` (otra pestaña, otro programa) se contesta y se ignora.
async fn wait_for_code(
    listener: TcpListener,
    path: &str,
    state: &str,
    timeout: Duration,
) -> Result<String> {
    let wait = async {
        loop {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|error| oauth_error(format!("callback listener failed: {error}")))?;
            let Some(target) = read_target(&mut stream).await else {
                continue;
            };
            let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
                respond(&mut stream, 400, PAGE_BAD).await;
                continue;
            };
            if url.path() != path {
                respond(&mut stream, 404, PAGE_BAD).await;
                continue;
            }
            let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
            if query.get("state").map(String::as_str) != Some(state) {
                respond(&mut stream, 400, PAGE_BAD).await;
                continue;
            }
            if let Some(error) = query.get("error") {
                respond(&mut stream, 400, PAGE_DENIED).await;
                let description = query
                    .get("error_description")
                    .map(|d| format!(" ({})", d.chars().take(200).collect::<String>()))
                    .unwrap_or_default();
                return Err(oauth_error(format!(
                    "the authorization server answered {error}{description}"
                )));
            }
            match query.get("code").filter(|code| !code.is_empty()) {
                Some(code) => {
                    respond(&mut stream, 200, PAGE_OK).await;
                    return Ok(code.clone());
                }
                None => respond(&mut stream, 400, PAGE_BAD).await,
            }
        }
    };
    tokio::time::timeout(timeout, wait).await.map_err(|_| {
        oauth_error(format!(
            "nobody signed in within {} minutes",
            timeout.as_secs() / 60
        ))
    })?
}

/// Inicio de sesión en curso.
struct Flow {
    port: u16,
    task: tokio::task::JoinHandle<()>,
    done: watch::Receiver<Option<Result<TokenInfo, String>>>,
}

/// Resultado de "Get new access token".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Started {
    /// Client credentials: el token ya está.
    Token { token: TokenInfo },
    /// Authorization code: abre `authorization_url` y espera `flow` (`OAuth::wait`).
    SignIn {
        flow: String,
        authorization_url: String,
    },
}

/// Tokens y sesiones en curso. Uno por proceso (lo tiene el `Executor`).
#[derive(Default)]
pub struct OAuth {
    tokens: Mutex<HashMap<String, Token>>,
    flows: Mutex<HashMap<String, Flow>>,
}

fn account(config: &Config) -> String {
    format!("oauth2/{}", config.key())
}

impl OAuth {
    /// Token guardado (memoria y, en authorization code, llavero), vigente o no.
    pub fn stored(&self, config: &Config, keyring: &dyn Keyring) -> Option<Token> {
        let key = config.key();
        if let Some(token) = lock(&self.tokens).get(&key) {
            return Some(token.clone());
        }
        if config.grant != Grant::AuthorizationCode {
            return None;
        }
        let text = match keyring.get(&account(config)) {
            Ok(text) => text?,
            Err(error) => {
                tracing::warn!("keyring: OAuth 2.0 token not read: {error}");
                return None;
            }
        };
        let token: Token = serde_json::from_str(&text).ok()?;
        lock(&self.tokens).insert(key, token.clone());
        Some(token)
    }

    fn save(&self, config: &Config, token: &Token, keyring: &dyn Keyring) {
        lock(&self.tokens).insert(config.key(), token.clone());
        if config.grant == Grant::AuthorizationCode
            && let Ok(text) = serde_json::to_string(token)
            && let Err(error) = keyring.set(&account(config), &text)
        {
            tracing::warn!("keyring: OAuth 2.0 token not saved: {error}");
        }
    }

    /// Olvida el token (memoria y llavero).
    pub fn clear(&self, config: &Config, keyring: &dyn Keyring) {
        lock(&self.tokens).remove(&config.key());
        if config.grant == Grant::AuthorizationCode
            && let Err(error) = keyring.delete(&account(config))
        {
            tracing::warn!("keyring: OAuth 2.0 token not deleted: {error}");
        }
    }

    pub fn info(&self, config: &Config, keyring: &dyn Keyring) -> Option<TokenInfo> {
        self.stored(config, keyring).map(|token| token.info())
    }

    /// Token vigente para enviar: el guardado; si caducó, renovado con el refresh token; en client
    /// credentials, uno nuevo. Authorization code sin token pide iniciar sesión.
    pub async fn access_token(
        &self,
        config: &Config,
        client: &reqwest::Client,
        keyring: &dyn Keyring,
    ) -> Result<Token> {
        config.check()?;
        if let Some(token) = self.stored(config, keyring) {
            if !token.expired(now_ms()) {
                return Ok(token);
            }
            if let Some(refresh) = token.refresh_token.clone() {
                let form = vec![
                    ("grant_type", "refresh_token".to_owned()),
                    ("refresh_token", refresh.clone()),
                ];
                match token_request(config, client, form).await {
                    Ok(mut fresh) => {
                        // El servidor puede no repetir el refresh token: se conserva.
                        if fresh.refresh_token.is_none() {
                            fresh.refresh_token = Some(refresh);
                        }
                        self.save(config, &fresh, keyring);
                        return Ok(fresh);
                    }
                    Err(error) if config.grant == Grant::AuthorizationCode => {
                        self.clear(config, keyring);
                        return Err(oauth_error(format!(
                            "the access token expired and could not be renewed ({}): sign in again with \"Get new access token\"",
                            reason(&error)
                        )));
                    }
                    Err(_) => {}
                }
            }
        }
        match config.grant {
            Grant::ClientCredentials => {
                let token = client_credentials(config, client).await?;
                self.save(config, &token, keyring);
                Ok(token)
            }
            Grant::AuthorizationCode => Err(oauth_error(
                "no access token yet: sign in with \"Get new access token\" in the Auth tab",
            )),
        }
    }

    /// "Get new access token". Client credentials pide uno ya. Authorization code escucha el
    /// callback y devuelve la URL de autorización (quien llama la abre en el navegador); el código
    /// se canjea en segundo plano y `wait` da el resultado. Un inicio de sesión anterior en el mismo
    /// puerto se cancela.
    pub async fn start(
        self: &Arc<Self>,
        config: Config,
        client: reqwest::Client,
        keyring: Arc<dyn Keyring>,
    ) -> Result<Started> {
        config.check()?;
        if config.grant == Grant::ClientCredentials {
            let token = client_credentials(&config, &client).await?;
            self.save(&config, &token, keyring.as_ref());
            return Ok(Started::Token {
                token: token.info(),
            });
        }
        let (address, path) = callback_address(&config.redirect_uri)?;
        self.cancel_port(address.port()).await;
        let listener = TcpListener::bind(address).await.map_err(|error| {
            oauth_error(format!(
                "cannot listen on {address} for the callback ({error}): close the program that uses that port or change the redirect URI"
            ))
        })?;
        let state = hex(&random_bytes(16)?);
        let (verifier, challenge) = pkce()?;
        let authorization_url = authorization_url(&config, &state, &challenge)?;
        let flow = hex(&random_bytes(8)?);
        let (sender, done) = watch::channel(None);
        let oauth = Arc::clone(self);
        let task = tokio::spawn(async move {
            let outcome = async {
                let code = wait_for_code(listener, &path, &state, SIGN_IN_TIMEOUT).await?;
                let form = vec![
                    ("grant_type", "authorization_code".to_owned()),
                    ("code", code),
                    ("redirect_uri", config.redirect_uri.clone()),
                    ("code_verifier", verifier),
                ];
                let token = token_request(&config, &client, form).await?;
                oauth.save(&config, &token, keyring.as_ref());
                Ok::<_, anyhow::Error>(token.info())
            }
            .await;
            let _ = sender.send(Some(outcome.map_err(|error| reason(&error))));
        });
        lock(&self.flows).insert(
            flow.clone(),
            Flow {
                port: address.port(),
                task,
                done,
            },
        );
        Ok(Started::SignIn {
            flow,
            authorization_url,
        })
    }

    /// Espera el final de un inicio de sesión de `start`.
    pub async fn wait(&self, flow: &str) -> Result<TokenInfo> {
        let mut done = lock(&self.flows)
            .get(flow)
            .map(|f| f.done.clone())
            .ok_or_else(|| CoreError::not_found("oauth2_flow", flow))?;
        let outcome = done
            .wait_for(Option::is_some)
            .await
            .map(|value| value.clone())
            .map_err(|_| oauth_error("the sign-in was cancelled"))?;
        lock(&self.flows).remove(flow);
        match outcome {
            Some(Ok(info)) => Ok(info),
            Some(Err(message)) => Err(oauth_error(message)),
            None => Err(oauth_error("the sign-in was cancelled")),
        }
    }

    /// Cancela lo que escucha en `port` y espera a que suelte el puerto.
    async fn cancel_port(&self, port: u16) {
        let stale: Vec<tokio::task::JoinHandle<()>> = {
            let mut flows = lock(&self.flows);
            let ids: Vec<String> = flows
                .iter()
                .filter(|(_, flow)| flow.port == port)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter()
                .filter_map(|id| flows.remove(id))
                .map(|flow| flow.task)
                .collect()
        };
        for task in stale {
            task.abort();
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod tests;
