use std::time::Duration;

use serde_json::json;

use super::*;

const SIGN: &str = r#"meta {
  name: Sign transaction
  type: http
  seq: 2
}

post {
  url: {{apiUrl}}/v1/:currency/accounts/:accountId/sign?dry=1
  body: json
  auth: bearer
}

params:query {
  dry: 1
  ~verbose: true
}

params:path {
  currency: usd
  accountId:
}

headers {
  Content-Type: application/json
  ~X-Debug: 1
}

auth:bearer {
  token: {{accessToken}}
}

body:json {
  {
    "draft": {{draftJson}},
    "nested": { "a": [1, 2] }
  }
}

vars:pre-request {
  scope: accounts_sign
}

vars:post-response {
  signature: res.body.signature
  firstId: res.body.items[0].id
  weird: res.body.map(x => x)
}

assert {
  res.status: eq 200
  res.body.ok: isTruthy
  res.headers['content-type']: contains json
  ~res.body.skip: eq 1
  res.body.items: weirdOp 3
}

script:pre-request {
  bru.setVar("nonce", Date.now());
}

tests {
  test("signed", function() {
    expect(res.getStatus()).to.equal(200);
  });
}

docs {
  Signs a draft.
}
"#;

fn write(root: &Path, path: &str, text: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn folder(item: &Item) -> &Folder {
    match item {
        Item::Folder(folder) => folder,
        Item::Request(request) => panic!("expected a folder, got {}", request.name),
    }
}

fn request(item: &Item) -> &Request {
    match item {
        Item::Request(request) => request,
        Item::Folder(folder) => panic!("expected a request, got {}", folder.name),
    }
}

fn pairs(list: &[KeyValue]) -> Vec<(&str, &str, bool)> {
    list.iter()
        .map(|kv| (kv.key.as_str(), kv.value.as_str(), kv.enabled))
        .collect()
}

#[test]
fn parses_the_blocks_of_a_request_file() {
    let parsed = parse_request(SIGN, "fallback");
    assert_eq!(parsed.name, "Sign transaction");
    assert_eq!(parsed.method, "POST");
    assert_eq!(parsed.seq.to_string(), "2");
    assert_eq!(parsed.body_mode, "json");
    assert_eq!(parsed.shared.auth_mode, "bearer");
    assert_eq!(parsed.shared.auth["bearer"]["token"], "{{accessToken}}");
    let query = parsed.query.as_ref().unwrap();
    assert_eq!(query.len(), 2);
    assert!(!query[1].enabled);
    assert_eq!(
        parsed.body.json,
        "{\n  \"draft\": {{draftJson}},\n  \"nested\": { \"a\": [1, 2] }\n}"
    );
    assert_eq!(parsed.shared.docs, "Signs a draft.");
    assert_eq!(parsed.asserts.len(), 5);

    // Valores multilínea, claves entre comillas, listas y archivos.
    let blocks = blocks(
        "body:form-urlencoded {\n  note: '''\n    line 1\n    line 2\n  '''\n  \"a key: x\": 1\n}\nvars:secret [\n  a,\n  ~b\n]\nbody:multipart-form {\n  doc: @file(a.pem|b.pem) @contentType(application/x-pem-file)\n  label: hi @contentType(text/plain)\n}\n",
    );
    let form = dict(&blocks[0].lines);
    assert_eq!(form[0].value, "line 1\nline 2");
    assert_eq!(form[1].key, "a key: x");
    assert_eq!(
        list(&blocks[1].lines),
        [("a".into(), true), ("b".into(), false)]
    );
    let file = dict(&blocks[2].lines);
    assert_eq!(
        file_ref(&file[0].value),
        ("a.pem|b.pem".into(), "application/x-pem-file".into())
    );
    assert_eq!(file_ref(&file[1].value).0, "hi");
}

#[test]
fn imports_a_collection_folder_without_reading_dotenv() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "bruno.json",
        r#"{"version": "1", "name": "Acme Gateway", "type": "collection", "ignore": ["node_modules", ".git", "drafts"]}"#,
    );
    write(
        root,
        "collection.bru",
        "headers {\n  Accept: application/json\n}\n\nauth {\n  mode: bearer\n}\n\nauth:bearer {\n  token: {{accessToken}}\n}\n\nvars:pre-request {\n  apiVersion: v1\n}\n\ndocs {\n  Fictional API.\n}\n",
    );
    write(
        root,
        "Accounts/folder.bru",
        "meta {\n  name: Accounts\n  seq: 1\n}\n\nheaders {\n  X-Scope: accounts_read\n}\n\nauth {\n  mode: inherit\n}\n\nvars:pre-request {\n  currency: usd\n}\n",
    );
    write(root, "Accounts/Sign.bru", SIGN);
    write(
        root,
        "Ping.bru",
        "meta {\n  name: Ping\n  type: http\n  seq: 1\n}\n\nget {\n  url: {{gatewayUrl}}/v1/ping?verbose=1\n  body: none\n  auth: none\n}\n\nheaders {\n  X-Key: {{process.env.ACME_KEY}}\n}\n",
    );
    write(root, "Live.bru", "meta {\n  name: Live\n  type: ws\n}\n");
    write(
        root,
        "drafts/Old.bru",
        "meta {\n  name: Old\n}\n\nget {\n  url: http://old.example.test\n}\n",
    );
    write(root, "node_modules/x/Bad.bru", "meta {\n  name: Bad\n}\n");
    write(
        root,
        "environments/Local.bru",
        "vars {\n  apiUrl: http://localhost:3080\n  ~unused: x\n}\nvars:secret [\n  accessToken,\n  encryptPassword\n]\n",
    );
    write(root, ".env", "ACME_KEY=fake-key-must-not-be-read\n");

    assert_eq!(
        collection_root(&root.join("bruno.json")).as_deref(),
        Some(root)
    );
    assert_eq!(collection_root(root).as_deref(), Some(root));
    assert_eq!(collection_root(&root.join("Ping.bru")), None);
    assert_eq!(collection_root(&root.join("Accounts")), None);

    let imported = import_folder(root).unwrap();
    let all = serde_json::to_string(&(&imported.collections, &imported.environments)).unwrap();
    assert!(!all.contains("fake-key-must-not-be-read"));
    assert!(!all.contains("old.example.test"));

    let collection = &imported.collections[0];
    assert_eq!(collection.name, "Acme Gateway");
    assert_eq!(collection.id, stable_id("col", "bruno:Acme Gateway"));
    assert_eq!(collection.description, "Fictional API.");
    assert_eq!(
        collection.auth,
        Some(Auth::Bearer {
            token: "{{accessToken}}".into()
        })
    );
    let vars: Vec<_> = collection
        .variables
        .iter()
        .map(|v| (v.key.as_str(), v.value.as_deref().unwrap_or(""), v.secret))
        .collect();
    assert_eq!(
        vars,
        [
            ("apiVersion", "v1", false),
            ("accountId", "", false),
            ("ACME_KEY", "", true)
        ]
    );

    // Carpetas primero; el `.bru` de WebSocket, `drafts` (ignore) y node_modules no entran.
    assert_eq!(collection.items.len(), 2);
    let accounts = folder(&collection.items[0]);
    assert_eq!(accounts.name, "Accounts");
    assert_eq!(accounts.auth, None);
    assert_eq!(
        accounts.scripts.pre,
        "pm.variables.set(\"currency\", \"usd\");"
    );

    let sign = request(&accounts.items[0]);
    assert_eq!(sign.method, "POST");
    assert_eq!(sign.url, "{{apiUrl}}/v1/usd/accounts/{{accountId}}/sign");
    assert_eq!(
        pairs(&sign.params),
        [("dry", "1", true), ("verbose", "true", false)]
    );
    assert_eq!(
        pairs(&sign.headers),
        [
            ("Accept", "application/json", true),
            ("X-Scope", "accounts_read", true),
            ("Content-Type", "application/json", true),
            ("X-Debug", "1", false)
        ]
    );
    assert!(
        matches!(&sign.body, Body::Raw { lang: RawLang::Json, text } if text.contains("{{draftJson}}"))
    );
    assert_eq!(
        sign.auth,
        Auth::Bearer {
            token: "{{accessToken}}".into()
        }
    );
    assert_eq!(sign.description, "Signs a draft.");
    assert!(
        sign.scripts
            .pre
            .starts_with("pm.variables.set(\"scope\", \"accounts_sign\");\n// Bruno script")
    );
    assert!(
        sign.scripts
            .pre
            .contains("// bru.setVar(\"nonce\", Date.now());")
    );
    let test = &sign.scripts.test;
    assert!(test.contains("pm.variables.set(\"signature\", pm.response.json().signature);"));
    assert!(test.contains("pm.variables.set(\"firstId\", pm.response.json().items[0].id);"));
    assert!(test.contains("// weird: res.body.map(x => x) (not translated)"));
    assert!(test.contains(
        "pm.test(\"res.status: eq 200\", () => pm.expect(pm.response.code).to.eql(200));"
    ));
    assert!(test.contains("pm.response.headers.get(\"content-type\")).to.include(\"json\")"));
    assert!(!test.contains("res.body.skip"));
    assert!(test.contains("// assert res.body.items: weirdOp 3 (not translated)"));
    assert!(test.contains("//   expect(res.getStatus()).to.equal(200);"));

    let ping = request(&collection.items[1]);
    assert_eq!(ping.url, "{{gatewayUrl}}/v1/ping");
    assert_eq!(pairs(&ping.params), [("verbose", "1", true)]);
    assert_eq!(ping.auth, Auth::None);
    assert_eq!(
        pairs(&ping.headers),
        [
            ("Accept", "application/json", true),
            ("X-Key", "{{ACME_KEY}}", true)
        ]
    );

    let env = &imported.environments[0];
    assert_eq!(env.name, "Acme Gateway - Local");
    assert_eq!(env.id, stable_id("env", "bruno:Acme Gateway:Local"));
    let env_vars: Vec<_> = env
        .variables
        .iter()
        .map(|v| (v.key.as_str(), v.secret, v.enabled))
        .collect();
    assert_eq!(
        env_vars,
        [
            ("apiUrl", false, true),
            ("unused", false, false),
            ("accessToken", true, true),
            ("encryptPassword", true, true)
        ]
    );

    let warnings = imported.warnings.join("\n");
    assert!(warnings.contains("fill in the secret variables ACME_KEY"));
    assert!(warnings.contains("2 Bruno scripts kept as comments"));
    assert!(warnings.contains("2 assertions or post-response variables"));
    assert!(warnings.contains("1 items skipped"));
}

