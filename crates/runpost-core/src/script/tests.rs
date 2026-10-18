use super::*;

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

fn tokio() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

fn host(rt: &tokio::runtime::Runtime) -> ScriptHost {
    ScriptHost {
        runtime: rt.handle().clone(),
        client: reqwest::Client::new(),
        timeout: Duration::from_secs(5),
        user_agent: "runpost-test".into(),
    }
}

fn response(code: u16, status: &str, body: &str) -> Json {
    json!({
        "code": code, "status": status,
        "headers": [{"key": "Content-Type", "value": "application/json"}, {"key": "X-Id", "value": "7"}],
        "body": body, "responseTime": 12, "size": body.len(),
    })
}

fn input(phase: &str, response: Option<Json>) -> Json {
    json!({
        "phase": phase,
        "info": {"requestName": "ping", "requestId": "req_1", "iteration": 0, "iterationCount": 1},
        "vars": {
            "environment": {"name": "DESA", "values": [{"key": "scopeLabel", "value": "fake-scope"}, {"key": "shared", "value": "env"}]},
            "globals": [{"key": "shared", "value": "global"}, {"key": "g", "value": "G"}],
            "collection": [{"key": "shared", "value": "collection"}, {"key": "c", "value": "C"}],
            "iteration": {"row": "1"},
            "local": {},
        },
        "request": {
            "url": "{{gatewayUrl}}/v1/ping?x=1&y",
            "method": "POST",
            "headers": [{"key": "Content-Type", "value": "application/json"}, {"key": "X-Off", "value": "1", "disabled": true}],
            "body": {"mode": "raw", "raw": "{\"a\":1}"},
        },
        "response": response,
    })
}

fn test_phase(code: &str, res: Json) -> ScriptOutput {
    let rt = tokio();
    run_script(
        "test script of request \"ping\"",
        code,
        &input("test", Some(res)),
        &host(&rt),
    )
}

fn pre_phase(code: &str) -> ScriptOutput {
    let rt = tokio();
    run_script(
        "pre-request script of request \"ping\"",
        code,
        &input("prerequest", None),
        &host(&rt),
    )
}

fn msgs(out: &ScriptOutput) -> Vec<(String, bool, String)> {
    out.tests
        .iter()
        .map(|t| (t.name.clone(), t.passed, t.msg.clone().unwrap_or_default()))
        .collect()
}

#[test]
fn pm_test_reports_pass_and_fail_with_postman_messages() {
    let out = test_phase(
        r#"
        pm.test('status 200', () => { pm.response.to.have.status(200); });
        pm.test('status 401', function () { pm.response.to.have.status(401); });
        pm.test('reason', () => pm.response.to.have.status('OK'));
        pm.test('2xx', () => pm.response.to.be.ok);
        pm.test('not 2xx', () => pm.response.to.not.be.ok);
        pm.test.skip('later', () => {});
        "#,
        response(401, "Unauthorized", "{}"),
    );
    assert_eq!(
        msgs(&out),
        vec![
            ("status 200".into(), false, "AssertionError: expected response to have status code 200 but got 401".into()),
            ("status 401".into(), true, String::new()),
            ("reason".into(), false, "AssertionError: expected response to have status reason 'OK' but got 'Unauthorized'".into()),
            ("2xx".into(), false, "AssertionError: expected response code to be 2XX but found 401".into()),
            ("not 2xx".into(), true, String::new()),
            ("later".into(), true, String::new()),
        ]
    );
    assert!(out.tests[5].skipped);
}

