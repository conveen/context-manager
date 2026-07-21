use std::path::{Path, PathBuf};

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
fn data_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> PathBuf {
    app.path().app_data_dir().unwrap().join("data.json")
}

/// Loads `AppData` from `path`, returning a default value on any failure.
///
/// If the data file does not exist, `AppData::default()` is returned silently.
/// If the file exists but cannot be read or parsed, an error is printed to
/// stderr and `AppData::default()` is returned.
///
/// # Preconditions/Assumptions
/// - Failures are non-fatal; the caller always receives a usable `AppData`.
pub fn load_from(path: &Path) -> AppData {
    if path.exists() {
        match std::fs::read_to_string(path) {
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

/// Loads `AppData` from the platform app-data directory ([`load_from`]).
///
/// # Arguments
/// - `app`: Handle to the running Tauri application.
pub fn load<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> AppData {
    load_from(&data_path(app))
}

/// Serializes `AppData` and writes it to `path`, creating parent directories
/// as needed.
///
/// Errors are printed to stderr but are otherwise non-fatal; the caller is not
/// notified of failure.
///
/// # Preconditions/Assumptions
/// - Should only be called from the saver task spawned by `spawn_saver`; direct
///   calls elsewhere bypass the debounce and may cause excessive disk writes.
pub fn save_to(path: &Path, data: &AppData) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create data directory: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(data) {
        Ok(content) => {
            if let Err(e) = std::fs::write(path, content) {
                eprintln!("Failed to write data file: {e}");
            }
        },
        Err(e) => eprintln!("Failed to serialize data: {e}"),
    }
}

/// Debounce-saves `AppData` to `path` whenever the watch channel receives a
/// new value; the body of the saver task spawned by [`spawn_saver`].
///
/// Each change notification triggers a `DEBOUNCE_MS`-millisecond sleep before
/// writing. Any additional changes that arrive during the sleep are absorbed so
/// that only the latest snapshot is written, preventing excessive disk I/O
/// during rapid successive mutations.
///
/// Runs until the corresponding `watch::Sender` is dropped.
///
/// # Invariants
/// - At most one write is issued per `DEBOUNCE_MS` window, even under
///   continuous state mutations.
pub(crate) async fn run_saver(path: PathBuf, mut rx: watch::Receiver<AppData>) {
    loop {
        if rx.changed().await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
        // borrow_and_update absorbs any writes that arrived during the
        // sleep window, so only the latest snapshot is written.
        let data = rx.borrow_and_update().clone();
        save_to(&path, &data);
    }
}

/// Spawns the background task running [`run_saver`] against the platform
/// app-data path.
///
/// # Arguments
/// - `app`: Handle to the running Tauri application, used to resolve the path.
/// - `rx`: Receiver end of the `AppData` watch channel. Changes are signalled
///   by the `AppState::save_tx` sender after every state mutation.
///
/// # Preconditions/Assumptions
/// - Must be called after `AppState` is registered with `app.manage(...)`.
/// - The corresponding `watch::Sender` must outlive this task (guaranteed by
///   `AppState` being managed for the application lifetime).
pub fn spawn_saver<R: tauri::Runtime>(app: tauri::AppHandle<R>, rx: watch::Receiver<AppData>) {
    let path = data_path(&app);
    tauri::async_runtime::spawn(run_saver(path, rx));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{app_data, ctx, main_ctx, win};

    /// Asserts `data` matches the shape of `AppData::default()`. Defaults
    /// can't be compared directly — each carries a freshly generated Main
    /// Context UUID.
    fn assert_is_default(data: &AppData) {
        assert_eq!(data.contexts.len(), 1);
        assert!(data.contexts[0].is_main);
        assert!(data.contexts[0].windows.is_empty());
        assert!(!data.settings.single_context_mode);
    }

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_is_default(&load_from(&dir.path().join("does-not-exist.json")));
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        std::fs::write(&path, "{ not valid json").unwrap();
        assert_is_default(&load_from(&path));
    }

    #[test]
    fn save_to_load_from_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("data.json");
        let data = app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", false, vec![win(1, true)])]);
        save_to(&path, &data);
        assert_eq!(load_from(&path), data);
    }
}
