//! Buscar y reemplazar en una colección (fase 4, como "Find and replace" de Postman): nombres, URL,
//! parámetros, cabeceras, cuerpo, auth, scripts, descripciones y variables.
//!
//! Las credenciales (contraseñas, tokens, secretos de cliente, valores de API key y variables
//! secretas) también se buscan y se reemplazan, pero su vista previa sale como `***`.

use regex::{NoExpand, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::{
    error::CoreError,
    model::{Auth, Body, Collection, Folder, Item, KeyValue, Request, Scripts, Variable},
};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FindOptions {
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    /// El texto es una expresión regular (y el reemplazo puede usar `$1`).
    #[serde(default)]
    pub regex: bool,
}

/// Coincidencias de un campo de una petición, carpeta o de la colección.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindMatch {
    /// Id de la petición o carpeta; vacío para la colección.
    pub item_id: String,
    /// `collection`, `folder` o `request`.
    pub kind: &'static str,
    /// `Colección/Carpeta/Petición`.
    pub path: String,
    /// `name`, `url`, `params`, `headers`, `body`, `auth`, `scripts.pre`, `scripts.test`,
    /// `description` o `variables`.
    pub field: &'static str,
    pub count: usize,
    /// El texto alrededor de la primera coincidencia (`***` si es una credencial).
    pub preview: String,
}

/// Un texto de la colección: dónde está y si es una credencial.
struct Place<'a> {
    item_id: &'a str,
    kind: &'static str,
    path: &'a str,
    field: &'static str,
    secret: bool,
}

type Visitor<'v> = dyn FnMut(&Place, &mut String) + 'v;

fn pairs(list: &mut [KeyValue], place: &Place, f: &mut Visitor) {
    for kv in list {
        f(place, &mut kv.key);
        f(place, &mut kv.value);
    }
}

fn scripts(scripts: &mut Scripts, item_id: &str, kind: &'static str, path: &str, f: &mut Visitor) {
    for (field, text) in [
        ("scripts.pre", &mut scripts.pre),
        ("scripts.test", &mut scripts.test),
    ] {
        let place = Place {
            item_id,
            kind,
            path,
            field,
            secret: false,
        };
        f(&place, text);
    }
}

fn auth(auth: &mut Auth, item_id: &str, kind: &'static str, path: &str, f: &mut Visitor) {
    let plain = Place {
        item_id,
        kind,
        path,
        field: "auth",
        secret: false,
    };
    let secret = Place {
        secret: true,
        ..plain
    };
    match auth {
        Auth::None | Auth::Inherit => {}
        Auth::Bearer { token } => f(&secret, token),
        Auth::Basic { username, password } | Auth::Digest { username, password } => {
            f(&plain, username);
            f(&secret, password);
        }
        Auth::ApiKey { key, value, .. } => {
            f(&plain, key);
            f(&secret, value);
        }
        Auth::OAuth2Cc(cc) => {
            for text in [&mut cc.token_url, &mut cc.client_id, &mut cc.scope] {
                f(&plain, text);
            }
            f(&secret, &mut cc.client_secret);
        }
        Auth::OAuth2Code(code) => {
            for text in [
                &mut code.auth_url,
                &mut code.token_url,
                &mut code.client_id,
                &mut code.scope,
                &mut code.redirect_uri,
            ] {
                f(&plain, text);
            }
            f(&secret, &mut code.client_secret);
        }
    }
}

fn request(request: &mut Request, parent: &str, f: &mut Visitor) {
    let path = format!("{parent}/{}", request.name);
    let id = request.id.clone();
    let place = |field| Place {
        item_id: &id,
        kind: "request",
        path: &path,
        field,
        secret: false,
    };
    f(&place("name"), &mut request.name);
    f(&place("url"), &mut request.url);
    pairs(&mut request.params, &place("params"), f);
    pairs(&mut request.headers, &place("headers"), f);
    let body = place("body");
    match &mut request.body {
        Body::None => {}
        Body::Raw { text, .. } => f(&body, text),
        Body::Urlencoded { fields } => pairs(fields, &body, f),
        Body::Multipart { fields } => {
            for field in fields {
                f(&body, &mut field.key);
                f(&body, &mut field.value);
                f(&body, &mut field.path);
            }
        }
        Body::Binary { path } => f(&body, path),
        Body::Graphql { query, variables } => {
            f(&body, query);
            f(&body, variables);
        }
    }
    auth(&mut request.auth, &id, "request", &path, f);
    scripts(&mut request.scripts, &id, "request", &path, f);
    f(&place("description"), &mut request.description);
}

