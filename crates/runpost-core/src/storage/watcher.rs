//! Cambios externos del workspace (`notify`) → eventos para la UI (vía WS en el sprint 3).

use std::path::{Path, PathBuf};

use anyhow::Result;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceEvent {
    /// `slug` = nombre del archivo sin `.collection.json`.
    Collection {
        slug: String,
    },
    Environment {
        slug: String,
    },
    Settings,
    Globals,
}

/// Traduce una ruta del workspace a evento. Ignora temporales (`.x.tmp`), logs e historial.
pub fn classify(root: &Path, path: &Path) -> Option<WorkspaceEvent> {
    let relative = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    match parts.as_slice() {
        [file] if file == "settings.json" => Some(WorkspaceEvent::Settings),
        [file] if file == "globals.json" => Some(WorkspaceEvent::Globals),
        [dir, file] if dir == "collections" && !file.starts_with('.') => file
            .strip_suffix(".collection.json")
            .map(|slug| WorkspaceEvent::Collection { slug: slug.into() }),
        [dir, file] if dir == "environments" && !file.starts_with('.') => file
            .strip_suffix(".env.json")
            .map(|slug| WorkspaceEvent::Environment { slug: slug.into() }),
        _ => None,
    }
}

pub(crate) struct WatchState {
    _watcher: RecommendedWatcher,
    pub sender: broadcast::Sender<WorkspaceEvent>,
}

pub(crate) fn start(root: &Path) -> Result<WatchState> {
    let (sender, _) = broadcast::channel(64);
    let events = sender.clone();
    let base: PathBuf = root.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        let Ok(event) = result else { return };
        if event.kind.is_access() {
            return;
        }
        let mut sent = Vec::new();
        for path in &event.paths {
            if let Some(change) = classify(&base, path)
                && !sent.contains(&change)
            {
                // Sin suscriptores no es un error: nadie escucha todavía.
                let _ = events.send(change.clone());
                sent.push(change);
            }
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    Ok(WatchState {
        _watcher: watcher,
        sender,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_workspace_paths() {
        let root = Path::new("/ws");
        let at = |p: &str| classify(root, &root.join(p));
        assert_eq!(at("settings.json"), Some(WorkspaceEvent::Settings));
        assert_eq!(at("globals.json"), Some(WorkspaceEvent::Globals));
        assert_eq!(
            at("collections/acme.collection.json"),
            Some(WorkspaceEvent::Collection {
                slug: "acme".into()
            })
        );
        assert_eq!(
            at("environments/desa.env.json"),
            Some(WorkspaceEvent::Environment {
                slug: "desa".into()
            })
        );
        assert_eq!(at("collections/.acme.collection.json.abc.tmp"), None);
        assert_eq!(at("history.db-wal"), None);
        assert_eq!(at("logs/mcp.log"), None);
        assert_eq!(at("collections/readme.md"), None);
        assert_eq!(classify(root, Path::new("/elsewhere/settings.json")), None);
    }
}
