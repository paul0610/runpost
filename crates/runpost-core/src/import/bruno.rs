//! Bruno → colección (fase 4): la carpeta de la colección (`bruno.json`, `collection.bru`,
//! `folder.bru`, un `.bru` por petición y `environments/*.bru`) o su exportación JSON.
//!
//! RunPost no tiene cabeceras ni variables por carpeta: las cabeceras de la colección y de las
//! carpetas se copian a cada petición, y las variables de carpeta y de petición van a su script
//! previo (`pm.variables.set`). Las aserciones (`assert`) y las variables posteriores a la respuesta
//! se traducen a `pm.*`; los scripts y los `tests` usan la API de Bruno (`bru`, `req`, `res`) y
//! entran comentados. `{{process.env.X}}` pasa a `{{X}}` con una variable secreta vacía: el `.env`
//! de la colección no se lee nunca (guarda secretos), ni nada que no sea `.bru` o `bruno.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::model::{
    ApiKeyIn, Auth, Body, ClientAuth, Collection, Environment, FieldKind, Folder, Item, KeyValue,
    MultipartField, OAuth2Cc, OAuth2Code, RawLang, Request, Scripts, Variable,
};

use super::{Imported, array, fill_path_params, merge_headers, split_query, stable_id, text};

mod opencollection;
#[cfg(test)]
mod tests;

// ---------- archivos .bru ----------

/// Un bloque de un `.bru` (`nombre {` … `}` o `nombre [` … `]`), con el contenido sin la sangría
/// de dos espacios que escribe Bruno. El bloque acaba en un `}` en la columna 0.
struct Block {
    name: String,
    lines: Vec<String>,
}

fn blocks(source: &str) -> Vec<Block> {
    let source = source.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut out = Vec::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        let head = line.trim();
        if let Some(name) = head.strip_suffix("{}").or_else(|| head.strip_suffix("[]")) {
            out.push(Block {
                name: name.trim().to_owned(),
                lines: Vec::new(),
            });
            continue;
        }
        let (name, close) = if let Some(name) = head.strip_suffix('{') {
            (name.trim(), "}")
        } else if let Some(name) = head.strip_suffix('[') {
            (name.trim(), "]")
        } else {
            continue;
        };
        let mut body = Vec::new();
        for line in lines.by_ref() {
            if line.trim_end() == close {
                break;
            }
            body.push(line.strip_prefix("  ").unwrap_or(line).to_owned());
        }
        out.push(Block {
            name: name.to_owned(),
            lines: body,
        });
    }
    out
}

/// Par de un bloque diccionario; `~` delante lo desactiva.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Pair {
    key: String,
    value: String,
    enabled: bool,
}

fn dict(lines: &[String]) -> Vec<Pair> {
    let mut out = Vec::new();
    let mut iter = lines.iter();
    while let Some(line) = iter.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Anotación (`@description('…')`, `@number`): va con el par siguiente y no se usa.
        if is_annotation(line) {
            if line.contains("('''") {
                for next in iter.by_ref() {
                    if next.trim_start().starts_with("'''") {
                        break;
                    }
                }
            }
            continue;
        }
        let (enabled, line) = match line.strip_prefix('~') {
            Some(rest) => (false, rest),
            None => (true, line),
        };
        let (key, value) = split_pair(line);
        let value = if value == "'''" {
            // Multilínea: las líneas llevan dos espacios más y el cierre puede llevar
            // `@contentType(…)` detrás.
            let mut text = Vec::new();
            for next in iter.by_ref() {
                if next.trim_start().starts_with("'''") {
                    break;
                }
                text.push(next.strip_prefix("  ").unwrap_or(next));
            }
            text.join("\n").trim().to_owned()
        } else if value == "[" {
            // Lista (`tags`, `omitHeaders`): no se usa.
            for next in iter.by_ref() {
                if next.trim() == "]" {
                    break;
                }
            }
            continue;
        } else {
            value.to_owned()
        };
        out.push(Pair {
            key,
            value,
            enabled,
        });
    }
    out
}

/// `@nombre` o `@nombre(…)` sin `:` detrás del nombre (`@token: x` es un par).
fn is_annotation(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('@') else {
        return false;
    };
    let end = rest
        .find(|c: char| c == '(' || c == ':' || c.is_whitespace())
        .unwrap_or(rest.len());
    end > 0 && !rest[end..].trim_start().starts_with(':')
}

/// `clave: valor`; la clave va entre comillas si lleva `:` o espacios.
fn split_pair(line: &str) -> (String, &str) {
    if let Some(rest) = line.strip_prefix('"') {
        let mut key = String::new();
        let mut chars = rest.char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                '\\' => {
                    if let Some((_, next)) = chars.next() {
                        key.push(next);
                    }
                }
                '"' => {
                    let after = rest[at + 1..].trim_start();
                    return (key, after.strip_prefix(':').unwrap_or(after).trim());
                }
                c => key.push(c),
            }
        }
        return (key, "");
    }
    match line.split_once(':') {
        Some((key, value)) => (key.trim().to_owned(), value.trim()),
        None => (line.to_owned(), ""),
    }
}

/// `vars:secret [ a, ~b ]` → `[("a", true), ("b", false)]`.
fn list(lines: &[String]) -> Vec<(String, bool)> {
    lines
        .iter()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| match item.strip_prefix('~') {
            Some(rest) => (rest.trim().to_owned(), false),
            None => (item.to_owned(), true),
        })
        .collect()
}

