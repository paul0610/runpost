// SPEC-DEVIATION: S0-D1: Paul eligió runpost_app/runpost_mcp para evitar colisiones en Windows.
// SPEC-DEVIATION: S0-D2: los scripts usan el CLI Tauri oficial local y fijado, sin cargo-tauri global.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use clap::Parser;
use runpost_core::{
    APP_NAME, WORKSPACE_ENV, Workspace, model::Theme as SettingsTheme, resolve_workspace,
};
use runpost_server::{ServerHandle, display_path};
use serde::{Deserialize, Serialize};
use tauri::{Manager, Theme, WebviewWindowBuilder, window::Color};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

#[derive(Parser)]
#[command(name = APP_NAME, version, author = "Paul Rojas")]
struct Arguments {
    #[arg(long, value_name = "DIR")]
    workspace: Option<PathBuf>,
    #[arg(long)]
    dev: bool,
}

struct Backend {
    server: Mutex<Option<ServerHandle>>,
    workspace: PathBuf,
}

#[derive(Serialize)]
struct BackendInfo {
    port: u16,
    token: String,
    mode: &'static str,
    workspace: String,
}

impl Backend {
    fn info(&self) -> Result<BackendInfo, String> {
        let server = self.server.lock().map_err(|_| "backend lock failed")?;
        let server = server.as_ref().ok_or("backend stopped")?;
        Ok(BackendInfo {
            port: server.port,
            token: server.token.expose().to_owned(),
            mode: "tauri",
            workspace: display_path(&self.workspace),
        })
    }

    fn stop(&self) {
        if let Ok(mut lock) = self.server.lock()
            && let Some(server) = lock.take()
            && let Err(error) = tauri::async_runtime::block_on(server.shutdown())
        {
            eprintln!("local server shutdown: {error:#}");
        }
    }
}

/// Fondo de la ventana antes de que cargue la SPA: igual a `--bg` de `tokens.css`.
fn background(theme: Theme) -> Color {
    match theme {
        Theme::Light => Color(0xff, 0xff, 0xff, 0xff),
        _ => Color(0x0f, 0x11, 0x15, 0xff),
    }
}

/// `"light" | "dark"` → tema fijo; cualquier otra cosa (`null`, `"system"`) → seguir al sistema.
fn parse_theme(theme: Option<&str>) -> Option<Theme> {
    match theme {
        Some("light") => Some(Theme::Light),
        Some("dark") => Some(Theme::Dark),
        _ => None,
    }
}

fn settings_theme(theme: SettingsTheme) -> Option<Theme> {
    match theme {
        SettingsTheme::Light => Some(Theme::Light),
        SettingsTheme::Dark => Some(Theme::Dark),
        SettingsTheme::System => None,
    }
}

/// Momento de arranque del proceso, para medir el arranque hasta la ventana usable (sección 14).
static STARTED: OnceLock<Instant> = OnceLock::new();
static UI_READY: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn backend_info(state: tauri::State<'_, Backend>) -> Result<BackendInfo, String> {
    // La SPA ya cargada pide `backend_info` lo primero: con RUNPOST_STARTUP_LOG se anota cuánto
    // tardó desde el arranque (lo usa scripts/dev/startup_time.py; nunca se registra el token).
    if std::env::var_os("RUNPOST_STARTUP_LOG").is_some()
        && !UI_READY.swap(true, Ordering::Relaxed)
        && let Some(started) = STARTED.get()
    {
        eprintln!(
            "startup: ui ready after {} ms",
            started.elapsed().as_millis()
        );
    }
    state.info()
}

#[derive(Deserialize)]
struct Filter {
    name: String,
    extensions: Vec<String>,
}

/// Diálogo nativo de abrir. Devuelve rutas (una o varias) o `null` si se cancela.
#[tauri::command]
async fn pick_file(
    app: tauri::AppHandle,
    filters: Option<Vec<Filter>>,
    multiple: Option<bool>,
) -> Result<Option<Vec<String>>, String> {
    let mut dialog = app.dialog().file();
    for filter in filters.unwrap_or_default() {
        let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(filter.name, &extensions);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    if multiple.unwrap_or(false) {
        dialog.pick_files(move |paths| {
            let _ = tx.send(paths);
        });
    } else {
        dialog.pick_file(move |path| {
            let _ = tx.send(path.map(|p| vec![p]));
        });
    }
    let picked = rx.await.map_err(|_| "dialog closed")?;
    Ok(picked.map(|paths| {
        paths
            .into_iter()
            .filter_map(|p| p.into_path().ok())
            .map(|p| display_path(&p))
            .collect()
    }))
}

/// Diálogo nativo de guardar. Devuelve la ruta elegida o `null`.
#[tauri::command]
async fn pick_save_path(
    app: tauri::AppHandle,
    default_name: String,
    filters: Option<Vec<Filter>>,
) -> Result<Option<String>, String> {
    let mut dialog = app.dialog().file().set_file_name(default_name);
    for filter in filters.unwrap_or_default() {
        let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(filter.name, &extensions);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let picked = rx.await.map_err(|_| "dialog closed")?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| display_path(&p)))
}