#[test]
fn expect_supports_the_chai_subset_with_chai_messages() {
    let out = test_phase(
        r#"
        const t = (name, fn) => pm.test(name, fn);
        t('above', () => pm.expect(0, 'el token no reporta scopes').to.be.above(0));
        t('property', () => pm.expect({}).to.have.property('pub'));
        t('a string', () => pm.expect(undefined, 'falta el campo encrypted').to.be.a('string'));
        t('true', () => pm.expect(false, 'INCONCLUSO (500: x)').to.be.true);
        t('equal', () => pm.expect('a').to.equal('b'));
        t('eql', () => pm.expect({ a: [1, 2] }).to.eql({ a: [1, 2] }));
        t('deep equal', () => pm.expect({ a: 1 }).to.deep.equal({ a: 2 }));
        t('include', () => pm.expect([1, 2, 3]).to.include(2));
        t('include string', () => pm.expect('key must be an object').to.include('key'));
        t('match', () => pm.expect('abc').to.match(/^b/));
        t('length', () => pm.expect([1, 2]).to.have.length(3));
        t('lengthOf above', () => pm.expect('abc').to.have.lengthOf.above(2));
        t('not', () => pm.expect(1).to.not.equal(1));
        t('an array', () => pm.expect([]).to.be.an('array').that.is.empty);
        t('null', () => pm.expect(null).to.be.null);
        t('undefined', () => pm.expect(1).to.be.undefined);
        t('property value', () => pm.expect({ a: 1 }).to.have.property('a', 2));
        t('missing value', () => pm.expect({ a: 1 }).to.have.property('b', 2));
        t('oneOf', () => pm.expect(3).to.be.oneOf([1, 2]));
        t('keys', () => pm.expect({ a: 1, b: 2 }).to.have.all.keys('a', 'b'));
        t('long object', () => pm.expect({ alpha: 'aaaaaaaaaaaa', beta: 'bbbbbbbbbbbb', gamma: 1 }).to.equal(1));
        t('below', () => pm.expect(5).to.be.below(3));
        t('exist', () => pm.expect(undefined).to.exist);
        t('fail', () => pm.expect.fail('boom'));
        "#,
        response(200, "OK", "{}"),
    );
    let got: Vec<(String, bool, String)> = msgs(&out);
    let expect = [
        (
            "above",
            false,
            "AssertionError: el token no reporta scopes: expected 0 to be above 0",
        ),
        (
            "property",
            false,
            "AssertionError: expected {} to have property 'pub'",
        ),
        (
            "a string",
            false,
            "AssertionError: falta el campo encrypted: expected undefined to be a string",
        ),
        (
            "true",
            false,
            "AssertionError: INCONCLUSO (500: x): expected false to be true",
        ),
        ("equal", false, "AssertionError: expected 'a' to equal 'b'"),
        ("eql", true, ""),
        (
            "deep equal",
            false,
            "AssertionError: expected { a: 1 } to deeply equal { a: 2 }",
        ),
        ("include", true, ""),
        ("include string", true, ""),
        (
            "match",
            false,
            "AssertionError: expected 'abc' to match /^b/",
        ),
        (
            "length",
            false,
            "AssertionError: expected [ 1, 2 ] to have a length of 3 but got 2",
        ),
        ("lengthOf above", true, ""),
        ("not", false, "AssertionError: expected 1 to not equal 1"),
        ("an array", true, ""),
        ("null", true, ""),
        (
            "undefined",
            false,
            "AssertionError: expected 1 to be undefined",
        ),
        (
            "property value",
            false,
            "AssertionError: expected { a: 1 } to have property 'a' of 2, but got 1",
        ),
        (
            "missing value",
            false,
            "AssertionError: expected { a: 1 } to have property 'b' of 2, but got undefined",
        ),
        (
            "oneOf",
            false,
            "AssertionError: expected 3 to be one of [ 1, 2 ]",
        ),
        ("keys", true, ""),
        (
            "long object",
            false,
            "AssertionError: expected { Object (alpha, beta, ...) } to equal 1",
        ),
        ("below", false, "AssertionError: expected 5 to be below 3"),
        (
            "exist",
            false,
            "AssertionError: expected undefined to exist",
        ),
        ("fail", false, "AssertionError: boom"),
    ];
    assert_eq!(got.len(), expect.len(), "{got:#?}");
    for ((name, passed, msg), (want_name, want_passed, want_msg)) in got.iter().zip(expect) {
        assert_eq!(
            (name.as_str(), *passed, msg.as_str()),
            (want_name, want_passed, want_msg)
        );
    }
}

