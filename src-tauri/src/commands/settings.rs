//! Settings commands, plus the app-window helpers reached from the native
//! menu and tray.

use tauri::{Emitter, Manager};

use super::visibility;
use crate::hotkeys;
use crate::state::{AppState, Settings};

/// Updates application settings and saves to disk.
///
/// If the update turns Single Context Mode **on**, or changes the chosen Context
/// while it is on, the chosen Context is force-shown — which, because the mode is
/// now on, causes the show logic to hide every other Context. Unrelated settings
/// edits (meta key, toggling the mode off) don't move any
/// windows. The chosen Context is resolved from `single_context_id`, falling back
/// to Main when it is unset or names a Context that no longer exists.
///
/// If the update changes `meta_key`, all global shortcuts are unregistered and
/// re-registered under the new modifier so the change takes effect immediately.
/// If the new registration fails (e.g. the combination is claimed by another
/// application), the previous modifier is restored — in settings and in the OS
/// registration — and an error is returned.
#[tauri::command]
pub fn update_settings<R: tauri::Runtime>(app: tauri::AppHandle<R>, settings: Settings) -> Result<(), String> {
    // Store settings and decide whether to enforce single-context visibility,
    // all under a brief lock. `visibility::show` (which re-acquires the lock)
    // is called only after the lock is released.
    let (enforce_id, prev_meta) = {
        let state = app.state::<AppState>();
        let mut data = state.data.lock().unwrap();
        let was_enabled = data.settings.single_context_mode;
        let prev_target = data.settings.single_context_id.clone();
        let prev_meta = data.settings.meta_key.clone();
        data.settings = settings;
        let _ = state.save_tx.send(data.clone());

        let target_changed = data.settings.single_context_id != prev_target;
        let enforce_id = if data.settings.single_context_mode && (!was_enabled || target_changed) {
            data.settings
                .single_context_id
                .clone()
                .filter(|id| data.contexts.iter().any(|c| &c.id == id))
                .or_else(|| data.contexts.iter().find(|c| c.is_main).map(|c| c.id.clone()))
        } else {
            None
        };
        let prev_meta = (data.settings.meta_key != prev_meta).then_some(prev_meta);
        (enforce_id, prev_meta)
    };

    // Rebind the global shortcuts under the new modifier. On failure, roll the
    // modifier back so the shortcuts don't go dead entirely (the previous one
    // was registered successfully before, so restoring it should succeed).
    if let Some(prev_meta) = prev_meta {
        if let Err(e) = hotkeys::reregister_all(&app) {
            let state = app.state::<AppState>();
            {
                let mut data = state.data.lock().unwrap();
                data.settings.meta_key = prev_meta;
                let _ = state.save_tx.send(data.clone());
            }
            if let Err(restore_err) = hotkeys::reregister_all(&app) {
                eprintln!("failed to restore previous shortcut modifier: {restore_err}");
            }
            return Err(format!("failed to apply the new shortcut modifier: {e}"));
        }
    }

    if let Some(id) = enforce_id {
        visibility::show(&app, &id)?;
        // Nudge the frontend to refresh visibility indicators immediately.
        let _ = app.emit(crate::events::CONTEXTS_CHANGED, ());
    }
    Ok(())
}

/// Opens the settings view: shows/focuses the main window and emits the
/// `show-settings` event so the frontend switches to the settings panel.
/// Reached only from the application menu's Settings item; deliberately not a
/// Tauri command — the frontend opens its settings panel directly.
pub fn open_settings<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit(crate::events::SHOW_SETTINGS, ());
    }
    Ok(())
}

/// Shows and focuses the main window. Reached only from the tray menu's
/// "Open Context Manager" item; the frontend runs inside this window, so this
/// is deliberately not a Tauri command.
pub fn open_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Opens DevTools for the main window (debug builds only).
#[tauri::command]
#[cfg(debug_assertions)]
pub fn open_devtools<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        win.open_devtools();
    }
}
