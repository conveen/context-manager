//! Settings commands, plus the app-window helpers reached from the native
//! menu and tray.

use tauri::{Emitter, Manager};

use super::visibility;
use crate::state::{AppState, Settings};

/// Updates application settings and saves to disk.
///
/// If the update turns Single Context Mode **on**, or changes the chosen Context
/// while it is on, the chosen Context is force-shown — which, because the mode is
/// now on, causes the show logic to hide every other Context. Unrelated settings
/// edits (meta key, launch-at-login, toggling the mode off) don't move any
/// windows. The chosen Context is resolved from `single_context_id`, falling back
/// to Main when it is unset or names a Context that no longer exists.
#[tauri::command]
pub fn update_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
    // Store settings and decide whether to enforce single-context visibility,
    // all under a brief lock. `visibility::show` (which re-acquires the lock)
    // is called only after the lock is released.
    let enforce_id = {
        let state = app.state::<AppState>();
        let mut data = state.data.lock().unwrap();
        let was_enabled = data.settings.single_context_mode;
        let prev_target = data.settings.single_context_id.clone();
        data.settings = settings;
        let _ = state.save_tx.send(data.clone());

        let target_changed = data.settings.single_context_id != prev_target;
        if data.settings.single_context_mode && (!was_enabled || target_changed) {
            data.settings
                .single_context_id
                .clone()
                .filter(|id| data.contexts.iter().any(|c| &c.id == id))
                .or_else(|| data.contexts.iter().find(|c| c.is_main).map(|c| c.id.clone()))
        } else {
            None
        }
    };

    if let Some(id) = enforce_id {
        visibility::show(&app, &id)?;
        // Nudge the frontend to refresh visibility indicators immediately.
        let _ = app.emit("contexts-changed", ());
    }
    Ok(())
}

/// Opens the settings view: shows/focuses the main window and emits the
/// `show-settings` event so the frontend switches to the settings panel.
/// Reached only from the application menu's Settings item; deliberately not a
/// Tauri command — the frontend opens its settings panel directly.
pub fn open_settings(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("show-settings", ());
    }
    Ok(())
}

/// Shows and focuses the main window. Reached only from the tray menu's
/// "Open Context Manager" item; the frontend runs inside this window, so this
/// is deliberately not a Tauri command.
pub fn open_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Opens DevTools for the main window (debug builds only).
#[tauri::command]
#[cfg(debug_assertions)]
pub fn open_devtools(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        win.open_devtools();
    }
}
