use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::state::{AppState, MetaKey};

/// Returns the Tauri accelerator prefix (the modifier portion) for the
/// configured meta key.
///
/// `CmdOpt` maps to `CommandOrControl+Alt` so it resolves to Cmd+Option on
/// macOS and Ctrl+Alt elsewhere, matching the documented behaviour in
/// [`MetaKey`].
fn meta_prefix(meta: &MetaKey) -> &'static str {
    match meta {
        MetaKey::CtrlAlt => "Ctrl+Alt",
        MetaKey::CmdOpt => "CommandOrControl+Alt",
    }
}

/// Registers all global Context shortcuts with the plugin using the meta key
/// from the current settings:
/// - `<meta>+0` .. `<meta>+9` — toggle the Context assigned to that
///   `shortcut_index` (dispatched by [`crate::commands::handle_shortcut`]).
/// - `<meta>+H` — hide all Contexts.
///
/// Registration is what causes the plugin's global handler (installed in
/// `lib.rs`) to actually fire for these key combinations; without it the OS
/// never forwards the key events to the app.
///
/// # Preconditions
/// - `AppState` must already be registered via `app.manage(...)`.
///
/// # Errors
/// Returns `Err` with the offending accelerator if any registration fails
/// (e.g. the combination is already claimed by another application).
pub fn register_all(app: &AppHandle) -> Result<(), String> {
    let prefix = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        meta_prefix(&data.settings.meta_key)
    };

    let gs = app.global_shortcut();
    for n in 0..=9 {
        let accel = format!("{prefix}+{n}");
        gs.register(accel.as_str()).map_err(|e| format!("register '{accel}': {e}"))?;
    }
    let accel = format!("{prefix}+H");
    gs.register(accel.as_str()).map_err(|e| format!("register '{accel}': {e}"))?;

    Ok(())
}

/// Replaces every registered global shortcut with a fresh registration derived
/// from the meta key in the current settings.
///
/// Called when the meta key setting changes so the new modifier takes effect
/// immediately; calling `register_all` alone would layer the new accelerators
/// on top of the old ones, leaving both modifiers active.
///
/// # Errors
/// Returns `Err` if unregistering or any registration fails (e.g. the new
/// combination is already claimed by another application). On failure the
/// shortcut set may be left partially registered; the caller should restore a
/// known-good meta key in settings and call this again.
pub fn reregister_all(app: &AppHandle) -> Result<(), String> {
    app.global_shortcut().unregister_all().map_err(|e| format!("unregister_all: {e}"))?;
    register_all(app)
}