#[test]
fn imports_the_json_export() {
    let export = json!({
        "version": "1",
        "name": "Acme Gateway",
        "items": [
            {"type": "graphql-request", "name": "Query", "seq": 1, "request": {
                "url": "{{apiUrl}}/graphql", "method": "POST",
                "body": {"mode": "graphql", "graphql": {"query": "{ ping }", "variables": "{}"}},
                "auth": {"mode": "apikey", "apikey": {"key": "X-Api-Key", "value": "{{apiKey}}", "placement": "queryparams"}}
            }},
            {"type": "folder", "name": "Keys", "seq": 2,
             "root": {"request": {
                 "headers": [{"name": "X-Scope", "value": "keys", "enabled": true}],
                 "auth": {"mode": "basic", "basic": {"username": "{{user}}", "password": "{{pass}}"}}}},
             "items": [
                {"type": "http-request", "name": "Create key", "seq": 1, "request": {
                    "url": "{{apiUrl}}/v1/:keyType/keys/local", "method": "post",
                    "headers": [],
                    "params": [{"name": "keyType", "value": "{{keyType}}", "type": "path", "enabled": true}],
                    "body": {"mode": "multipartForm", "multipartForm": [
                        {"name": "label", "value": "{{scopeLabel}}", "type": "text", "enabled": true},
                        {"name": "files", "value": ["a.pem", "b.pem"], "type": "file", "enabled": true}]},
                    "auth": {"mode": "inherit"},
                    "assertions": [{"name": "res.status", "value": "in 200, 201", "enabled": true}],
                    "docs": "Creates a key."
                }},
                {"type": "http-request", "name": "Upload", "seq": 2, "request": {
                    "url": "{{apiUrl}}/v1/upload", "method": "PUT",
                    "body": {"mode": "file", "file": [
                        {"filePath": "old.bin", "contentType": "", "selected": false},
                        {"filePath": "draft.bin", "contentType": "application/octet-stream", "selected": true}]}
                }}
             ]},
            {"type": "js", "name": "util.js"}
        ],
        "environments": [{"name": "DESA", "variables": [
            {"name": "apiUrl", "value": "https://desa.example.test", "enabled": true, "secret": false},
            {"name": "accessToken", "value": "", "enabled": true, "secret": true}]}],
        "root": {"request": {"vars": {"req": [{"name": "keyType", "value": "ed25519", "enabled": true}]}}, "docs": "Docs"},
        "brunoConfig": {"version": "1", "name": "Acme Gateway", "type": "collection"}
    });
    assert!(detect(&export));
    assert_eq!(
        crate::import::detect_value(&export),
        Some(crate::import::Format::Bruno)
    );
    assert!(!detect(&json!({"info": {"name": "x"}, "item": []})));

    let imported = import_json(&export).unwrap();
    let collection = &imported.collections[0];
    // Mismo id que la carpeta: reimportar desde cualquiera de los dos sustituye.
    assert_eq!(collection.id, stable_id("col", "bruno:Acme Gateway"));
    assert_eq!(collection.description, "Docs");
    assert_eq!(collection.variables[0].key, "keyType");

    let keys = folder(&collection.items[0]);
    assert_eq!(
        keys.auth,
        Some(Auth::Basic {
            username: "{{user}}".into(),
            password: "{{pass}}".into()
        })
    );
    let create = request(&keys.items[0]);
    assert_eq!(create.method, "POST");
    assert_eq!(create.url, "{{apiUrl}}/v1/{{keyType}}/keys/local");
    assert_eq!(pairs(&create.headers), [("X-Scope", "keys", true)]);
    assert_eq!(create.auth, Auth::Inherit);
    assert_eq!(create.description, "Creates a key.");
    let Body::Multipart { fields } = &create.body else {
        panic!("multipart")
    };
    let parts: Vec<_> = fields
        .iter()
        .map(|f| (f.key.as_str(), f.kind, f.value.as_str(), f.path.as_str()))
        .collect();
    assert_eq!(
        parts,
        [
            ("label", FieldKind::Text, "{{scopeLabel}}", ""),
            ("files", FieldKind::File, "", "a.pem"),
            ("files", FieldKind::File, "", "b.pem")
        ]
    );
    assert_eq!(
        create.scripts.test,
        "pm.test(\"res.status: in 200, 201\", () => pm.expect(pm.response.code).to.be.oneOf([200, 201]));"
    );

    let upload = request(&keys.items[1]);
    assert_eq!(
        upload.body,
        Body::Binary {
            path: "draft.bin".into()
        }
    );
    assert_eq!(
        pairs(&upload.headers),
        [
            ("X-Scope", "keys", true),
            ("Content-Type", "application/octet-stream", true)
        ]
    );

    let query = request(&collection.items[1]);
    assert_eq!(
        query.body,
        Body::Graphql {
            query: "{ ping }".into(),
            variables: "{}".into()
        }
    );
    assert_eq!(
        query.auth,
        Auth::ApiKey {
            key: "X-Api-Key".into(),
            value: "{{apiKey}}".into(),
            location: ApiKeyIn::Query
        }
    );

    let env = &imported.environments[0];
    assert_eq!(env.name, "Acme Gateway - DESA");
    assert!(env.variables[1].secret);
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("1 items skipped"))
    );
}