fn folder(folder: &mut Folder, parent: &str, f: &mut Visitor) {
    let path = format!("{parent}/{}", folder.name);
    let id = folder.id.clone();
    let place = |field| Place {
        item_id: &id,
        kind: "folder",
        path: &path,
        field,
        secret: false,
    };
    f(&place("name"), &mut folder.name);
    f(&place("description"), &mut folder.description);
    if let Some(folder_auth) = &mut folder.auth {
        auth(folder_auth, &id, "folder", &path, f);
    }
    scripts(&mut folder.scripts, &id, "folder", &path, f);
    items(&mut folder.items, &path, f);
}

fn items(list: &mut [Item], parent: &str, f: &mut Visitor) {
    for item in list {
        match item {
            Item::Folder(sub) => folder(sub, parent, f),
            Item::Request(req) => request(req, parent, f),
        }
    }
}

fn variables(list: &mut [Variable], path: &str, f: &mut Visitor) {
    for var in list {
        let place = Place {
            item_id: "",
            kind: "collection",
            path,
            field: "variables",
            secret: false,
        };
        f(&place, &mut var.key);
        // Los secretos del llavero no tienen valor aquí.
        if let Some(value) = &mut var.value {
            let secret = Place {
                secret: var.secret,
                ..place
            };
            f(&secret, value);
        }
    }
}

/// Recorre todos los textos de la colección.
fn visit(collection: &mut Collection, f: &mut Visitor) {
    let path = collection.name.clone();
    let place = |field| Place {
        item_id: "",
        kind: "collection",
        path: &path,
        field,
        secret: false,
    };
    f(&place("description"), &mut collection.description);
    variables(&mut collection.variables, &path, f);
    if let Some(collection_auth) = &mut collection.auth {
        auth(collection_auth, "", "collection", &path, f);
    }
    scripts(&mut collection.scripts, "", "collection", &path, f);
    items(&mut collection.items, &path, f);
}

fn pattern(query: &str, options: &FindOptions) -> Result<Regex, CoreError> {
    if query.is_empty() {
        return Err(CoreError::InvalidRequest("the search text is empty".into()));
    }
    let mut source = if options.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    if options.whole_word {
        source = format!(r"\b(?:{source})\b");
    }
    RegexBuilder::new(&source)
        .case_insensitive(!options.case_sensitive)
        .size_limit(1 << 20)
        .build()
        .map_err(|error| CoreError::InvalidRequest(format!("invalid regular expression: {error}")))
}

/// Unos 30 caracteres antes y 50 después de la coincidencia, en una línea.
fn preview(text: &str, start: usize, end: usize) -> String {
    let before: String = text[..start]
        .chars()
        .rev()
        .take(30)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let after: String = text[end..].chars().take(50).collect();
    let line = format!("{before}{}{after}", &text[start..end]);
    let mut out = line.replace(['\r', '\n', '\t'], " ");
    if before.len() < start {
        out.insert(0, '…');
    }
    if after.len() < text.len() - end {
        out.push('…');
    }
    out
}

/// Dónde está `query` en la colección: una entrada por campo con coincidencias.
pub fn find(
    collection: &Collection,
    query: &str,
    options: &FindOptions,
) -> Result<Vec<FindMatch>, CoreError> {
    let regex = pattern(query, options)?;
    let mut copy = collection.clone();
    let mut out: Vec<FindMatch> = Vec::new();
    visit(&mut copy, &mut |place, text| {
        let mut found = regex.find_iter(text);
        let Some(first) = found.next() else {
            return;
        };
        let count = 1 + found.count();
        match out
            .iter_mut()
            .find(|m| m.item_id == place.item_id && m.field == place.field && m.kind == place.kind)
        {
            Some(existing) => existing.count += count,
            None => out.push(FindMatch {
                item_id: place.item_id.to_owned(),
                kind: place.kind,
                path: place.path.to_owned(),
                field: place.field,
                count,
                preview: if place.secret {
                    "***".into()
                } else {
                    preview(text, first.start(), first.end())
                },
            }),
        }
    });
    Ok(out)
}

