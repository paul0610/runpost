//! Papelera (fase 4): borrar una colección, carpeta, petición o entorno lo deja en `.trash/` del
//! workspace, con dónde estaba, para restaurarlo ("Deshacer" en la app o desde la papelera). Lo de
//! más de 30 días se vacía solo. `.trash/.gitignore` la deja fuera de Git.
//!
//! Un entorno va a la papelera tal como está en su archivo: sus secretos del llavero no se copian y
//! se quedan en el llavero hasta vaciarla, para que restaurarlo lo deje como estaba.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{ENVIRONMENT_EXT, Workspace, fs, in_keyring, load_all, read_json};
use crate::{
    error::CoreError,
    model::{Collection, Environment, Item, Position, SCHEMA, new_id},
    secrets,
};

/// Lo de más de 30 días se borra solo.
const KEEP_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Trashed {
    Collection {
        collection: Collection,
    },
    /// Carpeta o petición, con su colección, su carpeta padre y su posición.
    Item {
        collection_id: String,
        collection_name: String,
        parent_id: Option<String>,
        index: usize,
        item: Item,
    },
    Environment {
        environment: Environment,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrashEntry {
    schema: u32,
    id: String,
    /// Milisegundos Unix.
    deleted_at: i64,
    trashed: Trashed,
}

/// Lo que la app ve de la papelera (sin el contenido).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrashSummary {
    pub id: String,
    /// `collection`, `folder`, `request` o `environment`.
    pub kind: &'static str,
    pub name: String,
    /// Dónde estaba: la colección (y la carpeta) de una carpeta o petición.
    pub location: String,
    pub deleted_at: i64,
    /// Peticiones que contiene (colecciones y carpetas).
    pub requests: usize,
}

/// Lo restaurado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Restored {
    pub kind: &'static str,
    pub id: String,
    /// La colección donde volvió una carpeta o petición.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection_id: Option<String>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn count_requests(item: &Item) -> usize {
    match item {
        Item::Request(_) => 1,
        Item::Folder(folder) => folder.items.iter().map(count_requests).sum(),
    }
}

impl TrashEntry {
    fn summary(&self) -> TrashSummary {
        let (kind, name, location, requests) = match &self.trashed {
            Trashed::Collection { collection } => (
                "collection",
                collection.name.clone(),
                String::new(),
                collection.request_count(),
            ),
            Trashed::Item {
                collection_name,
                item,
                ..
            } => (
                if matches!(item, Item::Folder(_)) {
                    "folder"
                } else {
                    "request"
                },
                item.name().to_owned(),
                collection_name.clone(),
                count_requests(item),
            ),
            Trashed::Environment { environment } => {
                ("environment", environment.name.clone(), String::new(), 0)
            }
        };
        TrashSummary {
            id: self.id.clone(),
            kind,
            name,
            location,
            deleted_at: self.deleted_at,
            requests,
        }
    }
}

impl Workspace {
    fn trash_dir(&self) -> PathBuf {
        self.root.join(".trash")
    }