/// El script real de "#7 POST sign (sonda de autorización)" de la colección Acme.
const SIGN_PATH: &str = r#"
const label = pm.variables.get('scopeLabel') || '(sin etiqueta)';
const code = pm.response.code;
let b = {}; try { b = pm.response.json(); } catch (e) {}
const msg = (b.message || b.error || '').toString();
const rechazadoPorLaApi = b.name === 'ApiError' || code === 401 || code === 403;
const falloLocalDeFirma = /key must be an object|private key|prv|passphrase/i.test(msg);
let veredicto;
if (code === 200 && (b.signature || b.signed)) veredicto = 'AUTORIZADO (firmó)';
else if (rechazadoPorLaApi) veredicto = `RECHAZADO por Acme (${code}: ${msg})`;
else if (falloLocalDeFirma) veredicto = 'AUTORIZADO (Acme dejó leer la cuenta)';
else veredicto = `INCONCLUSO (${code}: ${msg})`;
console.log(`#7 sign [${label}] -> ${veredicto}`);
pm.test(`#7 sign con [${label}] → ${veredicto}`, () => {
    pm.expect(veredicto.startsWith('AUTORIZADO'), veredicto).to.be.true;
});
"#;

#[test]
fn acme_sign_script_gives_the_postman_verdicts() {
    let authorised = test_phase(
        SIGN_PATH,
        response(
            500,
            "Internal Server Error",
            r#"{"error":"key must be an object","name":"Error"}"#,
        ),
    );
    assert_eq!(
        msgs(&authorised),
        vec![(
            "#7 sign con [fake-scope] → AUTORIZADO (Acme dejó leer la cuenta)".into(),
            true,
            String::new()
        )]
    );
    assert_eq!(
        authorised.console,
        vec!["#7 sign [fake-scope] -> AUTORIZADO (Acme dejó leer la cuenta)"]
    );

    let rejected = test_phase(
        SIGN_PATH,
        response(
            401,
            "Unauthorized",
            r#"{"error":"unauthorized","name":"ApiError"}"#,
        ),
    );
    assert_eq!(
        msgs(&rejected),
        vec![(
            "#7 sign con [fake-scope] → RECHAZADO por Acme (401: unauthorized)".into(),
            false,
            "AssertionError: RECHAZADO por Acme (401: unauthorized): expected false to be true"
                .into()
        )]
    );

    // Cuerpo que no es JSON: el `try` del script lo absorbe.
    let html = test_phase(
        SIGN_PATH,
        response(502, "Bad Gateway", "<html>bad gateway</html>"),
    );
    assert_eq!(
        msgs(&html)[0].2,
        "AssertionError: INCONCLUSO (502: ): expected false to be true"
    );
}

