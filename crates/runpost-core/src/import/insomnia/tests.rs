use serde_json::json;

use super::*;
use crate::import::parse_document;

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

fn vars(list: &[Variable]) -> Vec<(&str, &str, bool)> {
    list.iter()
        .map(|v| (v.key.as_str(), v.value.as_deref().unwrap_or(""), v.secret))
        .collect()
}

#[test]
fn templates_tags_and_scripts() {
    let mut b = Builder::default();
    assert_eq!(
        b.t("{{ _.apiUrl }}/v1/{{ keyType }}/{{_.auth.token}}/{{ _['scope-label'] }}"),
        "{{apiUrl}}/v1/{{keyType}}/{{auth.token}}/{{scope-label}}"
    );
    assert_eq!(
        b.t("Bearer {{ vault.accessToken }}"),
        "Bearer {{accessToken}}"
    );
    assert_eq!(b.vault, ["accessToken"]);
    assert_eq!(
        b.t("{% uuid 'v4' %} {% now 'iso-8601', '' %} {% now 'unix', '' %}{# note #}"),
        "{{$guid}} {{$isoTimestamp}} {{$timestamp}}"
    );
    let kept =
        "{% response 'body', 'req_1', 'b64::JC50b2tlbg==::46b', 'always', 60 %} {{ name | upper }}";
    assert_eq!(b.t(kept), kept);
    assert_eq!(b.kept, ["response", "expression"]);
    assert_eq!(b.t("no templates {here}"), "no templates {here}");

    let code = "insomnia.test('ok', () => {\n  insomnia.expect(insomnia.response.code).to.eql(200);\n  myinsomnia.x; a.insomnia.y; log('insomnia.rest');\n});";
    assert_eq!(
        b.translate(code),
        "pm.test('ok', () => {\n  pm.expect(pm.response.code).to.eql(200);\n  myinsomnia.x; a.insomnia.y; log('insomnia.rest');\n});"
    );
    assert_eq!(b.translate("  "), "");
    assert_eq!(b.scripts, 1);
}

fn v4_export() -> Value {
    json!({
        "_type": "export", "__export_format": 4,
        "__export_date": "2026-10-01T00:00:00.000Z", "__export_source": "insomnia.desktop.app:v10.3.1",
        "resources": [
            {"_id": "wrk_acme", "parentId": "proj_1", "_type": "workspace", "name": "Acme Gateway",
             "description": "Fictional API.", "scope": "collection"},
            {"_id": "wrk_mock", "parentId": "proj_1", "_type": "workspace", "name": "Mocks", "scope": "mock-server"},
            {"_id": "env_base", "parentId": "wrk_acme", "_type": "environment", "name": "Base Environment",
             "data": {"apiUrl": "http://localhost:3080", "auth": {"token": "fake-token"}, "currency": "usd",
                      "__insomnia_vault": {"encryptPassword": "••••••"}}},
            {"_id": "env_desa", "parentId": "env_base", "_type": "environment", "name": "DESA", "metaSortKey": 1,
             "data": {"apiUrl": "https://desa.example.test", "accountPassphrase": "fake"}},
            {"_id": "fld_accounts", "parentId": "wrk_acme", "_type": "request_group", "name": "Accounts",
             "metaSortKey": -2, "environment": {"scopeLabel": "accounts_sign"},
             "headers": [{"name": "X-Client", "value": "runpost"}, {"name": "X-Off", "value": "1", "disabled": true}],
             "authentication": {"type": "bearer", "token": "{{ _.auth.token }}", "prefix": ""},
             "preRequestScript": "insomnia.environment.set('ts', Date.now());"},
            {"_id": "req_sign", "parentId": "fld_accounts", "_type": "request", "name": "Sign", "metaSortKey": -1,
             "method": "post", "url": "{{ _.apiUrl }}/v1/:currency/accounts/:accountId/sign?dry=1",
             "description": "Signs a draft.",
             "pathParameters": [{"name": "currency", "value": "{{ _.currency }}"}, {"name": "accountId", "value": null}],
             "parameters": [{"name": "dry", "value": "1"}, {"name": "verbose", "value": "true", "disabled": true}],
             "headers": [{"name": "Content-Type", "value": "application/json"}, {"name": "X-Client", "value": "sign"}],
             "body": {"mimeType": "application/json", "text": "{\"draft\": {{ _.draftJson }}}"},
             "authentication": {},
             "afterResponseScript": "insomnia.test('signed', () => insomnia.expect(insomnia.response.code).to.eql(200));",
             "settingFollowRedirects": "off"},
            {"_id": "req_upload", "parentId": "fld_accounts", "_type": "request", "name": "Upload", "metaSortKey": -3,
             "method": "PUT", "url": "{{ _.apiUrl }}/v1/upload",
             "body": {"mimeType": "application/octet-stream", "fileName": "draft.bin"},
             "authentication": {"type": "bearer", "token": "{{ vault.accessToken }}", "prefix": "Token"}},
            {"_id": "req_keys", "parentId": "wrk_acme", "_type": "request", "name": "Create key", "metaSortKey": 5,
             "method": "POST", "url": "{{ _.apiUrl }}/v1/ed25519/keys/local",
             "body": {"mimeType": "multipart/form-data", "params": [
                 {"name": "label", "value": "{{ _['scope-label'] }}"},
                 {"name": "pem", "type": "file", "fileName": "key.pem"},
                 {"name": "off", "value": "x", "disabled": true}]},
             "authentication": {"type": "apikey", "key": "session", "value": "fake", "addTo": "cookie"}},
            {"_id": "req_graphql", "parentId": "wrk_acme", "_type": "request", "name": "Graph", "metaSortKey": 6,
             "method": "POST", "url": "{{ _.apiUrl }}/graphql",
             "body": {"mimeType": "application/graphql", "text": "{\"query\":\"{ ping }\",\"variables\":{\"a\":1}}"},
             "authentication": {"type": "oauth1", "consumerKey": "x"}},
            {"_id": "req_none", "parentId": "wrk_acme", "_type": "request", "name": "Ping", "metaSortKey": 4,
             "method": "GET", "url": "{{ _.apiUrl }}/v1/ping",
             "authentication": {"type": "none"}},
            {"_id": "ws-req_live", "parentId": "wrk_acme", "_type": "websocket_request", "name": "Live"},
            {"_id": "jar_1", "parentId": "wrk_acme", "_type": "cookie_jar", "name": "Default Jar"}
        ]
    })
}

