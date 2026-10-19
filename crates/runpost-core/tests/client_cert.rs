//! Certificados de cliente por host (TLS mutuo) contra un servidor rustls que los exige. La CA y
//! los certificados se generan en la prueba con rcgen: el repositorio no guarda ninguna clave.

use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use runpost_core::{
    Executor, NetworkError, RequestDraft, RunContext, Source, Workspace, model::ClientCert,
    secrets::MemoryKeyring,
};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

struct Pki {
    acceptor: TlsAcceptor,
    client_cert: String,
    client_key: String,
}

/// CA de prueba, certificado de servidor para `localhost` y uno de cliente firmado por la CA.
fn pki() -> Pki {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "RunPost test CA");
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();

    let server_key = KeyPair::generate().unwrap();
    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params.signed_by(&server_key, &ca).unwrap();

    let client_key = KeyPair::generate().unwrap();
    let mut client_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    client_params
        .distinguished_name
        .push(DnType::CommonName, "acme-client");
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_cert = client_params.signed_by(&client_key, &ca).unwrap();

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .build()
    .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![server_cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
        )
        .unwrap();
    Pki {
        acceptor: TlsAcceptor::from(Arc::new(config)),
        client_cert: client_cert.pem(),
        client_key: client_key.serialize_pem(),
    }
}

/// Servidor HTTPS que solo contesta a quien presenta un certificado de la CA.
async fn serve(acceptor: TlsAcceptor) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // Sin certificado de cliente, el apretón de manos falla aquí.
                let Ok(mut tls) = acceptor.accept(stream).await else {
                    return;
                };
                let mut buffer = vec![0u8; 4096];
                let mut read = 0;
                while let Ok(n) = tls.read(&mut buffer[read..]).await {
                    if n == 0 {
                        break;
                    }
                    read += n;
                    if buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") || read == buffer.len() {
                        break;
                    }
                }
                let body = "mutual TLS ok";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(response.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    port
}

fn draft(url: String) -> RequestDraft {
    RequestDraft {
        method: "GET".into(),
        url,
        ..RequestDraft::default()
    }
}

#[tokio::test]
async fn the_certificate_goes_only_to_its_host() {
    let pki = pki();
    let port = serve(pki.acceptor.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("client.pem");
    let key_path = dir.path().join("client.key");
    std::fs::write(&cert_path, &pki.client_cert).unwrap();
    std::fs::write(&key_path, &pki.client_key).unwrap();

    let ws_dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open_with_keyring(ws_dir.path(), Arc::new(MemoryKeyring::new())).unwrap();
    let mut settings = ws.settings();
    // La CA de prueba no está en el almacén del sistema.
    settings.http.verify_tls = false;
    settings.http.certificates = vec![ClientCert {
        host: "localhost".into(),
        port: Some(port),
        cert: cert_path.to_string_lossy().into_owned(),
        key: key_path.to_string_lossy().into_owned(),
    }];
    ws.save_settings(&settings).unwrap();
    let executor = Executor::new();

    let ok = executor
        .send_adhoc(
            &ws,
            draft(format!("https://localhost:{port}/v1/ping")),
            RunContext::new(Source::Cli),
        )
        .await
        .unwrap();
    assert_eq!(ok.response.status, 200);
    assert_eq!(&ok.response.body[..], b"mutual TLS ok");

    // Otro host (la misma máquina por IP): no se presenta el certificado y el servidor corta.
    let refused = executor
        .send_adhoc(
            &ws,
            draft(format!("https://127.0.0.1:{port}/v1/ping")),
            RunContext::new(Source::Cli),
        )
        .await
        .unwrap_err();
    assert!(
        refused.downcast_ref::<NetworkError>().is_some(),
        "{refused:#}"
    );

    // Certificado y clave en un mismo PEM (lo que deja openssl al convertir un PFX).
    let combined = dir.path().join("client-combined.pem");
    std::fs::write(&combined, format!("{}{}", pki.client_cert, pki.client_key)).unwrap();
    let combined = combined.to_string_lossy().into_owned();
    settings.http.certificates[0].cert = combined.clone();
    settings.http.certificates[0].key = combined;
    ws.save_settings(&settings).unwrap();
    let ok = executor
        .send_adhoc(
            &ws,
            draft(format!("https://localhost:{port}/v1/ping")),
            RunContext::new(Source::Cli),
        )
        .await
        .unwrap();
    assert_eq!(ok.response.status, 200);

    // Una clave que no existe: error claro antes de enviar.
    settings.http.certificates[0].cert = cert_path.to_string_lossy().into_owned();
    settings.http.certificates[0].key = dir
        .path()
        .join("missing.key")
        .to_string_lossy()
        .into_owned();
    ws.save_settings(&settings).unwrap();
    let error = executor
        .send_adhoc(
            &ws,
            draft(format!("https://localhost:{port}/v1/ping")),
            RunContext::new(Source::Cli),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("client certificate for localhost: cannot read the key"),
        "{error}"
    );
}