#[test]
fn variables_follow_postman_scopes_and_record_changes() {
    let out = test_phase(
        r#"
        pm.test('order', () => {
            pm.expect(pm.variables.get('shared')).to.equal('env');
            pm.expect(pm.variables.get('row')).to.equal('1');
            pm.expect(pm.variables.get('c')).to.equal('C');
            pm.expect(pm.variables.get('g')).to.equal('G');
            pm.expect(pm.variables.get('nope')).to.be.undefined;
            pm.variables.set('shared', 'local');
            pm.expect(pm.variables.get('shared')).to.equal('local');
        });
        pm.environment.set('token', 42);
        pm.expect(pm.environment.get('token')).to.equal(42);
        pm.environment.set('obj', { a: 1 });
        pm.environment.unset('shared');
        pm.environment.unset('missing');
        pm.globals.set('g', 'G2');
        pm.collectionVariables.set('c', 'C2');
        pm.test('replaceIn', () => {
            pm.expect(pm.variables.replaceIn('{{c}}-{{g}}-{{scopeLabel}}-{{nope}}')).to.equal('C2-G2-fake-scope-{{nope}}');
            pm.expect(pm.variables.replaceIn('{{$guid}}')).to.match(/^[0-9a-f-]{36}$/);
            pm.expect(pm.environment.name).to.equal('DESA');
            pm.expect(pm.iterationData.get('row')).to.equal('1');
            pm.expect(pm.info.requestName).to.equal('ping');
            pm.expect(pm.info.iteration).to.equal(0);
        });
        "#,
        response(200, "OK", "{}"),
    );
    assert!(out.tests.iter().all(|t| t.passed), "{:#?}", out.tests);
    let ops: Vec<(VarScope, OpKind, &str, Option<&str>)> = out
        .changes
        .iter()
        .map(|o| (o.scope, o.op, o.key.as_str(), o.value.as_deref()))
        .collect();
    assert_eq!(
        ops,
        vec![
            (VarScope::Local, OpKind::Set, "shared", Some("local")),
            (VarScope::Environment, OpKind::Set, "token", Some("42")),
            (
                VarScope::Environment,
                OpKind::Set,
                "obj",
                Some(r#"{"a":1}"#)
            ),
            (VarScope::Environment, OpKind::Unset, "shared", None),
            (VarScope::Globals, OpKind::Set, "g", Some("G2")),
            (VarScope::Collection, OpKind::Set, "c", Some("C2")),
        ]
    );

    let mut state = ScriptState {
        environment: Some(Environment {
            schema: 1,
            id: "env_1".into(),
            name: "DESA".into(),
            variables: vec![
                Variable::secret("shared", "s3cr3t"),
                Variable::new("other", "x"),
            ],
        }),
        ..ScriptState::default()
    };
    for op in out.changes {
        state.apply(op);
    }
    let env = state.environment.unwrap();
    let keys: Vec<_> = env.variables.iter().map(|v| v.key.as_str()).collect();
    assert_eq!(keys, ["other", "token", "obj"]);
    assert_eq!(state.local.get("shared").map(String::as_str), Some("local"));
    assert_eq!(state.globals[0].value.as_deref(), Some("G2"));
}

#[test]
fn skip_request_only_works_in_pre_request() {
    let skip = "if (pm.info.iteration > 0) { pm.execution.skipRequest(); }";
    assert!(!pre_phase(skip).skip);
    let rt = tokio();
    let mut later = input("prerequest", None);
    later["info"]["iteration"] = json!(1);
    assert!(run_script("pre", skip, &later, &host(&rt)).skip);
    let in_test = test_phase("pm.execution.skipRequest();", response(200, "OK", "{}"));
    assert!(!in_test.skip);
}

#[test]
fn pre_request_can_change_headers_and_read_the_request() {
    let out = pre_phase(
        r#"
        pm.test('request', () => {
            pm.expect(pm.request.url.toString()).to.equal('{{gatewayUrl}}/v1/ping?x=1&y');
            pm.expect(pm.request.url.getPath()).to.equal('/v1/ping');
            pm.expect(pm.request.url.query.get('x')).to.equal('1');
            pm.expect(pm.request.method).to.equal('POST');
            pm.expect(pm.request.headers.get('content-type')).to.equal('application/json');
            pm.expect(pm.request.headers.has('X-Off')).to.be.false;
            pm.expect(pm.request.body.raw).to.equal('{"a":1}');
            pm.expect(`${pm.request.url}`).to.include('/v1/ping');
        });
        pm.request.headers.add({ key: 'X-Trace', value: 'abc' });
        pm.request.headers.upsert({ key: 'content-type', value: 'text/plain' });
        pm.request.headers.remove('X-Off');
        pm.request.headers.add('X-Raw: 1');
        "#,
    );
    assert!(out.tests.iter().all(|t| t.passed), "{:#?}", out.tests);
    let headers: Vec<(String, String, bool)> = out
        .request_headers
        .unwrap()
        .into_iter()
        .map(|h| (h.key, h.value, h.disabled))
        .collect();
    assert_eq!(
        headers,
        vec![
            ("Content-Type".into(), "text/plain".into(), false),
            ("X-Trace".into(), "abc".into(), false),
            ("X-Raw".into(), "1".into(), false),
        ]
    );
}

#[test]
fn script_errors_become_failed_tests_and_never_escape() {
    let cases = [
        (
            "pm.test('ok', () => {}); undefinedFunction();",
            "ReferenceError: undefinedFunction is not defined",
        ),
        ("let = ;", "SyntaxError"),
        ("throw 'plain';", "Error: plain"),
        (
            "require('lodash');",
            "Error: require('lodash') is not supported in RunPost scripts",
        ),
        (
            "CryptoJS.SHA256('x');",
            "Error: CryptoJS is not supported in RunPost scripts: use runpost.crypto.sha256 / hmac / base64",
        ),
        (
            "postman.setEnvironmentVariable('a', 1);",
            "Error: postman.* (legacy API) is not supported in RunPost scripts: use pm.*",
        ),
        ("pm.response.json();", "JSONError"),
        (
            "setTimeout(() => { throw new TypeError('late'); }, 1);",
            "TypeError: late",
        ),
        (
            "await Promise.reject(new RangeError('async'));",
            "RangeError: async",
        ),
    ];
    for (code, want) in cases {
        let out = test_phase(code, response(200, "OK", "not json"));
        let error = out
            .tests
            .iter()
            .find(|t| t.name == SCRIPT_ERROR)
            .unwrap_or_else(|| panic!("{code}: {:#?}", out.tests));
        let msg = error.msg.as_deref().unwrap();
        assert!(!error.passed);
        assert!(
            msg.starts_with("test script of request \"ping\": "),
            "{msg}"
        );
        assert!(msg.contains(want), "{code} -> {msg}");
    }
    // Lo que pasó antes del error se conserva.
    let out = test_phase(
        "pm.test('ok', () => {}); nope();",
        response(200, "OK", "{}"),
    );
    assert_eq!(out.tests[0].name, "ok");
    assert!(out.tests[0].passed);
}

#[test]
fn infinite_loops_stop_after_five_seconds() {
    let started = Instant::now();
    let out = test_phase(
        "pm.test('before', () => {}); while (true) {}",
        response(200, "OK", "{}"),
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= TIMEOUT && elapsed < TIMEOUT + Duration::from_secs(3),
        "{elapsed:?}"
    );
    assert_eq!(out.tests[0].name, "before");
    assert_eq!(
        out.tests.last().unwrap().msg.as_deref(),
        Some("test script of request \"ping\": script timed out after 5 s")
    );
}

#[test]
fn memory_is_limited_to_32_mb() {
    let out = test_phase(
        "const chunks = []; while (true) { chunks.push('x'.repeat(1024 * 1024) + chunks.length); }",
        response(200, "OK", "{}"),
    );
    let msg = out.tests.last().unwrap().msg.clone().unwrap_or_default();
    assert!(
        msg.contains("out of memory") || msg.contains("cannot read the script result"),
        "{msg}"
    );
}

#[test]
fn console_lines_are_captured_with_levels() {
    let out = test_phase(
        r#"
        console.log('[fake-scope] scopes:', [], '| expira:', undefined, { a: 1, b: 'x' }, null, 3);
        console.warn('careful');
        console.error(new Error('bad'));
        console.info('x'.repeat(3000));
        "#,
        response(200, "OK", "{}"),
    );
    assert_eq!(
        out.console[0],
        "[fake-scope] scopes: [] | expira: undefined { a: 1, b: 'x' } null 3"
    );
    assert_eq!(out.console[1], "[warn] careful");
    assert_eq!(out.console[2], "[error] Error: bad");
    assert_eq!(out.console[3].chars().count(), "[info] ".len() + 2001);
}

#[test]
fn timers_promises_and_async_tests_run_before_finishing() {
    let out = test_phase(
        r#"
        const order = [];
        setTimeout(() => order.push('timeout 20'), 20);
        setTimeout(() => order.push('timeout 0'), 0);
        Promise.resolve().then(() => order.push('microtask'));
        const id = setTimeout(() => order.push('cancelled'), 5);
        clearTimeout(id);
        let ticks = 0;
        const iv = setInterval(() => { ticks += 1; if (ticks === 3) clearInterval(iv); }, 1);
        order.push('sync');
        await new Promise((resolve) => setTimeout(resolve, 30));
        pm.test('order', () => {
            pm.expect(order).to.eql(['sync', 'microtask', 'timeout 0', 'timeout 20']);
            pm.expect(ticks).to.equal(3);
        });
        pm.test('async fails later', async () => { await null; throw new Error('async boom'); });
        "#,
        response(200, "OK", "{}"),
    );
    assert_eq!(msgs(&out)[0], ("order".into(), true, String::new()));
    assert_eq!(
        msgs(&out)[1],
        (
            "async fails later".into(),
            false,
            "Error: async boom".into()
        )
    );
}

#[test]
fn crypto_and_base64_helpers() {
    let out = test_phase(
        r#"
        const c = runpost.crypto;
        pm.test('sha256', () => pm.expect(c.sha256('abc')).to.equal('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'));
        pm.test('hmac', () => pm.expect(c.hmac('key', 'The quick brown fox jumps over the lazy dog')).to.equal('f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8'));
        pm.test('hmac base64', () => pm.expect(c.hmac('key', 'x', 'sha1', 'base64')).to.equal('KgDo6DQ9+GxMGIsmPuyjMa+jFUg='));
        pm.test('base64', () => { pm.expect(c.base64('ñ')).to.equal('w7E='); pm.expect(c.base64.decode('w7E')).to.equal('ñ'); });
        pm.test('btoa', () => { pm.expect(btoa('ñ')).to.equal('8Q=='); pm.expect(atob('8Q==')).to.equal('ñ'); pm.expect(btoa('user:pass')).to.equal('dXNlcjpwYXNz'); });
        pm.test('btoa unicode', () => btoa('€'));
        pm.test('bad hash', () => c.sha256('x', 'binary'));
        "#,
        response(200, "OK", "{}"),
    );
    let got = msgs(&out);
    assert!(got[..5].iter().all(|(_, passed, _)| *passed), "{got:#?}");
    assert!(
        got[5].2.starts_with("TypeError: InvalidCharacterError"),
        "{}",
        got[5].2
    );
    assert_eq!(
        got[6].2,
        "TypeError: unknown encoding \"binary\" (use \"hex\" or \"base64\")"
    );
}

#[test]
fn response_assertions_cover_headers_json_and_body() {
    let out = test_phase(
        r#"
        pm.test('header', () => pm.response.to.have.header('content-type'));
        pm.test('header value', () => pm.response.to.have.header('X-Id', '8'));
        pm.test('json', () => pm.response.to.be.json);
        pm.test('jsonBody path', () => pm.response.to.have.jsonBody('data.items[1].id', 2));
        pm.test('jsonBody missing', () => pm.response.to.have.jsonBody('data.nope'));
        pm.test('headers api', () => {
            pm.expect(pm.response.headers.get('x-id')).to.equal('7');
            pm.expect(pm.response.headers.toObject()).to.have.property('X-Id');
            pm.expect(pm.response.responseTime).to.be.below(100);
            pm.expect(pm.response.json().data.items).to.have.length(2);
        });
        pm.test('error class', () => pm.response.to.be.error);
        "#,
        response(200, "OK", r#"{"data":{"items":[{"id":1},{"id":2}]}}"#),
    );
    assert_eq!(
        msgs(&out),
        vec![
            ("header".into(), true, String::new()),
            (
                "header value".into(),
                false,
                "AssertionError: expected 'X-Id' response header to be '8' but got '7'".into()
            ),
            ("json".into(), true, String::new()),
            ("jsonBody path".into(), true, String::new()),
            (
                "jsonBody missing".into(),
                false,
                "AssertionError: expected response body json to have path 'data.nope'".into()
            ),
            ("headers api".into(), true, String::new()),
            (
                "error class".into(),
                false,
                "AssertionError: expected response code to be 4XX or 5XX but found 200".into()
            ),
        ]
    );
}

#[test]
fn send_request_uses_callbacks_promises_and_a_limit() {
    let rt = tokio();
    let server = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/scopes"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"scope": ["accounts_read"]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/echo"))
            .and(header("x-k", "v"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .respond_with(ResponseTemplate::new(201).set_body_string("created"))
            .mount(&server)
            .await;
        server
    });
    let code = format!(
        r#"
        const base = '{base}';
        const order = [];
        pm.sendRequest(base + '/scopes', (err, res) => {{
            order.push('callback');
            pm.test('callback', () => {{
                pm.expect(err).to.be.null;
                pm.expect(res.code).to.equal(200);
                pm.expect(res.json().scope).to.include('accounts_read');
                res.to.have.status(200);
            }});
        }});
        order.push('after call');
        const res = await pm.sendRequest({{ url: base + '/echo', method: 'post', header: {{ 'x-k': 'v' }}, body: {{ mode: 'urlencoded', urlencoded: [{{ key: 'a', value: 'b c' }}] }} }});
        pm.test('promise', () => {{ pm.expect(res.code).to.equal(201); pm.expect(res.text()).to.equal('created'); }});
        pm.sendRequest('http://127.0.0.1:9/closed', (err, res) => {{
            pm.test('refused', () => {{ pm.expect(res).to.be.null; pm.expect(err.message).to.match(/^connect/); }});
        }});
        let limited = null;
        for (let i = 0; i < 8; i += 1) pm.sendRequest(base + '/scopes', (err) => {{ if (err) limited = err.message; }});
        setTimeout(() => {{
            pm.test('order', () => pm.expect(order).to.eql(['after call', 'callback']));
            pm.test('limit', () => pm.expect(limited).to.equal('pm.sendRequest: no more than 10 calls per script'));
        }}, 5);
        "#,
        base = server.uri()
    );
    let out = run_script(
        "test script of request \"ping\"",
        &code,
        &input("test", Some(response(200, "OK", "{}"))),
        &host(&rt),
    );
    assert!(out.tests.iter().all(|t| t.passed), "{:#?}", out.tests);
    let names: Vec<_> = out.tests.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["promise", "callback", "refused", "order", "limit"]);
}