#[test]
fn imports_a_v4_export() {
    let export = v4_export();
    assert!(detect(&export));
    assert_eq!(
        crate::import::detect_value(&export),
        Some(crate::import::Format::Insomnia)
    );
    let imported = import(&export).unwrap();
    assert_eq!(imported.collections.len(), 1);
    let collection = &imported.collections[0];
    assert_eq!(collection.name, "Acme Gateway");
    assert_eq!(collection.id, stable_id("col", "insomnia:wrk_acme"));
    assert_eq!(collection.description, "Fictional API.");
    assert_eq!(
        vars(&collection.variables),
        [
            ("apiUrl", "http://localhost:3080", false),
            ("auth.token", "fake-token", false),
            ("currency", "usd", false),
            ("encryptPassword", "", true),
            ("accountId", "", false),
            ("accessToken", "", true)
        ]
    );

    // Orden de Insomnia: clave de orden ascendente.
    let names: Vec<_> = collection.items.iter().map(Item::name).collect();
    assert_eq!(names, ["Accounts", "Ping", "Create key", "Graph"]);

    let accounts = folder(&collection.items[0]);
    assert_eq!(
        accounts.auth,
        Some(Auth::Bearer {
            token: "{{auth.token}}".into()
        })
    );
    assert_eq!(
        accounts.scripts.pre,
        "pm.variables.set(\"scopeLabel\", \"accounts_sign\");\npm.environment.set('ts', Date.now());"
    );
    let upload = request(&accounts.items[0]);
    assert_eq!(upload.name, "Upload");
    assert_eq!(
        upload.body,
        Body::Binary {
            path: "draft.bin".into()
        }
    );
    // Prefijo propio: la cabecera tal cual, sin auth.
    assert_eq!(upload.auth, Auth::None);
    assert_eq!(
        pairs(&upload.headers),
        [
            ("X-Client", "runpost", true),
            ("Authorization", "Token {{accessToken}}", true)
        ]
    );

    let sign = request(&accounts.items[1]);
    assert_eq!(sign.method, "POST");
    assert_eq!(
        sign.url,
        "{{apiUrl}}/v1/{{currency}}/accounts/{{accountId}}/sign"
    );
    assert_eq!(
        pairs(&sign.params),
        [("dry", "1", true), ("verbose", "true", false)]
    );
    // Cabeceras de la carpeta y luego las propias (Insomnia envía las dos `X-Client`).
    assert_eq!(
        pairs(&sign.headers),
        [
            ("X-Client", "runpost", true),
            ("Content-Type", "application/json", true),
            ("X-Client", "sign", true)
        ]
    );
    assert_eq!(
        sign.body,
        Body::Raw {
            lang: RawLang::Json,
            text: "{\"draft\": {{draftJson}}}".into()
        }
    );
    assert_eq!(sign.auth, Auth::Inherit);
    assert_eq!(sign.settings.follow_redirects, Some(false));
    assert_eq!(sign.description, "Signs a draft.");
    assert_eq!(
        sign.scripts.test,
        "pm.test('signed', () => pm.expect(pm.response.code).to.eql(200));"
    );

    let ping = request(&collection.items[1]);
    assert_eq!(ping.auth, Auth::None);

    let keys = request(&collection.items[2]);
    let Body::Multipart { fields } = &keys.body else {
        panic!("multipart")
    };
    let parts: Vec<_> = fields
        .iter()
        .map(|f| {
            (
                f.key.as_str(),
                f.kind,
                f.value.as_str(),
                f.path.as_str(),
                f.enabled,
            )
        })
        .collect();
    assert_eq!(
        parts,
        [
            ("label", FieldKind::Text, "{{scope-label}}", "", true),
            ("pem", FieldKind::File, "", "key.pem", true),
            ("off", FieldKind::Text, "x", "", false)
        ]
    );
    assert_eq!(pairs(&keys.headers), [("Cookie", "session=fake", true)]);
    assert_eq!(keys.auth, Auth::None);

    let graph = request(&collection.items[3]);
    assert_eq!(
        graph.body,
        Body::Graphql {
            query: "{ ping }".into(),
            variables: "{\n  \"a\": 1\n}".into()
        }
    );
    assert_eq!(graph.auth, Auth::None);

    let env = &imported.environments[0];
    assert_eq!(env.name, "Acme Gateway - DESA");
    assert_eq!(env.id, stable_id("env", "insomnia:env_desa"));
    assert_eq!(
        vars(&env.variables),
        [
            ("apiUrl", "https://desa.example.test", false),
            ("accountPassphrase", "fake", false)
        ]
    );

    let warnings = imported.warnings.join("\n");
    assert!(
        warnings.contains("Acme Gateway/Graph: auth \"oauth1\" not supported"),
        "{warnings}"
    );
    assert!(warnings.contains("fill in the secret variables accessToken"));
    assert!(
        warnings.contains("2 Insomnia scripts translated"),
        "{warnings}"
    );
    assert!(warnings.contains("2 environment variables look like credentials"));
    assert!(warnings.contains("1 gRPC, WebSocket or Socket.IO requests skipped"));
    assert!(warnings.contains("1 mock server or MCP workspaces skipped"));
}

