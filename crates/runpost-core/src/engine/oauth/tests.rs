use std::sync::Arc;

use wiremock::{
    Mock, MockServer, Request as MockRequest, ResponseTemplate,
    matchers::{body_string_contains, header, method, path},
};

use super::*;
use crate::{
    model::{OAuth2Cc, OAuth2Code, Variable},
    secrets::MemoryKeyring,
    vars::{Resolver, Scope},
};

fn form(request: &MockRequest) -> HashMap<String, String> {
    url::form_urlencoded::parse(&request.body)
        .into_owned()
        .collect()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn cc(server: &MockServer, client_auth: ClientAuth) -> Config {
    Config {
        grant: Grant::ClientCredentials,
        auth_url: String::new(),
        token_url: format!("{}/oauth/token", server.uri()),
        client_id: "acme-client".into(),
        client_secret: "fake-secret".into(),
        scope: "accounts_read accounts_sign".into(),
        redirect_uri: String::new(),
        client_auth,
    }
}

fn code_config(server: &MockServer, port: u16) -> Config {
    Config {
        grant: Grant::AuthorizationCode,
        auth_url: format!("{}/oauth/authorize?audience=acme", server.uri()),
        token_url: format!("{}/oauth/token", server.uri()),
        client_id: "acme-app".into(),
        client_secret: String::new(),
        scope: "openid accounts_read".into(),
        redirect_uri: format!("http://127.0.0.1:{port}/callback"),
        client_auth: ClientAuth::Basic,
    }
}

#[test]
fn resolves_variables_checks_and_keys() {
    let resolver = Resolver::new().with_variables(
        Scope::Environment,
        &[
            Variable::new("idp", "https://id.example.test"),
            Variable::secret("clientSecret", "fake-secret"),
        ],
    );
    let auth = Auth::oauth2_cc(OAuth2Cc {
        token_url: "{{idp}}/oauth/token".into(),
        client_id: "acme".into(),
        client_secret: "{{clientSecret}}".into(),
        scope: "{{scope}}".into(),
        client_auth: ClientAuth::Body,
    });
    let (config, missing) = Config::resolve(&auth, &resolver).unwrap();
    assert_eq!(config.token_url, "https://id.example.test/oauth/token");
    assert_eq!(config.client_secret, "fake-secret");
    assert_eq!(missing, ["scope"]);
    assert!(!format!("{config:?}").contains("fake-secret"));
    assert!(Config::resolve(&Auth::None, &resolver).is_none());

    let key = config.key();
    assert_eq!(key.len(), 32);
    let other = Config {
        client_secret: "rotated".into(),
        ..config.clone()
    };
    assert_ne!(key, other.key());
    assert!(config.check().is_ok());
    for (broken, message) in [
        (
            Config {
                token_url: String::new(),
                ..config.clone()
            },
            "access token URL is empty",
        ),
        (
            Config {
                token_url: "ftp://x".into(),
                ..config.clone()
            },
            "not an http(s) URL",
        ),
        (
            Config {
                client_id: String::new(),
                ..config.clone()
            },
            "client ID is empty",
        ),
    ] {
        let error = broken.check().unwrap_err().to_string();
        assert!(error.contains(message), "{error}");
    }

    let code = Auth::oauth2_code(OAuth2Code {
        auth_url: "{{idp}}/authorize".into(),
        token_url: "{{idp}}/token".into(),
        client_id: "app".into(),
        client_secret: String::new(),
        scope: String::new(),
        redirect_uri: String::new(),
        client_auth: ClientAuth::Basic,
    });
    let (config, _) = Config::resolve(&code, &resolver).unwrap();
    assert_eq!(config.redirect_uri, OAUTH_REDIRECT_URI);
    assert_eq!(config.grant, Grant::AuthorizationCode);
}

#[test]
fn the_callback_only_listens_on_this_computer() {
    let (address, path) = callback_address("http://127.0.0.1:7880/callback").unwrap();
    assert_eq!(address.to_string(), "127.0.0.1:7880");
    assert_eq!(path, "/callback");
    assert_eq!(
        callback_address("http://localhost:9000/cb")
            .unwrap()
            .0
            .to_string(),
        "127.0.0.1:9000"
    );
    assert_eq!(
        callback_address("http://[::1]:9000/")
            .unwrap()
            .0
            .to_string(),
        "[::1]:9000"
    );
    for (uri, message) in [
        ("https://127.0.0.1:7880/callback", "must use http://"),
        (
            "http://example.test:7880/callback",
            "point to this computer",
        ),
        (
            "http://192.168.1.10:7880/callback",
            "point to this computer",
        ),
        ("http://127.0.0.1/callback", "needs a port"),
        ("not a url", "invalid redirect URI"),
    ] {
        let error = callback_address(uri).unwrap_err().to_string();
        assert!(error.contains(message), "{uri}: {error}");
    }
}

#[tokio::test]
async fn client_credentials_are_cached_and_renewed() {
    let server = MockServer::start().await;
    let basic = format!("Basic {}", STANDARD.encode("acme-client:fake-secret"));
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(header("authorization", basic.as_str()))
        .and(body_string_contains("grant_type=client_credentials"))
        .and(body_string_contains("scope=accounts_read+accounts_sign"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fake-access-1", "token_type": "Bearer", "expires_in": 3600,
            "scope": "accounts_read accounts_sign"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let oauth = OAuth::default();
    let keyring = MemoryKeyring::new();
    let client = reqwest::Client::new();
    let config = cc(&server, ClientAuth::Basic);
    let token = oauth
        .access_token(&config, &client, &keyring)
        .await
        .unwrap();
    assert_eq!(token.access_token, "fake-access-1");
    assert!(token.expires_at.unwrap() > now_ms());
    assert!(!format!("{token:?}").contains("fake-access-1"));
    // Segunda vez: de memoria (el mock espera una sola llamada).
    let again = oauth
        .access_token(&config, &client, &keyring)
        .await
        .unwrap();
    assert_eq!(again, token);
    let info = oauth.info(&config, &keyring).unwrap();
    assert_eq!(info.scope, "accounts_read accounts_sign");
    assert!(!info.refresh);
    // Client credentials no se guardan en el llavero.
    assert_eq!(keyring.get(&account(&config)).unwrap(), None);
    server.verify().await;

    // Caducado (sin refresh token): se pide otro.
    lock(&oauth.tokens)
        .get_mut(&config.key())
        .unwrap()
        .expires_at = Some(now_ms() + 1_000);
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fake-access-2", "token_type": "bearer"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let fresh = oauth
        .access_token(&config, &client, &keyring)
        .await
        .unwrap();
    assert_eq!(fresh.access_token, "fake-access-2");
    assert_eq!(fresh.expires_at, None);
    oauth.clear(&config, &keyring);
    assert!(oauth.info(&config, &keyring).is_none());
}

#[tokio::test]
async fn credentials_in_the_body_and_clear_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(|request: &MockRequest| {
            let fields = form(request);
            let ok = !request.headers.contains_key("authorization")
                && fields.get("client_id").map(String::as_str) == Some("acme-client")
                && fields.get("client_secret").map(String::as_str) == Some("fake-secret");
            if ok {
                ResponseTemplate::new(200)
                    .set_body_string("access_token=fake-form&token_type=bearer&expires_in=60")
            } else {
                ResponseTemplate::new(401).set_body_json(serde_json::json!({
                    "error": "invalid_client", "error_description": "Client authentication failed"
                }))
            }
        })
        .mount(&server)
        .await;
    let oauth = OAuth::default();
    let keyring = MemoryKeyring::new();
    let client = reqwest::Client::new();
    let body = cc(&server, ClientAuth::Body);
    // Respuesta form-urlencoded (como GitHub) también vale.
    let token = oauth.access_token(&body, &client, &keyring).await.unwrap();
    assert_eq!(token.access_token, "fake-form");

    let basic = cc(&server, ClientAuth::Basic);
    let error = oauth
        .access_token(&basic, &client, &keyring)
        .await
        .unwrap_err();
    let message = error.to_string();
    assert_eq!(
        message,
        "OAuth 2.0: the token endpoint answered HTTP 401: invalid_client (Client authentication failed)"
    );
    assert!(!message.contains("fake-secret"));
    assert_eq!(
        error
            .downcast_ref::<CoreError>()
            .map(CoreError::code)
            .as_deref(),
        Some("oauth2")
    );
}

/// Comprueba en el canje que `S256(code_verifier) == code_challenge` de la URL de autorización.
struct Pkce(Arc<Mutex<String>>);

impl wiremock::Match for Pkce {
    fn matches(&self, request: &MockRequest) -> bool {
        use aws_lc_rs::digest;
        let fields = form(request);
        let Some(verifier) = fields.get("code_verifier") else {
            return false;
        };
        let challenge =
            URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()).as_ref());
        challenge == *lock(&self.0)
            && fields.get("grant_type").map(String::as_str) == Some("authorization_code")
            && fields.get("code").map(String::as_str) == Some("fake-code")
            && fields.get("client_id").map(String::as_str) == Some("acme-app")
    }
}