/// Las aserciones traducidas corren en el motor de scripts y dan el veredicto de Bruno.
#[test]
fn translated_assertions_run_in_the_script_engine() {
    let asserts = [
        ("res.status", "eq 200"),
        ("res.status", "neq 500"),
        ("res.status", "in 200, 201"),
        ("res.status", "between 199, 299"),
        ("res.status", "gte 200"),
        ("res.responseTime", "lt 10000"),
        ("res.body.ok", "isTruthy"),
        ("res.body.name", "startsWith 'Acme'"),
        ("res.body.name", "endsWith Gateway"),
        ("res.body.name", "matches ^Acme"),
        ("res.body.items", "isArray"),
        ("res.body.items", "length 2"),
        ("res.body.items[0].id", "isNumber"),
        ("res.body['missing']", "isUndefined"),
        ("res.body.items", "isNotEmpty"),
        ("res.body", "isJson"),
        ("res.headers['content-type']", "contains json"),
        ("res.headers.x-id", "eq 7"),
    ];
    let mut code = String::new();
    for (key, rule) in asserts {
        match assert_js(key, rule) {
            Some(line) => {
                code.push_str(&line);
                code.push('\n');
            }
            // `res.headers.x-id` no es una lectura de propiedad válida.
            None => assert_eq!(key, "res.headers.x-id"),
        }
    }
    code.push_str(&format!(
        "pm.variables.set(\"first\", {});\n",
        post_value("res.body.items[1].id").unwrap()
    ));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let host = crate::script::ScriptHost {
        runtime: rt.handle().clone(),
        client: reqwest::Client::new(),
        timeout: Duration::from_secs(5),
        user_agent: "runpost-test".into(),
    };
    let body = r#"{"ok": true, "name": "Acme Gateway", "items": [{"id": 1}, {"id": 2}]}"#;
    let input = json!({
        "phase": "test",
        "info": {"requestName": "ping", "requestId": "req_1", "iteration": 0, "iterationCount": 1},
        "vars": {"environment": null, "globals": [], "collection": [], "iteration": {}, "local": {}},
        "request": {"url": "http://localhost/v1/ping", "method": "GET", "headers": [], "body": {"mode": "none"}},
        "response": {
            "code": 200, "status": "OK",
            "headers": [{"key": "Content-Type", "value": "application/json"}],
            "body": body, "responseTime": 12, "size": body.len(),
        },
    });
    let out = crate::script::run_script("bruno asserts", &code, &input, &host);
    let failed: Vec<_> = out
        .tests
        .iter()
        .filter(|t| !t.passed)
        .map(|t| format!("{}: {}", t.name, t.msg.clone().unwrap_or_default()))
        .collect();
    assert!(failed.is_empty(), "{failed:#?}\n{code}");
    assert_eq!(out.tests.len(), 17);
    assert!(
        out.changes
            .iter()
            .any(|op| op.key == "first" && op.value.as_deref() == Some("2"))
    );
}

