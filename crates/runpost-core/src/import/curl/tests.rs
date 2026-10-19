use super::*;

fn pairs(rows: &[KeyValue]) -> Vec<(&str, &str)> {
    rows.iter()
        .map(|r| (r.key.as_str(), r.value.as_str()))
        .collect()
}

#[test]
fn postman_bash_with_escaped_quote() {
    let request = parse(
        r#"curl --location --request POST 'https://api.test/v1/items?x=1' \
--header 'Content-Type: application/json' \
--header 'Authorization: Bearer {{token}}' \
--data-raw '{"name": "it'\''s"}'"#,
    )
    .unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, "https://api.test/v1/items");
    assert_eq!(request.name, "https://api.test/v1/items");
    assert_eq!(pairs(&request.params), [("x", "1")]);
    assert_eq!(
        pairs(&request.headers),
        [
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer {{token}}")
        ]
    );
    assert_eq!(
        request.body,
        Body::Raw {
            lang: RawLang::Json,
            text: r#"{"name": "it's"}"#.into()
        }
    );
}

#[test]
fn chrome_bash_with_ansi_c_quotes() {
    let request = parse(
        r#"curl 'https://api.test/graphql' \
  -H 'accept: */*' \
  -H 'content-type: application/json' \
  --data-raw $'{"query":"{ me }","v":"a\\nb"}' \
  --compressed"#,
    )
    .unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.body,
        Body::Raw {
            lang: RawLang::Json,
            text: r#"{"query":"{ me }","v":"a\nb"}"#.into()
        }
    );
    assert_eq!(request.headers.len(), 2);
}

#[test]
fn chrome_cmd_with_carets() {
    let request = parse(
        r#"curl "https://api.test/v1/search?q=a^&b" ^
  -H "accept: application/json" ^
  --data-raw "^{^\^"q^\^":1^}" ^
  --compressed"#,
    )
    .unwrap();
    assert_eq!(request.url, "https://api.test/v1/search");
    assert_eq!(pairs(&request.params), [("q", "a"), ("b", "")]);
    assert_eq!(
        request.body,
        Body::Raw {
            lang: RawLang::Json,
            text: r#"{"q":1}"#.into()
        }
    );
}

#[test]
fn powershell_backticks_and_urlencoded_body() {
    let request = parse(
        "curl.exe -X PUT \"https://api.test/v1/items/7\" `\n  -H \"Content-Type: application/x-www-form-urlencoded\" `\n  -d \"name=Ana%20Mar&tags=a+b\"",
    )
    .unwrap();
    assert_eq!(request.method, "PUT");
    assert!(request.headers.is_empty(), "{:?}", request.headers);
    assert_eq!(
        request.body,
        Body::Urlencoded {
            fields: vec![
                KeyValue::new("name", "Ana Mar"),
                KeyValue::new("tags", "a b")
            ]
        }
    );
}

#[test]
fn combined_flags_get_basic_insecure_and_timeout() {
    let request = parse(
        "curl -sSL -u ana:{{password}} -G https://api.test/s -d q=1 -d page=2 -k --max-time 2.5",
    )
    .unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(pairs(&request.params), [("q", "1"), ("page", "2")]);
    assert_eq!(request.body, Body::None);
    assert_eq!(
        request.auth,
        Auth::Basic {
            username: "ana".into(),
            password: "{{password}}".into()
        }
    );
    assert_eq!(request.settings.verify_tls, Some(false));
    assert_eq!(request.settings.follow_redirects, Some(true));
    assert_eq!(request.settings.timeout_ms, Some(2500));

    let attached = parse("curl -sXPOST https://api.test/a -H'X-A: 1'").unwrap();
    assert_eq!(attached.method, "POST");
    assert_eq!(pairs(&attached.headers), [("X-A", "1")]);
    let separate = parse("curl -sX DELETE https://api.test/a").unwrap();
    assert_eq!(separate.method, "DELETE");
}

#[test]
fn json_form_binary_and_urlencode() {
    let json = parse(r#"curl --json '{"a":1}' https://api.test/x"#).unwrap();
    assert_eq!(json.method, "POST");
    assert_eq!(
        pairs(&json.headers),
        [
            ("Content-Type", "application/json"),
            ("Accept", "application/json")
        ]
    );

    let form =
        parse("curl -F 'file=@C:/tmp/a.txt;type=text/plain' -F 'name=Ana' https://api.test/up")
            .unwrap();
    let Body::Multipart { fields } = &form.body else {
        panic!("{:?}", form.body);
    };
    assert_eq!(fields[0].kind, FieldKind::File);
    assert_eq!(fields[0].path, "C:/tmp/a.txt");
    assert_eq!(
        (fields[1].key.as_str(), fields[1].value.as_str()),
        ("name", "Ana")
    );

    let binary = parse("curl -XPATCH https://api.test/f --data-binary @./body.bin").unwrap();
    assert_eq!(binary.method, "PATCH");
    assert_eq!(
        binary.body,
        Body::Binary {
            path: "./body.bin".into()
        }
    );

    let encoded = parse("curl https://api.test/q --data-urlencode 'q=hello world'").unwrap();
    assert_eq!(
        encoded.body,
        Body::Urlencoded {
            fields: vec![KeyValue::new("q", "hello world")]
        }
    );
}

#[test]
fn errors_and_detection() {
    assert!(parse("curl -H 'A: b'").is_err());
    assert!(parse("wget https://x").is_err());
    assert!(parse("curl 'https://x").is_err());
    assert!(looks_like("curl https://x"));
    assert!(looks_like("  curl.exe -X GET"));
    assert!(!looks_like("{\"info\": {}}"));
}
