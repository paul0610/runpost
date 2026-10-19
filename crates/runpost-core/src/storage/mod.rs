//! Workspace en disco (D5/D6): JSON legible por colección y entorno, historial en SQLite.

pub mod fs;
pub mod history;
pub mod watcher;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::broadcast;

pub use history::{HistoryDb, HistoryEntry, HistorySummary, Snapshot, Source};
pub use watcher::WorkspaceEvent;

use crate::{
    error::CoreError,
    import::{
        self, Format, Imported, bruno,
        postman_v21::{self, PostmanKind},
    },
    model::{
        Collection, Environment, Globals, Item, Request, SCHEMA, SecretStore, Settings, Variable,
        new_id,
    },
    secrets::{self, Keyring},
};

const COLLECTION_EXT: &str = ".collection.json";
const ENVIRONMENT_EXT: &str = ".env.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollectionSummary {
    pub id: String,
    pub name: String,
    /// Nombre del archivo sin `.collection.json`.
    pub slug: String,
    pub folders: usize,
    pub requests: usize,
}

/// Selector de petición: id (`req_…`) o ruta `"Colección/Carpeta/Nombre"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestSelector {
    Id(String),
    Path(String),
}

impl RequestSelector {
    /// `req_…` sin espacios ni `/` se trata como id; cualquier otra cosa, como ruta.
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        if text.starts_with("req_") && !text.contains(['/', ' ']) {
            Self::Id(text.to_owned())
        } else {
            Self::Path(text.to_owned())
        }
    }
}

impl From<&str> for RequestSelector {
    fn from(text: &str) -> Self {
        Self::parse(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub kind: PostmanKind,
    /// Formato de origen: `postman_v21`, `openapi`, `har`, `insomnia` o `bruno`.
    pub format: &'static str,
    pub id: String,
    pub name: String,
    pub folders: usize,
    pub requests: usize,
    pub variables: usize,
    /// Ruta relativa al workspace del archivo escrito.
    pub file: String,
    /// `true` si sustituyó una colección/entorno con el mismo id.
    pub replaced: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
pub struct Workspace {
    root: PathBuf,
    history: Arc<HistoryDb>,
    watch: Arc<Mutex<Option<watcher::WatchState>>>,
    keyring: Arc<dyn Keyring>,
}

fn in_keyring(var: &Variable) -> bool {
    var.secret && var.store == Some(SecretStore::Keyring)
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("root", &self.root)
            .finish()
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    if let Some(found) = value.get("schema").and_then(serde_json::Value::as_u64)
        && found != u64::from(SCHEMA)
    {
        return Err(CoreError::UnsupportedSchema {
            file: path.display().to_string(),
            found: found as u32,
        }
        .into());
    }
    serde_json::from_value(value).with_context(|| format!("invalid content in {}", path.display()))
}

/// Archivos `*.ext` de `dir`, ordenados por nombre, excluyendo temporales ocultos.
fn files_with(dir: &Path, ext: &str) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Some(slug) = name.strip_suffix(ext)
            && !name.starts_with('.')
        {
            files.push((slug.to_owned(), path));
        }
    }
    files.sort();
    Ok(files)
}

/// Carga todos los documentos válidos; los corruptos se avisan y se saltan.
fn load_all<T: DeserializeOwned>(dir: &Path, ext: &str) -> Result<Vec<(String, PathBuf, T)>> {
    let mut out = Vec::new();
    for (slug, path) in files_with(dir, ext)? {
        match read_json::<T>(&path) {
            Ok(doc) => out.push((slug, path, doc)),
            Err(error) => tracing::warn!("skipping {}: {error:#}", path.display()),
        }
    }
    Ok(out)
}

/// Elige `<slug>.ext` libre (o el que ya ocupa el mismo id): `slug`, `slug-2`, `slug-3`…
fn free_path(dir: &Path, slug: &str, ext: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{slug}{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{slug}-{n}{ext}"));
        n += 1;
    }
    candidate
}

/// Coincidencia única por nombre/id: exacta, luego sin distinguir mayúsculas.
/// Por niveles: exacto → sin mayúsculas → mismo slug (`"Acme Gateway Scopes"` encuentra
/// `acme-gateway-scopes`) → subcadena única (`"fake"` encuentra `"Acme DESA (fake)"`). Si un
/// nivel da varios resultados, es ambiguo y no se sigue buscando.
fn pick<'a, T>(
    kind: &'static str,
    selector: &str,
    items: &'a [T],
    keys: impl Fn(&T) -> Vec<&str>,
) -> Result<&'a T> {
    if selector.trim().is_empty() {
        return Err(CoreError::not_found(kind, selector).into());
    }
    let lower = selector.to_lowercase();
    let slug = fs::slugify(selector);
    let tiers: [&dyn Fn(&str) -> bool; 4] = [
        &|key| key == selector,
        &|key| key.to_lowercase() == lower,
        &|key| fs::slugify(key) == slug,
        &|key| key.to_lowercase().contains(&lower),
    ];
    for tier in tiers {
        let pool: Vec<&T> = items
            .iter()
            .filter(|item| keys(item).into_iter().any(tier))
            .collect();
        match pool.as_slice() {
            [one] => return Ok(one),
            [] => {}
            many => {
                return Err(CoreError::Ambiguous {
                    kind,
                    selector: selector.into(),
                    candidates: many.len(),
                }
                .into());
            }
        }
    }
    Err(CoreError::not_found(kind, selector).into())
}

