//! Lo que la app ve del agente (secciones 19.2, 19.3 y 19.6): cada llamada MCP por HTTP, el
//! medidor de tokens y las aprobaciones pendientes. Por stdio no hay hub: nadie está mirando.
//!
//! Los eventos salen por el mismo canal que el resto de eventos de la app (`/ws`):
//! `agent_call`, `agent_client`, `agent_approval` y `agent_approval_done`.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use runpost_core::model::new_id;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

/// Tiempo que una tool espera la decisión del usuario (UX R4: 60 s).
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
/// Llamadas recientes que se guardan para la UI (al recargar la página).
const KEEP_CALLS: usize = 500;

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Quién llama: el cliente MCP (de `initialize`) y su sesión HTTP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Caller {
    /// Nombre legible: `Claude Code`, `Codex`, `Claude Desktop` o el que envíe el cliente.
    pub client: String,
    pub version: String,
    /// Primeros 8 caracteres de `Mcp-Session-Id`, o `""` sin sesión.
    pub session: String,
}

impl Caller {
    pub fn new(name: &str, version: &str, session: Option<&str>) -> Self {
        let client = match name {
            "claude-code" => "Claude Code",
            "claude-ai" => "Claude Desktop",
            n if n.starts_with("codex") => "Codex",
            "" => "Agente",
            n => n,
        };
        Self {
            client: client.to_owned(),
            version: version.to_owned(),
            session: session
                .map(|s| s.chars().take(8).collect())
                .unwrap_or_default(),
        }
    }
}

/// Una fila del panel Agente.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentCall {
    pub id: u64,
    pub ts: i64,
    pub tool: String,
    /// Petición, colección o variables afectadas (`00.2 Gateway - ping · select $.status`).
    pub target: String,
    /// Petición guardada que se puede abrir como pestaña.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// `200`, `3/4`, `ok` o el código de error (`policy`, `connect`…).
    pub status: String,
    pub ok: bool,
    pub ms: u64,
    /// Bytes crudos (respuesta completa) frente a bytes entregados al modelo (8.3, regla 9).
    pub raw: u64,
    pub sent: u64,
    pub client: String,
    pub session: String,
}

/// Acumulado de la sesión de la app (19.6). Tokens estimados = bytes / 4.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Meter {
    pub calls: u64,
    pub raw: u64,
    pub sent: u64,
}

/// Cliente visto por HTTP (estado de conexión del panel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientSeen {
    pub client: String,
    pub version: String,
    pub session: String,
    pub first_seen: i64,
    pub last_seen: i64,
    pub calls: u64,
}

/// Petición protegida que espera al usuario (aviso de la esquina superior derecha).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Approval {
    pub id: String,
    pub ts: i64,
    /// Hasta cuándo espera la tool (ms desde epoch).
    pub expires: i64,
    pub client: String,
    pub tool: String,
    pub method: String,
    /// URL sin resolver (`{{apiUrl}}/…`): nunca lleva secretos.
    pub url: String,
    pub request: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub collection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// `require_approval: DELETE`.
    pub rule: String,
    /// Peticiones protegidas que cubre (más de una en `run_collection`).
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Approved,
    Denied,
    /// Pasaron 60 s sin respuesta.
    Expired,
    /// El cliente MCP abandonó la llamada mientras esperaba.
    Cancelled,
    /// No hay ninguna ventana de la app conectada para preguntar.
    Unavailable,
}

#[derive(Default)]
struct HubState {
    calls: VecDeque<AgentCall>,
    meter: Meter,
    clients: Vec<ClientSeen>,
    pending: HashMap<String, (Approval, oneshot::Sender<bool>)>,
}

pub struct AgentHub {
    events: broadcast::Sender<Value>,
    seq: AtomicU64,
    state: Mutex<HubState>,
}

impl std::fmt::Debug for AgentHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentHub")
    }
}

/// Si la tool deja de esperar (el cliente cancela), la aprobación no se queda colgada en la UI.
struct Pending<'a> {
    hub: &'a AgentHub,
    id: String,
    decision: Decision,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.hub.state().pending.remove(&self.id);
        self.hub
            .emit(json!({"type": "agent_approval_done", "id": self.id,
                             "decision": self.decision}));
    }
}

impl AgentHub {
    /// `events` es el canal de eventos de la app hacia la UI.
    pub fn new(events: broadcast::Sender<Value>) -> Self {
        Self {
            events,
            seq: AtomicU64::new(0),
            state: Mutex::new(HubState::default()),
        }
    }

