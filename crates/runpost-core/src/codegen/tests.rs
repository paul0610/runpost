use super::*;
use crate::{
    model::{KeyValue, MultipartField, OAuth2Cc, Variable},
    vars::Scope,
};

fn resolver() -> Resolver {
    let mut off = Variable::new("off", "x");
    off.enabled = false;
    Resolver::new().with_variables(
        Scope::Environment,
        &[
            Variable::new("base", "https://api.test"),
            Variable::secret("token", "s3cr3t-T0ken"),
            Variable::new("user", "ana"),
            off,
        ],
    )
}

fn req(method: &str) -> Request {
    Request::new("r", method, "{{base}}/v2/ping")
}

fn lines(parts: &[&str]) -> String {
    parts.join(" \\\n")
}

const URL_LINE: &str = "curl --location --request POST 'https://api.test/v2/ping'";

fn curl_of(request: &Request, auth: &Auth) -> String {
    render(Lang::Curl, request, auth, &resolver())
}

// ---- curl: los mismos casos que "Copiar como curl" del S4 ----

#[test]
fn shell_single_quotes() {
    assert_eq!(sh("it's"), r"'it'\''s'");
}

#[test]
fn only_plain_variables_are_replaced() {
    assert_eq!(
        resolver().resolve_safe("{{base}}/{{token}}/{{$guid}}/{{nope}}/{{off}}"),
        "https://api.test/{{token}}/{{$guid}}/{{nope}}/{{off}}"
    );
}

#[test]
fn get_with_params_headers_and_secret_bearer() {
    let mut request = req("GET");
    request.params = vec![KeyValue::new("a", "1"), {
        let mut off = KeyValue::new("b", "2");
        off.enabled = false;
        off
    }];
    request.headers = vec![KeyValue::new("Accept", "application/json"), {
        let mut off = KeyValue::new("X-Off", "1");
        off.enabled = false;
        off
    }];
    let text = curl_of(
        &request,
        &Auth::Bearer {
            token: "{{token}}".into(),
        },
    );
    assert_eq!(
        text,
        lines(&[
            "curl --location 'https://api.test/v2/ping?a=1'",
            "--header 'Accept: application/json'",
            "--header 'Authorization: Bearer {{token}}'",
        ])
    );
    assert!(!text.contains("s3cr3t"));
}

#[test]
fn post_raw_json_adds_content_type_and_escapes_quotes() {
    let mut request = req("post");
    request.body = Body::Raw {
        lang: RawLang::Json,
        text: r#"{"user":"{{user}}","note":"it's"}"#.into(),
    };
    assert_eq!(
        curl_of(&request, &Auth::None),
        lines(&[
            URL_LINE,
            "--header 'Content-Type: application/json'",
            r#"--data-raw '{"user":"ana","note":"it'\''s"}'"#,
        ])
    );
}

#[test]
fn basic_user_api_key_in_query_and_header() {
    assert_eq!(
        curl_of(
            &req("GET"),
            &Auth::Basic {
                username: "{{user}}".into(),
                password: "{{token}}".into(),
            }
        ),
        lines(&[
            "curl --location 'https://api.test/v2/ping'",
            "--user 'ana:{{token}}'"
        ])
    );
    assert_eq!(
        curl_of(
            &req("GET"),
            &Auth::ApiKey {
                key: "api_key".into(),
                value: "{{token}}".into(),
                location: ApiKeyIn::Query,
            }
        ),
        "curl --location 'https://api.test/v2/ping?api_key={{token}}'"
    );
    let mut request = req("GET");
    request.headers = vec![KeyValue::new("x-api-key", "old")];
    assert_eq!(
        curl_of(
            &request,
            &Auth::ApiKey {
                key: "X-API-Key".into(),
                value: "k".into(),
                location: ApiKeyIn::Header,
            }
        ),
        lines(&[
            "curl --location 'https://api.test/v2/ping'",
            "--header 'X-API-Key: k'"
        ])
    );
}

