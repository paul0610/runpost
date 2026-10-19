use super::*;
use crate::import::parse_document;

const SPEC: &str = r##"openapi: 3.0.3
info:
  title: Acme Accounts
  version: 1.2.0
  description: Accounts API
servers:
  - url: https://{region}.api.acme.test/v1/
    variables:
      region:
        default: eu
security:
  - bearerAuth: []
tags:
  - name: Accounts
    description: Account operations
paths:
  /accounts:
    get:
      tags: [Accounts]
      summary: List accounts
      parameters:
        - name: page
          in: query
          schema: { type: integer, example: 2 }
        - name: limit
          in: query
          required: true
          example: 50
        - name: X-Trace
          in: header
          schema: { type: string, default: on }
      responses:
        200:
          description: ok
    post:
      tags: [Accounts]
      operationId: createAccount
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/NewAccount'
      responses:
        '201':
          description: created
  /accounts/{accountId}:
    parameters:
      - name: accountId
        in: path
        required: true
        schema: { type: string }
    delete:
      summary: Delete account
      security: []
      responses:
        '204': { description: deleted }
  /health:
    get:
      responses: { '200': { description: ok } }
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
  schemas:
    NewAccount:
      type: object
      properties:
        currency: { type: string, enum: [usd, eur] }
        owner: { $ref: '#/components/schemas/Owner' }
        tags: { type: array, items: { type: string } }
        active: { type: boolean }
    Owner:
      type: object
      properties:
        email: { type: string, format: email }
        age: { type: integer }
"##;

fn request(item: &Item) -> &Request {
    match item {
        Item::Request(request) => request,
        Item::Folder(folder) => panic!("folder {}", folder.name),
    }
}

#[test]
fn operations_tags_parameters_bodies_and_security() {
    let doc = parse_document(SPEC).unwrap();
    assert!(detect(&doc));
    let imported = import(&doc).unwrap();
    let collection = &imported.collections[0];
    assert_eq!(collection.name, "Acme Accounts");
    assert_eq!(collection.description, "Accounts API\n\nVersion: 1.2.0");
    assert_eq!(
        collection.auth,
        Some(Auth::Bearer {
            token: "{{bearerToken}}".into()
        })
    );
    let vars: Vec<_> = collection
        .variables
        .iter()
        .map(|v| (v.key.as_str(), v.value.as_deref().unwrap_or(""), v.secret))
        .collect();
    assert_eq!(
        vars,
        [
            ("baseUrl", "https://eu.api.acme.test/v1", false),
            ("accountId", "", false),
            ("bearerToken", "", true)
        ]
    );

    let Item::Folder(folder) = &collection.items[0] else {
        panic!("first item is the Accounts folder")
    };
    assert_eq!(
        (folder.name.as_str(), folder.description.as_str()),
        ("Accounts", "Account operations")
    );
    let list = request(&folder.items[0]);
    assert_eq!(
        (list.name.as_str(), list.method.as_str(), list.url.as_str()),
        ("List accounts", "GET", "{{baseUrl}}/accounts")
    );
    let params: Vec<_> = list
        .params
        .iter()
        .map(|p| (p.key.as_str(), p.value.as_str(), p.enabled))
        .collect();
    assert_eq!(params, [("page", "2", false), ("limit", "50", true)]);
    assert_eq!(list.headers, vec![KeyValue::new("X-Trace", "on")]);
    assert_eq!(list.auth, Auth::Inherit);

    let create = request(&folder.items[1]);
    assert_eq!(create.name, "createAccount");
    let Body::Raw { lang, text } = &create.body else {
        panic!("{:?}", create.body)
    };
    assert_eq!(*lang, RawLang::Json);
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap(),
        json!({"currency": "usd", "owner": {"email": "user@example.com", "age": 0},
               "tags": ["string"], "active": true})
    );

    let delete = request(&collection.items[1]);
    assert_eq!(
        (delete.name.as_str(), delete.url.as_str()),
        ("Delete account", "{{baseUrl}}/accounts/{{accountId}}")
    );
    assert_eq!(delete.auth, Auth::None);
    assert_eq!(request(&collection.items[2]).name, "GET /health");
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
}

#[test]
fn forms_files_and_unsupported() {
    let doc = json!({"openapi": "3.1.0", "info": {"title": "Upload"}, "paths": {"/files": {"post": {
        "parameters": [{"name": "sid", "in": "cookie"}],
        "requestBody": {"content": {"multipart/form-data": {"schema": {"type": "object", "properties": {
            "file": {"type": "string", "format": "binary"},
            "note": {"type": ["string", "null"], "default": "hi"}}}}}}}}}});
    let imported = import(&doc).unwrap();
    let upload = request(&imported.collections[0].items[0]);
    let Body::Multipart { fields } = &upload.body else {
        panic!("{:?}", upload.body)
    };
    assert_eq!(fields[0].kind, FieldKind::File);
    assert_eq!(
        (fields[1].key.as_str(), fields[1].value.as_str()),
        ("note", "hi")
    );
    assert!(imported.warnings.iter().any(|w| w.contains("no servers")));
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("cookie parameter sid"))
    );

    let swagger = json!({"swagger": "2.0", "info": {"title": "Old"}, "paths": {}});
    assert!(detect(&swagger));
    assert!(
        import(&swagger)
            .unwrap_err()
            .to_string()
            .contains("Swagger 2.0")
    );
}
