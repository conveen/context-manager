//! Settings commands, plus the app-window helpers reached from the native
//! menu and tray.

use tauri::{Emitter, Manager};

use super::visibility;
use crate::hotkeys;
use crate::state::{AppState, Settings};

/// Updates application settings and saves to disk.
///
/// Enabling Single Context Mode (SCM) when more than one Context is visible,
/// or changing the chosen Context while it's enabled, causes window visibility changes.
/// If SCM is enabled with one Context visible, it becomes the `single_context_id` and no windows change
/// (the frontend must re-read settings because the chosen ID is overwritten).
/// Unrelated settings edits (meta key, toggling the mode off) don't move any windows.
/// The chosen Context is resolved from `single_context_id`, falling back
/// to Main when it is unset or names a Context that no longer exists.
///
/// If the update changes `meta_key`, all global shortcuts are unregistered and
/// re-registered under the new modifier so the change takes effect immediately.
/// Accelerators the OS refuses (e.g. ones another application already owns) are
/// reported to the frontend by [`crate::hotkeys::register_all`] and otherwise
/// tolerated. Only a modifier under which *nothing* registers is rejected: the
/// previous modifier is restored in settings and in the OS registration, and an
/// error is returned.
#[tauri::command]
pub fn update_settings(app: tauri::AppHandle, settings: Settings) -> Result<(), String> {
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

        let turning_on = data.settings.single_context_mode && !was_enabled;
        let target_changed = data.settings.single_context_id != prev_target;
        // On the off→on transition, a screen already showing exactly one
        // Context *is* a single-Context view, so adopt that Context as the
        // choice and enforce nothing — switching the user away from what they
        // are looking at, to whatever the dropdown happens to name, is
        // surprising. Any other visible count (zero, or several) has no current
        // Context to preserve and collapses to the chosen one as before.
        let shown = turning_on.then(|| data.single_visible()).flatten();

        let enforce_id = if let Some(i) = shown {
            data.settings.single_context_id = Some(data.contexts[i].id.clone());
            None
        } else if data.settings.single_context_mode && (turning_on || target_changed) {
            data.settings
                .single_context_id
                .clone()
                .filter(|id| data.contexts.iter().any(|c| &c.id == id))
                .or_else(|| data.contexts.iter().find(|c| c.is_main).map(|c| c.id.clone()))
        } else {
            None
        };

        // Sent after the adoption above so the persisted choice matches what is
        // on screen.
        let _ = state.save_tx.send(data.clone());
        let prev_meta = (data.settings.meta_key != prev_meta).then_some(prev_meta);
        (enforce_id, prev_meta)
    };

    // Rebind the global shortcuts under the new modifier. A partial failure is
    // kept: the accelerators that did register work, and `register_all` has
    // already reported the refused ones to the frontend. Only a modifier that
    // registers *nothing* is rolled back, since that leaves the user with no
    // shortcuts at all (the previous modifier registered successfully before,
    // so restoring it should succeed).
    if let Some(prev_meta) = prev_meta {
        let total_failure = match hotkeys::reregister_all(&app) {
            Ok(reg) => reg.all_failed().then(|| {
                "the OS refused every shortcut — another application may already own that combination".to_string()
            }),
            Err(e) => Some(e),
        };
        if let Some(reason) = total_failure {
            let state = app.state::<AppState>();
            {
                let mut data = state.data.lock().unwrap();
                data.settings.meta_key = prev_meta;
                let _ = state.save_tx.send(data.clone());
            }
            if let Err(restore_err) = hotkeys::reregister_all(&app) {
                eprintln!("failed to restore previous shortcut modifier: {restore_err}");
            }
            return Err(format!("failed to apply the new shortcut modifier: {reason}"));
        }
    }

    if let Some(id) = enforce_id {
        visibility::show(&app, &id)?;
        // Nudge the frontend to refresh visibility indicators immediately.
        let _ = app.emit("contexts-changed", ());
    }
    Ok(())
}

/// Returns the accelerators the OS refused at the last (re)registration, e.g.
/// `["Ctrl+Alt+3"]`; empty when every Context shortcut is live.
///
/// Exists for the startup case: shortcuts are registered during `setup`, long
/// before the webview can listen for the `shortcuts-failed` event, so the
/// frontend reads the recorded list once on mount instead of missing the only
/// notification it would ever get.
#[tauri::command]
pub fn get_failed_shortcuts(app: tauri::AppHandle) -> Vec<String> {
    app.state::<AppState>().failed_shortcuts.lock().unwrap().clone()
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