#[test]
fn annotations_custom_verbs_and_missing_auth() {
    let parsed = parse_request(
        "meta {\n  name: Purge\n}\n\nhttp {\n  method: purge\n  url: {{apiUrl}}/v1/cache\n}\n\nheaders {\n  @description('sent by the importer')\n  X-Client: runpost\n  @description('''\n    two\n    lines\n  ''')\n  X-Two: 2\n}\n\nvars:pre-request {\n  @number\n  retries: 3\n  @local: yes\n}\n\nbody:form-urlencoded {\n  note: '''\n    line 1\n    line 2\n  ''' @contentType(text/plain)\n  tags: [\n    a\n    b\n  ]\n  after: 1\n}\n",
        "fallback",
    );
    assert_eq!(parsed.method, "PURGE");
    assert_eq!(parsed.url, "{{apiUrl}}/v1/cache");
    // Sin `auth:` en el bloque del método, Bruno no manda auth.
    assert_eq!(parsed.shared.auth_mode, "none");
    let headers: Vec<_> = parsed
        .shared
        .headers
        .iter()
        .map(|p| (p.key.as_str(), p.value.as_str()))
        .collect();
    assert_eq!(headers, [("X-Client", "runpost"), ("X-Two", "2")]);
    let vars: Vec<_> = parsed
        .shared
        .vars_pre
        .iter()
        .map(|p| (p.key.as_str(), p.value.as_str()))
        .collect();
    assert_eq!(vars, [("retries", "3"), ("local", "yes")]);
    let form: Vec<_> = parsed
        .body
        .form
        .iter()
        .map(|p| (p.key.as_str(), p.value.as_str()))
        .collect();
    assert_eq!(form, [("note", "line 1\nline 2"), ("after", "1")]);

    let built = build(
        BruFolder {
            name: "Acme".into(),
            requests: vec![parsed],
            ..Default::default()
        },
        Vec::new(),
        0,
    );
    let purge = request(&built.collections[0].items[0]);
    assert_eq!(purge.auth, Auth::None);
    assert_eq!(
        purge.scripts.pre,
        "pm.variables.set(\"retries\", \"3\");\npm.variables.set(\"local\", \"yes\");"
    );
}

