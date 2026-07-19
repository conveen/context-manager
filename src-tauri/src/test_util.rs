//! Shared helpers for backend tests: a mock Tauri app wired to a real
//! [`AppState`], plus terse constructors for test fixtures.
//!
//! Compiled only for `cfg(test)`. The mock app runs under
//! [`tauri::test::MockRuntime`], so command bodies execute exactly as in
//! production — minus the OS: the `wm` and hotkey layers route to their
//! scripted mocks in test builds.

use std::sync::Mutex;

use tauri::test::MockRuntime;
use tauri::Manager;
use tokio::sync::watch;

use crate::state::{AppData, AppState, Context, MetaKey, Settings, WindowRef};

/// Builds a mock app managing `data` as its [`AppState`], resetting the `wm`
/// and hotkey mocks first.
///
/// Returns the app plus the receiver end of the save channel: a test can call
/// `rx.has_changed()` to assert whether the code under test signalled the
/// persistence worker (the initial value counts as seen).
pub fn mock_app(data: AppData) -> (tauri::App<MockRuntime>, watch::Receiver<AppData>) {
    crate::wm::mock::reset();
    crate::hotkeys::mock::reset();
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app");
    let (save_tx, save_rx) = watch::channel(data.clone());
    app.manage(AppState { data: Mutex::new(data), save_tx });
    (app, save_rx)
}

/// A `WindowRef` with deterministic name/title derived from `platform_id`.
pub fn win(platform_id: u64, hidden: bool) -> WindowRef {
    WindowRef {
        platform_id,
        #[cfg(target_os = "macos")]
        pid: platform_id as u32 + 1,
        app_name: format!("App{platform_id}"),
        window_title: format!("Win{platform_id}"),
        hidden,
        #[cfg(target_os = "macos")]
        hidden_z: None,
    }
}

/// A non-Main `Context` with the given windows and no shortcut.
pub fn ctx(id: &str, visible: bool, windows: Vec<WindowRef>) -> Context {
    Context {
        id: id.to_string(),
        name: format!("name-{id}"),
        is_main: false,
        windows,
        shortcut_index: None,
        order: 0,
        visible,
    }
}

/// The Main Context (`shortcut_index` 0) with the given windows.
pub fn main_ctx(id: &str, visible: bool, windows: Vec<WindowRef>) -> Context {
    Context {
        id: id.to_string(),
        name: "main".to_string(),
        is_main: true,
        windows,
        shortcut_index: Some(0),
        order: 0,
        visible,
    }
}

/// An `AppData` with default settings and the given Contexts (normalized so
/// `order` values are dense, as they would be after a load).
pub fn app_data(contexts: Vec<Context>) -> AppData {
    let mut data = AppData {
        contexts,
        settings: Settings { meta_key: MetaKey::CtrlAlt, single_context_mode: false, single_context_id: None },
    };
    data.normalize_order();
    data
}