/// Reemplaza `query` por `replacement` en toda la colección o solo en `targets`
/// (`(item_id, field)`, con `item_id` vacío para la colección). Devuelve cuántos cambios hizo.
pub fn replace(
    collection: &mut Collection,
    query: &str,
    replacement: &str,
    options: &FindOptions,
    targets: Option<&[(String, String)]>,
) -> Result<usize, CoreError> {
    let regex = pattern(query, options)?;
    let mut replaced = 0;
    visit(collection, &mut |place, text| {
        if let Some(targets) = targets
            && !targets
                .iter()
                .any(|(id, field)| id == place.item_id && field == place.field)
        {
            return;
        }
        let count = regex.find_iter(text).count();
        if count == 0 {
            return;
        }
        let new = if options.regex {
            regex.replace_all(text, replacement).into_owned()
        } else {
            regex.replace_all(text, NoExpand(replacement)).into_owned()
        };
        *text = new;
        replaced += count;
    });
    Ok(replaced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Folder, RawLang};

    fn collection() -> Collection {
        let mut c = Collection::new("Acme");
        c.variables = vec![
            Variable::new("apiUrl", "https://api.acme.test"),
            Variable::secret("token", "acme-fake-secret"),
        ];
        let mut folder = Folder::new("Accounts");
        folder.description = "Old API v1 endpoints".into();
        let mut req = Request::new("Sign v1", "POST", "{{apiUrl}}/v1/sign?mode=v1");
        req.headers = vec![KeyValue::new("X-Api-Version", "v1")];
        req.body = Body::Raw {
            lang: RawLang::Json,
            text: "{\n  \"version\": \"v1\"\n}".into(),
        };
        req.auth = Auth::Basic {
            username: "acme".into(),
            password: "pass-v1".into(),
        };
        req.scripts.test = "pm.test('v1', () => {});".into();
        folder.items.push(Item::Request(Box::new(req)));
        c.items.push(Item::Folder(folder));
        c
    }

    #[test]
    fn finds_by_field_and_hides_credentials() {
        let c = collection();
        let found = find(&c, "V1", &FindOptions::default()).unwrap();
        let fields: Vec<_> = found.iter().map(|m| (m.kind, m.field, m.count)).collect();
        assert_eq!(
            fields,
            [
                ("folder", "description", 1),
                ("request", "name", 1),
                ("request", "url", 2),
                ("request", "headers", 1),
                ("request", "body", 1),
                ("request", "auth", 1),
                ("request", "scripts.test", 1),
            ]
        );
        let url = found.iter().find(|m| m.field == "url").unwrap();
        assert_eq!(url.path, "Acme/Accounts/Sign v1");
        assert_eq!(url.preview, "{{apiUrl}}/v1/sign?mode=v1");
        assert_eq!(
            found.iter().find(|m| m.field == "auth").unwrap().preview,
            "***"
        );
        // Mayúsculas exactas y palabra completa.
        let exact = FindOptions {
            case_sensitive: true,
            ..FindOptions::default()
        };
        assert!(find(&c, "V1", &exact).unwrap().is_empty());
        let word = FindOptions {
            whole_word: true,
            ..FindOptions::default()
        };
        assert!(
            find(&c, "acme", &word)
                .unwrap()
                .iter()
                .any(|m| m.field == "auth")
        );
        assert!(
            !find(&c, "acm", &word)
                .unwrap()
                .iter()
                .any(|m| m.field == "auth")
        );
        // Una variable secreta coincide, pero no se enseña.
        let secret = find(&c, "fake-secret", &FindOptions::default()).unwrap();
        assert_eq!(secret[0].field, "variables");
        assert_eq!(secret[0].preview, "***");
        assert!(find(&c, "", &FindOptions::default()).is_err());
    }

    #[test]
    fn replaces_everywhere_or_only_targets() {
        let mut c = collection();
        let options = FindOptions::default();
        let request_id = match &c.items[0] {
            Item::Folder(folder) => folder.items[0].id().to_owned(),
            Item::Request(_) => unreachable!(),
        };
        let targets = [(request_id.clone(), "url".to_owned())];
        assert_eq!(
            replace(&mut c, "v1", "v2", &options, Some(&targets)).unwrap(),
            2
        );
        let Item::Folder(folder) = &c.items[0] else {
            unreachable!()
        };
        let Item::Request(req) = &folder.items[0] else {
            unreachable!()
        };
        assert_eq!(req.url, "{{apiUrl}}/v2/sign?mode=v2");
        assert_eq!(req.headers[0].value, "v1");

        assert_eq!(replace(&mut c, "v1", "$0-new", &options, None).unwrap(), 6);
        assert!(
            find(&c, "v1", &options)
                .unwrap()
                .iter()
                .all(|m| m.count == 1)
        );
        let Item::Folder(folder) = &c.items[0] else {
            unreachable!()
        };
        let Item::Request(req) = &folder.items[0] else {
            unreachable!()
        };
        // Sin regex, `$0` es texto.
        assert_eq!(req.headers[0].value, "$0-new");

        let regex = FindOptions {
            regex: true,
            ..FindOptions::default()
        };
        assert_eq!(
            replace(&mut c, r"v(\d)", "version-$1", &regex, None).unwrap(),
            2
        );
        assert!(replace(&mut c, "(", "x", &regex, None).is_err());
    }
}