    fn state(&self) -> MutexGuard<'_, HubState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn emit(&self, event: Value) {
        // Sin pestañas de la UI conectadas no es un error.
        let _ = self.events.send(event);
    }

    /// ¿Hay alguna ventana de la app mirando? (cada pestaña de la UI se suscribe por `/ws`).
    pub fn has_viewer(&self) -> bool {
        self.events.receiver_count() > 0
    }

    /// Estado completo para la UI al arrancar o recargar.
    pub fn snapshot(&self) -> Value {
        let state = self.state();
        let pending: Vec<&Approval> = state.pending.values().map(|(a, _)| a).collect();
        json!({"calls": state.calls, "meter": state.meter, "clients": state.clients,
               "pending": pending})
    }

    /// Registra la actividad de un cliente (también `tools/list`, que llega al conectar).
    pub fn seen(&self, caller: &Caller, counts_call: bool) {
        let now = now_ms();
        let client = {
            let mut state = self.state();
            let index = match state
                .clients
                .iter()
                .position(|c| c.client == caller.client && c.session == caller.session)
            {
                Some(index) => index,
                None => {
                    state.clients.push(ClientSeen {
                        client: caller.client.clone(),
                        version: caller.version.clone(),
                        session: caller.session.clone(),
                        first_seen: now,
                        last_seen: now,
                        calls: 0,
                    });
                    state.clients.len() - 1
                }
            };
            let seen = &mut state.clients[index];
            seen.last_seen = now;
            if counts_call {
                seen.calls += 1;
            }
            seen.clone()
        };
        self.emit(json!({"type": "agent_client", "client": client}));
    }

    /// Añade una fila al panel y la suma al medidor.
    pub fn record(&self, mut call: AgentCall) -> AgentCall {
        call.id = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        if call.ts == 0 {
            call.ts = now_ms();
        }
        {
            let mut state = self.state();
            state.meter.calls += 1;
            state.meter.raw += call.raw;
            state.meter.sent += call.sent;
            state.calls.push_back(call.clone());
            while state.calls.len() > KEEP_CALLS {
                state.calls.pop_front();
            }
        }
        self.emit(json!({"type": "agent_call", "call": call}));
        call
    }

    /// Pide aprobación al usuario y espera hasta 60 s (o `timeout` en las pruebas).
    pub async fn ask(&self, approval: Approval) -> Decision {
        self.ask_within(approval, APPROVAL_TIMEOUT).await
    }

    pub async fn ask_within(&self, mut approval: Approval, timeout: Duration) -> Decision {
        if !self.has_viewer() {
            return Decision::Unavailable;
        }
        let (sender, receiver) = oneshot::channel();
        approval.id = new_id("apr");
        approval.ts = now_ms();
        approval.expires = approval.ts + timeout.as_millis() as i64;
        let mut pending = Pending {
            hub: self,
            id: approval.id.clone(),
            decision: Decision::Cancelled,
        };
        self.state()
            .pending
            .insert(approval.id.clone(), (approval.clone(), sender));
        self.emit(json!({"type": "agent_approval", "approval": approval}));
        pending.decision = match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(true)) => Decision::Approved,
            Ok(Ok(false)) | Ok(Err(_)) => Decision::Denied,
            Err(_) => Decision::Expired,
        };
        pending.decision
    }

    /// Respuesta del usuario desde la app. `false` si ya no estaba pendiente.
    pub fn decide(&self, id: &str, approve: bool) -> bool {
        let Some((_, sender)) = self.state().pending.remove(id) else {
            return false;
        };
        sender.send(approve).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn caller_names_and_meter() {
        assert_eq!(
            Caller::new("claude-code", "2.1", Some("abcdef0123456789")).client,
            "Claude Code"
        );
        assert_eq!(
            Caller::new("claude-code", "2.1", Some("abcdef0123456789")).session,
            "abcdef01"
        );
        assert_eq!(Caller::new("codex-mcp-client", "0.1", None).client, "Codex");
        assert_eq!(Caller::new("my-agent", "1", None).client, "my-agent");
        let (events, mut rx) = broadcast::channel(16);
        let hub = AgentHub::new(events);
        let caller = Caller::new("claude-code", "2.1", Some("s1"));
        hub.seen(&caller, false);
        let call = hub.record(AgentCall {
            tool: "run_request".into(),
            raw: 1000,
            sent: 100,
            ..AgentCall::default()
        });
        hub.record(AgentCall {
            tool: "env_get".into(),
            raw: 50,
            sent: 50,
            ..AgentCall::default()
        });
        assert_eq!(call.id, 1);
        let snapshot = hub.snapshot();
        assert_eq!(
            snapshot["meter"],
            json!({"calls": 2, "raw": 1050, "sent": 150})
        );
        assert_eq!(snapshot["clients"][0]["client"], "Claude Code");
        assert_eq!(rx.try_recv().unwrap()["type"], "agent_client");
        assert_eq!(rx.try_recv().unwrap()["call"]["tool"], "run_request");
    }

    #[tokio::test]
    async fn approvals_are_decided_expire_or_are_unavailable() {
        let (events, _) = broadcast::channel(16);
        let hub = Arc::new(AgentHub::new(events.clone()));
        // Sin ninguna ventana conectada no hay a quién preguntar.
        assert_eq!(hub.ask(Approval::default()).await, Decision::Unavailable);

        let mut ui = events.subscribe();
        let waiting = tokio::spawn({
            let hub = hub.clone();
            async move { hub.ask(Approval::default()).await }
        });
        let shown = ui.recv().await.unwrap();
        assert_eq!(shown["type"], "agent_approval");
        let id = shown["approval"]["id"].as_str().unwrap().to_owned();
        assert_eq!(hub.snapshot()["pending"][0]["id"], id.as_str());
        assert!(hub.decide(&id, true));
        assert_eq!(waiting.await.unwrap(), Decision::Approved);
        let done = ui.recv().await.unwrap();
        assert_eq!(
            done,
            json!({"type": "agent_approval_done", "id": id, "decision": "approved"})
        );
        assert!(!hub.decide(&id, false));

        let denied = tokio::spawn({
            let hub = hub.clone();
            async move { hub.ask(Approval::default()).await }
        });
        let id = ui.recv().await.unwrap()["approval"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        hub.decide(&id, false);
        assert_eq!(denied.await.unwrap(), Decision::Denied);
        ui.recv().await.unwrap();

        let expired = hub
            .ask_within(Approval::default(), Duration::from_millis(30))
            .await;
        assert_eq!(expired, Decision::Expired);
        assert!(hub.snapshot()["pending"].as_array().unwrap().is_empty());
    }
}
