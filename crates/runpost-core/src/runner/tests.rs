use std::path::PathBuf;

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, header_exists, method, path, path_regex},
};

use super::*;
use crate::model::{Request as Req, Scripts};

const VIEW: &str = "fake-view-token";
const SPEND: &str = "fake-spend-token";
const SESSION: &str = "/v1/session";
const SIGN_PATH: &str = "/v1/usd/accounts/acc_0123456789abcdef/sign";

fn workspace() -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    (dir, ws)
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn acme_workspace() -> (tempfile::TempDir, Workspace) {
    let (dir, ws) = workspace();
    ws.import_postman_file(
        &fixtures().join("Acme-Gateway-Scopes.postman_collection.json"),
        None,
    )
    .unwrap();
    ws.import_postman_file(
        &fixtures().join("Acme-DESA-fake.postman_environment.json"),
        None,
    )
    .unwrap();
    (dir, ws)
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Acme Gateway y la API de Acme simuladas: la respuesta depende del token, como en la realidad
/// (un token de solo lectura no puede firmar; uno con permiso de firma llega al error local de
/// Gateway porque la sonda no lleva clave privada).
async fn acme_mock() -> MockServer {
    let server = MockServer::start().await;
    let session = |scopes: &[&str]| {
        ResponseTemplate::new(200).set_body_json(json!({
            "session": {"scope": scopes, "expires": "2026-12-31T00:00:00.000Z", "ipRestrict": []}
        }))
    };
    for (token, scopes) in [
        (VIEW, &["accounts_read"][..]),
        (SPEND, &["accounts_read", "accounts_sign"][..]),
    ] {
        Mock::given(method("GET"))
            .and(path(SESSION))
            .and(header("authorization", bearer(token).as_str()))
            .respond_with(session(scopes))
            .with_priority(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/v1/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "gateway is ok!"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/ed25519/keys/local"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"pub": "pub-fake", "prv": "prv-fake"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/encrypt"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"encrypted": "{\"iv\":\"fake\"}"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(SIGN_PATH))
        .and(header("authorization", bearer(SPEND).as_str()))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({"error": "key must be an object", "name": "Error"})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(SIGN_PATH))
        .and(header("authorization", bearer(VIEW).as_str()))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": "unauthorized", "name": "ApiError"})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(SIGN_PATH))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "unauthorized"})))
        .with_priority(5)
        .mount(&server)
        .await;
    server
}

fn options(server: &MockServer) -> RunnerOptions {
    RunnerOptions {
        env: Some("Acme DESA (fake)".into()),
        overrides: [("gatewayUrl", server.uri()), ("apiUrl", server.uri())]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        source: Source::Cli,
        ..RunnerOptions::default()
    }
}

/// Lo que da Postman con los scripts de la colección y estas respuestas (`(iteración, petición,
/// estado o `None` si se omite, tests)`). Fijado a mano una vez, a partir de los propios scripts.
type Row = (
    u32,
    &'static str,
    Option<u16>,
    &'static [(&'static str, bool)],
);
/// Lo mismo, con dueño, para comparar con el informe.
type Outcome = (u32, String, Option<u16>, Vec<(String, bool)>);

const D00: &str = "00 - Diagnóstico/";
const D01: &str = "01 - Matriz de scopes (token bajo prueba)/";
const D02: &str = "02 - Controles negativos (sin token / token inválido)/";