async fn browser(url: &str) -> (u16, String) {
    let response = reqwest::get(url).await.unwrap();
    let status = response.status().as_u16();
    (status, response.text().await.unwrap())
}

#[tokio::test]
async fn authorization_code_with_pkce_and_a_local_callback() {
    let server = MockServer::start().await;
    let challenge = Arc::new(Mutex::new(String::new()));
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(Pkce(challenge.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fake-user-token", "refresh_token": "fake-refresh",
            "token_type": "Bearer", "expires_in": 3600
        })))
        .expect(1)
        .mount(&server)
        .await;

    let oauth = Arc::new(OAuth::default());
    let keyring: Arc<dyn Keyring> = Arc::new(MemoryKeyring::new());
    let client = reqwest::Client::new();
    let port = free_port();
    let config = code_config(&server, port);
    let error = oauth
        .access_token(&config, &client, keyring.as_ref())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Get new access token"),
        "{error}"
    );

    let Started::SignIn {
        flow,
        authorization_url,
    } = oauth
        .start(config.clone(), client.clone(), keyring.clone())
        .await
        .unwrap()
    else {
        panic!("expected a sign-in")
    };
    let url = url::Url::parse(&authorization_url).unwrap();
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(url.path(), "/oauth/authorize");
    assert_eq!(query["audience"], "acme");
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["client_id"], "acme-app");
    assert_eq!(query["redirect_uri"], config.redirect_uri);
    assert_eq!(query["scope"], "openid accounts_read");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["state"].len(), 32);
    *lock(&challenge) = query["code_challenge"].clone();

    // El "navegador": otra ruta, un `state` ajeno y por fin el bueno.
    let base = format!("http://127.0.0.1:{port}");
    assert_eq!(browser(&format!("{base}/favicon.ico")).await.0, 404);
    let forged = browser(&format!("{base}/callback?code=evil&state=nope")).await;
    assert_eq!(forged.0, 400);
    let (status, page) = browser(&format!(
        "{base}/callback?code=fake-code&state={}",
        query["state"]
    ))
    .await;
    assert_eq!(status, 200);
    assert!(page.contains("Signed in to RunPost"));

    let info = oauth.wait(&flow).await.unwrap();
    assert!(info.refresh);
    server.verify().await;
    let token = oauth
        .access_token(&config, &client, keyring.as_ref())
        .await
        .unwrap();
    assert_eq!(token.access_token, "fake-user-token");

    // Otro proceso (reinicio): el token sale del llavero.
    let restarted = OAuth::default();
    let stored = restarted.stored(&config, keyring.as_ref()).unwrap();
    assert_eq!(stored.refresh_token.as_deref(), Some("fake-refresh"));

    // Caducado: se renueva con el refresh token y se conserva el refresh si no llega otro.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=fake-refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fake-user-token-2", "expires_in": 3600
        })))
        .expect(1)
        .mount(&server)
        .await;
    lock(&restarted.tokens)
        .get_mut(&config.key())
        .unwrap()
        .expires_at = Some(0);
    let renewed = restarted
        .access_token(&config, &client, keyring.as_ref())
        .await
        .unwrap();
    assert_eq!(renewed.access_token, "fake-user-token-2");
    assert_eq!(renewed.refresh_token.as_deref(), Some("fake-refresh"));
    assert!(
        keyring
            .get(&account(&config))
            .unwrap()
            .unwrap()
            .contains("fake-user-token-2")
    );

    // El refresh token ya no vale: se olvida todo y se pide iniciar sesión.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(serde_json::json!({"error": "invalid_grant"})),
        )
        .mount(&server)
        .await;
    lock(&restarted.tokens)
        .get_mut(&config.key())
        .unwrap()
        .expires_at = Some(0);
    let error = restarted
        .access_token(&config, &client, keyring.as_ref())
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "OAuth 2.0: the access token expired and could not be renewed (the token endpoint answered HTTP 400: invalid_grant): sign in again with \"Get new access token\""
    );
    assert_eq!(keyring.get(&account(&config)).unwrap(), None);
}