fn text_block(lines: &[String]) -> String {
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let mut text = lines[start..].join("\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text
}

/// `@file(a.png) @contentType(image/png)` → `("a.png", "image/png")`; sin `@file`, el texto.
fn file_ref(value: &str) -> (String, String) {
    let (value, content_type) = match value.rfind("@contentType(") {
        Some(at) if value.ends_with(')') => (
            value[..at].trim_end(),
            &value[at + "@contentType(".len()..value.len() - 1],
        ),
        _ => (value, ""),
    };
    let path = value
        .strip_prefix("@file(")
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(value);
    (path.to_owned(), content_type.to_owned())
}

// ---------- lo leído, igual para la carpeta y para el JSON ----------

/// Atributos de cada auth: `bearer` → `{token: …}`.
type AuthAttrs = HashMap<String, HashMap<String, String>>;

/// Lo que comparten la colección, las carpetas y las peticiones.
#[derive(Debug, Default)]
struct Shared {
    headers: Vec<Pair>,
    auth_mode: String,
    auth: AuthAttrs,
    script_pre: String,
    script_post: String,
    vars_pre: Vec<Pair>,
    vars_post: Vec<Pair>,
    tests: String,
    docs: String,
}

impl Shared {
    /// Bloques comunes de un `.bru`; los demás se ignoran.
    fn block(&mut self, block: &Block) {
        match block.name.as_str() {
            "headers" => self.headers.extend(dict(&block.lines)),
            "auth" => {
                if let Some(mode) = dict(&block.lines).into_iter().find(|p| p.key == "mode") {
                    self.auth_mode = mode.value;
                }
            }
            "script:pre-request" => self.script_pre = text_block(&block.lines),
            "script:post-response" => self.script_post = text_block(&block.lines),
            // `@x: …` es una variable local llamada `x`.
            "vars:pre-request" => {
                self.vars_pre
                    .extend(dict(&block.lines).into_iter().map(|mut pair| {
                        if let Some(name) = pair.key.strip_prefix('@') {
                            pair.key = name.to_owned();
                        }
                        pair
                    }));
            }
            "vars:post-response" => self.vars_post.extend(dict(&block.lines)),
            "tests" => self.tests = text_block(&block.lines),
            "docs" => self.docs = text_block(&block.lines),
            name => {
                if let Some(kind) = name.strip_prefix("auth:") {
                    let attrs = dict(&block.lines)
                        .into_iter()
                        .map(|p| (p.key, p.value))
                        .collect();
                    self.auth.insert(kind.to_owned(), attrs);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct BruFile {
    path: String,
    content_type: String,
    selected: bool,
}

#[derive(Debug, Default)]
struct BruBody {
    json: String,
    text: String,
    xml: String,
    sparql: String,
    form: Vec<Pair>,
    /// `(campo, es archivo)`; un archivo puede llevar varias rutas separadas por `|`.
    multipart: Vec<(Pair, bool)>,
    graphql_query: String,
    graphql_vars: String,
    file: Vec<BruFile>,
}

#[derive(Debug, Default)]
struct BruRequest {
    name: String,
    /// `http` o `graphql`; lo demás (gRPC, WebSocket) no se importa.
    kind: String,
    seq: f64,
    method: String,
    url: String,
    /// `None`: sin bloque `params:query`, los parámetros salen de la URL.
    query: Option<Vec<Pair>>,
    path: Vec<Pair>,
    body_mode: String,
    body: BruBody,
    asserts: Vec<Pair>,
    shared: Shared,
}

#[derive(Debug, Default)]
struct BruFolder {
    name: String,
    seq: f64,
    shared: Shared,
    folders: Vec<BruFolder>,
    requests: Vec<BruRequest>,
}

#[derive(Debug, Default)]
struct BruEnv {
    name: String,
    /// `(variable, secreta)`.
    vars: Vec<(Pair, bool)>,
}

const METHODS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "options", "head", "connect", "trace",
];

fn parse_request(source: &str, fallback_name: &str) -> BruRequest {
    let mut request = BruRequest {
        name: fallback_name.to_owned(),
        kind: "http".into(),
        method: "GET".into(),
        ..Default::default()
    };
    // Sin `auth:` en el bloque del método, Bruno no manda auth.
    request.shared.auth_mode = "none".into();
    for block in blocks(source) {
        let name = block.name.as_str();
        // `get { … }` o `http { method: PURGE … }` (verbo propio).
        if METHODS.contains(&name) || name == "http" {
            request.method = name.to_ascii_uppercase();
            for pair in dict(&block.lines) {
                match pair.key.as_str() {
                    "method" => request.method = pair.value.to_ascii_uppercase(),
                    "url" => request.url = pair.value,
                    "body" => request.body_mode = pair.value,
                    "auth" => request.shared.auth_mode = pair.value,
                    _ => {}
                }
            }
            continue;
        }
        match name {
            "meta" => {
                for pair in dict(&block.lines) {
                    match pair.key.as_str() {
                        "name" => request.name = pair.value,
                        "type" => request.kind = pair.value,
                        "seq" => request.seq = pair.value.parse().unwrap_or_default(),
                        _ => {}
                    }
                }
            }
            "params:query" | "query" => request
                .query
                .get_or_insert_with(Vec::new)
                .extend(dict(&block.lines)),
            "params:path" => request.path.extend(dict(&block.lines)),
            "body:json" => request.body.json = text_block(&block.lines),
            "body:text" => request.body.text = text_block(&block.lines),
            "body:xml" => request.body.xml = text_block(&block.lines),
            "body:sparql" => request.body.sparql = text_block(&block.lines),
            "body:graphql" => request.body.graphql_query = text_block(&block.lines),
            "body:graphql:vars" => request.body.graphql_vars = text_block(&block.lines),
            "body:form-urlencoded" => request.body.form = dict(&block.lines),
            "body:multipart-form" => {
                request.body.multipart = dict(&block.lines)
                    .into_iter()
                    .map(|mut pair| {
                        let file = pair.value.trim_start().starts_with("@file(");
                        pair.value = file_ref(&pair.value).0;
                        (pair, file)
                    })
                    .collect();
            }
            "body:file" => {
                request.body.file = dict(&block.lines)
                    .into_iter()
                    .map(|pair| {
                        let (path, content_type) = file_ref(&pair.value);
                        BruFile {
                            path,
                            content_type,
                            selected: pair.enabled,
                        }
                    })
                    .collect();
            }
            "assert" => request.asserts.extend(dict(&block.lines)),
            _ => request.shared.block(&block),
        }
    }
    request
}

/// `collection.bru` o `folder.bru`: `(nombre, seq, lo común)`.
fn parse_root(source: &str) -> (String, f64, Shared) {
    let mut name = String::new();
    let mut seq = 0.0;
    let mut shared = Shared::default();
    for block in blocks(source) {
        if block.name == "meta" {
            for pair in dict(&block.lines) {
                match pair.key.as_str() {
                    "name" => name = pair.value,
                    "seq" => seq = pair.value.parse().unwrap_or_default(),
                    _ => {}
                }
            }
        } else {
            shared.block(&block);
        }
    }
    (name, seq, shared)
}

fn parse_environment(name: &str, source: &str) -> BruEnv {
    let mut env = BruEnv {
        name: name.to_owned(),
        vars: Vec::new(),
    };
    for block in blocks(source) {
        match block.name.as_str() {
            "vars" => env
                .vars
                .extend(dict(&block.lines).into_iter().map(|pair| (pair, false))),
            // Bruno no guarda el valor de los secretos en el archivo.
            "vars:secret" => {
                env.vars
                    .extend(list(&block.lines).into_iter().map(|(key, enabled)| {
                        (
                            Pair {
                                key,
                                value: String::new(),
                                enabled,
                            },
                            true,
                        )
                    }))
            }
            _ => {}
        }
    }
    env
}

// ---------- carpeta de la colección ----------

const MAX_FILES: usize = 10_000;
const MAX_DEPTH: usize = 16;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Formato de la carpeta: `.bru` (`bruno.json`) u OpenCollection YAML (`opencollection.yml`,
/// el de Bruno desde la 3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Bru,
    Yaml,
}

impl Layout {
    fn ext(self) -> &'static str {
        match self {
            Self::Bru => ".bru",
            Self::Yaml => ".yml",
        }
    }

    fn folder_file(self) -> &'static str {
        match self {
            Self::Bru => "folder.bru",
            Self::Yaml => "folder.yml",
        }
    }

    fn root_file(self) -> &'static str {
        match self {
            Self::Bru => "collection.bru",
            Self::Yaml => "opencollection.yml",
        }
    }
}

/// Carpeta de la colección si `path` es esa carpeta (con `opencollection.yml` o `bruno.json`)
/// o uno de esos dos archivos.
pub fn collection_root(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let dir = if meta.is_dir() {
        path.to_path_buf()
    } else if path.file_name().is_some_and(|name| {
        name.eq_ignore_ascii_case("bruno.json") || name.eq_ignore_ascii_case("opencollection.yml")
    }) {
        match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        }
    } else {
        return None;
    };
    (dir.join("opencollection.yml").is_file() || dir.join("bruno.json").is_file()).then_some(dir)
}

struct Walker {
    layout: Layout,
    files: usize,
    ignore: Vec<String>,
    skipped: usize,
}

impl Walker {
    fn read(&mut self, path: &Path) -> Result<String> {
        self.files += 1;
        if self.files > MAX_FILES {
            bail!("the collection has more than {MAX_FILES} files");
        }
        let size = std::fs::metadata(path)?.len();
        if size > MAX_FILE_BYTES {
            bail!("{} is larger than 8 MB", path.display());
        }
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
    }

    /// `(nombre, seq, lo común)` de `folder.bru` / `folder.yml`.
    fn folder_meta(&mut self, path: &Path) -> Result<(String, f64, Shared)> {
        let source = self.read(path)?;
        match self.layout {
            Layout::Bru => Ok(parse_root(&source)),
            Layout::Yaml => opencollection::folder_file(&source)
                .with_context(|| format!("invalid {}", path.display())),
        }
    }

    fn request(&mut self, path: &Path, stem: &str) -> Result<Option<BruRequest>> {
        let source = self.read(path)?;
        let request = match self.layout {
            Layout::Bru => Some(parse_request(&source, stem)),
            Layout::Yaml => opencollection::request_file(&source, stem)
                .with_context(|| format!("invalid {}", path.display()))?,
        };
        Ok(request.filter(|r| matches!(r.kind.as_str(), "http" | "graphql")))
    }

    /// Los enlaces simbólicos y lo oculto (`.git`, `.env`) no se tocan.
    fn dir(&mut self, dir: &Path, folder: &mut BruFolder, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            bail!("folders nested more than {MAX_DEPTH} levels deep");
        }
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .with_context(|| format!("cannot read {}", dir.display()))?
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        let ext = self.layout.ext();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                if name == "node_modules"
                    || (depth == 0 && name == "environments")
                    || self.ignore.contains(&name)
                {
                    continue;
                }
                let mut sub = BruFolder {
                    name: name.clone(),
                    ..Default::default()
                };
                let meta = path.join(self.layout.folder_file());
                if meta.is_file() {
                    let (meta_name, seq, shared) = self.folder_meta(&meta)?;
                    if !meta_name.is_empty() {
                        sub.name = meta_name;
                    }
                    sub.seq = seq;
                    sub.shared = shared;
                }
                self.dir(&path, &mut sub, depth + 1)?;
                folder.folders.push(sub);
            } else if kind.is_file()
                && name.ends_with(ext)
                && name != self.layout.folder_file()
                && name != self.layout.root_file()
            {
                match self.request(&path, name.trim_end_matches(ext))? {
                    Some(request) => folder.requests.push(request),
                    None => self.skipped += 1,
                }
            }
        }
        Ok(())
    }

    fn environments(&mut self, root: &Path) -> Result<Vec<BruEnv>> {
        let dir = root.join("environments");
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let ext = self.layout.ext();
        let mut files: Vec<_> = std::fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter(|e| e.file_name().to_string_lossy().ends_with(ext))
            .collect();
        files.sort_by_key(|entry| entry.file_name());
        let mut out = Vec::new();
        for entry in files {
            let path = entry.path();
            let file = entry.file_name().to_string_lossy().into_owned();
            let name = file.trim_end_matches(ext);
            let source = self.read(&path)?;
            out.push(match self.layout {
                Layout::Bru => parse_environment(name, &source),
                Layout::Yaml => opencollection::environment(name, &source)
                    .with_context(|| format!("invalid {}", path.display()))?,
            });
        }
        Ok(out)
    }
}