const EXPECTED: &[Row] = &[
    (
        0,
        "00.1 API - ¿qué scopes tiene el token?",
        Some(200),
        &[
            ("[solo-view] el token es válido en Acme (HTTP 200)", true),
            ("[solo-view] scopes reales del token: accounts_read", true),
        ],
    ),
    (
        0,
        "00.2 Gateway - ping",
        Some(200),
        &[("Gateway responde (ping)", true)],
    ),
    (
        0,
        "#5 POST keys/local",
        Some(200),
        &[("#5 keys/local con [solo-view] → FUNCIONA", true)],
    ),
    (
        0,
        "#6 POST encrypt",
        Some(200),
        &[("#6 encrypt con [solo-view] → FUNCIONA", true)],
    ),
    (
        0,
        "#7 POST sign (sonda de autorización)",
        Some(401),
        &[(
            "#7 sign con [solo-view] → RECHAZADO por Acme (401: unauthorized)",
            false,
        )],
    ),
    (0, "#7b POST sign (firma real, opcional)", None, &[]),
    (
        0,
        "#5 keys/local SIN token",
        Some(200),
        &[("#5 SIN token → 200 (confirma que no requiere scope)", true)],
    ),
    (
        0,
        "#6 encrypt SIN token",
        Some(200),
        &[("#6 SIN token → 200 (confirma que no requiere scope)", true)],
    ),
    (
        0,
        "#7 sign SIN token",
        Some(401),
        &[("#7 SIN token → 401 (confirma que sí requiere token)", true)],
    ),
    (
        0,
        "#7 sign token INVÁLIDO",
        Some(401),
        &[("#7 token INVÁLIDO → 401", true)],
    ),
    (
        1,
        "00.1 API - ¿qué scopes tiene el token?",
        Some(200),
        &[
            ("[solo-spend] el token es válido en Acme (HTTP 200)", true),
            (
                "[solo-spend] scopes reales del token: accounts_read, accounts_sign",
                true,
            ),
        ],
    ),
    (1, "00.2 Gateway - ping", None, &[]),
    (
        1,
        "#5 POST keys/local",
        Some(200),
        &[("#5 keys/local con [solo-spend] → FUNCIONA", true)],
    ),
    (
        1,
        "#6 POST encrypt",
        Some(200),
        &[("#6 encrypt con [solo-spend] → FUNCIONA", true)],
    ),
    (
        1,
        "#7 POST sign (sonda de autorización)",
        Some(500),
        &[(
            "#7 sign con [solo-spend] → AUTORIZADO (Acme dejó leer la cuenta)",
            true,
        )],
    ),
    (1, "#7b POST sign (firma real, opcional)", None, &[]),
    (1, "#5 keys/local SIN token", None, &[]),
    (1, "#6 encrypt SIN token", None, &[]),
    (1, "#7 sign SIN token", None, &[]),
    (1, "#7 sign token INVÁLIDO", None, &[]),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acme_collection_runs_like_postman_with_a_data_file() -> anyhow::Result<()> {
    let server = acme_mock().await;
    let (_dir, ws) = acme_workspace();
    let mut opts = options(&server);
    // Como tokens-matriz.csv: una fila por token, cada uno con su etiqueta.
    opts.data = data::parse(
        "tokens-matriz.csv",
        &format!("scopeLabel,accessToken\nsolo-view,{VIEW}\nsolo-spend,{SPEND}\n"),
    )?;
    opts.data_name = Some("tokens-matriz.csv".into());
    opts.record_history = true;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let report = Executor::new()
        .run_collection(&ws, "Acme Gateway Scopes", opts, Some(tx))
        .await?;

    let got: Vec<Outcome> = report
        .items
        .iter()
        .map(|item| {
            (
                item.iteration,
                item.name.clone(),
                item.status,
                item.tests
                    .iter()
                    .map(|t| (t.name.clone(), t.passed))
                    .collect(),
            )
        })
        .collect();
    let want: Vec<Outcome> = EXPECTED
        .iter()
        .map(|(iteration, name, status, tests)| {
            (
                *iteration,
                (*name).to_owned(),
                *status,
                tests.iter().map(|(n, p)| ((*n).to_owned(), *p)).collect(),
            )
        })
        .collect();
    assert_eq!(got, want);
    for item in &report.items {
        assert_eq!(item.skipped, item.status.is_none(), "{}", item.name);
        assert!(item.error.is_none(), "{}: {:?}", item.name, item.error);
    }
    assert_eq!(
        report.items[0].path,
        format!("{D00}00.1 API - ¿qué scopes tiene el token?")
    );
    assert_eq!(
        report.items[4].path,
        format!("{D01}#7 POST sign (sonda de autorización)")
    );
    assert_eq!(report.items[9].path, format!("{D02}#7 sign token INVÁLIDO"));
    assert_eq!(
        report.items[4].tests[0].msg.as_deref(),
        Some("AssertionError: RECHAZADO por Acme (401: unauthorized): expected false to be true")
    );
    assert_eq!(
        report.items[5].console,
        ["7b omitida: faltan variables draftJson, userEncryptedKey, accountPassphrase"]
    );
    assert_eq!(
        report.items[0].console,
        [
            "[solo-view] scopes: [ 'accounts_read' ] | expira: 2026-12-31T00:00:00.000Z | ipRestrict: []"
        ]
    );
    assert_eq!(
        report.totals,
        RunnerTotals {
            requests: 20,
            skipped: 7,
            errors: 0,
            tests: 15,
            passed: 14,
            failed: 1
        }
    );
    assert!(report.failed());
    assert_eq!(report.iterations, 2);
    assert_eq!(report.environment.as_deref(), Some("Acme DESA (fake)"));
    assert_eq!(
        report.data,
        Some(DataInfo {
            name: Some("tokens-matriz.csv".into()),
            rows: 2
        })
    );

    // Progreso: un `started` y un evento por petición.
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert!(matches!(
        events[0],
        RunnerEvent::Started {
            total: 20,
            iterations: 2
        }
    ));
    assert_eq!(events.len(), 21);
    assert!(matches!(
        &events[20],
        RunnerEvent::Item {
            done: 20,
            total: 20,
            ..
        }
    ));

    // Los tokens del archivo de datos son secretos en el entorno: el historial los enmascara.
    for summary in ws.history().list(100, 0, None)? {
        let entry = ws.history().get(&summary.id)?.unwrap();
        let text = serde_json::to_string(&entry.req_headers)?;
        assert!(!text.contains(VIEW) && !text.contains(SPEND), "{text}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folder_fail_fast_guard_and_cancel() -> anyhow::Result<()> {
    let server = acme_mock().await;
    let (_dir, ws) = acme_workspace();

    // Solo una carpeta, por prefijo de su nombre.
    let mut opts = options(&server);
    opts.folder = Some("01".into());
    let report = Executor::new()
        .run_collection(&ws, "acme-gateway-scopes", opts, None)
        .await?;
    assert_eq!(
        report.folder.as_ref().map(|f| f.name.as_str()),
        Some("01 - Matriz de scopes (token bajo prueba)")
    );
    assert_eq!(report.items.len(), 4);
    assert!(report.items.iter().all(|i| i.path.starts_with(D01)));

    // Fail-fast: con el token del entorno (falso) 00.1 recibe 404 del mock y su primer test falla.
    let mut opts = options(&server);
    opts.fail_fast = true;
    let report = Executor::new()
        .run_collection(&ws, "Acme Gateway Scopes", opts, None)
        .await?;
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].status, Some(404));
    assert!(report.failed());

    // Guardia (política del agente): bloquea los POST, que quedan como error sin enviarse.
    let mut opts = options(&server);
    opts.folder = Some("02 - Controles negativos (sin token / token inválido)".into());
    opts.guard = Some(Arc::new(|_, request| {
        if request.method == "POST" {
            Err("policy: allow_methods".to_owned())
        } else {
            Ok(())
        }
    }));
    let report = Executor::new()
        .run_collection(&ws, "Acme Gateway Scopes", opts, None)
        .await?;
    assert_eq!(report.totals.errors, 4);
    assert_eq!(
        report.items[0].error.as_deref(),
        Some("policy: allow_methods")
    );

    // Cancelada antes de empezar: informe vacío y marcado.
    let opts = options(&server);
    opts.cancel.cancel();
    let report = Executor::new()
        .run_collection(&ws, "Acme Gateway Scopes", opts, None)
        .await?;
    assert!(report.cancelled);
    assert!(report.items.is_empty());

    // Errores de selección.
    let missing = Executor::new()
        .run_collection(
            &ws,
            "Acme Gateway Scopes",
            RunnerOptions {
                folder: Some("zzz".into()),
                ..options(&server)
            },
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        missing.downcast_ref::<CoreError>().unwrap().code(),
        "folder_not_found"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locals_carry_over_requests_and_delay_and_cancel_mid_run() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("^/n/"))
        .and(header_exists("x-seen"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let (_dir, ws) = workspace();
    let mut collection = Collection::new("Local");
    let mut first = Req::new("first", "GET", format!("{}/first", server.uri()));
    first.scripts = Scripts {
        pre: String::new(),
        test: "pm.variables.set('seen', 'yes');".into(),
    };
    let mut second = Req::new("second", "GET", format!("{}/n/second", server.uri()));
    second
        .headers
        .push(crate::model::KeyValue::new("X-Seen", "{{seen}}"));
    second.scripts.test =
        "pm.test('local from first', () => pm.expect(pm.variables.get('seen')).to.equal('yes'));"
            .into();
    let third = Req::new("third", "GET", format!("{}/third", server.uri()));
    collection.items = vec![
        Item::Request(Box::new(first)),
        Item::Request(Box::new(second)),
        Item::Request(Box::new(third)),
    ];
    ws.save_collection(&collection)?;

    let report = Executor::new()
        .run_collection(
            &ws,
            "Local",
            RunnerOptions {
                delay_ms: 30,
                ..RunnerOptions::default()
            },
            None,
        )
        .await?;
    assert_eq!(report.totals.passed, 1);
    assert!(report.ms >= 60, "{} ms", report.ms);

    // Cancelar durante el retardo deja lo hecho y marca el informe.
    let cancel = CancellationToken::new();
    let opts = RunnerOptions {
        delay_ms: 2000,
        cancel: cancel.clone(),
        ..RunnerOptions::default()
    };
    let stopper = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let started = Instant::now();
    let report = Executor::new()
        .run_collection(&ws, "Local", opts, None)
        .await?;
    stopper.await?;
    assert!(report.cancelled);
    assert_eq!(report.items.len(), 1);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn report_formats_junit_pretty_and_html() -> anyhow::Result<()> {
    let server = acme_mock().await;
    let (_dir, ws) = acme_workspace();
    let mut opts = options(&server);
    opts.data = data::parse(
        "d.json",
        &format!(r#"[{{"scopeLabel":"solo-view","accessToken":"{VIEW}"}}]"#),
    )?;
    let report = Executor::new()
        .run_collection(&ws, "Acme Gateway Scopes", opts, None)
        .await?;

    let junit = report::to_junit(&report);
    assert!(junit.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"Acme Gateway - Validación de scopes\" tests=\"11\" failures=\"1\" errors=\"0\""), "{junit}");
    assert_eq!(junit.matches("<testsuite ").count(), 10);
    assert_eq!(junit.matches("</testsuite>").count(), 10);
    assert_eq!(junit.matches("<failure ").count(), 1);
    assert!(junit.contains("<skipped message=\"pm.execution.skipRequest()\"/>"));
    assert!(junit.contains("message=\"AssertionError: RECHAZADO por Acme (401: unauthorized): expected false to be true\""));
    assert!(
        junit.contains("name=\"#7 sign con [solo-view] → RECHAZADO por Acme (401: unauthorized)\"")
    );
    assert!(!junit.contains(VIEW));

    let pretty = report::to_pretty(&report);
    assert!(
        pretty.starts_with(
            "Acme Gateway - Validación de scopes · entorno Acme DESA (fake) · 1 iteración\n"
        ),
        "{pretty}"
    );
    assert!(pretty.contains("  ✘ POST  01 - Matriz de scopes (token bajo prueba)/#7 POST sign (sonda de autorización)  401 · "));
    assert!(pretty.contains("          AssertionError: RECHAZADO por Acme (401: unauthorized): expected false to be true\n"));
    assert!(pretty.contains("  ↷ POST  01 - Matriz de scopes (token bajo prueba)/#7b POST sign (firma real, opcional)  omitida\n"));
    assert!(pretty.trim_end().ends_with(" ms"), "{pretty}");
    assert!(
        pretty.contains("10 peticiones (1 omitida) · tests: 9 ✔ 1 ✘ · "),
        "{pretty}"
    );

    let html = report::to_html(&report);
    assert!(html.contains("<title>Runner · Acme Gateway - Validación de scopes</title>"));
    assert_eq!(html.matches("<tr class=").count(), 10);
    assert!(!html.contains("<script"));
    Ok(())
}
