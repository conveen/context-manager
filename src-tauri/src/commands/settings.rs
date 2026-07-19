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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tauri::Listener;

    use super::*;
    use crate::hotkeys::mock as hotkey_mock;
    use crate::state::{AppData, MetaKey};
    use crate::test_util::{app_data, ctx, main_ctx, mock_app, win};
    use crate::wm::mock as wm_mock;

    fn visible(data: &AppData, id: &str) -> bool {
        data.contexts.iter().find(|c| c.id == id).unwrap().visible
    }

    /// `update_settings` takes a full Settings value; build one from the
    /// current state with the given tweaks applied.
    fn settings_from(app: &tauri::AppHandle<tauri::test::MockRuntime>, f: impl FnOnce(&mut Settings)) -> Settings {
        let state = app.state::<AppState>();
        let mut settings = state.data.lock().unwrap().settings.clone();
        f(&mut settings);
        settings
    }

    #[test]
    fn enabling_single_context_mode_force_shows_the_chosen_context() {
        let (app, _rx) =
            mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", false, vec![win(2, true)])]));
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        app.listen_any(crate::events::CONTEXTS_CHANGED, move |_| {
            h.fetch_add(1, Ordering::SeqCst);
        });

        let new = settings_from(app.handle(), |s| {
            s.single_context_mode = true;
            s.single_context_id = Some("a".to_string());
        });
        update_settings(app.handle().clone(), new).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(visible(&data, "a"), "the chosen Context is force-shown");
        assert!(!visible(&data, "m"), "other Contexts are hidden under the mode");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "the frontend is nudged to refresh");
    }

    #[test]
    fn stale_or_missing_choice_falls_back_to_main() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", false, vec![win(1, true)]), ctx("a", true, vec![])]));
        let new = settings_from(app.handle(), |s| {
            s.single_context_mode = true;
            s.single_context_id = Some("deleted-long-ago".to_string());
        });
        update_settings(app.handle().clone(), new).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(visible(&data, "m"), "a stale id resolves to Main");
        assert!(!visible(&data, "a"));
    }

    #[test]
    fn changing_the_choice_while_the_mode_is_on_switches_contexts() {
        let mut initial = app_data(vec![main_ctx("m", false, vec![]), ctx("a", true, vec![]), ctx("b", false, vec![])]);
        initial.settings.single_context_mode = true;
        initial.settings.single_context_id = Some("a".to_string());
        let (app, _rx) = mock_app(initial);

        let new = settings_from(app.handle(), |s| s.single_context_id = Some("b".to_string()));
        update_settings(app.handle().clone(), new).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(visible(&data, "b"));
        assert!(!visible(&data, "a"));
    }

    #[test]
    fn unrelated_edits_move_no_windows_and_touch_no_hotkeys() {
        // Turning the mode OFF (and leaving the meta key alone) is the
        // documented no-op case for window movement and rebinding.
        let mut initial = app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", false, vec![])]);
        initial.settings.single_context_mode = true;
        let (app, rx) = mock_app(initial);

        let new = settings_from(app.handle(), |s| s.single_context_mode = false);
        update_settings(app.handle().clone(), new).unwrap();

        assert!(wm_mock::calls().is_empty(), "no windows move");
        assert!(hotkey_mock::calls().is_empty(), "no rebinding");
        assert!(rx.has_changed().unwrap(), "the edit itself is saved");
    }

    #[test]
    fn changing_the_meta_key_rebinds_the_global_shortcuts() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![])]));
        let new = settings_from(app.handle(), |s| s.meta_key = MetaKey::CmdOpt);
        update_settings(app.handle().clone(), new).unwrap();

        assert_eq!(hotkey_mock::calls(), vec![hotkey_mock::Call::Reregister]);
        let state = app.state::<AppState>();
        assert_eq!(state.data.lock().unwrap().settings.meta_key, MetaKey::CmdOpt);
    }

    #[test]
    fn failed_rebinding_rolls_back_the_meta_key_and_restores_the_old_binding() {
        let (app, rx) = mock_app(app_data(vec![main_ctx("m", true, vec![])]));
        hotkey_mock::push_result(Err("combination already claimed".to_string()));

        let new = settings_from(app.handle(), |s| s.meta_key = MetaKey::CmdOpt);
        let result = update_settings(app.handle().clone(), new);

        assert!(result.is_err());
        // First call failed to apply the new modifier; the second restores the old one.
        assert_eq!(hotkey_mock::calls(), vec![hotkey_mock::Call::Reregister, hotkey_mock::Call::Reregister]);
        let state = app.state::<AppState>();
        assert_eq!(state.data.lock().unwrap().settings.meta_key, MetaKey::CtrlAlt, "the meta key is rolled back");
        assert!(rx.has_changed().unwrap(), "the rollback is saved");
    }
}
