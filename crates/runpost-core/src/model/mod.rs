//! Formato nativo en disco (sección 5). Todo lleva `"schema": 1`.

use serde::{Deserialize, Serialize};

pub mod tree;

pub use tree::Position;

pub const SCHEMA: u32 = 1;

fn schema() -> u32 {
    SCHEMA
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Identificador estable con prefijo legible (`col_`, `fld_`, `req_`, `env_`, `h_`).
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::now_v7().simple())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretStore {
    File,
    Keyring,
}

/// Variable de colección, entorno o globales (5.1, 5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    pub key: String,
    /// Ausente en disco cuando el secreto vive en el llavero (`store: keyring`): `Workspace` lo
    /// lee del llavero al cargar y lo guarda ahí al escribir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<SecretStore>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

impl Variable {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: Some(value.into()),
            secret: false,
            store: None,
            enabled: true,
        }
    }

    pub fn secret(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            secret: true,
            store: Some(SecretStore::File),
            ..Self::new(key, value)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scripts {
    #[serde(default)]
    pub pre: String,
    #[serde(default)]
    pub test: String,
}

impl Scripts {
    pub fn is_empty(&self) -> bool {
        self.pre.is_empty() && self.test.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyIn {
    #[default]
    Header,
    Query,
}

/// Autenticación (5.4). `oauth2_cc` llega en la fase 4.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Auth {
    None,
    #[default]
    Inherit,
    Bearer {
        #[serde(default)]
        token: String,
    },
    Basic {
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
    },
    #[serde(rename = "apikey")]
    ApiKey {
        #[serde(default)]
        key: String,
        #[serde(default)]
        value: String,
        #[serde(default, rename = "in")]
        location: ApiKeyIn,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RawLang {
    Json,
    #[default]
    Text,
    Xml,
    Html,
    Javascript,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    #[default]
    Text,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultipartField {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub kind: FieldKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
}

/// Cuerpo de la petición (5.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Body {
    #[default]
    None,
    Raw {
        #[serde(default)]
        lang: RawLang,
        #[serde(default)]
        text: String,
    },
    Urlencoded {
        #[serde(default)]
        fields: Vec<KeyValue>,
    },
    Multipart {
        #[serde(default)]
        fields: Vec<MultipartField>,
    },
    Binary {
        #[serde(default)]
        path: String,
    },
    Graphql {
        #[serde(default)]
        query: String,
        #[serde(default)]
        variables: String,
    },
}

/// Ajustes por petición; `None` = heredar de `settings.http`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestSettings {
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub verify_tls: Option<bool>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub name: String,
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<KeyValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<KeyValue>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub settings: RequestSettings,
    #[serde(default)]
    pub description: String,
}

impl Request {
    pub fn new(name: impl Into<String>, method: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: new_id("req"),
            name: name.into(),
            method: method.into(),
            url: url.into(),
            params: Vec::new(),
            headers: Vec::new(),
            auth: Auth::Inherit,
            body: Body::None,
            scripts: Scripts::default(),
            settings: RequestSettings::default(),
            description: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `None` (= `null` en disco) hereda del padre.
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub items: Vec<Item>,
}

impl Folder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id("fld"),
            name: name.into(),
            description: String::new(),
            auth: None,
            scripts: Scripts::default(),
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Item {
    Folder(Folder),
    Request(Box<Request>),
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Item::Folder(folder) => &folder.id,
            Item::Request(request) => &request.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Item::Folder(folder) => &folder.name,
            Item::Request(request) => &request.name,
        }
    }
}

/// Guardarraíles para llamadas MCP (sección 19.3). Se evalúan en el sprint 2.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_methods: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require_approval: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_calls_per_minute: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default = "schema")]
    pub schema: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub agent_policy: Option<AgentPolicy>,
    #[serde(default)]
    pub variables: Vec<Variable>,
    #[serde(default)]
    pub scripts: Scripts,
    #[serde(default)]
    pub items: Vec<Item>,
}

impl Collection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: SCHEMA,
            id: new_id("col"),
            name: name.into(),
            description: String::new(),
            auth: None,
            agent_policy: None,
            variables: Vec::new(),
            scripts: Scripts::default(),
            items: Vec::new(),
        }
    }

    /// Recorre el árbol en orden; `visit` recibe las carpetas padre y la petición.
    pub fn walk_requests<'a>(&'a self, mut visit: impl FnMut(&[&'a Folder], &'a Request)) {
        fn walk<'a>(
            items: &'a [Item],
            parents: &mut Vec<&'a Folder>,
            visit: &mut dyn FnMut(&[&'a Folder], &'a Request),
        ) {
            for item in items {
                match item {
                    Item::Folder(folder) => {
                        parents.push(folder);
                        walk(&folder.items, parents, visit);
                        parents.pop();
                    }
                    Item::Request(request) => visit(parents, request),
                }
            }
        }
        walk(&self.items, &mut Vec::new(), &mut visit);
    }

    pub fn request_count(&self) -> usize {
        let mut count = 0;
        self.walk_requests(|_, _| count += 1);
        count
    }

    pub fn folder_count(&self) -> usize {
        fn count(items: &[Item]) -> usize {
            items
                .iter()
                .map(|item| match item {
                    Item::Folder(folder) => 1 + count(&folder.items),
                    Item::Request(_) => 0,
                })
                .sum()
        }
        count(&self.items)
    }

    /// Carpetas padre (de fuera hacia dentro) de la petición `request_id`.
    pub fn parents_of(&self, request_id: &str) -> Option<Vec<&Folder>> {
        let mut found = None;
        self.walk_requests(|parents, request| {
            if found.is_none() && request.id == request_id {
                found = Some(parents.to_vec());
            }
        });
        found
    }

    /// Ruta legible `"Colección/Carpeta/Nombre"` de cada petición.
    pub fn request_paths(&self) -> Vec<(String, &Request)> {
        let mut paths = Vec::new();
        self.walk_requests(|parents, request| {
            let mut path = self.name.clone();
            for folder in parents {
                path.push('/');
                path.push_str(&folder.name);
            }
            path.push('/');
            path.push_str(&request.name);
            paths.push((path, request));
        });
        paths
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    #[serde(default = "schema")]
    pub schema: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub variables: Vec<Variable>,
}