#[test]
fn urlencoded_multipart_binary_and_graphql() {
    let mut request = req("POST");
    request.body = Body::Urlencoded {
        fields: vec![KeyValue::new("a", "{{user}}"), {
            let mut off = KeyValue::new("b", "");
            off.enabled = false;
            off
        }],
    };
    assert_eq!(
        curl_of(&request, &Auth::None),
        lines(&[URL_LINE, "--data-urlencode 'a=ana'"])
    );

    request.body = Body::Multipart {
        fields: vec![
            MultipartField {
                key: "f".into(),
                value: String::new(),
                kind: FieldKind::File,
                path: "C:/tmp/a.txt".into(),
                enabled: true,
            },
            MultipartField {
                key: "n".into(),
                value: "say \"hi\"".into(),
                kind: FieldKind::Text,
                path: String::new(),
                enabled: true,
            },
        ],
    };
    assert_eq!(
        curl_of(&request, &Auth::None),
        lines(&[
            URL_LINE,
            r#"--form 'f=@"C:/tmp/a.txt"'"#,
            r#"--form 'n="say \"hi\""'"#
        ])
    );

    request.body = Body::Binary {
        path: "C:/tmp/b.bin".into(),
    };
    assert_eq!(
        curl_of(&request, &Auth::None),
        lines(&[
            URL_LINE,
            "--header 'Content-Type: application/octet-stream'",
            "--data-binary '@C:/tmp/b.bin'"
        ])
    );

    request.body = Body::Graphql {
        query: "{ me { id } }".into(),
        variables: r#"{"u":"{{user}}"}"#.into(),
    };
    assert_eq!(
        curl_of(&request, &Auth::None),
        lines(&[
            URL_LINE,
            "--header 'Content-Type: application/json'",
            r#"--data-raw '{"query":"{ me { id } }","variables":{"u":"ana"}}'"#,
        ])
    );
}

// ---- fetch, PowerShell y Python ----

fn json_post() -> Request {
    let mut request = req("POST");
    request.headers = vec![KeyValue::new("X-Trace", "on")];
    request.body = Body::Raw {
        lang: RawLang::Json,
        text: r#"{"user":"{{user}}","admin":false,"tags":[],"n":null}"#.into(),
    };
    request
}

fn basic() -> Auth {
    Auth::Basic {
        username: "{{user}}".into(),
        password: "{{token}}".into(),
    }
}

#[test]
fn fetch_snippet() {
    assert_eq!(
        render(Lang::Fetch, &json_post(), &basic(), &resolver()),
        r#"const myHeaders = new Headers();
myHeaders.append("X-Trace", "on");
myHeaders.append("Content-Type", "application/json");
myHeaders.append("Authorization", "Basic " + btoa("ana:{{token}}"));

const raw = JSON.stringify({
  "user": "ana",
  "admin": false,
  "tags": [],
  "n": null
});

const requestOptions = {
  method: "POST",
  headers: myHeaders,
  body: raw,
  redirect: "follow"
};

fetch("https://api.test/v2/ping", requestOptions)
  .then((response) => response.text())
  .then((result) => console.log(result))
  .catch((error) => console.error(error));"#
    );
    let get = render(Lang::Fetch, &req("GET"), &Auth::None, &resolver());
    assert!(
        get.starts_with("const requestOptions = {\n  method: \"GET\",\n  redirect: \"follow\"\n};")
    );
}

#[test]
fn powershell_snippet() {
    assert_eq!(
        render(Lang::Powershell, &json_post(), &basic(), &resolver()),
        r#"$headers = New-Object "System.Collections.Generic.Dictionary[[String],[String]]"
$headers.Add('X-Trace', 'on')
$headers.Add('Content-Type', 'application/json')
$headers.Add('Authorization', 'Basic ' + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('ana:{{token}}')))

$body = @'
{
  "user": "ana",
  "admin": false,
  "tags": [],
  "n": null
}
'@

$response = Invoke-RestMethod 'https://api.test/v2/ping' -Method 'POST' -Headers $headers -Body $body
$response | ConvertTo-Json"#
    );
    let mut form = req("POST");
    form.body = Body::Urlencoded {
        fields: vec![KeyValue::new("it's", "$HOME")],
    };
    assert!(
        render(Lang::Powershell, &form, &Auth::None, &resolver())
            .contains("$body = @{\n    'it''s' = '$HOME'\n}")
    );
}