const V5: &str = r#"type: collection.insomnia.rest/5.0
schema_version: "5.1"
name: Acme Gateway
meta:
  id: wrk_acme5
  description: Fictional API.
collection:
  - name: Keys
    meta:
      id: fld_keys
      sortKey: -10
    headers:
      - name: X-Scope
        value: keys
    authentication:
      type: basic
      username: "{{ _.user }}"
      password: "{{ vault.password }}"
    children:
      - name: Nested
        meta:
          id: fld_nested
          sortKey: -5
        children:
          - url: "{{ _.apiUrl }}/v1/:keyType/keys/local"
            name: Create key
            meta:
              id: req_create
              sortKey: -1
              description: Creates a key.
            method: POST
            pathParameters:
              - name: keyType
                value: ed25519
            body:
              mimeType: application/x-www-form-urlencoded
              params:
                - name: label
                  value: "{{ scopeLabel }}"
                - name: off
                  value: "1"
                  disabled: true
            scripts:
              preRequest: insomnia.variables.set('n', 1);
              afterResponse: insomnia.test('ok', () => {});
            settings:
              followRedirects: "on"
  - url: wss://example.test/live
    name: Live
    meta:
      id: ws-req_1
  - url: "{{ _.apiUrl }}/v1/ping"
    name: Ping
    meta:
      id: req_ping
      sortKey: 3
    method: GET
    body:
      mimeType: text/plain
      text: hello
environments:
  name: Base Environment
  meta:
    id: env_base5
  environmentType: kv
  kvPairData:
    - name: apiUrl
      value: http://localhost:3080
      type: str
      enabled: true
    - name: accessToken
      value: ""
      type: secret
      enabled: true
    - name: legacy
      value: "1"
      type: str
      enabled: false
  subEnvironments:
    - name: DESA
      meta:
        id: env_desa5
      data:
        apiUrl: https://desa.example.test
"#;

