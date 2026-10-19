//! Núcleo compartido. No depende de axum, rmcp ni Tauri.
use std::{ffi::OsString, path::PathBuf};

include!(concat!(env!("OUT_DIR"), "/app.rs"));

pub mod budget;
pub mod codegen;
pub mod diff;
pub mod engine;
pub mod error;
pub mod executor;
pub mod export;
pub mod import;
pub mod model;
pub mod policy;
pub mod runner;
pub mod script;
pub mod search;
pub mod secrets;
pub mod storage;
pub mod vars;

pub use error::{CoreError, NetworkError};
pub use executor::{Executor, RequestDraft, RunContext, RunResult};
pub use storage::{
    CollectionSummary, HistoryDb, ImportReport, RequestSelector, Source, Workspace, WorkspaceEvent,
};

/// Función pura para que la precedencia no dependa del entorno de los tests.
pub fn resolve_workspace(explicit: Option<PathBuf>, environment: Option<OsString>) -> PathBuf {
    explicit
        .or_else(|| environment.filter(|s| !s.is_empty()).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("./workspace"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_precedence() {
        let arg = PathBuf::from("argument");
        assert_eq!(
            resolve_workspace(Some(arg.clone()), Some("env".into())),
            arg
        );
        assert_eq!(
            resolve_workspace(None, Some("env".into())),
            PathBuf::from("env")
        );
        assert_eq!(resolve_workspace(None, None), PathBuf::from("./workspace"));
        assert_eq!(
            resolve_workspace(None, Some("".into())),
            PathBuf::from("./workspace")
        );
    }
}
