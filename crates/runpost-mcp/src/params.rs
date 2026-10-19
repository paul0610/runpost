//! Entradas de las tools. Los esquemas se generan con `schemars` y se compactan en `lib.rs`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

pub type Pairs = BTreeMap<String, String>;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListRequests {
    pub collection: Option<String>,
    pub folder: Option<String>,
    /// Filter by name or url
    pub q: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GetRequest {
    /// Id (req_…) or path "Collection/Folder/Name"
    pub request: String,
    /// Default: method,url,params,headers,body. Also: auth,scripts,settings,description
    pub fields: Option<Vec<String>>,
    /// Instead of fields: code in curl, fetch, powershell or python (secrets as {{name}})
    pub snippet: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RunRequest {
    /// Id (req_…) or path "Collection/Folder/Name"
    pub request: String,
    /// Environment id or name; default: active
    pub env: Option<String>,
    /// Variable overrides
    pub vars: Option<Pairs>,
    /// JSONPath on the JSON body
    pub select: Option<String>,
    pub body_max: Option<usize>,
    /// Include response headers
    pub headers: Option<bool>,
    pub save_history: Option<bool>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SaveAs {
    pub collection: String,
    pub folder: Option<String>,
    pub name: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Send {
    pub method: String,
    /// May use {{vars}}
    pub url: String,
    pub headers: Option<Pairs>,
    /// String or JSON
    pub body: Option<Value>,
    /// Native auth, e.g. {"type":"bearer","token":"{{t}}"}
    pub auth: Option<Value>,
    pub env: Option<String>,
    pub select: Option<String>,
    pub body_max: Option<usize>,
    pub save_as: Option<SaveAs>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GetResponse {
    pub response_id: String,
    pub select: Option<String>,
    /// [from,to) in chars of the (selected) body
    pub range: Option<[usize; 2]>,
    /// Return only the headers
    pub headers: Option<bool>,
    // SPEC-DEVIATION: S6-D2 — `save_ref` guarda la referencia sin una 14.ª tool; las referencias
    // viven en history.db y los campos volátiles en settings.json (`diff.ignore`).
    /// Diff against "ref" (the request's saved reference) or another response_id
    pub diff_against: Option<String>,
    /// Extra volatile fields for the diff: names or $.paths
    pub ignore: Option<Vec<String>>,
    /// Save this response as the request's reference
    pub save_ref: Option<bool>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EnvGet {
    pub env: Option<String>,
    pub keys: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EnvSet {
    /// Id or name; created if missing
    pub env: String,
    pub vars: Pairs,
    /// Keys to mark secret
    pub secret: Option<Vec<String>>,
}

#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
pub struct Scripts {
    pub pre: Option<String>,
    pub test: Option<String>,
}

// Una petición a crear o actualizar (por carpeta + nombre); sin doc para no gastar tokens en el esquema.
#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
pub struct UpsertItem {
    /// "A/B" creates nested folders
    pub folder: Option<String>,
    pub name: String,
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: Option<Pairs>,
    /// String or JSON
    pub body: Option<Value>,
    /// Native auth, e.g. {"type":"bearer","token":"{{t}}"}
    pub auth: Option<Value>,
    pub scripts: Option<Scripts>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Upsert {
    /// Id or name; created if missing
    pub collection: String,
    pub folder: Option<String>,
    pub name: Option<String>,
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: Option<Pairs>,
    pub body: Option<Value>,
    pub auth: Option<Value>,
    pub scripts: Option<Scripts>,
    /// Batch: many requests in one call
    pub requests: Option<Vec<UpsertItem>>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ImportPostman {
    /// File path, or a Bruno collection folder
    pub path: Option<String>,
    /// Or the text (JSON or YAML)
    pub text: Option<String>,
    /// Postman: collection | environment (auto-detected)
    pub kind: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct History {
    pub limit: Option<usize>,
    pub q: Option<String>,
    pub status: Option<u16>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Search {
    pub q: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RunCollection {
    /// Id, name or slug
    pub collection: String,
    /// Folder id, path or name
    pub folder: Option<String>,
    /// Environment id or name; default: active
    pub env: Option<String>,
    /// Default: one per data row, or 1
    pub iterations: Option<u32>,
    /// Path to a CSV or JSON data file
    pub data: Option<String>,
    /// summary (default) | failures | full
    pub output: Option<String>,
}