pub fn import_folder(root: &Path) -> Result<Imported> {
    let mut walker = Walker {
        layout: Layout::Bru,
        files: 0,
        ignore: Vec::new(),
        skipped: 0,
    };
    let yaml = root.join("opencollection.yml");
    let mut collection = if yaml.is_file() {
        let config = super::parse_document(&walker.read(&yaml)?)
            .context("opencollection.yml is not valid YAML")?;
        // Un único archivo empaquetado que se llama así.
        if config.get("bundled").and_then(Value::as_bool) == Some(true) {
            return opencollection::import(&config);
        }
        let (collection, ignore) = opencollection::root(&config);
        walker.layout = Layout::Yaml;
        walker.ignore = ignore;
        collection
    } else {
        let config: Value = serde_json::from_str(&walker.read(&root.join("bruno.json"))?)
            .context("bruno.json is not valid JSON")?;
        walker.ignore = array(config.get("ignore"))
            .iter()
            .map(|v| text(Some(v)))
            .collect();
        let mut collection = BruFolder {
            name: text(config.get("name")),
            ..Default::default()
        };
        let meta = root.join("collection.bru");
        if meta.is_file() {
            collection.shared = parse_root(&walker.read(&meta)?).2;
        }
        collection
    };
    if collection.name.trim().is_empty() {
        collection.name = root
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
    }
    walker.dir(root, &mut collection, 0)?;
    let environments = walker.environments(root)?;
    Ok(build(collection, environments, walker.skipped))
}

