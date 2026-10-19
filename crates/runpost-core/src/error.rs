//! Errores tipados que las capas superiores (API, MCP) traducen a códigos cortos.

/// Se recuperan con `anyhow::Error::downcast_ref::<CoreError>()`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    #[error("{kind} not found: {selector}")]
    NotFound {
        kind: &'static str,
        selector: String,
    },
    #[error("{kind} selector is ambiguous: {selector} ({candidates} matches)")]
    Ambiguous {
        kind: &'static str,
        selector: String,
        candidates: usize,
    },
    #[error("unsupported schema {found} in {file} (expected 1)")]
    UnsupportedSchema { file: String, found: u32 },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// El llavero del sistema no está disponible o falló. Nunca lleva el secreto.
    #[error("keyring: {0}")]
    Keyring(String),
    /// OAuth 2.0: falta el token, el servidor de autorización lo negó o la configuración no vale.
    /// Nunca lleva tokens ni el secreto del cliente.
    #[error("OAuth 2.0: {0}")]
    OAuth(String),
}

impl CoreError {
    pub fn not_found(kind: &'static str, selector: impl Into<String>) -> Self {
        Self::NotFound {
            kind,
            selector: selector.into(),
        }
    }

    /// Código corto estable para MCP/API (`env_not_found`, `request_ambiguous`, …).
    pub fn code(&self) -> String {
        match self {
            Self::NotFound { kind, .. } => format!("{kind}_not_found"),
            Self::Ambiguous { kind, .. } => format!("{kind}_ambiguous"),
            Self::UnsupportedSchema { .. } => "unsupported_schema".into(),
            Self::InvalidRequest(_) => "invalid_request".into(),
            Self::Keyring(_) => "keyring".into(),
            Self::OAuth(_) => "oauth2".into(),
        }
    }
}

/// Fallo de red al enviar. El mensaje y la URL ya van enmascarados.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct NetworkError {
    /// `connect` | `timeout` | `redirect` | `body` | `decode` | `request`.
    pub kind: &'static str,
    pub message: String,
    pub url: String,
    /// Id del historial si la ejecución se grabó.
    pub history_id: Option<String>,
}