    fn trash_file(&self, id: &str) -> Result<PathBuf> {
        // Solo ids de la papelera: nada de rutas.
        if !id.starts_with("trash_") || !id[6..].chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(CoreError::not_found("trash", id).into());
        }
        Ok(self.trash_dir().join(format!("{id}.json")))
    }

    fn put_in_trash(&self, trashed: Trashed) -> Result<TrashSummary> {
        let dir = self.trash_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        let ignore = dir.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(&ignore, "# RunPost trash: keep it out of Git.\n*\n")
                .with_context(|| format!("cannot write {}", ignore.display()))?;
        }
        let entry = TrashEntry {
            schema: SCHEMA,
            id: new_id("trash"),
            deleted_at: now_ms(),
            trashed,
        };
        fs::write_json(&dir.join(format!("{}.json", entry.id)), &entry)?;
        Ok(entry.summary())
    }

    /// Borra una colección dejándola en la papelera.
    pub fn trash_collection(&self, id: &str) -> Result<TrashSummary> {
        let collection = self.load_collection(id)?;
        let summary = self.put_in_trash(Trashed::Collection { collection })?;
        self.delete_collection(id)?;
        Ok(summary)
    }

    /// Borra una carpeta o petición dejándola en la papelera, con su sitio.
    pub fn trash_item(&self, collection_id: &str, item_id: &str) -> Result<TrashSummary> {
        let mut collection = self.load_collection(collection_id)?;
        let (parent_id, index) = collection
            .locate(item_id)
            .ok_or_else(|| CoreError::not_found("item", item_id))?;
        let item = collection
            .remove_item(item_id)
            .ok_or_else(|| CoreError::not_found("item", item_id))?;
        let summary = self.put_in_trash(Trashed::Item {
            collection_id: collection.id.clone(),
            collection_name: collection.name.clone(),
            parent_id,
            index,
            item,
        })?;
        self.save_collection(&collection)?;
        Ok(summary)
    }

    /// Borra un entorno dejándolo en la papelera. Sus secretos siguen en el llavero.
    pub fn trash_environment(&self, id: &str) -> Result<TrashSummary> {
        let (_, path, environment) =
            load_all::<Environment>(&self.environments_dir(), ENVIRONMENT_EXT)?
                .into_iter()
                .find(|(_, _, e)| e.id == id)
                .ok_or_else(|| CoreError::not_found("env", id))?;
        let summary = self.put_in_trash(Trashed::Environment { environment })?;
        std::fs::remove_file(&path).with_context(|| format!("cannot delete {}", path.display()))?;
        Ok(summary)
    }

    fn trash_entries(&self) -> Result<Vec<(PathBuf, TrashEntry)>> {
        let dir = self.trash_dir();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)?.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match read_json::<TrashEntry>(&path) {
                    Ok(trashed) => out.push((path, trashed)),
                    Err(error) => tracing::warn!("trash entry skipped: {error:#}"),
                }
            }
        }
        Ok(out)
    }

    /// Lo que hay en la papelera, lo último primero. Lo de más de 30 días se vacía antes.
    pub fn trash(&self) -> Result<Vec<TrashSummary>> {
        let cutoff = now_ms() - KEEP_MS;
        let mut out = Vec::new();
        for (path, entry) in self.trash_entries()? {
            if entry.deleted_at < cutoff {
                self.forget(&path, &entry);
            } else {
                out.push(entry.summary());
            }
        }
        out.sort_by_key(|entry| std::cmp::Reverse(entry.deleted_at));
        Ok(out)
    }

    /// ¿Hay ya un entorno con este id? Uno reimportado (los importadores dan ids estables) comparte
    /// con el de la papelera sus cuentas del llavero.
    fn has_environment(&self, id: &str) -> Result<bool> {
        Ok(
            load_all::<Environment>(&self.environments_dir(), ENVIRONMENT_EXT)?
                .iter()
                .any(|(_, _, e)| e.id == id),
        )
    }

    /// Borra para siempre, y los secretos del llavero de un entorno salvo que ya los use otro con
    /// su id.
    fn forget(&self, path: &PathBuf, entry: &TrashEntry) {
        if let Trashed::Environment { environment } = &entry.trashed
            // Si no se puede saber, mejor un secreto huérfano que borrar el de un entorno en uso.
            && !self.has_environment(&environment.id).unwrap_or(true)
        {
            for var in environment.variables.iter().filter(|v| in_keyring(v)) {
                if let Err(error) = self
                    .keyring
                    .delete(&secrets::account(&environment.id, &var.key))
                {
                    tracing::warn!(key = %var.key, "keyring delete failed: {error}");
                }
            }
        }
        if let Err(error) = std::fs::remove_file(path) {
            tracing::warn!("trash entry not deleted: {error}");
        }
    }

    /// Vacía una entrada o, sin id, toda la papelera. Devuelve cuántas borró.
    pub fn purge(&self, id: Option<&str>) -> Result<usize> {
        match id {
            Some(id) => {
                let path = self.trash_file(id)?;
                let entry: TrashEntry =
                    read_json(&path).map_err(|_| CoreError::not_found("trash", id))?;
                self.forget(&path, &entry);
                Ok(1)
            }
            None => {
                let entries = self.trash_entries()?;
                for (path, entry) in &entries {
                    self.forget(path, entry);
                }
                Ok(entries.len())
            }
        }
    }

    /// Devuelve a su sitio lo que hay en la papelera con ese id.
    pub fn restore(&self, id: &str) -> Result<Restored> {
        let path = self.trash_file(id)?;
        let entry: TrashEntry = read_json(&path).map_err(|_| CoreError::not_found("trash", id))?;
        let restored = match entry.trashed {
            Trashed::Collection { mut collection } => {
                // Si ya hay otra con ese id (p. ej. se reimportó), vuelve como una copia.
                if self.load_collection(&collection.id).is_ok() {
                    collection.fresh_ids();
                }
                self.save_collection(&collection)?;
                Restored {
                    kind: "collection",
                    id: collection.id,
                    collection_id: None,
                }
            }
            Trashed::Item {
                collection_id,
                collection_name,
                parent_id,
                index,
                item,
            } => {
                let mut collection = self.load_collection(&collection_id).map_err(|_| {
                    CoreError::InvalidRequest(format!(
                        "the collection \"{collection_name}\" is not in the workspace: restore it first"
                    ))
                })?;
                let kind = if matches!(item, Item::Folder(_)) {
                    "folder"
                } else {
                    "request"
                };
                // Si su carpeta ya no está, vuelve a la raíz de la colección.
                let parent = parent_id.filter(|p| collection.find_item(p).is_some());
                let id = collection.insert_item(
                    &Position {
                        parent,
                        before: None,
                        index: Some(index),
                    },
                    item,
                )?;
                self.save_collection(&collection)?;
                Restored {
                    kind,
                    id,
                    collection_id: Some(collection.id),
                }
            }
            Trashed::Environment { mut environment } => {
                if self.has_environment(&environment.id)? {
                    // Hay otro con ese id: vuelve con uno nuevo y sus secretos se mueven a él.
                    let old = environment.id.clone();
                    for var in environment.variables.iter_mut().filter(|v| in_keyring(v)) {
                        if let Ok(Some(value)) = self.keyring.get(&secrets::account(&old, &var.key))
                        {
                            var.value = Some(value);
                        }
                    }
                    environment.id = new_id("env");
                }
                self.save_environment(&environment)?;
                Restored {
                    kind: "environment",
                    id: environment.id,
                    collection_id: None,
                }
            }
        };
        std::fs::remove_file(&path).with_context(|| format!("cannot delete {}", path.display()))?;
        Ok(restored)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        model::{Folder, Request, SecretStore, Variable},
        secrets::{Keyring, MemoryKeyring},
    };

    fn workspace() -> (tempfile::TempDir, Workspace, Arc<MemoryKeyring>) {
        let dir = tempfile::tempdir().unwrap();
        let keyring = Arc::new(MemoryKeyring::new());
        let ws = Workspace::open_with_keyring(dir.path(), keyring.clone()).unwrap();
        (dir, ws, keyring)
    }

    fn sample(ws: &Workspace) -> Collection {
        let mut collection = Collection::new("Acme");
        let mut folder = Folder::new("Accounts");
        folder.items.push(Item::Request(Box::new(Request::new(
            "a", "GET", "http://a",
        ))));
        folder.items.push(Item::Request(Box::new(Request::new(
            "b", "GET", "http://b",
        ))));
        collection.items.push(Item::Folder(folder));
        collection.items.push(Item::Request(Box::new(Request::new(
            "ping", "GET", "http://p",
        ))));
        ws.save_collection(&collection).unwrap();
        collection
    }

    #[test]
    fn items_go_back_where_they_were() {
        let (dir, ws, _) = workspace();
        let collection = sample(&ws);
        let Item::Folder(folder) = &collection.items[0] else {
            unreachable!()
        };
        let b = folder.items[1].id().to_owned();
        let trashed = ws.trash_item(&collection.id, &b).unwrap();
        assert_eq!(
            (
                trashed.kind,
                trashed.name.as_str(),
                trashed.location.as_str()
            ),
            ("request", "b", "Acme")
        );
        assert!(
            ws.load_collection(&collection.id)
                .unwrap()
                .find_item(&b)
                .is_none()
        );
        assert!(dir.path().join(".trash/.gitignore").is_file());

        let folder_id = folder.id.clone();
        let trashed_folder = ws.trash_item(&collection.id, &folder_id).unwrap();
        assert_eq!(
            (trashed_folder.kind, trashed_folder.requests),
            ("folder", 1)
        );
        let list = ws.trash().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, trashed_folder.id, "newest first");

        // Restaurar la carpeta y luego la petición: cada una en su sitio.
        let restored = ws.restore(&trashed_folder.id).unwrap();
        assert_eq!(restored.kind, "folder");
        let restored = ws.restore(&trashed.id).unwrap();
        assert_eq!(
            restored.collection_id.as_deref(),
            Some(collection.id.as_str())
        );
        assert_eq!(ws.load_collection(&collection.id).unwrap(), collection);
        assert!(ws.trash().unwrap().is_empty());
        assert!(ws.restore(&trashed.id).is_err());
        assert!(ws.restore("../collections/x").is_err());
    }

    #[test]
    fn collections_and_environments_with_keyring_secrets() {
        let (_dir, ws, keyring) = workspace();
        let collection = sample(&ws);
        let trashed = ws.trash_collection(&collection.id).unwrap();
        assert_eq!((trashed.kind, trashed.requests), ("collection", 3));
        assert!(ws.load_collection(&collection.id).is_err());
        ws.restore(&trashed.id).unwrap();
        assert_eq!(ws.load_collection(&collection.id).unwrap(), collection);

        let mut env = Environment::new("DESA");
        let mut secret = Variable::secret("accessToken", "fake-keyring-token");
        secret.store = Some(SecretStore::Keyring);
        env.variables = vec![Variable::new("apiUrl", "http://x"), secret];
        ws.save_environment(&env).unwrap();
        let account = secrets::account(&env.id, "accessToken");
        assert_eq!(
            keyring.get(&account).unwrap().as_deref(),
            Some("fake-keyring-token")
        );

        let trashed = ws.trash_environment(&env.id).unwrap();
        assert!(ws.find_environment(&env.id).is_err());
        // El secreto sigue en el llavero y no está en el archivo de la papelera.
        assert!(keyring.get(&account).unwrap().is_some());
        let file = std::fs::read_to_string(ws.trash_file(&trashed.id).unwrap()).unwrap();
        assert!(!file.contains("fake-keyring-token"));
        ws.restore(&trashed.id).unwrap();
        let back = ws.find_environment(&env.id).unwrap();
        assert_eq!(
            back.variables[1].value.as_deref(),
            Some("fake-keyring-token")
        );

        // Vaciar la papelera borra el secreto del llavero.
        let trashed = ws.trash_environment(&env.id).unwrap();
        assert_eq!(ws.purge(Some(&trashed.id)).unwrap(), 1);
        assert_eq!(keyring.get(&account).unwrap(), None);
        assert!(ws.trash().unwrap().is_empty());
        ws.trash_collection(&collection.id).unwrap();
        assert_eq!(ws.purge(None).unwrap(), 1);
    }

    #[test]
    fn emptying_the_trash_keeps_the_secrets_of_a_reimported_environment() {
        let (_dir, ws, keyring) = workspace();
        let mut env = Environment::new("DESA");
        let mut secret = Variable::secret("accessToken", "fake-old-token");
        secret.store = Some(SecretStore::Keyring);
        env.variables = vec![secret];
        ws.save_environment(&env).unwrap();
        let trashed = ws.trash_environment(&env.id).unwrap();
        // Se vuelve a importar con el mismo id: usa las mismas cuentas del llavero.
        env.variables[0].value = Some("fake-new-token".into());
        ws.save_environment(&env).unwrap();
        assert_eq!(ws.purge(Some(&trashed.id)).unwrap(), 1);
        let account = secrets::account(&env.id, "accessToken");
        assert_eq!(
            keyring.get(&account).unwrap().as_deref(),
            Some("fake-new-token")
        );
    }

    #[test]
    fn old_entries_are_emptied_and_missing_collections_explain() {
        let (_dir, ws, _) = workspace();
        let collection = sample(&ws);
        let ping = collection.items[1].id().to_owned();
        let trashed = ws.trash_item(&collection.id, &ping).unwrap();
        ws.delete_collection(&collection.id).unwrap();
        let error = ws.restore(&trashed.id).unwrap_err().to_string();
        assert!(
            error.contains("\"Acme\" is not in the workspace"),
            "{error}"
        );

        // Una entrada de hace 31 días se vacía al listar.
        let path = ws.trash_file(&trashed.id).unwrap();
        let mut entry: TrashEntry = read_json(&path).unwrap();
        entry.deleted_at -= 31 * 24 * 60 * 60 * 1000;
        fs::write_json(&path, &entry).unwrap();
        assert!(ws.trash().unwrap().is_empty());
        assert!(!path.exists());
    }
}