// ---------- exportación JSON (y OpenCollection en un archivo) ----------

/// Exportación JSON de Bruno u OpenCollection en un único archivo.
pub fn detect(value: &Value) -> bool {
    opencollection::detect(value)
        || value.get("brunoConfig").is_some_and(Value::is_object)
        || (value.get("version").and_then(Value::as_str) == Some("1")
            && value.get("name").is_some_and(Value::is_string)
            && value.get("items").is_some_and(Value::is_array))
}

fn json_pair(entry: &Value) -> Pair {
    Pair {
        key: text(entry.get("name")),
        value: text(entry.get("value")),
        enabled: entry
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    }
}

fn json_pairs(value: Option<&Value>) -> Vec<Pair> {
    array(value).iter().map(json_pair).collect()
}

fn json_shared(request: Option<&Value>, docs: Option<&Value>) -> Shared {
    let field = |key: &str| request.and_then(|r| r.get(key));
    let auth = field("auth");
    let mut attrs = AuthAttrs::new();
    if let Some(Value::Object(map)) = auth {
        for (kind, value) in map {
            if let Value::Object(fields) = value {
                attrs.insert(
                    kind.clone(),
                    fields
                        .iter()
                        .map(|(k, v)| (k.clone(), text(Some(v))))
                        .collect(),
                );
            }
        }
    }
    let script = field("script");
    let vars = field("vars");
    Shared {
        headers: json_pairs(field("headers")),
        auth_mode: text(auth.and_then(|a| a.get("mode"))),
        auth: attrs,
        script_pre: text(script.and_then(|s| s.get("req"))),
        script_post: text(script.and_then(|s| s.get("res"))),
        vars_pre: json_pairs(vars.and_then(|v| v.get("req"))),
        vars_post: json_pairs(vars.and_then(|v| v.get("res"))),
        tests: text(field("tests")),
        docs: text(docs),
    }
}

fn json_request(item: &Value) -> BruRequest {
    let request = item.get("request");
    let field = |key: &str| request.and_then(|r| r.get(key));
    let body = field("body");
    let part = |key: &str| body.and_then(|b| b.get(key));
    let mut query = Vec::new();
    let mut path = Vec::new();
    for param in array(field("params")) {
        if text(param.get("type")) == "path" {
            path.push(json_pair(param));
        } else {
            query.push(json_pair(param));
        }
    }
    // Formato antiguo: `request.query[]`.
    query.extend(json_pairs(field("query")));
    let graphql = part("graphql");
    let method = text(field("method")).to_ascii_uppercase();
    let mut shared = json_shared(request, field("docs"));
    if shared.auth_mode.is_empty() {
        shared.auth_mode = "none".into();
    }
    BruRequest {
        name: text(item.get("name")),
        kind: if text(item.get("type")).starts_with("graphql") {
            "graphql".into()
        } else {
            "http".into()
        },
        seq: item.get("seq").and_then(Value::as_f64).unwrap_or_default(),
        method: if method.is_empty() {
            "GET".into()
        } else {
            method
        },
        url: text(field("url")),
        query: (!query.is_empty()).then_some(query),
        path,
        body_mode: text(part("mode")),
        body: BruBody {
            json: text(part("json")),
            text: text(part("text")),
            xml: text(part("xml")),
            sparql: text(part("sparql")),
            form: json_pairs(part("formUrlEncoded")),
            multipart: array(part("multipartForm"))
                .iter()
                .map(|field| {
                    let mut pair = json_pair(field);
                    if let Some(Value::Array(paths)) = field.get("value") {
                        pair.value = paths
                            .iter()
                            .map(|p| text(Some(p)))
                            .collect::<Vec<_>>()
                            .join("|");
                    }
                    (pair, text(field.get("type")) == "file")
                })
                .collect(),
            graphql_query: text(graphql.and_then(|g| g.get("query"))),
            graphql_vars: text(graphql.and_then(|g| g.get("variables"))),
            file: array(part("file"))
                .iter()
                .map(|f| BruFile {
                    path: text(f.get("filePath")),
                    content_type: text(f.get("contentType")),
                    selected: f.get("selected").and_then(Value::as_bool).unwrap_or(true),
                })
                .collect(),
        },
        asserts: json_pairs(field("assertions")),
        shared,
    }
}