#[test]
fn current_json_exports_use_http_and_graphql_types() {
    let export = json!({
        "name": "Acme Gateway", "version": "1",
        "items": [
            {"type": "http", "name": "Ping", "seq": 1, "request": {
                "url": "{{apiUrl}}/v1/ping", "method": "GET",
                "params": [{"name": "verbose", "value": "1", "type": "query", "enabled": false}],
                "body": {"mode": "none"}, "auth": {"mode": "inherit"}}},
            {"type": "graphql", "name": "Graph", "seq": 2, "request": {
                "url": "{{apiUrl}}/graphql", "method": "POST",
                "body": {"mode": "graphql", "graphql": {"query": "{ ping }"}}}},
            {"type": "grpc", "name": "Stream", "seq": 3}
        ],
        "exportedUsing": "Bruno/3.1.0"
    });
    assert!(detect(&export));
    let imported = import_json(&export).unwrap();
    let collection = &imported.collections[0];
    assert_eq!(collection.request_count(), 2);
    let ping = request(&collection.items[0]);
    assert_eq!(pairs(&ping.params), [("verbose", "1", false)]);
    assert_eq!(ping.auth, Auth::Inherit);
    // Sin `auth`, ninguno.
    assert_eq!(request(&collection.items[1]).auth, Auth::None);
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("1 items skipped"))
    );
}