#[test]
fn python_snippet() {
    assert_eq!(
        render(Lang::Python, &json_post(), &basic(), &resolver()),
        r#"import requests
import json

url = "https://api.test/v2/ping"

payload = json.dumps({
  "user": "ana",
  "admin": False,
  "tags": [],
  "n": None
})
headers = {
  "X-Trace": "on",
  "Content-Type": "application/json"
}
auth = ("ana", "{{token}}")

response = requests.request("POST", url, headers=headers, data=payload, auth=auth)

print(response.text)"#
    );
    let mut upload = req("POST");
    upload.body = Body::Multipart {
        fields: vec![
            MultipartField {
                key: "doc".into(),
                value: String::new(),
                kind: FieldKind::File,
                path: "C:/tmp/a.txt".into(),
                enabled: true,
            },
            MultipartField {
                key: "n".into(),
                value: "1".into(),
                kind: FieldKind::Text,
                path: String::new(),
                enabled: true,
            },
        ],
    };
    let text = render(Lang::Python, &upload, &Auth::None, &resolver());
    assert!(text.contains("payload = [\n  (\"n\", \"1\")\n]"), "{text}");
    assert!(
        text.contains("files = [\n  (\"doc\", (\"a.txt\", open(\"C:/tmp/a.txt\", \"rb\")))\n]"),
        "{text}"
    );
    assert!(text.contains("requests.request(\"POST\", url, data=payload, files=files)"));
}

#[test]
fn no_language_leaks_a_secret() {
    let mut request = json_post();
    request.url = "{{base}}/v2/{{token}}?q={{token}}".into();
    request.headers.push(KeyValue::new("X-Key", "{{token}}"));
    for lang in Lang::ALL {
        for auth in [
            basic(),
            Auth::Bearer {
                token: "{{token}}".into(),
            },
        ] {
            let text = render(lang, &request, &auth, &resolver());
            assert!(!text.contains("s3cr3t"), "{lang:?}: {text}");
            assert!(text.contains("{{token}}"), "{lang:?}");
        }
    }
}

#[test]
fn language_names() {
    assert_eq!(Lang::parse("PowerShell"), Some(Lang::Powershell));
    assert_eq!(Lang::parse("js"), Some(Lang::Fetch));
    assert_eq!(Lang::parse("go"), None);
    assert!(
        Lang::ALL
            .iter()
            .all(|l| Lang::parse(l.as_str()) == Some(*l))
    );
}

#[test]
fn digest_and_oauth2_snippets_never_carry_the_secret() {
    let request = req("GET");
    let digest = Auth::Digest {
        username: "{{user}}".into(),
        password: "{{token}}".into(),
    };
    let curl = render(Lang::Curl, &request, &digest, &resolver());
    assert!(curl.contains("--digest --user 'ana:{{token}}'"), "{curl}");
    let python = render(Lang::Python, &request, &digest, &resolver());
    assert!(python.contains("from requests.auth import HTTPDigestAuth"));
    assert!(python.contains("auth = HTTPDigestAuth(\"ana\", \"{{token}}\")"));
    let powershell = render(Lang::Powershell, &request, &digest, &resolver());
    assert!(
        powershell.contains("-Credential $credential"),
        "{powershell}"
    );
    let fetch = render(Lang::Fetch, &request, &digest, &resolver());
    assert!(fetch.starts_with("// Digest auth: fetch cannot answer the challenge"));

    let oauth = Auth::oauth2_cc(OAuth2Cc {
        token_url: "{{base}}/oauth/token".into(),
        client_id: "acme".into(),
        client_secret: "{{token}}".into(),
        scope: String::new(),
        client_auth: crate::model::ClientAuth::Basic,
    });
    for lang in Lang::ALL {
        let code = render(lang, &request, &oauth, &resolver());
        assert!(code.contains("Bearer {{access_token}}"), "{lang:?}: {code}");
        assert!(code.contains("OAuth 2.0: {{access_token}} is the token"));
        assert!(!code.contains("s3cr3t"));
    }
}
