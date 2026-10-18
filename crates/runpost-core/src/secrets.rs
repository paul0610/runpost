//! Secretos en el llavero del sistema (sección 19.4): Windows Credential Manager, Keychain de
//! macOS o Secret Service en Linux, mediante el crate `keyring`. En el JSON del entorno solo queda
//! `{"key":"accessToken","secret":true,"store":"keyring"}`; el valor se lee al cargar el entorno y
//! se escribe al guardarlo (`Workspace`).

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use crate::{APP_SLUG, CoreError};

/// Elige el llavero: sin valor (u `os`), el del sistema; `memory`, uno en memoria que se pierde al
/// cerrar el proceso. `memory` existe para las pruebas: nunca tocan el llavero real del usuario.
pub const KEYRING_ENV: &str = "RUNPOST_KEYRING";

/// Almacén de secretos por cuenta (`<id del entorno>/<variable>`).
pub trait Keyring: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, CoreError>;
    fn set(&self, account: &str, secret: &str) -> Result<(), CoreError>;
    /// Borrar una cuenta que no existe no es un error.
    fn delete(&self, account: &str) -> Result<(), CoreError>;
}

/// Cuenta de una variable: el id del dueño (entorno o globales) y su nombre.
pub fn account(owner: &str, key: &str) -> String {
    format!("{owner}/{key}")
}

/// Cuenta con los datos de conexión (`{"port","token"}`) de la app que sirve MCP por HTTP en ese
/// puerto. La escribe la app al arrancar su MCP y la borra al cerrar; la lee `runpost_mcp
/// mcp-headers`, el `headersHelper` de los clientes MCP. Va por puerto (no por workspace) para que
/// una configuración genérica, como la del plugin de Claude Code, sirva a cualquiera. El token sigue
/// siendo de un solo arranque (sección 13) y no se escribe en ningún archivo.
pub fn session_account(port: u16) -> String {
    format!("session/{port}")
}

/// Llavero del sistema operativo. Servicio `runpost`.
pub struct OsKeyring {
    service: String,
}

impl Default for OsKeyring {
    fn default() -> Self {
        Self {
            service: APP_SLUG.to_owned(),
        }
    }
}

impl OsKeyring {
    /// Otro nombre de servicio (la prueba con el llavero real usa uno propio).
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry, CoreError> {
        keyring::Entry::new(&self.service, account).map_err(describe)
    }
}

/// Mensaje sin datos sensibles: `BadEncoding` lleva los bytes guardados y no se muestra.
fn describe(error: keyring::Error) -> CoreError {
    let message = match error {
        keyring::Error::BadEncoding(_) => "stored value is not UTF-8".to_owned(),
        keyring::Error::NoDefaultStore => "no credential store on this system".to_owned(),
        other => other.to_string(),
    };
    CoreError::Keyring(message)
}

impl Keyring for OsKeyring {
    fn get(&self, account: &str) -> Result<Option<String>, CoreError> {
        match self.entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(describe(error)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), CoreError> {
        self.entry(account)?.set_password(secret).map_err(describe)
    }

    fn delete(&self, account: &str) -> Result<(), CoreError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(describe(error)),
        }
    }
}

/// Llavero en memoria para pruebas.
#[derive(Default)]
pub struct MemoryKeyring {
    entries: Mutex<HashMap<String, String>>,
}

impl MemoryKeyring {
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Keyring for MemoryKeyring {
    fn get(&self, account: &str) -> Result<Option<String>, CoreError> {
        Ok(self.entries().get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), CoreError> {
        self.entries().insert(account.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), CoreError> {
        self.entries().remove(account);
        Ok(())
    }
}

/// Un único llavero en memoria por proceso, para que todos los `Workspace` lo compartan.
static MEMORY: LazyLock<Arc<MemoryKeyring>> = LazyLock::new(Arc::default);

/// Llavero según `RUNPOST_KEYRING`.
pub fn from_env() -> Arc<dyn Keyring> {
    match std::env::var(KEYRING_ENV).as_deref() {
        Ok("memory") => MEMORY.clone(),
        _ => Arc::new(OsKeyring::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_account_names_the_port() {
        assert_eq!(session_account(7879), "session/7879");
    }

    #[test]
    fn memory_keyring_round_trip() {
        let keyring = MemoryKeyring::new();
        let account = account("env_1", "accessToken");
        assert_eq!(account, "env_1/accessToken");
        assert_eq!(keyring.get(&account).unwrap(), None);
        keyring.set(&account, "fake-token").unwrap();
        assert_eq!(
            keyring.get(&account).unwrap().as_deref(),
            Some("fake-token")
        );
        keyring.delete(&account).unwrap();
        keyring.delete(&account).unwrap();
        assert_eq!(keyring.get(&account).unwrap(), None);
    }

    /// Llavero real del sistema con un servicio propio que se borra al terminar. Se ejecuta a mano
    /// (`cargo test -p runpost-core -- --ignored os_keyring`) porque CI no siempre tiene llavero.
    #[test]
    #[ignore = "uses the real OS keyring"]
    fn os_keyring_round_trip() {
        let keyring = OsKeyring::with_service(format!("runpost-test-{}", uuid::Uuid::new_v4()));
        let account = account("env_test", "accessToken");
        assert_eq!(keyring.get(&account).unwrap(), None);
        keyring.set(&account, "fake-token-not-real").unwrap();
        assert_eq!(
            keyring.get(&account).unwrap().as_deref(),
            Some("fake-token-not-real")
        );
        keyring.delete(&account).unwrap();
        assert_eq!(keyring.get(&account).unwrap(), None);
        keyring.delete(&account).unwrap();
    }
}
