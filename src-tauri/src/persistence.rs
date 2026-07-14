use std::path::PathBuf;

use tauri::Manager;
use tokio::sync::watch;
use tokio::time::Duration;

use crate::state::AppData;

const DEBOUNCE_MS: u64 = 250;

/// Returns the absolute path to the application data JSON file.
///
/// # Arguments
/// - `app`: Handle to the running Tauri application.
///
/// # Preconditions/Assumptions
/// - The platform app-data directory is accessible and the Tauri path resolver
///   returns a valid result. Panics if the directory cannot be resolved.
fn data_path(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("data.json")
}

/// Loads `AppData` from disk, returning a default value on any failure.
///
/// If the data file does not exist, `AppData::default()` is returned silently.
/// If the file exists but cannot be read or parsed, an error is printed to
/// stderr and `AppData::default()` is returned.
///
/// # Arguments
/// - `app`: Handle to the running Tauri application.
///
/// # Preconditions/Assumptions
/// - Failures are non-fatal; the caller always receives a usable `AppData`.
pub fn load(app: &tauri::AppHandle) -> AppData {
    let path = data_path(app);
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<AppData>(&content) {
                Ok(mut data) => {
                    // Densify/repair Context `order` values (and migrate state
                    // saved before the field existed) before handing them out.
                    data.normalize_order();
                    return data;
                },
                Err(e) => eprintln!("Failed to parse data file: {e}"),
            },
            Err(e) => eprintln!("Failed to read data file: {e}"),
        }
    }
    AppData::default()
}

/// Serializes `AppData` and writes it to the data file, creating parent
/// directories as needed.
///
/// Errors are printed to stderr but are otherwise non-fatal; the caller is not
/// notified of failure.
///
/// # Arguments
/// - `app`: Handle to the running Tauri application.
/// - `data`: The application state to persist.
///
/// # Preconditions/Assumptions
/// - Should only be called from the saver task spawned by `spawn_saver`; direct
///   calls elsewhere bypass the debounce and may cause excessive disk writes.
pub fn save(app: &tauri::AppHandle, data: &AppData) {
    let path = data_path(app);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create data directory: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(data) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                eprintln!("Failed to write data file: {e}");
            }
        },
        Err(e) => eprintln!("Failed to serialize data: {e}"),
    }
}

/// Spawns a background task that debounce-saves `AppData` whenever the watch
/// channel receives a new value.
///
/// Each change notification triggers a `DEBOUNCE_MS`-millisecond sleep before
/// writing. Any additional changes that arrive during the sleep are absorbed so
/// that only the latest snapshot is written, preventing excessive disk I/O
/// during rapid successive mutations.
///
/// The task runs until the `watch::Sender` held in `AppState` is dropped (i.e.
/// for the entire lifetime of the application).
///
/// # Arguments
/// - `app`: Handle to the running Tauri application, passed to `save`.
/// - `rx`: Receiver end of the `AppData` watch channel. Changes are signalled
///   by the `AppState::save_tx` sender after every state mutation.
///
/// # Preconditions/Assumptions
/// - Must be called after `AppState` is registered with `app.manage(...)`.
/// - The corresponding `watch::Sender` must outlive this task (guaranteed by
///   `AppState` being managed for the application lifetime).
///
/// # Invariants
/// - At most one write is issued per `DEBOUNCE_MS` window, even under
///   continuous state mutations.
pub fn spawn_saver(app: tauri::AppHandle, mut rx: watch::Receiver<AppData>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if rx.changed().await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
            // Absorb any writes that arrived during the sleep window
            let _ = rx.has_changed();
            let data = rx.borrow_and_update().clone();
            save(&app, &data);
        }
    });
}