#[test]
fn run_chain_threads_variables_and_headers_and_stops_on_skip() {
    let rt = tokio();
    let scripts = [
        ScriptSource {
            owner: "collection \"C\"".into(),
            code: "pm.environment.set('fromCollection', 'yes'); pm.request.headers.add({key: 'X-Col', value: '1'});".into(),
        },
        ScriptSource {
            owner: "folder \"F\"".into(),
            code: "pm.test('sees collection', () => { pm.expect(pm.environment.get('fromCollection')).to.equal('yes'); pm.expect(pm.request.headers.get('X-Col')).to.equal('1'); }); pm.execution.skipRequest();".into(),
        },
        ScriptSource {
            owner: "request \"R\"".into(),
            code: "pm.test('never runs', () => {});".into(),
        },
    ];
    let mut state = ScriptState {
        environment: Some(Environment::new("DESA")),
        ..ScriptState::default()
    };
    let request = ScriptRequest {
        url: "http://h".into(),
        method: "GET".into(),
        ..ScriptRequest::default()
    };
    let out = run_chain(
        Phase::Pre,
        &scripts,
        &ScriptInfo::default(),
        &request,
        None,
        &mut state,
        &host(&rt),
    );
    assert!(out.skip);
    assert_eq!(out.tests.len(), 1, "{:#?}", out.tests);
    assert!(out.tests[0].passed, "{:#?}", out.tests);
    assert_eq!(out.headers.unwrap()[0].key, "X-Col");
    assert_eq!(state.ops.len(), 1);
    assert_eq!(
        state.environment.unwrap().variables[0].value.as_deref(),
        Some("yes")
    );

    // Sin entorno activo, pm.environment.set no se guarda y se avisa en consola.
    let mut empty = ScriptState::default();
    let out = run_chain(
        Phase::Pre,
        &scripts[..1],
        &ScriptInfo::default(),
        &request,
        None,
        &mut empty,
        &host(&rt),
    );
    assert!(empty.ops.is_empty());
    assert_eq!(
        out.console,
        ["[warn] pm.environment: no active environment, value not saved"]
    );
}
