//! Ejemplos guardados por petición (fase 4): "Guardar como ejemplo" copia una respuesta del
//! historial, que ya está enmascarado, así que un ejemplo nunca lleva secretos.

use anyhow::Result;

use super::Workspace;
use crate::{
    error::CoreError,
    model::{Example, ExampleRequest, Item, KeyValue, Request},
};

/// Tamaño máximo del cuerpo de un ejemplo: va dentro del JSON de la colección.
const MAX_BODY: usize = 1024 * 1024;

fn pairs(headers: &[(String, String)]) -> Vec<KeyValue> {
    headers
        .iter()
        .map(|(key, value)| KeyValue::new(key, value))
        .collect()
}

fn text(body: Option<&Vec<u8>>) -> String {
    body.map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default()
}

impl Workspace {
    /// Cambia la petición `request_id` de la colección `collection_id` y guarda la colección.
    fn with_request<T>(
        &self,
        collection_id: &str,
        request_id: &str,
        change: impl FnOnce(&mut Request) -> Result<T>,
    ) -> Result<T> {
        let mut collection = self.load_collection(collection_id)?;
        let Some(Item::Request(request)) = collection.find_item_mut(request_id) else {
            return Err(CoreError::not_found("request", request_id).into());
        };
        let value = change(request)?;
        self.save_collection(&collection)?;
        Ok(value)
    }

    /// "Guardar como ejemplo": la respuesta `history_id` como ejemplo de la petición `request_id`.
    /// Sin nombre, se llama como su estado (`200 OK`).
    pub fn save_example(
        &self,
        collection_id: &str,
        request_id: &str,
        history_id: &str,
        name: Option<&str>,
    ) -> Result<Example> {
        let entry = self
            .history()
            .get(history_id)?
            .ok_or_else(|| CoreError::not_found("history", history_id))?;
        let status = entry.status.ok_or_else(|| {
            CoreError::InvalidRequest("that execution has no response to save".into())
        })?;
        if entry.res_body.as_ref().is_some_and(|b| b.len() > MAX_BODY) {
            return Err(CoreError::InvalidRequest(
                "the response is too large for an example (1 MB at most)".into(),
            )
            .into());
        }
        let status_text = reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|code| code.canonical_reason())
            .unwrap_or_default()
            .to_owned();
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(
            || format!("{status} {status_text}").trim().to_owned(),
            str::to_owned,
        );
        let mut example = Example::new(name, status);
        example.status_text = status_text;
        example.headers = pairs(&entry.res_headers);
        example.body = text(entry.res_body.as_ref());
        example.request = ExampleRequest {
            method: entry.method.clone(),
            url: entry.url.clone(),
            headers: pairs(&entry.req_headers),
            body: text(entry.req_body.as_ref()),
        };
        self.with_request(collection_id, request_id, |request| {
            request.examples.push(example.clone());
            Ok(example)
        })
    }

    pub fn rename_example(
        &self,
        collection_id: &str,
        request_id: &str,
        example_id: &str,
        name: &str,
    ) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::InvalidRequest("the example needs a name".into()).into());
        }
        self.with_request(collection_id, request_id, |request| {
            let example = request
                .examples
                .iter_mut()
                .find(|e| e.id == example_id)
                .ok_or_else(|| CoreError::not_found("example", example_id))?;
            example.name = name.to_owned();
            Ok(())
        })
    }

    pub fn delete_example(
        &self,
        collection_id: &str,
        request_id: &str,
        example_id: &str,
    ) -> Result<()> {
        self.with_request(collection_id, request_id, |request| {
            let before = request.examples.len();
            request.examples.retain(|e| e.id != example_id);
            if request.examples.len() == before {
                return Err(CoreError::not_found("example", example_id).into());
            }
            Ok(())
        })
    }
}