#[test]
fn imports_a_v5_collection() {
    let value = parse_document(V5).unwrap();
    assert!(detect(&value));
    assert_eq!(
        crate::import::detect_value(&value),
        Some(crate::import::Format::Insomnia)
    );
    let imported = import(&value).unwrap();
    let collection = &imported.collections[0];
    assert_eq!(collection.id, stable_id("col", "insomnia:wrk_acme5"));
    assert_eq!(collection.description, "Fictional API.");
    assert_eq!(
        vars(&collection.variables),
        [
            ("apiUrl", "http://localhost:3080", false),
            ("accessToken", "", true),
            ("legacy", "1", false),
            ("password", "", true)
        ]
    );
    assert!(!collection.variables[2].enabled);

    let names: Vec<_> = collection.items.iter().map(Item::name).collect();
    assert_eq!(names, ["Keys", "Ping"]);
    let keys = folder(&collection.items[0]);
    assert_eq!(
        keys.auth,
        Some(Auth::Basic {
            username: "{{user}}".into(),
            password: "{{password}}".into()
        })
    );
    let nested = folder(&keys.items[0]);
    let create = request(&nested.items[0]);
    assert_eq!(create.url, "{{apiUrl}}/v1/ed25519/keys/local");
    assert_eq!(create.description, "Creates a key.");
    assert_eq!(pairs(&create.headers), [("X-Scope", "keys", true)]);
    assert_eq!(
        create.body,
        Body::Urlencoded {
            fields: vec![
                KeyValue::new("label", "{{scopeLabel}}"),
                KeyValue {
                    key: "off".into(),
                    value: "1".into(),
                    enabled: false
                }
            ]
        }
    );
    assert_eq!(create.scripts.pre, "pm.variables.set('n', 1);");
    assert_eq!(create.scripts.test, "pm.test('ok', () => {});");
    assert_eq!(create.settings.follow_redirects, Some(true));

    let ping = request(&collection.items[1]);
    assert_eq!(
        ping.body,
        Body::Raw {
            lang: RawLang::Text,
            text: "hello".into()
        }
    );

    assert_eq!(imported.environments.len(), 1);
    assert_eq!(imported.environments[0].name, "Acme Gateway - DESA");
    let warnings = imported.warnings.join("\n");
    assert!(warnings.contains("fill in the secret variables password"));
    assert!(warnings.contains("1 gRPC, WebSocket or Socket.IO requests skipped"));
}

#[test]
fn global_environments_specs_and_other_v5_files() {
    let global = parse_document(
        "type: environment.insomnia.rest/5.0\nname: Shared\nmeta:\n  id: wrk_env\nenvironments:\n  name: Base\n  meta:\n    id: env_shared\n  data:\n    region: eu\n  subEnvironments:\n    - name: Prod\n      meta:\n        id: env_prod\n      data:\n        region: us\n        host: api.example.test\n",
    )
    .unwrap();
    let imported = import(&global).unwrap();
    assert!(imported.collections.is_empty());
    assert_eq!(imported.environments.len(), 1);
    let env = &imported.environments[0];
    assert_eq!(env.name, "Shared - Prod");
    assert_eq!(
        vars(&env.variables),
        [("region", "us", false), ("host", "api.example.test", false)]
    );

    let only_base = parse_document(
        "type: environment.insomnia.rest/5.0\nname: Dev\nmeta:\n  id: wrk_dev\nenvironments:\n  name: Dev\n  meta:\n    id: env_dev\n  data:\n    base_url: http://localhost:8000\n",
    )
    .unwrap();
    let imported = import(&only_base).unwrap();
    assert_eq!(imported.environments[0].name, "Dev");
    assert_eq!(
        imported.environments[0].id,
        stable_id("env", "insomnia:wrk_dev")
    );

    let spec = parse_document(
        "type: spec.insomnia.rest/5.0\nname: Acme Design\nmeta:\n  id: wrk_spec\nspec:\n  contents: \"openapi: 3.0.0\"\ncollection:\n  - url: http://localhost/v1/ping\n    name: Ping\n    meta:\n      id: req_1\n    method: GET\n",
    )
    .unwrap();
    let imported = import(&spec).unwrap();
    assert_eq!(imported.collections[0].request_count(), 1);
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("import it as OpenAPI"))
    );

    let mock = parse_document("type: mock.insomnia.rest/5.0\nname: Mocks\n").unwrap();
    assert!(detect(&mock));
    assert!(import(&mock).unwrap_err().to_string().contains("mock"));
    assert!(!detect(&json!({"type": "collection"})));
    assert!(!detect(&json!({"_type": "request"})));
}