/// Sincroniza la barra de título nativa con el tema de la SPA (9.4).
#[tauri::command]
fn set_window_theme(window: tauri::WebviewWindow, theme: Option<String>) -> Result<(), String> {
    let theme = parse_theme(theme.as_deref());
    window.set_theme(theme).map_err(|e| e.to_string())?;
    let effective = theme.or_else(|| window.theme().ok()).unwrap_or(Theme::Dark);
    window
        .set_background_color(Some(background(effective)))
        .map_err(|e| e.to_string())
}

/// Abre la carpeta del workspace en el explorador del sistema.
#[tauri::command]
fn reveal_workspace(app: tauri::AppHandle, state: tauri::State<'_, Backend>) -> Result<(), String> {
    app.opener()
        .open_path(display_path(&state.workspace), None::<&str>)
        .map_err(|e| e.to_string())
}

fn main() {
    STARTED.get_or_init(Instant::now);
    if let Err(error) = run() {
        eprintln!("{APP_NAME}: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args = Arguments::parse();
    let root = resolve_workspace(args.workspace, std::env::var_os(WORKSPACE_ENV));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // La visibilidad la decide `setup` (tras fijar el color de fondo), no el estado guardado.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let workspace = Workspace::open(&root)?;
            let path = workspace.root().to_path_buf();
            let settings = workspace.settings();
            let theme = settings_theme(settings.theme);
            // El MCP por HTTP (sprint 6) escucha en un puerto fijo para que el agente se
            // configure una sola vez; la API de la UI sigue en un puerto aleatorio.
            let server = tauri::async_runtime::block_on(runpost_server::start_with_options(
                workspace,
                None,
                runpost_server::ServerOptions {
                    dev: args.dev,
                    browser: false,
                    mcp_port: settings.mcp.http_port,
                },
            ))?;
            app.manage(Backend {
                server: Mutex::new(Some(server)),
                workspace: path,
            });
            // Ventana creada aquí (no desde tauri.conf.json) para fijar tema y fondo antes de mostrarla.
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("main window missing in tauri.conf.json"))?;
            let window = WebviewWindowBuilder::from_config(app.handle(), &config)?
                .title(APP_NAME)
                .theme(theme)
                .background_color(background(theme.unwrap_or(Theme::Dark)))
                .visible(false)
                .build()?;
            if theme.is_none()
                && let Ok(system) = window.theme()
            {
                window.set_background_color(Some(background(system)))?;
            }
            window.show()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend_info,
            pick_file,
            pick_save_path,
            set_window_theme,
            reveal_workspace
        ])
        .build(tauri::generate_context!())?;
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            app.state::<Backend>().stop();
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workspace() {
        let arguments = Arguments::try_parse_from(["runpost_app", "--workspace", "fake"]).unwrap();
        assert_eq!(arguments.workspace, Some(PathBuf::from("fake")));
    }

    #[test]
    fn theme_mapping_and_background() {
        assert_eq!(parse_theme(Some("light")), Some(Theme::Light));
        assert_eq!(parse_theme(Some("dark")), Some(Theme::Dark));
        assert_eq!(parse_theme(Some("system")), None);
        assert_eq!(parse_theme(None), None);
        assert_eq!(settings_theme(SettingsTheme::System), None);
        assert_eq!(settings_theme(SettingsTheme::Light), Some(Theme::Light));
        let Color(r, g, b, _) = background(Theme::Dark);
        assert_eq!((r, g, b), (0x0f, 0x11, 0x15));
        let Color(r, ..) = background(Theme::Light);
        assert_eq!(r, 0xff);
    }

    #[tokio::test]
    async fn backend_info_reports_tauri_and_server_can_stop() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let workspace = Workspace::open(dir.path())?;
        let server = runpost_server::start(workspace, None).await?;
        let backend = Backend {
            server: Mutex::new(Some(server)),
            workspace: dir.path().to_path_buf(),
        };
        let info = backend.info().unwrap();
        assert_eq!(info.mode, "tauri");
        assert_ne!(info.port, 0);
        assert_eq!(info.token.len(), 64);
        assert!(!info.workspace.starts_with(r"\\?\"));
        let server = backend.server.lock().unwrap().take().unwrap();
        server.shutdown().await?;
        assert!(backend.info().is_err());
        Ok(())
    }
}
