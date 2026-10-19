use std::{path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use runpost_core::{
    APP_SLUG, Executor, Source, TOKEN_HEADER, WORKSPACE_ENV, Workspace,
    export::postman_v21::export_collection,
    import::postman_v21::PostmanKind,
    resolve_workspace,
    runner::{self, RunnerOptions},
    secrets,
    storage::fs::write_atomic,
};

/// `--version`: versión, autor y repositorio (sección 21).
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (Paul Rojas, ",
    env!("CARGO_PKG_REPOSITORY"),
    ")"
);

#[derive(Debug, Parser)]
#[command(name = APP_SLUG, version = VERSION, author = "Paul Rojas", about = "Cliente HTTP local y servidor MCP")]
struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    workspace: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve {
        #[arg(long)]
        browser: bool,
        #[arg(long, default_value_t = 7878)]
        port: u16,
        #[arg(long)]
        open: bool,
        #[arg(long)]
        dev: bool,
    },
    Mcp,
    #[command(
        about = "Imprime la cabecera con el token de la app abierta (headersHelper de Claude Code y Codex)"
    )]
    McpHeaders {
        /// Puerto del MCP por HTTP; por defecto el de la URL que pasa Claude Code
        /// (CLAUDE_CODE_MCP_SERVER_URL) o 7879.
        #[arg(long)]
        port: Option<u16>,
    },
    #[command(about = "Ejecuta una colección o carpeta con sus scripts (Collection Runner)")]
    Run {
        /// Colección: id, nombre o slug.
        collection: String,
        /// Carpeta: id, ruta ("A/B") o nombre.
        #[arg(long)]
        folder: Option<String>,
        /// Entorno: id o nombre; por defecto, el activo.
        #[arg(long)]
        env: Option<String>,
        /// Por defecto, una por fila de datos (o 1).
        #[arg(long)]
        iterations: Option<u32>,
        /// Archivo de datos CSV o JSON (pm.iterationData).
        #[arg(long, value_name = "FILE")]
        data: Option<PathBuf>,
        /// Pausa entre peticiones, en milisegundos.
        #[arg(long, default_value_t = 0, value_name = "MS")]
        delay: u64,
        #[arg(long, value_enum, default_value_t = Out::Pretty)]
        out: Out,
        /// Para en el primer test fallido o petición sin respuesta.
        #[arg(long)]
        fail_fast: bool,
        /// Archivo de salida; sin él, a stdout.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    #[command(
        about = "Importa al workspace una colección o entorno Postman v2.1, OpenAPI 3, HAR, Insomnia o Bruno (archivo o carpeta)"
    )]
    Import {
        file: PathBuf,
        /// Por defecto se detecta por el contenido.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
    },
    #[command(about = "Exporta una colección (id, nombre o slug) a Postman v2.1")]
    Export {
        collection: String,
        #[arg(long, value_enum, default_value_t = Format::PostmanV21)]
        format: Format,
        /// Vacía los valores de las variables secretas.
        #[arg(long)]
        strip_secrets: bool,
        /// Archivo de salida; sin él, a stdout.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Kind {
    Collection,
    Environment,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Out {
    Pretty,
    Json,
    Junit,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    #[value(name = "postman_v21")]
    PostmanV21,
}

/// `runpost run` sale con 1 si un test falla o una petición no obtiene respuesta, y con 2 si no
/// pudo ni empezar (colección, carpeta, entorno o datos inválidos).
const RUN_FAILED: u8 = 1;
const RUN_INVALID: u8 = 2;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let run = matches!(cli.command, Command::Run { .. });
    match execute(cli).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{APP_SLUG}: {error:#}");
            if run {
                ExitCode::from(RUN_INVALID)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

/// `{"X-RunPost-Token":"…"}` a partir de lo que la app publicó en el llavero, o `{}`.
fn headers_json(session: Option<&str>) -> String {
    session
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| value["token"].as_str().map(str::to_owned))
        .map(|token| serde_json::json!({ TOKEN_HEADER: token }).to_string())
        .unwrap_or_else(|| "{}".to_owned())
}

/// Puerto de `http://127.0.0.1:7879/mcp` (la URL que Claude Code da al `headersHelper`).
fn port_of(url: &str) -> Option<u16> {
    let authority = url.split("://").nth(1)?.split('/').next()?;
    authority.rsplit_once(':')?.1.parse().ok()
}

/// `headersHelper` de los clientes MCP por HTTP: solo escribe el JSON en stdout. Sin la app
/// abierta imprime `{}` (el cliente no podrá conectar, como es de esperar) y lo explica en stderr.
fn mcp_headers(port: Option<u16>) -> String {
    let port = port
        .or_else(|| {
            std::env::var("CLAUDE_CODE_MCP_SERVER_URL")
                .ok()
                .and_then(|url| port_of(&url))
        })
        .unwrap_or(runpost_core::model::MCP_HTTP_PORT);
    match secrets::from_env().get(&secrets::session_account(port)) {
        Ok(Some(session)) => headers_json(Some(&session)),
        Ok(None) => {
            eprintln!(
                "{APP_SLUG}: no hay ninguna app de RunPost sirviendo MCP en el puerto {port}"
            );
            headers_json(None)
        }
        Err(error) => {
            eprintln!("{APP_SLUG}: {error}");
            headers_json(None)
        }
    }
}

async fn execute(cli: Cli) -> Result<ExitCode> {
    if let Command::McpHeaders { port } = cli.command {
        // Sin abrir el workspace: no crea carpetas ni toca el historial.
        println!("{}", mcp_headers(port));
        return Ok(ExitCode::SUCCESS);
    }
    let root = resolve_workspace(cli.workspace, std::env::var_os(WORKSPACE_ENV));
    let workspace = Workspace::open(&root)?;
    match cli.command {
        Command::Mcp => runpost_mcp::serve(workspace).await?,
        Command::McpHeaders { .. } => unreachable!("handled before opening the workspace"),
        Command::Serve {
            browser,
            port,
            open,
            dev,
        } => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .init();
            let mcp_port = workspace.settings().mcp.http_port;
            let server = runpost_server::start_with_options(
                workspace,
                Some(port),
                runpost_server::ServerOptions {
                    browser,
                    dev,
                    mcp_port,
                },
            )
            .await?;
            let url = format!("http://127.0.0.1:{}", server.port);
            eprintln!("{APP_SLUG}: {url} (Ctrl+C para salir)");
            if let Some(mcp_port) = server.mcp_port {
                eprintln!("{APP_SLUG}: MCP para agentes en http://127.0.0.1:{mcp_port}/mcp");
            }
            if open && browser {
                open::that(&url)?;
            }
            tokio::signal::ctrl_c().await?;
            server.shutdown().await?;
        }
        Command::Import { file, kind } => {
            let kind = kind.map(|kind| match kind {
                Kind::Collection => PostmanKind::Collection,
                Kind::Environment => PostmanKind::Environment,
            });
            for report in workspace.import_path(&file, None, kind)? {
                for warning in &report.warnings {
                    eprintln!("{APP_SLUG}: aviso: {warning}");
                }
                let from = match report.format {
                    "postman_v21" => String::new(),
                    other => format!(" [{other}]"),
                };
                let action = if report.replaced {
                    "reemplazada"
                } else {
                    "importada"
                };
                match report.kind {
                    PostmanKind::Collection => println!(
                        "colección {action}{from}: \"{}\" ({}) con {} carpetas y {} peticiones -> {}",
                        report.name, report.id, report.folders, report.requests, report.file
                    ),
                    PostmanKind::Environment => println!(
                        "entorno {}{from}: \"{}\" ({}) con {} variables -> {}",
                        if report.replaced {
                            "reemplazado"
                        } else {
                            "importado"
                        },
                        report.name,
                        report.id,
                        report.variables,
                        report.file
                    ),
                }
            }
        }
        Command::Export {
            collection,
            format: Format::PostmanV21,
            strip_secrets,
            output,
        } => {
            let collection = workspace.find_collection(&collection)?;
            let mut text =
                serde_json::to_string_pretty(&export_collection(&collection, strip_secrets))?;
            text.push('\n');
            match output {
                Some(path) => {
                    write_atomic(&path, text.as_bytes())
                        .with_context(|| format!("cannot write {}", path.display()))?;
                    eprintln!(
                        "{APP_SLUG}: exportada \"{}\" -> {}",
                        collection.name,
                        path.display()
                    );
                }
                None => print!("{text}"),
            }
        }
        Command::Run {
            collection,
            folder,
            env,
            iterations,
            data,
            delay,
            out,
            fail_fast,
            output,
        } => {
            let (rows, data_name) = match &data {
                Some(path) => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("cannot read {}", path.display()))?;
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    (
                        runner::data::parse(name.as_deref().unwrap_or_default(), &text)?,
                        name,
                    )
                }
                None => (Vec::new(), None),
            };
            let opts = RunnerOptions {
                folder,
                env,
                iterations,
                data: rows,
                data_name,
                delay_ms: delay,
                fail_fast,
                source: Source::Cli,
                ..RunnerOptions::default()
            };
            // Ctrl+C para la ejecución y aun así se imprime el informe parcial.
            let cancel = opts.cancel.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    cancel.cancel();
                }
            });
            let report = Executor::new()
                .run_collection(&workspace, &collection, opts, None)
                .await?;
            let text = match out {
                Out::Pretty => runner::report::to_pretty(&report),
                Out::Json => serde_json::to_string_pretty(&report)? + "\n",
                Out::Junit => runner::report::to_junit(&report),
            };
            match output {
                Some(path) => {
                    write_atomic(&path, text.as_bytes())
                        .with_context(|| format!("cannot write {}", path.display()))?;
                    eprintln!("{APP_SLUG}: informe -> {}", path.display());
                }
                None => print!("{text}"),
            }
            return Ok(if report.failed() {
                ExitCode::from(RUN_FAILED)
            } else {
                ExitCode::SUCCESS
            });
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_headers_prints_only_the_token_header() {
        assert_eq!(
            headers_json(Some(r#"{"port":7879,"token":"abc123"}"#)),
            r#"{"X-RunPost-Token":"abc123"}"#
        );
        assert_eq!(headers_json(None), "{}");
        assert_eq!(headers_json(Some("not json")), "{}");
        let cli = Cli::try_parse_from(["runpost_mcp", "mcp-headers", "--port", "7979"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::McpHeaders { port: Some(7979) }
        ));
        assert_eq!(port_of("http://127.0.0.1:7979/mcp"), Some(7979));
        assert_eq!(port_of("http://localhost:7879"), Some(7879));
        assert_eq!(port_of("no url"), None);
    }

    #[test]
    fn parses_global_workspace_before_and_after_command() {
        for arguments in [
            ["runpost_mcp", "--workspace", "fake", "mcp"],
            ["runpost_mcp", "mcp", "--workspace", "fake"],
        ] {
            assert_eq!(
                Cli::try_parse_from(arguments).unwrap().workspace,
                Some(PathBuf::from("fake"))
            );
        }
    }
}
