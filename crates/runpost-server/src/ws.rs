//! WebSocket `/ws`: eventos para la UI. Hoy `workspace_changed` (cambios en disco, también
//! los que hace `runpost mcp` desde otro proceso); el runner añadirá los suyos en el sprint 5.
//!
//! El navegador no puede poner cabeceras en un WebSocket: el token viaja como subprotocolo
//! (`new WebSocket(url, ["runpost", token])`) y el servidor responde con `runpost`.

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;

pub const PROTOCOL: &str = "runpost";

pub(crate) async fn handler(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .protocols([PROTOCOL])
        .on_upgrade(move |socket| session(state, socket))
}

/// `WorkspaceEvent` → `{"type":"workspace_changed","kind":"collection","slug":"…"}`.
pub fn event_json(event: &runpost_core::WorkspaceEvent) -> Value {
    let mut value = serde_json::to_value(event).unwrap_or_else(|_| json!({}));
    value["type"] = json!("workspace_changed");
    value
}

async fn session(state: AppState, mut socket: WebSocket) {
    let mut changes = state.workspace.watch();
    let mut events = state.events.subscribe();
    let _ = socket
        .send(Message::Text(json!({"type": "hello"}).to_string().into()))
        .await;
    loop {
        let outgoing = tokio::select! {
            change = changes.recv() => match change {
                Ok(change) => event_json(&change),
                // Se perdieron eventos: pedir a la UI que recargue todo.
                Err(RecvError::Lagged(_)) => json!({"type": "workspace_changed", "kind": "all"}),
                Err(RecvError::Closed) => break,
            },
            event = events.recv() => match event {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => continue,
            },
        };
        if socket
            .send(Message::Text(outgoing.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
    }
}
