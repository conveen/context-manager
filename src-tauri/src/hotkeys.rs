use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::state::{AppState, MetaKey};

/// Event emitted with the list of accelerators the OS refused, whenever a
/// (re)registration leaves any shortcut dead. The frontend shows them in the
/// error toast.
const SHORTCUTS_FAILED_EVENT: &str = "shortcuts-failed";

/// What a bulk (re)registration achieved.
///
/// Registration is per-accelerator and failures are non-fatal, so the result is
/// a count rather than a `Result`: an accelerator another process already owns
/// costs only itself.
pub struct Registration {
    /// Accelerators the OS refused, in registration order. Empty on full success.
    pub failed: Vec<String>,
    /// How many accelerators were attempted.
    pub attempted: usize,
}

impl Registration {
    /// Whether every accelerator was refused — the one outcome that leaves the
    /// user with no shortcuts at all, and so the only one worth undoing a
    /// modifier change for.
    pub fn all_failed(&self) -> bool {
        self.attempted > 0 && self.failed.len() == self.attempted
    }
}

/// Returns the Tauri accelerator prefix (the modifier portion) for the
/// configured meta key.
///
/// `CmdOpt` maps to `CommandOrControl+Alt` so it resolves to Cmd+Option on
/// macOS and Ctrl+Alt elsewhere, matching the documented behaviour in
/// [`MetaKey`]. `Super` likewise resolves to the Windows key on Windows and
/// Command on macOS.
fn meta_prefix(meta: &MetaKey) -> &'static str {
    match meta {
        MetaKey::CtrlAlt => "Ctrl+Alt",
        MetaKey::CmdOpt => "CommandOrControl+Alt",
        MetaKey::CtrlAltSuper => "Ctrl+Alt+Super",
    }
}

/// The full accelerator set: `<prefix>+0` .. `<prefix>+9`, then `<prefix>+H`.
fn accelerators(prefix: &str) -> Vec<String> {
    (0..=9).map(|n| format!("{prefix}+{n}")).chain(std::iter::once(format!("{prefix}+H"))).collect()
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
/// Every accelerator is attempted independently and failures are collected
/// rather than propagated: on Windows `RegisterHotKey` refuses any combination
/// another process already owns, and bailing on the first refusal used to
/// sacrifice every shortcut after it — including all of them when the very
/// first (`<meta>+0`) was the one taken.
///
/// The refused accelerators are recorded in [`AppState::failed_shortcuts`] and,
/// when there are any, emitted as `shortcuts-failed`. Both paths exist because
/// neither covers startup and settings changes alone: the event is lost when it
/// fires before the webview is listening, which is exactly the case at startup,
/// and the stored list is what the frontend reads to catch up.
///
/// # Preconditions
/// - `AppState` must already be registered via `app.manage(...)`.
pub fn register_all(app: &AppHandle) -> Registration {
    let prefix = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        meta_prefix(&data.settings.meta_key)
    };

    let gs = app.global_shortcut();
    let accels = accelerators(prefix);
    let mut failed = Vec::new();
    for accel in &accels {
        if let Err(e) = gs.register(accel.as_str()) {
            // Kept for debug builds; release builds have no console, which is
            // why the failures are also reported through the UI.
            eprintln!("register '{accel}': {e}");
            failed.push(accel.clone());
        }
    }

    *app.state::<AppState>().failed_shortcuts.lock().unwrap() = failed.clone();
    if !failed.is_empty() {
        let _ = app.emit(SHORTCUTS_FAILED_EVENT, failed.clone());
    }

    Registration { failed, attempted: accels.len() }
}

/// Replaces every registered global shortcut with a fresh registration derived
/// from the meta key in the current settings.
///
/// Called when the meta key setting changes so the new modifier takes effect
/// immediately; calling `register_all` alone would layer the new accelerators
/// on top of the old ones, leaving both modifiers active.
///
/// # Errors
/// Returns `Err` only if *unregistering* fails, which leaves the old modifier
/// registered and nothing new attempted. Individual registration failures are
/// reported in the returned [`Registration`]; the caller decides whether the
/// outcome is bad enough to restore the previous meta key.
pub fn reregister_all(app: &AppHandle) -> Result<Registration, String> {
    app.global_shortcut().unregister_all().map_err(|e| format!("unregister_all: {e}"))?;
    Ok(register_all(app))
}