impl Environment {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: SCHEMA,
            id: new_id("env"),
            name: name.into(),
            variables: Vec::new(),
        }
    }
}

/// `globals.json`: misma forma que un entorno, sin `name`. El `id` aparece al guardar el primer
/// secreto en el llavero: es la cuenta de esos secretos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Globals {
    #[serde(default = "schema")]
    pub schema: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default)]
    pub variables: Vec<Variable>,
}

impl Default for Globals {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            id: String::new(),
            variables: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpSettings {
    pub timeout_ms: u64,
    pub verify_tls: bool,
    pub follow_redirects: bool,
    pub max_redirects: usize,
    pub proxy: Option<String>,
    pub user_agent: String,
}

impl Default for HttpSettings {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            verify_tls: true,
            follow_redirects: true,
            max_redirects: 10,
            proxy: None,
            user_agent: format!("{}/0.1", crate::APP_SLUG),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub body_max_chars: usize,
    pub headers_default: bool,
    pub mask_secrets: bool,
    pub cache_responses: usize,
    /// Puerto fijo del MCP por HTTP de la app (`http://127.0.0.1:<port>/mcp`); `null` lo apaga.
    /// Fijo para que la configuración del agente se pegue una sola vez.
    pub http_port: Option<u16>,
}

pub const MCP_HTTP_PORT: u16 = 7879;

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            body_max_chars: 2000,
            headers_default: false,
            mask_secrets: true,
            cache_responses: 50,
            http_port: Some(MCP_HTTP_PORT),
        }
    }
}

/// Diff de respuestas (19.7): campos volátiles que no cuentan como cambio. Un nombre vale a
/// cualquier profundidad; una ruta `$.a[*].b`, solo ahí.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffSettings {
    pub ignore: Vec<String>,
}

impl Default for DiffSettings {
    fn default() -> Self {
        Self {
            ignore: ["timestamp", "requestId", "traceId"]
                .map(String::from)
                .to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub font_size: u32,
    pub wrap_response: bool,
    pub lang: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            font_size: 13,
            wrap_response: true,
            lang: "es".into(),
        }
    }
}