fn json_items(list: &[Value], folder: &mut BruFolder, skipped: &mut usize) {
    for item in list {
        match text(item.get("type")).as_str() {
            "folder" => {
                let root = item.get("root");
                let seq = item
                    .get("seq")
                    .or_else(|| root.and_then(|r| r.get("meta")).and_then(|m| m.get("seq")))
                    .and_then(Value::as_f64)
                    .unwrap_or_default();
                let mut sub = BruFolder {
                    name: text(item.get("name")),
                    seq,
                    shared: json_shared(
                        root.and_then(|r| r.get("request")),
                        root.and_then(|r| r.get("docs")),
                    ),
                    ..Default::default()
                };
                json_items(array(item.get("items")), &mut sub, skipped);
                folder.folders.push(sub);
            }
            // La exportación escribe `http`/`graphql`; la app, `http-request`/`graphql-request`.
            "http" | "graphql" | "http-request" | "graphql-request" => {
                folder.requests.push(json_request(item));
            }
            _ => *skipped += 1,
        }
    }
}

pub fn import_json(value: &Value) -> Result<Imported> {
    if opencollection::detect(value) {
        return opencollection::import(value);
    }
    if !detect(value) {
        bail!("not a Bruno collection export");
    }
    let root = value.get("root");
    let mut collection = BruFolder {
        name: text(value.get("name")),
        shared: json_shared(
            root.and_then(|r| r.get("request")),
            root.and_then(|r| r.get("docs")),
        ),
        ..Default::default()
    };
    let mut skipped = 0;
    json_items(array(value.get("items")), &mut collection, &mut skipped);
    let environments = array(value.get("environments"))
        .iter()
        .map(|env| BruEnv {
            name: text(env.get("name")),
            vars: array(env.get("variables"))
                .iter()
                .map(|var| {
                    let secret = var.get("secret").and_then(Value::as_bool).unwrap_or(false);
                    (json_pair(var), secret)
                })
                .collect(),
        })
        .collect();
    Ok(build(collection, environments, skipped))
}

// ---------- traducción a RunPost ----------

/// Texto como literal de JS (JSON lo es).
fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

/// `'x'` → `x`.
fn unquote(text: &str) -> &str {
    text.strip_prefix('\'')
        .and_then(|t| t.strip_suffix('\''))
        .unwrap_or(text)
}

/// Valor de una aserción: número, booleano, `null`, texto entre comillas o texto suelto.
fn js_value(arg: &str) -> String {
    let arg = arg.trim();
    if arg == "undefined" || serde_json::from_str::<Value>(arg).is_ok() {
        arg.to_owned()
    } else {
        js_string(unquote(arg))
    }
}

