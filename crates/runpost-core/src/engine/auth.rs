//! Auth: none, inherit, bearer, basic, apikey (header/query). Digest firma el segundo intento en
//! `engine::send`; OAuth 2.0 llega aquí ya convertida en bearer (`executor`, con `engine::oauth`).

use base64::{Engine, engine::general_purpose::STANDARD};

use crate::{
    model::{ApiKeyIn, Auth, Collection, Folder, Request},
    vars::{Masker, Resolver},
};

/// Auth efectiva: la de la petición o la del primer padre (carpeta más interna → colección)
/// que no herede. Si nadie define una, `none`.
pub fn effective_auth(
    collection: Option<&Collection>,
    parents: &[&Folder],
    request: &Request,
) -> Auth {
    if request.auth != Auth::Inherit {
        return request.auth.clone();
    }
    parents
        .iter()
        .rev()
        .filter_map(|folder| folder.auth.as_ref())
        .chain(collection.and_then(|c| c.auth.as_ref()))
        .find(|auth| **auth != Auth::Inherit)
        .cloned()
        .unwrap_or(Auth::None)
}

/// Lo que la auth añade a la petición, ya resuelto.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub header: Option<(String, String)>,
    pub query: Option<(String, String)>,
}

/// Resuelve la auth. Si una credencial viene de un secreto, la forma derivada
/// (p. ej. el base64 de basic) se registra en `masker` para no filtrarla.
pub fn apply(
    auth: &Auth,
    resolver: &Resolver,
    masker: &mut Masker,
    unresolved: &mut Vec<String>,
) -> Applied {
    let mut resolve = |text: &str| {
        let resolved = resolver.resolve(text);
        unresolved.extend(resolved.unresolved.iter().cloned());
        resolved
    };
    match auth {
        Auth::None
        | Auth::Inherit
        | Auth::Digest { .. }
        | Auth::OAuth2Cc(_)
        | Auth::OAuth2Code(_) => Applied::default(),
        Auth::Bearer { token } => {
            let token = resolve(token);
            if token.text.is_empty() {
                return Applied::default();
            }
            let value = format!("Bearer {}", token.text);
            if token.secret {
                masker.add(&token.text);
            }
            Applied {
                header: Some(("Authorization".into(), value)),
                query: None,
            }
        }
        Auth::Basic { username, password } => {
            let user = resolve(username);
            let pass = resolve(password);
            let encoded = STANDARD.encode(format!("{}:{}", user.text, pass.text));
            if user.secret || pass.secret {
                masker.add(&encoded);
            }
            Applied {
                header: Some(("Authorization".into(), format!("Basic {encoded}"))),
                query: None,
            }
        }
        Auth::ApiKey {
            key,
            value,
            location,
        } => {
            let key = resolve(key).text;
            let value = resolve(value);
            if key.is_empty() {
                return Applied::default();
            }
            if value.secret {
                masker.add(&value.text);
            }
            let pair = Some((key, value.text));
            match location {
                ApiKeyIn::Header => Applied {
                    header: pair,
                    query: None,
                },
                ApiKeyIn::Query => Applied {
                    header: None,
                    query: pair,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Variable, vars::Scope};

    fn resolver() -> Resolver {
        Resolver::new().with_variables(
            Scope::Environment,
            &[
                Variable::secret("token", "s3cr3t"),
                Variable::new("user", "paul"),
            ],
        )
    }

    #[test]
    fn inheritance_walks_folders_then_collection() {
        let mut collection = Collection::new("c");
        let mut outer = Folder::new("outer");
        let mut inner = Folder::new("inner");
        let mut request = Request::new("r", "GET", "http://x");
        assert_eq!(effective_auth(None, &[], &request), Auth::None);
        collection.auth = Some(Auth::Bearer {
            token: "col".into(),
        });
        assert_eq!(
            effective_auth(Some(&collection), &[&outer, &inner], &request),
            Auth::Bearer {
                token: "col".into()
            }
        );
        outer.auth = Some(Auth::Bearer {
            token: "outer".into(),
        });
        inner.auth = Some(Auth::Inherit);
        assert_eq!(
            effective_auth(Some(&collection), &[&outer, &inner], &request),
            Auth::Bearer {
                token: "outer".into()
            }
        );
        inner.auth = Some(Auth::None);
        assert_eq!(
            effective_auth(Some(&collection), &[&outer, &inner], &request),
            Auth::None
        );
        request.auth = Auth::Bearer {
            token: "own".into(),
        };
        assert_eq!(
            effective_auth(Some(&collection), &[&outer, &inner], &request),
            Auth::Bearer {
                token: "own".into()
            }
        );
    }

    #[test]
    fn bearer_basic_apikey() {
        let resolver = resolver();
        let mut masker = resolver.masker().clone();
        let mut missing = Vec::new();
        let bearer = apply(
            &Auth::Bearer {
                token: "{{token}}".into(),
            },
            &resolver,
            &mut masker,
            &mut missing,
        );
        assert_eq!(
            bearer.header,
            Some(("Authorization".into(), "Bearer s3cr3t".into()))
        );
        assert_eq!(
            apply(
                &Auth::Bearer {
                    token: String::new()
                },
                &resolver,
                &mut masker,
                &mut missing
            ),
            Applied::default()
        );
        assert_eq!(
            apply(&Auth::None, &resolver, &mut masker, &mut missing),
            Applied::default()
        );

        let basic = apply(
            &Auth::Basic {
                username: "{{user}}".into(),
                password: "{{token}}".into(),
            },
            &resolver,
            &mut masker,
            &mut missing,
        );
        let (_, value) = basic.header.unwrap();
        assert_eq!(value, format!("Basic {}", STANDARD.encode("paul:s3cr3t")));
        assert_eq!(masker.mask(&value), "Basic ***");
        let plain = apply(
            &Auth::Basic {
                username: "a".into(),
                password: "b".into(),
            },
            &resolver,
            &mut masker,
            &mut missing,
        );
        assert_eq!(masker.mask(&plain.header.unwrap().1), "Basic YTpi");

        let header = apply(
            &Auth::ApiKey {
                key: "X-Api-Key".into(),
                value: "{{token}}".into(),
                location: ApiKeyIn::Header,
            },
            &resolver,
            &mut masker,
            &mut missing,
        );
        assert_eq!(header.header, Some(("X-Api-Key".into(), "s3cr3t".into())));
        let query = apply(
            &Auth::ApiKey {
                key: "api_key".into(),
                value: "{{nope}}".into(),
                location: ApiKeyIn::Query,
            },
            &resolver,
            &mut masker,
            &mut missing,
        );
        assert_eq!(query.query, Some(("api_key".into(), "{{nope}}".into())));
        assert_eq!(missing, ["nope"]);
        let empty_key = apply(
            &Auth::ApiKey {
                key: String::new(),
                value: "v".into(),
                location: ApiKeyIn::Header,
            },
            &resolver,
            &mut masker,
            &mut missing,
        );
        assert_eq!(empty_key, Applied::default());
    }
}