const OC_REQUEST: &str = r#"info:
  name: Create order
  type: http
  seq: 1

http:
  method: POST
  url: "{{base_url}}/orders/:shopId?dryRun=true"
  headers:
    - name: X-Client
      value: importer
  params:
    - { name: dryRun, value: "true", type: query, disabled: true }
    - { name: shopId, value: "42", type: path }
  body:
    type: multipart-form
    data:
      - { name: note, type: text, value: hello }
      - { name: invoice, type: file, value: [files/invoice.pdf], contentType: application/pdf }
  auth: inherit

runtime:
  variables:
    - { name: region, value: { type: string, data: eu } }
  scripts:
    - type: before-request
      code: bru.setVar("ts", Date.now());
  assertions:
    - { expression: res.status, operator: eq, value: "201" }
    - { expression: res.body.id, operator: isDefined }
  actions:
    - type: set-variable
      phase: after-response
      selector: { expression: res.body.id, method: jsonq }
      variable: { name: orderId, scope: runtime }

docs: Creates an order.
"#;

#[test]
fn imports_an_opencollection_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "opencollection.yml",
        "opencollection: 1.0.0\ninfo:\n  name: Shop API\nrequest:\n  headers:\n    - { name: Accept, value: application/json }\n  auth: { type: bearer, token: \"{{token}}\" }\n  variables:\n    - { name: apiVersion, value: v1 }\ndocs: { content: \"# Shop API\", type: text/markdown }\nbundled: false\nextensions:\n  bruno:\n    ignore: [node_modules, .git, drafts]\n",
    );
    write(
        root,
        "Orders/folder.yml",
        "info:\n  name: Orders\n  type: folder\n  seq: 1\nrequest:\n  auth: { type: apikey, key: X-Api-Key, value: \"{{apiKey}}\", placement: query }\n",
    );
    write(root, "Orders/Create order.yml", OC_REQUEST);
    write(
        root,
        "Graph.yml",
        "info:\n  name: Graph\n  type: graphql\n  seq: 2\ngraphql:\n  url: \"{{base_url}}/graphql\"\n  body:\n    query: \"{ orders { id } }\"\n    variables: { first: 2 }\n",
    );
    write(root, "Notes.yml", "info:\n  name: Notes\n  type: script\n");
    write(
        root,
        "drafts/Old.yml",
        "info:\n  name: Old\nhttp:\n  url: http://old.example.test\n",
    );
    write(
        root,
        "environments/Production.yml",
        "name: Ignored\ncolor: \"#e53e3e\"\nvariables:\n  - { name: base_url, value: https://api.shop.example.test }\n  - { name: retries, value: { type: number, data: \"3\" } }\n  - { secret: true, name: token }\n  - { name: old, value: x, disabled: true }\n",
    );
    write(root, ".env", "TOKEN=fake-token-must-not-be-read\n");

    assert_eq!(
        collection_root(&root.join("opencollection.yml")).as_deref(),
        Some(root)
    );
    let imported = import_folder(root).unwrap();
    let all = serde_json::to_string(&(&imported.collections, &imported.environments)).unwrap();
    assert!(!all.contains("fake-token-must-not-be-read"));
    assert!(!all.contains("old.example.test"));

    let collection = &imported.collections[0];
    assert_eq!(collection.name, "Shop API");
    assert_eq!(collection.description, "# Shop API");
    assert_eq!(
        collection.auth,
        Some(Auth::Bearer {
            token: "{{token}}".into()
        })
    );
    assert_eq!(collection.variables[0].key, "apiVersion");

    let orders = folder(&collection.items[0]);
    assert_eq!(
        orders.auth,
        Some(Auth::ApiKey {
            key: "X-Api-Key".into(),
            value: "{{apiKey}}".into(),
            location: ApiKeyIn::Query
        })
    );
    let create = request(&orders.items[0]);
    assert_eq!(create.name, "Create order");
    assert_eq!(create.url, "{{base_url}}/orders/42");
    assert_eq!(pairs(&create.params), [("dryRun", "true", false)]);
    assert_eq!(
        pairs(&create.headers),
        [
            ("Accept", "application/json", true),
            ("X-Client", "importer", true)
        ]
    );
    assert_eq!(create.auth, Auth::Inherit);
    let Body::Multipart { fields } = &create.body else {
        panic!("multipart")
    };
    assert_eq!(fields[1].path, "files/invoice.pdf");
    assert_eq!(fields[1].kind, FieldKind::File);
    assert!(
        create
            .scripts
            .pre
            .starts_with("pm.variables.set(\"region\", \"eu\");\n// Bruno script")
    );
    let test = &create.scripts.test;
    assert!(test.contains("pm.variables.set(\"orderId\", pm.response.json().id);"));
    assert!(test.contains("pm.expect(pm.response.code).to.eql(201)"));
    assert!(test.contains("pm.expect(pm.response.json().id).to.not.be.undefined"));
    assert_eq!(create.description, "Creates an order.");

    let graph = request(&collection.items[1]);
    assert_eq!(graph.method, "POST");
    assert_eq!(
        graph.body,
        Body::Graphql {
            query: "{ orders { id } }".into(),
            variables: "{\n  \"first\": 2\n}".into()
        }
    );
    assert_eq!(graph.auth, Auth::None);

    let env = &imported.environments[0];
    assert_eq!(env.name, "Shop API - Production");
    let env_vars: Vec<_> = env
        .variables
        .iter()
        .map(|v| {
            (
                v.key.as_str(),
                v.value.as_deref().unwrap_or(""),
                v.secret,
                v.enabled,
            )
        })
        .collect();
    assert_eq!(
        env_vars,
        [
            ("base_url", "https://api.shop.example.test", false, true),
            ("retries", "3", false, true),
            ("token", "", true, true),
            ("old", "x", false, false)
        ]
    );
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("1 items skipped"))
    );
}

