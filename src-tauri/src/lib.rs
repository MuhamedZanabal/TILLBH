//! TILLBH desktop shell.
//!
//! The shell is deliberately thin: it resolves the data folder, opens the
//! core, starts background services and exposes ONE typed IPC command
//! (`rpc`) that routes to the transport-neutral command dispatcher. There is no
//! IPC path to SQL, the file system or a shell.

mod hello;
mod secrets;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tauri::Manager;
use tillbh_core::service::{AppCore, LEGACY_DB_FILE, MARKER_FILE};
use tillbh_core::AppError;
use tillbh_hub::Runtime;

struct AppState {
    rt: Option<Arc<Runtime>>,
    startup_error: Option<AppError>,
}

#[tauri::command]
async fn rpc(state: tauri::State<'_, AppState>, cmd: String, token: Option<String>, args: Option<Value>) -> Result<Value, AppError> {
    match (&state.rt, &state.startup_error) {
        (Some(rt), _) => rt.dispatch(&cmd, token, args.unwrap_or_else(|| serde_json::json!({}))).await,
        (None, Some(e)) => Err(e.clone()),
        _ => Err(AppError::internal("The application did not start.")),
    }
}

fn dir_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path).ok().and_then(|mut entries| entries.next()).is_some()
}

/// Prefer an already-initialized TILLBH data folder. On an upgrade where the
/// installer has only created an empty TILLBH folder, keep using the deployed
/// AMWAPOS data folder when it contains a database or initialization marker.
/// This preserves the entire data set (database, images and local backups)
/// without a risky recursive move, while clean installs use the TILLBH path.
fn select_data_dir(current: PathBuf, legacy: PathBuf) -> PathBuf {
    if dir_has_entries(&current) {
        return current;
    }
    if legacy.join(LEGACY_DB_FILE).is_file() || legacy.join(MARKER_FILE).is_file() {
        return legacy;
    }
    current
}

/// Store data outside the install folder so upgrades and uninstalls never
/// touch business records. Clean Windows installs use %ProgramData%\TILLBH\data.
/// Existing AMWAPOS installs retain their legacy data directory until an
/// explicit, separately verified directory migration is performed. Override
/// both behaviors with TILLBH_DATA_DIR.
fn data_dir(app: &tauri::App) -> PathBuf {
    if let Ok(d) = std::env::var("TILLBH_DATA_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        if let Ok(pd) = std::env::var("PROGRAMDATA") {
            let root = PathBuf::from(pd);
            return select_data_dir(root.join("TILLBH").join("data"), root.join("AMWAPOS").join("data"));
        }
    }
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("tillbh-data")).join("data")
}

fn init_logging(dir: &std::path::Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let logs = dir.parent().unwrap_or(dir).join("logs");
    std::fs::create_dir_all(&logs).ok()?;
    // Daily files, 30 kept: a till runs for years and must not fill its disk with logs.
    let appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("tillbh")
        .filename_suffix("log")
        .max_log_files(30)
        .build(&logs)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .json()
        .with_writer(writer)
        .with_env_filter(std::env::var("TILLBH_LOG").unwrap_or_else(|_| "info".into()))
        .with_current_span(false)
        .init();
    Some(guard)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch focuses the running window instead of opening the database twice.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            let dir = data_dir(app);
            let guard = init_logging(&dir);
            // Keep the log writer alive for the process lifetime.
            app.manage(LogGuard(guard));
            tracing::info!(version = tillbh_core::audit::APP_VERSION, build = env!("TILLBH_BUILD_SHA"), data_dir = %dir.display(), "TILLBH starting");
            let state = match AppCore::open(&dir, Arc::new(secrets::OsSecretStore)) {
                Ok(core) => {
                    let rt = Runtime::new(Arc::new(core));
                    rt.set_resources(app.path().resource_dir().ok().as_deref());
                    *rt.step_up.lock().unwrap() = Some(Arc::new(hello::verify));
                    let rt2 = rt.clone();
                    tauri::async_runtime::spawn(async move { rt2.ensure_services() });
                    AppState { rt: Some(rt), startup_error: None }
                }
                Err(e) => {
                    tracing::error!(code = ?e.code, error = %e.message, "startup failed");
                    AppState { rt: None, startup_error: Some(e) }
                }
            };
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .run(tauri::generate_context!())
        .expect("error while running TILLBH");
}

struct LogGuard(#[allow(dead_code)] Option<tracing_appender::non_blocking::WorkerGuard>);

#[cfg(test)]
mod tests {
    use super::select_data_dir;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root(name: &str) -> PathBuf {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("tillbh-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn empty_installer_created_current_dir_does_not_hide_legacy_store() {
        let base = root("legacy-dir");
        let current = base.join("TILLBH").join("data");
        let legacy = base.join("AMWAPOS").join("data");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("amwapos.db"), b"legacy").unwrap();

        assert_eq!(select_data_dir(current.clone(), legacy.clone()), legacy);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn nonempty_tillbh_dir_always_takes_precedence() {
        let base = root("current-dir");
        let current = base.join("TILLBH").join("data");
        let legacy = base.join("AMWAPOS").join("data");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(current.join("store.marker"), b"current").unwrap();
        std::fs::write(legacy.join("amwapos.db"), b"legacy").unwrap();

        assert_eq!(select_data_dir(current.clone(), legacy), current);
        let _ = std::fs::remove_dir_all(base);
    }
}