/// `settings.json` (5.5). Campos ausentes toman el valor por defecto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    pub theme: Theme,
    pub active_environment: Option<String>,
    pub http: HttpSettings,
    pub mcp: McpSettings,
    pub ui: UiSettings,
    pub diff: DiffSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            theme: Theme::System,
            active_environment: None,
            http: HttpSettings::default(),
            mcp: McpSettings::default(),
            ui: UiSettings::default(),
            diff: DiffSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T>(value: serde_json::Value) -> T
    where
        T: serde::de::DeserializeOwned + Serialize + PartialEq + std::fmt::Debug,
    {
        let parsed: T = serde_json::from_value(value).unwrap();
        let again: T = serde_json::from_str(&serde_json::to_string(&parsed).unwrap()).unwrap();
        assert_eq!(parsed, again);
        parsed
    }

    #[test]
    fn collection_example_of_section_5_1_round_trips() {
        let collection: Collection = round_trip(json!({
          "schema": 1, "id": "col_1", "name": "Acme Gateway Scopes", "description": "",
          "auth": null,
          "agent_policy": {"allow_methods": ["GET","POST"], "require_approval": ["DELETE","req_x"], "max_calls_per_minute": 30},
          "variables": [{"key":"base","value":"https://x","secret":false,"enabled":true}],
          "scripts": {"pre":"","test":""},
          "items": [
            {"type":"folder","id":"fld_1","name":"00 - Diagnóstico","auth":null,"scripts":{"pre":"","test":""},
             "items":[
               {"type":"request","id":"req_1","name":"00.2 Gateway - ping","method":"GET",
                "url":"{{gatewayUrl}}/v1/ping",
                "params":[{"key":"x","value":"1","enabled":true}],
                "headers":[{"key":"Accept","value":"application/json","enabled":false}],
                "auth":{"type":"inherit"},"body":{"mode":"none"},
                "scripts":{"pre":"pm.environment.set('t', Date.now())","test":""},
                "settings":{"follow_redirects":true,"verify_tls":null,"timeout_ms":null},
                "description":""}
             ]}
          ]
        }));
        assert_eq!(collection.request_count(), 1);
        assert_eq!(collection.folder_count(), 1);
        let policy = collection.agent_policy.unwrap();
        assert_eq!(policy.max_calls_per_minute, Some(30));
        let Item::Folder(folder) = &collection.items[0] else {
            panic!("folder")
        };
        let Item::Request(request) = &folder.items[0] else {
            panic!("request")
        };
        assert!(!request.headers[0].enabled);
        assert_eq!(request.settings.follow_redirects, Some(true));
    }

    #[test]
    fn every_body_and_auth_variant_round_trips() {
        for body in [
            json!({"mode":"none"}),
            json!({"mode":"raw","lang":"json","text":"{}"}),
            json!({"mode":"urlencoded","fields":[{"key":"a","value":"1"}]}),
            json!({"mode":"multipart","fields":[{"key":"f","value":"","kind":"file","path":"a.bin"}]}),
            json!({"mode":"binary","path":"x.bin"}),
            json!({"mode":"graphql","query":"{a}","variables":"{}"}),
        ] {
            let parsed: Body = round_trip(body.clone());
            assert_eq!(serde_json::to_value(&parsed).unwrap()["mode"], body["mode"]);
        }
        for auth in [
            json!({"type":"none"}),
            json!({"type":"inherit"}),
            json!({"type":"bearer","token":"{{accessToken}}"}),
            json!({"type":"basic","username":"u","password":"p"}),
            json!({"type":"apikey","key":"X-Api-Key","value":"v","in":"query"}),
        ] {
            let parsed: Auth = round_trip(auth.clone());
            assert_eq!(serde_json::to_value(&parsed).unwrap(), auth);
        }
    }

    #[test]
    fn environment_with_keyring_secret_has_no_value() {
        let env: Environment = round_trip(
            json!({"schema":1,"id":"env_1","name":"Acme-DESA","variables":[
            {"key":"gatewayUrl","value":"https://x","secret":false,"enabled":true},
            {"key":"accessToken","secret":true,"store":"keyring","enabled":true}]}),
        );
        assert_eq!(env.variables[1].value, None);
        assert_eq!(env.variables[1].store, Some(SecretStore::Keyring));
        let text = serde_json::to_string(&env).unwrap();
        assert!(!text.contains("\"value\":null"));
    }

    #[test]
    fn settings_defaults_match_section_5_5() {
        let settings: Settings = round_trip(json!({"theme":"dark"}));
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(settings.http.timeout_ms, 30_000);
        assert_eq!(settings.mcp.body_max_chars, 2000);
        assert_eq!(settings.ui.lang, "es");
        assert_eq!(settings.schema, 1);
        let full = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(full["http"]["user_agent"], "runpost/0.1");
        assert_eq!(full["active_environment"], serde_json::Value::Null);
    }

    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = new_id("req");
        assert!(a.starts_with("req_") && a.len() == 36);
        assert_ne!(a, new_id("req"));
    }
}