impl Workspace {
    /// Crea la estructura si no existe; nunca sobrescribe archivos del usuario. Los secretos
    /// `store: keyring` van al llavero que elige `RUNPOST_KEYRING` (el del sistema por defecto).
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_keyring(root, secrets::from_env())
    }

    /// Como `open` con un llavero concreto (las pruebas usan `MemoryKeyring`).
    pub fn open_with_keyring(root: &Path, keyring: Arc<dyn Keyring>) -> Result<Self> {
        std::fs::create_dir_all(root).context("cannot create workspace")?;
        let root = root.canonicalize().context("cannot resolve workspace")?;
        for directory in ["collections", "environments", "logs"] {
            std::fs::create_dir_all(root.join(directory))
                .context("cannot create workspace directory")?;
        }
        Self::create_if_missing(&root.join("settings.json"), &Settings::default())?;
        Self::create_if_missing(&root.join("globals.json"), &Globals::default())?;
        let history = HistoryDb::open(&root.join("history.db"))?;
        Ok(Self {
            root,
            history: Arc::new(history),
            watch: Arc::new(Mutex::new(None)),
            keyring,
        })
    }

    // ---- llavero ----

    /// Publica en el llavero los datos de conexión del MCP por HTTP en `port` (`session_account`).
    pub fn publish_session(&self, port: u16, value: &str) -> Result<(), CoreError> {
        self.keyring.set(&secrets::session_account(port), value)
    }

    /// Al cerrar: borra la entrada de `port` solo si sigue siendo la de este arranque (`token`);
    /// si otra instancia la sustituyó, no se toca.
    pub fn clear_session(&self, port: u16, token: &str) {
        let account = secrets::session_account(port);
        let ours = match self.keyring.get(&account) {
            Ok(Some(text)) => serde_json::from_str::<serde_json::Value>(&text)
                .is_ok_and(|value| value["token"].as_str() == Some(token)),
            Ok(None) => false,
            Err(error) => {
                tracing::warn!("keyring: session not read: {error}");
                false
            }
        };
        if ours && let Err(error) = self.keyring.delete(&account) {
            tracing::warn!("keyring: session not cleared: {error}");
        }
    }

    /// Lo publicado por `publish_session` para `port`, si la app está abierta.
    pub fn session(&self, port: u16) -> Result<Option<String>, CoreError> {
        self.keyring.get(&secrets::session_account(port))
    }

    /// Rellena el valor de los secretos `store: keyring` con lo que guarda el llavero. Si el
    /// llavero falla, la variable queda sin valor (como en Postman, `{{nombre}}` sin resolver).
    fn hydrate(&self, owner: &str, vars: &mut [Variable]) {
        for var in vars
            .iter_mut()
            .filter(|v| in_keyring(v) && v.value.is_none())
        {
            match self.keyring.get(&secrets::account(owner, &var.key)) {
                Ok(value) => var.value = value,
                Err(error) => tracing::warn!(key = %var.key, "keyring read failed: {error}"),
            }
        }
    }

    /// Guarda en el llavero los secretos `store: keyring` y devuelve las variables tal como van al
    /// archivo (sin su valor). Borra del llavero los que estaban en `before` y ya no están ahí.
    fn stash(&self, owner: &str, vars: &[Variable], before: &[Variable]) -> Result<Vec<Variable>> {
        let mut stored = Vec::with_capacity(vars.len());
        for var in vars {
            let mut var = var.clone();
            if in_keyring(&var) {
                if let Some(value) = var.value.take() {
                    self.keyring
                        .set(&secrets::account(owner, &var.key), &value)?;
                }
            } else if var.store == Some(SecretStore::Keyring) {
                // `store: keyring` sin `secret` no tiene sentido: es una variable normal.
                var.store = None;
            }
            stored.push(var);
        }
        for old in before.iter().filter(|v| in_keyring(v)) {
            if !stored.iter().any(|v| v.key == old.key && in_keyring(v))
                && let Err(error) = self.keyring.delete(&secrets::account(owner, &old.key))
            {
                tracing::warn!(key = %old.key, "keyring delete failed: {error}");
            }
        }
        Ok(stored)
    }

    fn create_if_missing<T: Serialize>(path: &Path, value: &T) -> Result<()> {
        let mut text = serde_json::to_string_pretty(value)?;
        text.push('\n');
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                std::io::Write::write_all(&mut file, text.as_bytes())?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error).context("cannot initialize workspace"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// El llavero del workspace (el del sistema o, en pruebas, uno en memoria).
    pub fn keyring(&self) -> Arc<dyn Keyring> {
        Arc::clone(&self.keyring)
    }

    fn collections_dir(&self) -> PathBuf {
        self.root.join("collections")
    }

    fn environments_dir(&self) -> PathBuf {
        self.root.join("environments")
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    // ---- settings / globals ----

    /// Lee `settings.json`; si falta o es inválido, avisa y usa los valores por defecto.
    pub fn settings(&self) -> Settings {
        read_json(&self.root.join("settings.json")).unwrap_or_else(|error| {
            tracing::warn!("settings.json: {error:#}; using defaults");
            Settings::default()
        })
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        fs::write_json(&self.root.join("settings.json"), settings)
    }

    fn globals_on_disk(&self) -> Result<Globals> {
        let path = self.root.join("globals.json");
        if !path.exists() {
            return Ok(Globals::default());
        }
        read_json(&path)
    }

    pub fn globals(&self) -> Result<Globals> {
        let mut globals = self.globals_on_disk()?;
        let owner = globals.id.clone();
        self.hydrate(&owner, &mut globals.variables);
        Ok(globals)
    }

    pub fn save_globals(&self, globals: &Globals) -> Result<()> {
        let before = self.globals_on_disk().unwrap_or_default();
        let mut stored = globals.clone();
        if stored.id.is_empty() {
            stored.id = before.id.clone();
        }
        if stored.id.is_empty() && stored.variables.iter().any(in_keyring) {
            stored.id = new_id("glb");
        }
        stored.variables = self.stash(&stored.id, &globals.variables, &before.variables)?;
        fs::write_json(&self.root.join("globals.json"), &stored)
    }

    // ---- colecciones ----

    fn collection_files(&self) -> Result<Vec<(String, PathBuf, Collection)>> {
        load_all(&self.collections_dir(), COLLECTION_EXT)
    }

    pub fn collections(&self) -> Result<Vec<CollectionSummary>> {
        Ok(self
            .collection_files()?
            .into_iter()
            .map(|(slug, _, c)| CollectionSummary {
                folders: c.folder_count(),
                requests: c.request_count(),
                id: c.id,
                name: c.name,
                slug,
            })
            .collect())
    }

    pub fn load_collection(&self, id: &str) -> Result<Collection> {
        self.collection_files()?
            .into_iter()
            .find(|(_, _, c)| c.id == id)
            .map(|(_, _, c)| c)
            .ok_or_else(|| CoreError::not_found("collection", id).into())
    }

    /// Por id, nombre o slug de archivo (exacto, luego sin mayúsculas).
    pub fn find_collection(&self, selector: &str) -> Result<Collection> {
        let files = self.collection_files()?;
        let (_, _, found) = pick("collection", selector.trim(), &files, |(slug, _, c)| {
            vec![c.id.as_str(), c.name.as_str(), slug.as_str()]
        })?;
        Ok(found.clone())
    }

    pub fn save_collection(&self, collection: &Collection) -> Result<()> {
        self.save_collection_as(collection, None).map(|_| ())
    }

    /// Guarda en el archivo que ya tiene ese id o, si es nueva, en `<slug>.collection.json`
    /// (slug de `preferred_slug` o del nombre). Devuelve la ruta escrita.
    pub fn save_collection_as(
        &self,
        collection: &Collection,
        preferred_slug: Option<&str>,
    ) -> Result<PathBuf> {
        let dir = self.collections_dir();
        let existing = files_with(&dir, COLLECTION_EXT)?
            .into_iter()
            .find(|(_, path)| {
                read_json::<serde_json::Value>(path)
                    .ok()
                    .is_some_and(|v| v.get("id").and_then(|id| id.as_str()) == Some(&collection.id))
            })
            .map(|(_, path)| path);
        let path = existing.unwrap_or_else(|| {
            let slug = fs::slugify(preferred_slug.unwrap_or(&collection.name));
            free_path(&dir, &slug, COLLECTION_EXT)
        });
        fs::write_json(&path, collection)?;
        Ok(path)
    }

    /// Borra el archivo de la colección con ese id.
    pub fn delete_collection(&self, id: &str) -> Result<()> {
        let (_, path, _) = self
            .collection_files()?
            .into_iter()
            .find(|(_, _, c)| c.id == id)
            .ok_or_else(|| CoreError::not_found("collection", id))?;
        std::fs::remove_file(&path).with_context(|| format!("cannot delete {}", path.display()))
    }

    // ---- entornos ----

    pub fn environments(&self) -> Result<Vec<Environment>> {
        Ok(load_all(&self.environments_dir(), ENVIRONMENT_EXT)?
            .into_iter()
            .map(|(_, _, mut env): (_, _, Environment)| {
                self.hydrate(&env.id, &mut env.variables);
                env
            })
            .collect())
    }

    /// Por id o nombre (exacto, luego sin mayúsculas).
    pub fn find_environment(&self, selector: &str) -> Result<Environment> {
        let envs = self.environments()?;
        let found = pick("env", selector.trim(), &envs, |e| {
            vec![e.id.as_str(), e.name.as_str()]
        })?;
        Ok(found.clone())
    }

    /// Entorno pedido o, si no se indica, `settings.active_environment`.
    pub fn active_environment(&self, selector: Option<&str>) -> Result<Option<Environment>> {
        match selector.filter(|s| !s.trim().is_empty()) {
            Some(selector) => self.find_environment(selector).map(Some),
            None => match self.settings().active_environment {
                Some(active) => match self.find_environment(&active) {
                    Ok(env) => Ok(Some(env)),
                    Err(error) => {
                        tracing::warn!("active environment ignored: {error:#}");
                        Ok(None)
                    }
                },
                None => Ok(None),
            },
        }
    }

    pub fn save_environment(&self, env: &Environment) -> Result<()> {
        self.save_environment_as(env, None).map(|_| ())
    }

    pub fn save_environment_as(
        &self,
        env: &Environment,
        preferred_slug: Option<&str>,
    ) -> Result<PathBuf> {
        let dir = self.environments_dir();
        let existing = load_all::<Environment>(&dir, ENVIRONMENT_EXT)?
            .into_iter()
            .find(|(_, _, e)| e.id == env.id);
        let before = existing
            .as_ref()
            .map(|(_, _, e)| e.variables.as_slice())
            .unwrap_or_default();
        // Primero el llavero: si falla, el archivo queda como estaba.
        let stored = Environment {
            variables: self.stash(&env.id, &env.variables, before)?,
            ..env.clone()
        };
        let path = existing.map(|(_, path, _)| path).unwrap_or_else(|| {
            let slug = fs::slugify(preferred_slug.unwrap_or(&env.name));
            free_path(&dir, &slug, ENVIRONMENT_EXT)
        });
        fs::write_json(&path, &stored)?;
        Ok(path)
    }

    pub fn delete_environment(&self, id: &str) -> Result<()> {
        let (_, path, env) = load_all::<Environment>(&self.environments_dir(), ENVIRONMENT_EXT)?
            .into_iter()
            .find(|(_, _, e)| e.id == id)
            .ok_or_else(|| CoreError::not_found("env", id))?;
        std::fs::remove_file(&path).with_context(|| format!("cannot delete {}", path.display()))?;
        self.stash(id, &[], &env.variables)?;
        Ok(())
    }

    // ---- peticiones ----

    /// Por id o por ruta `"Colección/Carpeta/Nombre"`: exacta → sin mayúsculas → prefijo único.
    /// Además (para MCP, 8.1) acepta un sufijo único de la ruta (`"00.2 Gateway - ping"`) y, por último,
    /// un prefijo único de cualquier cola de la ruta (`"00.2 Gateway"`, `"Carpeta/#7b"`).
    pub fn find_request(&self, selector: &RequestSelector) -> Result<(Collection, Request)> {
        let collections: Vec<Collection> = self
            .collection_files()?
            .into_iter()
            .map(|(_, _, c)| c)
            .collect();
        match selector {
            RequestSelector::Id(id) => {
                for collection in &collections {
                    let mut found = None;
                    collection.walk_requests(|_, request| {
                        if found.is_none() && request.id == *id {
                            found = Some(request.clone());
                        }
                    });
                    if let Some(request) = found {
                        return Ok((collection.clone(), request));
                    }
                }
                Err(CoreError::not_found("request", id).into())
            }
            RequestSelector::Path(wanted) => {
                let mut paths = Vec::new();
                for (index, collection) in collections.iter().enumerate() {
                    for (path, request) in collection.request_paths() {
                        paths.push((path, index, request.clone()));
                    }
                }
                let lower = wanted.to_lowercase();
                let suffix = format!("/{lower}");
                let tiers: [&dyn Fn(&str) -> bool; 5] = [
                    &|p| p == wanted.as_str(),
                    &|p| p.to_lowercase() == lower,
                    &|p| p.to_lowercase().starts_with(&lower),
                    &|p| p.to_lowercase().ends_with(&suffix),
                    // Prefijo único de la cola de la ruta: `"00.1 API"`, `"Carpeta/#7b"`.
                    &|p| {
                        let p = p.to_lowercase();
                        p.match_indices('/')
                            .any(|(i, _)| p[i + 1..].starts_with(&lower))
                    },
                ];
                let mut ambiguous = 0;
                for tier in tiers {
                    let hits: Vec<_> = paths.iter().filter(|(p, _, _)| tier(p)).collect();
                    match hits.as_slice() {
                        [(_, index, request)] => {
                            return Ok((collections[*index].clone(), request.clone()));
                        }
                        [] => {}
                        many => ambiguous = ambiguous.max(many.len()),
                    }
                }
                if ambiguous > 0 {
                    Err(CoreError::Ambiguous {
                        kind: "request",
                        selector: wanted.clone(),
                        candidates: ambiguous,
                    }
                    .into())
                } else {
                    Err(CoreError::not_found("request", wanted).into())
                }
            }
        }
    }

    pub fn history(&self) -> &HistoryDb {
        &self.history
    }

    /// Suscripción a cambios en disco (incluidos los de otro proceso, p. ej. `runpost mcp`).
    /// El vigilante se crea con la primera suscripción y vive mientras viva el `Workspace`.
    pub fn watch(&self) -> broadcast::Receiver<WorkspaceEvent> {
        let mut state = self
            .watch
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.is_none() {
            match watcher::start(&self.root) {
                Ok(started) => *state = Some(started),
                Err(error) => {
                    tracing::warn!("workspace watcher unavailable: {error:#}");
                    let (sender, receiver) = broadcast::channel(1);
                    drop(sender);
                    return receiver;
                }
            }
        }
        state
            .as_ref()
            .map(|s| s.sender.subscribe())
            .expect("watcher started")
    }

    // ---- import ----

    /// Importa una colección o entorno Postman v2.1 (`kind` se detecta si es `None`).
    /// Reimportar la misma colección (mismo `_postman_id`) la sustituye y conserva los ids
    /// de las carpetas y peticiones cuya ruta no cambió.
    pub fn import_postman(
        &self,
        text: &str,
        kind: Option<PostmanKind>,
        preferred_slug: Option<&str>,
    ) -> Result<ImportReport> {
        let value: serde_json::Value = serde_json::from_str(text).context("invalid JSON")?;
        let kind = match kind {
            Some(kind) => kind,
            None => postman_v21::detect(&value)
                .context("not a Postman v2.1 collection or environment")?,
        };
        match kind {
            PostmanKind::Collection => {
                let imported = postman_v21::import_collection_value(&value)?;
                self.save_imported_collection(
                    imported.collection,
                    imported.warnings,
                    Format::Postman,
                    preferred_slug,
                )
            }
            PostmanKind::Environment => self.save_imported_environment(
                postman_v21::import_environment_value(&value)?,
                Format::Postman,
                preferred_slug,
            ),
        }
    }

    /// Como `import_postman` leyendo un archivo; el slug sale del nombre del archivo
    /// (`Acme-Gateway-Scopes.postman_collection.json` → `acme-gateway-scopes`).
    pub fn import_postman_file(
        &self,
        file: &Path,
        kind: Option<PostmanKind>,
    ) -> Result<ImportReport> {
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("cannot read {}", file.display()))?;
        self.import_postman(&text, kind, Some(&import_stem(file)))
    }

    /// Importa un texto en cualquier formato (fase 4): Postman v2.1, OpenAPI 3, HAR, Insomnia o
    /// Bruno (JSON); se detecta si `format` es `None`. Un comando curl no: es una petición suelta
    /// (`import::curl::parse`) que la app abre en una pestaña.
    pub fn import(
        &self,
        text: &str,
        format: Option<Format>,
        kind: Option<PostmanKind>,
        preferred_slug: Option<&str>,
    ) -> Result<Vec<ImportReport>> {
        let format = match format {
            Some(format) => format,
            None => import::detect(text).context(
                "unknown format: expected Postman v2.1, OpenAPI 3, HAR, Insomnia, Bruno or curl",
            )?,
        };
        let imported = match format {
            Format::Postman => return Ok(vec![self.import_postman(text, kind, preferred_slug)?]),
            Format::Curl => {
                return Err(CoreError::InvalidRequest(
                    "a curl command is a single request: open it in a tab".into(),
                )
                .into());
            }
            Format::Openapi => import::openapi::import(&import::parse_document(text)?)?,
            Format::Har => import::har::import(&import::parse_document(text)?)?,
            Format::Insomnia => import::insomnia::import(&import::parse_document(text)?)?,
            Format::Bruno => bruno::import_json(&import::parse_document(text)?)?,
        };
        self.save_imported(imported, format, preferred_slug)
    }

    /// Como `import` desde una ruta. Una carpeta de colección de Bruno (o su `bruno.json`) se
    /// importa entera.
    pub fn import_path(
        &self,
        path: &Path,
        format: Option<Format>,
        kind: Option<PostmanKind>,
    ) -> Result<Vec<ImportReport>> {
        if let Some(root) = bruno::collection_root(path) {
            let imported = bruno::import_folder(&root)?;
            let slug = root.file_name().map(|n| n.to_string_lossy().into_owned());
            return self.save_imported(imported, Format::Bruno, slug.as_deref());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        self.import(&text, format, kind, Some(&import_stem(path)))
    }

    /// Guarda lo importado; los avisos van en el primer informe.
    fn save_imported(
        &self,
        imported: Imported,
        format: Format,
        preferred_slug: Option<&str>,
    ) -> Result<Vec<ImportReport>> {
        let Imported {
            collections,
            environments,
            mut warnings,
        } = imported;
        let single = collections.len() == 1;
        let mut reports = Vec::new();
        for collection in collections {
            let slug = if single { preferred_slug } else { None };
            reports.push(self.save_imported_collection(
                collection,
                std::mem::take(&mut warnings),
                format,
                slug,
            )?);
        }
        for environment in environments {
            reports.push(self.save_imported_environment(environment, format, None)?);
        }
        if let Some(first) = reports.first_mut() {
            first.warnings.append(&mut warnings);
        }
        Ok(reports)
    }

    /// Reimportar la misma colección (mismo id) la sustituye y conserva los ids de las carpetas y
    /// peticiones cuya ruta no cambió.
    fn save_imported_collection(
        &self,
        mut collection: Collection,
        warnings: Vec<String>,
        format: Format,
        preferred_slug: Option<&str>,
    ) -> Result<ImportReport> {
        let previous = self.load_collection(&collection.id).ok();
        if let Some(previous) = &previous {
            reuse_ids(previous, &mut collection);
        }
        let path = self.save_collection_as(&collection, preferred_slug)?;
        Ok(ImportReport {
            kind: PostmanKind::Collection,
            format: format.as_str(),
            folders: collection.folder_count(),
            requests: collection.request_count(),
            variables: collection.variables.len(),
            id: collection.id,
            name: collection.name,
            file: self.relative(&path),
            replaced: previous.is_some(),
            warnings,
        })
    }

    fn save_imported_environment(
        &self,
        mut env: Environment,
        format: Format,
        preferred_slug: Option<&str>,
    ) -> Result<ImportReport> {
        let previous = self.find_environment(&env.id).ok();
        // Reimportar no saca del llavero lo que el usuario ya movió ahí.
        if let Some(previous) = &previous {
            for var in env.variables.iter_mut().filter(|v| v.secret) {
                if previous
                    .variables
                    .iter()
                    .any(|old| old.key == var.key && in_keyring(old))
                {
                    var.store = Some(SecretStore::Keyring);
                }
            }
        }
        let replaced = previous.is_some();
        let path = self.save_environment_as(&env, preferred_slug)?;
        Ok(ImportReport {
            kind: PostmanKind::Environment,
            format: format.as_str(),
            folders: 0,
            requests: 0,
            variables: env.variables.len(),
            id: env.id,
            name: env.name,
            file: self.relative(&path),
            replaced,
            warnings: Vec::new(),
        })
    }
}

/// Slug de un archivo importado: `Acme-Gateway-Scopes.postman_collection.json` → `Acme-Gateway-Scopes`.
pub fn import_stem(file: &Path) -> String {
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    [
        ".postman_collection.json",
        ".postman_environment.json",
        ".openapi.json",
        ".openapi.yaml",
        ".insomnia.json",
        ".insomnia.yaml",
        ".insomnia.yml",
        ".json",
        ".yaml",
        ".yml",
        ".har",
    ]
    .iter()
    .find_map(|ext| name.strip_suffix(ext))
    .unwrap_or(&name)
    .to_owned()
}

/// Copia los ids de `previous` a `fresh` para los elementos con la misma ruta.
fn reuse_ids(previous: &Collection, fresh: &mut Collection) {
    fn index(items: &[Item], prefix: &str, out: &mut HashMap<String, String>) {
        for item in items {
            let key = format!(
                "{prefix}/{}:{}",
                item.name(),
                matches!(item, Item::Folder(_))
            );
            out.entry(key.clone())
                .or_insert_with(|| item.id().to_owned());
            if let Item::Folder(folder) = item {
                index(&folder.items, &key, out);
            }
        }
    }
    fn apply(items: &mut [Item], prefix: &str, ids: &HashMap<String, String>) {
        for item in items {
            let key = format!(
                "{prefix}/{}:{}",
                item.name(),
                matches!(item, Item::Folder(_))
            );
            if let Some(id) = ids.get(&key) {
                match item {
                    Item::Folder(folder) => folder.id = id.clone(),
                    Item::Request(request) => request.id = id.clone(),
                }
            }
            if let Item::Folder(folder) = item {
                apply(&mut folder.items, &key, ids);
            }
        }
    }
    let mut ids = HashMap::new();
    index(&previous.items, "", &mut ids);
    apply(&mut fresh.items, "", &ids);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Folder, Variable};

    fn sample() -> Collection {
        let mut collection = Collection::new("Acme Gateway");
        let mut folder = Folder::new("00 - Diagnóstico");
        let mut ping = Request::new("00.2 Gateway - ping", "GET", "{{gatewayUrl}}/v1/ping");
        ping.id = "req_ping".into();
        folder.items.push(Item::Request(Box::new(ping)));
        folder.items.push(Item::Request(Box::new(Request::new(
            "#5 POST keys/local",
            "POST",
            "{{x}}/k",
        ))));
        collection.items.push(Item::Folder(folder));
        collection.items.push(Item::Request(Box::new(Request::new(
            "00.2 Gateway - ping v2",
            "GET",
            "{{x}}/p",
        ))));
        collection
    }

    #[test]
    fn workspace_precedence_and_no_overwrite() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        assert!(ws.root().join("collections").is_dir());
        assert!(ws.root().join("history.db").is_file());
        std::fs::write(ws.root().join("settings.json"), "preserved")?;
        Workspace::open(dir.path())?;
        assert_eq!(
            std::fs::read_to_string(ws.root().join("settings.json"))?,
            "preserved"
        );
        // Un settings.json corrupto no rompe la lectura.
        assert_eq!(ws.settings(), Settings::default());
        Ok(())
    }

    #[test]
    fn settings_and_globals_round_trip() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        let mut settings = ws.settings();
        settings.active_environment = Some("env_1".into());
        ws.save_settings(&settings)?;
        assert_eq!(ws.settings(), settings);
        let mut globals = ws.globals()?;
        globals.variables.push(Variable::new("g", "1"));
        ws.save_globals(&globals)?;
        assert_eq!(ws.globals()?, globals);
        Ok(())
    }

    #[test]
    fn collections_save_load_and_slug_collisions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        let mut first = sample();
        ws.save_collection(&first)?;
        let second = Collection::new("Acme Gateway");
        let path = ws.save_collection_as(&second, None)?;
        assert!(path.ends_with("acme-gateway-2.collection.json"));
        first.description = "updated".into();
        let again = ws.save_collection_as(&first, Some("ignored"))?;
        assert!(
            again.ends_with("acme-gateway.collection.json"),
            "same id keeps its file"
        );
        assert_eq!(ws.load_collection(&first.id)?.description, "updated");
        let summaries = ws.collections()?;
        assert_eq!(summaries.len(), 2);
        assert_eq!((summaries[0].folders, summaries[0].requests), (1, 3));
        assert_eq!(ws.find_collection("acme-gateway-2")?.id, second.id);
        let ambiguous = ws.find_collection("Acme Gateway").unwrap_err();
        assert!(matches!(
            ambiguous.downcast_ref(),
            Some(CoreError::Ambiguous { .. })
        ));
        let missing = ws.load_collection("col_none").unwrap_err();
        assert_eq!(
            missing.downcast_ref::<CoreError>().unwrap().code(),
            "collection_not_found"
        );
        Ok(())
    }

    #[test]
    fn delete_collection_and_environment() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        let collection = sample();
        ws.save_collection(&collection)?;
        let env = Environment::new("E");
        ws.save_environment(&env)?;
        ws.delete_collection(&collection.id)?;
        ws.delete_environment(&env.id)?;
        assert!(ws.collections()?.is_empty() && ws.environments()?.is_empty());
        let missing = ws.delete_collection(&collection.id).unwrap_err();
        assert_eq!(
            missing.downcast_ref::<CoreError>().unwrap().code(),
            "collection_not_found"
        );
        assert!(ws.delete_environment(&env.id).is_err());
        Ok(())
    }

    #[test]
    fn corrupt_and_future_schema_files_are_skipped() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        ws.save_collection(&sample())?;
        std::fs::write(ws.root().join("collections/broken.collection.json"), "{")?;
        std::fs::write(
            ws.root().join("collections/future.collection.json"),
            r#"{"schema":2,"id":"col_f","name":"F"}"#,
        )?;
        assert_eq!(ws.collections()?.len(), 1);
        let error = read_json::<Collection>(&ws.root().join("collections/future.collection.json"))
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<CoreError>().unwrap().code(),
            "unsupported_schema"
        );
        Ok(())
    }

    #[test]
    fn find_request_tiers() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        ws.save_collection(&sample())?;
        let name =
            |s: &str| -> Result<String> { Ok(ws.find_request(&RequestSelector::parse(s))?.1.name) };
        assert_eq!(name("req_ping")?, "00.2 Gateway - ping");
        assert_eq!(
            name("Acme Gateway/00 - Diagnóstico/00.2 Gateway - ping")?,
            "00.2 Gateway - ping"
        );
        assert_eq!(
            name("acme gateway/00 - diagnóstico/00.2 gateway - PING")?,
            "00.2 Gateway - ping"
        );
        assert_eq!(
            name("Acme Gateway/00 - Diagnóstico/#5")?,
            "#5 POST keys/local"
        );
        // Nombre con "/" dentro: la ruta se compara entera.
        assert_eq!(
            name("00 - Diagnóstico/#5 POST keys/local")?,
            "#5 POST keys/local"
        );
        // Solo el nombre (sufijo único), como en `run_request("00.2 Gateway - ping")`.
        assert_eq!(name("00.2 Gateway - ping")?, "00.2 Gateway - ping");
        assert_eq!(name("00.2 gateway - ping v2")?, "00.2 Gateway - ping v2");
        assert_eq!(name("#5 POST")?, "#5 POST keys/local");
        assert_eq!(name("00 - Diagnóstico/#5")?, "#5 POST keys/local");
        let ambiguous = ws.find_request(&"Acme Gateway/".into()).unwrap_err();
        assert!(matches!(
            ambiguous.downcast_ref(),
            Some(CoreError::Ambiguous { candidates: 3, .. })
        ));
        let missing = ws.find_request(&"req_nope".into()).unwrap_err();
        assert_eq!(
            missing.downcast_ref::<CoreError>().unwrap().code(),
            "request_not_found"
        );
        let missing = ws.find_request(&"nope".into()).unwrap_err();
        assert_eq!(
            missing.downcast_ref::<CoreError>().unwrap().code(),
            "request_not_found"
        );
        Ok(())
    }

    #[test]
    fn environments_and_active_selection() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        assert_eq!(ws.active_environment(None)?, None);
        let mut env = Environment::new("Acme DESA");
        env.variables.push(Variable::secret("accessToken", "fake"));
        ws.save_environment(&env)?;
        assert!(ws.root().join("environments/acme-desa.env.json").is_file());
        assert_eq!(ws.find_environment("acme desa")?.id, env.id);
        assert_eq!(
            ws.active_environment(Some(&env.id))?.unwrap().name,
            "Acme DESA"
        );
        let err = ws.active_environment(Some("nope")).unwrap_err();
        assert_eq!(
            err.downcast_ref::<CoreError>().unwrap().code(),
            "env_not_found"
        );
        let mut settings = ws.settings();
        settings.active_environment = Some("Acme DESA".into());
        ws.save_settings(&settings)?;
        assert_eq!(ws.active_environment(None)?.unwrap().id, env.id);
        settings.active_environment = Some("deleted".into());
        ws.save_settings(&settings)?;
        assert_eq!(ws.active_environment(None)?, None);
        Ok(())
    }

    #[test]
    fn watch_reports_external_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let ws = Workspace::open(dir.path())?;
        let mut events = ws.watch();
        ws.save_collection(&sample())?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let expected = WorkspaceEvent::Collection {
            slug: "acme-gateway".into(),
        };
        let mut seen = false;
        while std::time::Instant::now() < deadline && !seen {
            match events.try_recv() {
                Ok(event) => seen = event == expected,
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        assert!(seen, "collection change event received");
        Ok(())
    }
}
