//! `agent_policy` (secciones 5.1 y 19.3): se evalúa antes de enviar una petición por MCP.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::model::AgentPolicy;

pub const APPROVAL_HINT: &str = "ask the user to approve in the app";
const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// `allow_methods` | `require_approval` | `max_calls_per_minute`.
    pub rule: &'static str,
    pub hint: String,
    /// Lo que activó la regla: el método o el id de petición de `require_approval`, o el método
    /// no permitido. Para el aviso de la app; no va en la respuesta al modelo.
    pub detail: String,
}

impl Violation {
    /// `{"error":"policy","rule":"…","hint":"…"}` (regla 8 de 8.3).
    pub fn to_json(&self) -> Value {
        json!({"error": "policy", "rule": self.rule, "hint": self.hint})
    }
}

/// Reglas estáticas: métodos permitidos y aprobación. Por stdio no hay a quién pedir
/// aprobación, así que `require_approval` bloquea; por HTTP con la app abierta, la tool espera la
/// decisión del usuario (sección 8.3, regla 8).
pub fn check_static(policy: &AgentPolicy, method: &str, request_id: &str) -> Result<(), Violation> {
    if let Some(allowed) = &policy.allow_methods
        && !allowed.iter().any(|m| m.eq_ignore_ascii_case(method))
    {
        return Err(Violation {
            rule: "allow_methods",
            hint: APPROVAL_HINT.into(),
            detail: method.to_uppercase(),
        });
    }
    if let Some(entry) = policy.require_approval.iter().find(|entry| {
        entry.eq_ignore_ascii_case(method) || (!request_id.is_empty() && *entry == request_id)
    }) {
        return Err(Violation {
            rule: "require_approval",
            hint: APPROVAL_HINT.into(),
            detail: entry.clone(),
        });
    }
    Ok(())
}

/// Límite de llamadas por minuto, por colección (ventana deslizante de 60 s).
#[derive(Debug, Default)]
pub struct RateLimiter {
    calls: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra la llamada si cabe en el límite; si no, devuelve cuántos segundos esperar.
    pub fn check(&self, collection_id: &str, max_per_minute: Option<u32>) -> Result<(), Violation> {
        self.check_at(collection_id, max_per_minute, Instant::now())
    }

    fn check_at(
        &self,
        collection_id: &str,
        max_per_minute: Option<u32>,
        now: Instant,
    ) -> Result<(), Violation> {
        let Some(max) = max_per_minute else {
            return Ok(());
        };
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let window = calls.entry(collection_id.to_owned()).or_default();
        while window
            .front()
            .is_some_and(|t| now.duration_since(*t) >= WINDOW)
        {
            window.pop_front();
        }
        if window.len() >= max as usize {
            let wait = window.front().map_or(WINDOW, |oldest| {
                WINDOW.saturating_sub(now.duration_since(*oldest))
            });
            return Err(Violation {
                rule: "max_calls_per_minute",
                hint: format!("limit {max}/min; retry in {}s", wait.as_secs().max(1)),
                detail: max.to_string(),
            });
        }
        window.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_methods_and_require_approval() {
        let policy = AgentPolicy {
            allow_methods: Some(vec!["GET".into(), "post".into()]),
            require_approval: vec!["DELETE".into(), "req_x".into()],
            max_calls_per_minute: None,
        };
        assert!(check_static(&policy, "get", "req_a").is_ok());
        assert!(check_static(&policy, "POST", "req_a").is_ok());
        let denied = check_static(&policy, "PUT", "req_a").unwrap_err();
        assert_eq!(
            denied.to_json(),
            json!({"error":"policy","rule":"allow_methods","hint":APPROVAL_HINT})
        );
        let approval = check_static(&policy, "POST", "req_x").unwrap_err();
        assert_eq!(
            (approval.rule, approval.detail.as_str()),
            ("require_approval", "req_x")
        );
        let open = AgentPolicy {
            require_approval: vec!["delete".into()],
            ..AgentPolicy::default()
        };
        assert_eq!(
            check_static(&open, "DELETE", "").unwrap_err().rule,
            "require_approval"
        );
        assert!(check_static(&open, "PATCH", "").is_ok());
        assert!(check_static(&AgentPolicy::default(), "DELETE", "req_x").is_ok());
    }

    #[test]
    fn rate_limit_is_a_sliding_window_per_collection() {
        let limiter = RateLimiter::new();
        let t0 = Instant::now();
        assert!(limiter.check_at("c", None, t0).is_ok());
        assert!(limiter.check_at("c", Some(2), t0).is_ok());
        assert!(
            limiter
                .check_at("c", Some(2), t0 + Duration::from_secs(10))
                .is_ok()
        );
        let blocked = limiter
            .check_at("c", Some(2), t0 + Duration::from_secs(20))
            .unwrap_err();
        assert_eq!(blocked.rule, "max_calls_per_minute");
        assert_eq!(blocked.hint, "limit 2/min; retry in 40s");
        assert!(
            limiter
                .check_at("other", Some(2), t0 + Duration::from_secs(20))
                .is_ok()
        );
        assert!(
            limiter
                .check_at("c", Some(2), t0 + Duration::from_secs(61))
                .is_ok()
        );
    }
}