#[test]
fn imports_a_bundled_opencollection_file() {
    let request_yaml: Vec<String> = OC_REQUEST
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                format!("      - {line}")
            } else {
                format!("        {line}")
            }
        })
        .collect();
    let bundle = format!(
        "opencollection: 1.0.0\ninfo:\n  name: Shop API\nbundled: true\nitems:\n  - info:\n      name: Orders\n      type: folder\n      seq: 1\n    request:\n      headers:\n        - {{ name: X-Scope, value: orders }}\n    items:\n{}\nconfig:\n  environments:\n    - name: Local\n      variables:\n        - {{ name: base_url, value: http://localhost:3080 }}\n",
        request_yaml.join("\n")
    );
    let value = crate::import::parse_document(&bundle).unwrap();
    assert!(detect(&value));
    assert_eq!(
        crate::import::detect_value(&value),
        Some(crate::import::Format::Bruno)
    );
    let imported = import_json(&value).unwrap();
    let collection = &imported.collections[0];
    assert_eq!(collection.id, stable_id("col", "bruno:Shop API"));
    let orders = folder(&collection.items[0]);
    let create = request(&orders.items[0]);
    assert_eq!(create.url, "{{base_url}}/orders/42");
    assert_eq!(pairs(&create.headers)[0], ("X-Scope", "orders", true));
    assert_eq!(imported.environments[0].name, "Shop API - Local");

    // Un `opencollection.yml` empaquetado dentro de una carpeta también vale.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "opencollection.yml", &bundle);
    let imported = import_folder(dir.path()).unwrap();
    assert_eq!(imported.collections[0].request_count(), 1);
}