/// `200, 201` o `[200, 201]` → `200, 201`.
fn js_list(arg: &str) -> String {
    let arg = arg.trim();
    let inner = arg
        .strip_prefix('[')
        .and_then(|a| a.strip_suffix(']'))
        .unwrap_or(arg);
    inner
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(js_value)
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_ident(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// `'x'` o `"x"` sin comillas ni barras dentro.
fn quoted(text: &str) -> Option<&str> {
    let inner = text
        .strip_prefix('\'')
        .and_then(|t| t.strip_suffix('\''))
        .or_else(|| text.strip_prefix('"').and_then(|t| t.strip_suffix('"')))?;
    (!inner.contains(['\'', '"', '\\'])).then_some(inner)
}

/// `.a.b[0]["c"]`: solo lecturas de propiedades, nada que ejecute código.
fn property_path(mut path: &str) -> bool {
    while !path.is_empty() {
        if let Some(rest) = path.strip_prefix('.') {
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(rest.len());
            if !is_ident(&rest[..end]) {
                return false;
            }
            path = &rest[end..];
        } else if let Some(rest) = path.strip_prefix('[') {
            let Some(end) = rest.find(']') else {
                return false;
            };
            let index = &rest[..end];
            let numeric = !index.is_empty() && index.chars().all(|c| c.is_ascii_digit());
            if !numeric && quoted(index).is_none() {
                return false;
            }
            path = &rest[end + 1..];
        } else {
            return false;
        }
    }
    true
}

/// `res.status` → `pm.response.code`; `res.body.items[0].id` → `pm.response.json().items[0].id`.
fn response_expr(expr: &str) -> Option<String> {
    let expr = expr.trim();
    match expr {
        "res.status" | "res.getStatus()" => return Some("pm.response.code".into()),
        "res.responseTime" | "res.getResponseTime()" => {
            return Some("pm.response.responseTime".into());
        }
        _ => {}
    }
    for prefix in ["res.body", "res.getBody()"] {
        if let Some(path) = expr.strip_prefix(prefix)
            && property_path(path)
        {
            return Some(format!("pm.response.json(){path}"));
        }
    }
    let rest = expr.strip_prefix("res.headers")?;
    let name = match rest.strip_prefix('.') {
        Some(name) if is_ident(name) => name,
        _ => quoted(rest.strip_prefix('[')?.strip_suffix(']')?)?,
    };
    Some(format!("pm.response.headers.get({})", js_string(name)))
}

/// Aserción de Bruno (`res.status: eq 200`) → `pm.test(…)`; `None` si no tiene traducción segura.
fn assert_js(key: &str, rule: &str) -> Option<String> {
    let actual = response_expr(key)?;
    let rule = rule.trim();
    let (op, arg) = rule
        .split_once(char::is_whitespace)
        .map_or((rule, ""), |(op, arg)| (op, arg.trim()));
    if arg.contains("{{") {
        return None;
    }
    let value = || js_value(arg);
    let check = match op {
        "eq" => format!("to.eql({})", value()),
        "neq" => format!("to.not.eql({})", value()),
        "gt" => format!("to.be.above({})", value()),
        "gte" => format!("to.be.at.least({})", value()),
        "lt" => format!("to.be.below({})", value()),
        "lte" => format!("to.be.at.most({})", value()),
        "contains" => format!("to.include({})", value()),
        "notContains" => format!("to.not.include({})", value()),
        "length" => format!("to.have.lengthOf({})", value()),
        "matches" => format!("to.match(new RegExp({}))", js_string(unquote(arg))),
        "notMatches" => format!("to.not.match(new RegExp({}))", js_string(unquote(arg))),
        "startsWith" => format!(
            "to.satisfy((v) => String(v).startsWith({}))",
            js_string(unquote(arg))
        ),
        "endsWith" => format!(
            "to.satisfy((v) => String(v).endsWith({}))",
            js_string(unquote(arg))
        ),
        "in" => format!("to.be.oneOf([{}])", js_list(arg)),
        "notIn" => format!("to.not.be.oneOf([{}])", js_list(arg)),
        "between" => {
            let (low, high) = arg.split_once(',')?;
            format!("to.be.within({}, {})", js_value(low), js_value(high))
        }
        "isEmpty" => "to.be.empty".into(),
        "isNotEmpty" => "to.not.be.empty".into(),
        "isNull" => "to.be.null".into(),
        "isUndefined" => "to.be.undefined".into(),
        "isDefined" => "to.not.be.undefined".into(),
        "isTruthy" => "to.be.ok".into(),
        "isFalsy" => "to.not.be.ok".into(),
        "isNumber" => "to.be.a('number')".into(),
        "isString" => "to.be.a('string')".into(),
        "isBoolean" => "to.be.a('boolean')".into(),
        "isArray" => "to.be.an('array')".into(),
        "isJson" => "to.be.an('object')".into(),
        _ => return None,
    };
    Some(format!(
        "pm.test({}, () => pm.expect({actual}).{check});",
        js_string(&format!("{key}: {rule}"))
    ))
}

/// Valor de una variable posterior a la respuesta (`token: res.body.token`).
fn post_value(expr: &str) -> Option<String> {
    let expr = expr.trim();
    response_expr(expr).or_else(|| {
        serde_json::from_str::<Value>(expr)
            .is_ok()
            .then(|| expr.to_owned())
    })
}

fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

fn add_content_type(headers: &mut Vec<KeyValue>, value: &str) {
    if !headers
        .iter()
        .any(|h| h.key.eq_ignore_ascii_case("content-type"))
    {
        headers.push(KeyValue::new("Content-Type", value));
    }
}

#[derive(Default)]
struct Builder {
    /// Parámetros de ruta sin valor: pasan a variables de la colección.
    path_vars: Vec<String>,
    /// Nombres de `{{process.env.X}}`.
    env_vars: Vec<String>,
    unsupported: Vec<String>,
    commented: usize,
    untranslated: usize,
}

impl Builder {
    /// `{{process.env.X}}` → `{{X}}`.
    fn fix(&mut self, text: &str) -> String {
        if !text.contains("process.env.") {
            return text.to_owned();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                rest = &rest[start..];
                break;
            };
            match after[..end].trim().strip_prefix("process.env.") {
                Some(var) if !var.is_empty() => {
                    if !self.env_vars.iter().any(|v| v == var) {
                        self.env_vars.push(var.to_owned());
                    }
                    out.push_str("{{");
                    out.push_str(var);
                    out.push_str("}}");
                }
                _ => out.push_str(&rest[start..start + end + 4]),
            }
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        out
    }

    fn pairs(&mut self, pairs: &[Pair]) -> Vec<KeyValue> {
        pairs
            .iter()
            .map(|p| KeyValue {
                key: self.fix(&p.key),
                value: self.fix(&p.value),
                enabled: p.enabled,
            })
            .collect()
    }

    /// `None` = heredar (o sin auth en la colección).
    fn auth(&mut self, shared: &Shared, at: &str) -> Option<Auth> {
        let attr = |kind: &str, key: &str| {
            shared
                .auth
                .get(kind)
                .and_then(|attrs| attrs.get(key))
                .cloned()
                .unwrap_or_default()
        };
        Some(match shared.auth_mode.as_str() {
            "" | "inherit" => return None,
            "none" => Auth::None,
            "bearer" => Auth::Bearer {
                token: self.fix(&attr("bearer", "token")),
            },
            "basic" => Auth::Basic {
                username: self.fix(&attr("basic", "username")),
                password: self.fix(&attr("basic", "password")),
            },
            "apikey" => Auth::ApiKey {
                key: self.fix(&attr("apikey", "key")),
                value: self.fix(&attr("apikey", "value")),
                location: if attr("apikey", "placement").eq_ignore_ascii_case("queryparams") {
                    ApiKeyIn::Query
                } else {
                    ApiKeyIn::Header
                },
            },
            "digest" => Auth::Digest {
                username: self.fix(&attr("digest", "username")),
                password: self.fix(&attr("digest", "password")),
            },
            "oauth2" => {
                // `.bru` en snake_case, el JSON en camelCase y OpenCollection con `credentials.*`.
                let pick = |keys: &[&str]| {
                    keys.iter()
                        .map(|key| attr("oauth2", key))
                        .find(|value| !value.is_empty())
                        .unwrap_or_default()
                };
                let client_id = self.fix(&pick(&["client_id", "clientId", "credentials.clientId"]));
                let client_secret = self.fix(&pick(&[
                    "client_secret",
                    "clientSecret",
                    "credentials.clientSecret",
                ]));
                let token_url = self.fix(&pick(&["access_token_url", "accessTokenUrl"]));
                let scope = self.fix(&pick(&["scope"]));
                let client_auth = if pick(&[
                    "credentials_placement",
                    "credentialsPlacement",
                    "credentials.placement",
                ]) == "body"
                {
                    ClientAuth::Body
                } else {
                    ClientAuth::Basic
                };
                match pick(&["grant_type", "grantType", "flow"]).as_str() {
                    "client_credentials" => Auth::oauth2_cc(OAuth2Cc {
                        token_url,
                        client_id,
                        client_secret,
                        scope,
                        client_auth,
                    }),
                    "authorization_code" => {
                        let callback = self.fix(&pick(&["callback_url", "callbackUrl"]));
                        Auth::oauth2_code(OAuth2Code {
                            auth_url: self.fix(&pick(&["authorization_url", "authorizationUrl"])),
                            token_url,
                            client_id,
                            client_secret,
                            scope,
                            redirect_uri: super::local_redirect(
                                &callback,
                                at,
                                &mut self.unsupported,
                            ),
                            client_auth,
                        })
                    }
                    grant => {
                        self.unsupported.push(format!(
                            "{at}: OAuth 2.0 grant {grant:?} not supported (only client credentials and authorization code), imported as none"
                        ));
                        Auth::None
                    }
                }
            }
            other => {
                self.unsupported.push(format!(
                    "{at}: auth {other:?} not supported yet, imported as none"
                ));
                Auth::None
            }
        })
    }

    fn commented(&mut self, code: &str) -> String {
        self.commented += 1;
        let mut out = String::from(
            "// Bruno script (bru/req/res API), kept as a comment: rewrite it with pm.* to run it.\n",
        );
        for line in code.lines() {
            out.push_str("// ");
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Scripts de RunPost: variables y aserciones traducidas; los scripts de Bruno, comentados.
    /// `pre_vars`: las variables previas van al script (en la colección son variables).
    fn scripts(&mut self, shared: &Shared, asserts: &[Pair], pre_vars: bool) -> Scripts {
        let mut pre = String::new();
        if pre_vars {
            for var in shared.vars_pre.iter().filter(|v| v.enabled) {
                let value = self.fix(&var.value);
                pre.push_str(&format!(
                    "pm.variables.set({}, {});\n",
                    js_string(&var.key),
                    js_string(&value)
                ));
            }
        }
        if !shared.script_pre.trim().is_empty() {
            pre.push_str(&self.commented(&shared.script_pre));
        }
        let mut test = String::new();
        for var in shared.vars_post.iter().filter(|v| v.enabled) {
            match post_value(&var.value) {
                Some(value) => test.push_str(&format!(
                    "pm.variables.set({}, {value});\n",
                    js_string(&var.key)
                )),
                None => {
                    self.untranslated += 1;
                    test.push_str(&format!(
                        "// {}: {} (not translated)\n",
                        one_line(&var.key),
                        one_line(&var.value)
                    ));
                }
            }
        }
        for assert in asserts.iter().filter(|a| a.enabled) {
            match assert_js(&assert.key, &assert.value) {
                Some(line) => {
                    test.push_str(&line);
                    test.push('\n');
                }
                None => {
                    self.untranslated += 1;
                    test.push_str(&format!(
                        "// assert {}: {} (not translated)\n",
                        one_line(&assert.key),
                        one_line(&assert.value)
                    ));
                }
            }
        }
        for code in [&shared.script_post, &shared.tests] {
            if !code.trim().is_empty() {
                test.push_str(&self.commented(code));
            }
        }
        Scripts {
            pre: pre.trim_end().to_owned(),
            test: test.trim_end().to_owned(),
        }
    }

    fn body(&mut self, mode: &str, body: &BruBody, headers: &mut Vec<KeyValue>) -> Body {
        match mode {
            "json" => Body::Raw {
                lang: RawLang::Json,
                text: self.fix(&body.json),
            },
            "text" => Body::Raw {
                lang: RawLang::Text,
                text: self.fix(&body.text),
            },
            "xml" => Body::Raw {
                lang: RawLang::Xml,
                text: self.fix(&body.xml),
            },
            "sparql" => {
                add_content_type(headers, "application/sparql-query");
                Body::Raw {
                    lang: RawLang::Text,
                    text: self.fix(&body.sparql),
                }
            }
            "formUrlEncoded" | "form-urlencoded" => Body::Urlencoded {
                fields: self.pairs(&body.form),
            },
            "multipartForm" | "multipart-form" => {
                let mut fields = Vec::new();
                for (pair, file) in &body.multipart {
                    let key = self.fix(&pair.key);
                    if *file {
                        for path in pair.value.split('|').filter(|p| !p.is_empty()) {
                            fields.push(MultipartField {
                                key: key.clone(),
                                value: String::new(),
                                kind: FieldKind::File,
                                path: path.to_owned(),
                                enabled: pair.enabled,
                            });
                        }
                    } else {
                        fields.push(MultipartField {
                            key,
                            value: self.fix(&pair.value),
                            kind: FieldKind::Text,
                            path: String::new(),
                            enabled: pair.enabled,
                        });
                    }
                }
                Body::Multipart { fields }
            }
            "graphql" => Body::Graphql {
                query: self.fix(&body.graphql_query),
                variables: self.fix(&body.graphql_vars),
            },
            "file" => {
                let file = body
                    .file
                    .iter()
                    .find(|f| f.selected)
                    .or_else(|| body.file.first());
                match file {
                    Some(file) => {
                        if !file.content_type.is_empty() {
                            add_content_type(headers, &file.content_type);
                        }
                        Body::Binary {
                            path: file.path.clone(),
                        }
                    }
                    None => Body::None,
                }
            }
            _ => Body::None,
        }
    }

    fn request(&mut self, request: BruRequest, inherited: &[KeyValue], parent: &str) -> Request {
        let BruRequest {
            name,
            method,
            url,
            query,
            path,
            body_mode,
            body,
            asserts,
            shared,
            ..
        } = request;
        let at = format!("{parent}/{name}");
        let url = self.fix(&url);
        let (base, from_url) = split_query(&url);
        let params = match &query {
            Some(pairs) => self.pairs(pairs),
            None => from_url,
        };
        let path: Vec<(String, String)> = path
            .iter()
            .filter(|p| p.enabled)
            .map(|p| (p.key.clone(), self.fix(&p.value)))
            .collect();
        let url = fill_path_params(&base, &path, &mut self.path_vars);
        let own = self.pairs(&shared.headers);
        let mut headers = merge_headers(inherited, own);
        let body = self.body(&body_mode, &body, &mut headers);
        let mut out = Request::new(name, method, url);
        out.params = params;
        out.headers = headers;
        out.body = body;
        out.auth = self.auth(&shared, &at).unwrap_or(Auth::Inherit);
        out.scripts = self.scripts(&shared, &asserts, true);
        out.description = shared.docs;
        out
    }

    /// Carpetas primero y luego peticiones, cada grupo por `seq` y nombre (como Bruno).
    fn items(&mut self, folder: BruFolder, inherited: &[KeyValue], parent: &str) -> Vec<Item> {
        let BruFolder {
            mut folders,
            mut requests,
            ..
        } = folder;
        folders.sort_by(|a, b| a.seq.total_cmp(&b.seq).then_with(|| a.name.cmp(&b.name)));
        requests.sort_by(|a, b| a.seq.total_cmp(&b.seq).then_with(|| a.name.cmp(&b.name)));
        let mut items = Vec::new();
        for sub in folders {
            let at = format!("{parent}/{}", sub.name);
            let own = self.pairs(&sub.shared.headers);
            let headers = merge_headers(inherited, own);
            let mut out = Folder::new(sub.name.clone());
            out.description = sub.shared.docs.clone();
            out.auth = self.auth(&sub.shared, &at);
            out.scripts = self.scripts(&sub.shared, &[], true);
            out.items = self.items(sub, &headers, &at);
            items.push(Item::Folder(out));
        }
        for request in requests {
            items.push(Item::Request(Box::new(
                self.request(request, inherited, parent),
            )));
        }
        items
    }
}

fn build(root: BruFolder, environments: Vec<BruEnv>, skipped: usize) -> Imported {
    let mut b = Builder::default();
    let name = if root.name.trim().is_empty() {
        "Bruno import".to_owned()
    } else {
        root.name.clone()
    };
    let mut collection = Collection::new(name.clone());
    collection.id = stable_id("col", &format!("bruno:{name}"));
    collection.description = root.shared.docs.clone();
    collection.auth = b
        .auth(&root.shared, &name)
        .filter(|auth| *auth != Auth::None);
    for var in &root.shared.vars_pre {
        let mut variable = Variable::new(var.key.clone(), b.fix(&var.value));
        variable.enabled = var.enabled;
        collection.variables.push(variable);
    }
    collection.scripts = b.scripts(&root.shared, &[], false);
    let headers = b.pairs(&root.shared.headers);
    collection.items = b.items(root, &headers, &name);

    let environments: Vec<Environment> = environments
        .into_iter()
        .map(|env| {
            let mut out = Environment::new(format!("{name} - {}", env.name));
            out.id = stable_id("env", &format!("bruno:{name}:{}", env.name));
            for (pair, secret) in env.vars {
                let value = b.fix(&pair.value);
                let mut variable = if secret {
                    Variable::secret(pair.key, value)
                } else {
                    Variable::new(pair.key, value)
                };
                variable.enabled = pair.enabled;
                out.variables.push(variable);
            }
            out
        })
        .collect();

    for var in std::mem::take(&mut b.path_vars) {
        if !collection.variables.iter().any(|v| v.key == var) {
            collection.variables.push(Variable::new(var, ""));
        }
    }
    let mut warnings = std::mem::take(&mut b.unsupported);
    if !b.env_vars.is_empty() {
        for var in &b.env_vars {
            if !collection.variables.iter().any(|v| v.key == *var) {
                collection.variables.push(Variable::secret(var.clone(), ""));
            }
        }
        warnings.push(format!(
            "{} values come from the collection's .env file, which is not imported: fill in the secret variables {}",
            b.env_vars.len(),
            b.env_vars.join(", ")
        ));
    }
    if b.commented > 0 {
        warnings.push(format!(
            "{} Bruno scripts kept as comments: they use Bruno's API (bru, req, res); rewrite them with pm.*",
            b.commented
        ));
    }
    if b.untranslated > 0 {
        warnings.push(format!(
            "{} assertions or post-response variables could not be translated to pm.* and were kept as comments",
            b.untranslated
        ));
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} items skipped: only HTTP and GraphQL requests are imported"
        ));
    }
    Imported {
        collections: vec![collection],
        environments,
        warnings,
    }
}