#[tokio::test]
async fn denied_cancelled_and_busy_sign_ins() {
    let server = MockServer::start().await;
    let oauth = Arc::new(OAuth::default());
    let keyring: Arc<dyn Keyring> = Arc::new(MemoryKeyring::new());
    let client = reqwest::Client::new();
    let port = free_port();
    let config = code_config(&server, port);
    let state = |started: &Started| match started {
        Started::SignIn {
            authorization_url, ..
        } => url::Url::parse(authorization_url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .unwrap(),
        Started::Token { .. } => panic!("expected a sign-in"),
    };
    let flow_of = |started: &Started| match started {
        Started::SignIn { flow, .. } => flow.clone(),
        Started::Token { .. } => panic!("expected a sign-in"),
    };

    // El usuario dice que no.
    let first = oauth
        .start(config.clone(), client.clone(), keyring.clone())
        .await
        .unwrap();
    let (status, _) = browser(&format!(
        "http://127.0.0.1:{port}/callback?error=access_denied&error_description=User%20said%20no&state={}",
        state(&first)
    ))
    .await;
    assert_eq!(status, 400);
    let error = oauth.wait(&flow_of(&first)).await.unwrap_err().to_string();
    assert_eq!(
        error,
        "OAuth 2.0: the authorization server answered access_denied (User said no)"
    );

    // Un segundo inicio de sesión en el mismo puerto cancela el primero.
    let second = oauth
        .start(config.clone(), client.clone(), keyring.clone())
        .await
        .unwrap();
    let third = oauth
        .start(config.clone(), client.clone(), keyring.clone())
        .await
        .unwrap();
    let error = oauth.wait(&flow_of(&second)).await.unwrap_err().to_string();
    assert!(
        error.contains("not found") || error.contains("cancelled"),
        "{error}"
    );
    assert_ne!(state(&second), state(&third));

    // Puerto ocupado por otro programa.
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = Config {
        redirect_uri: format!(
            "http://127.0.0.1:{}/callback",
            busy.local_addr().unwrap().port()
        ),
        ..config.clone()
    };
    let error = oauth
        .start(taken, client.clone(), keyring.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot listen on"), "{error}");
    assert!(oauth.wait("nope").await.is_err());
}
