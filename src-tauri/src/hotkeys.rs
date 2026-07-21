#[cfg(not(test))]
use tauri::Manager;
use tauri::{AppHandle, Runtime};
#[cfg(not(test))]
use tauri_plugin_global_shortcut::GlobalShortcutExt;

#[cfg(not(test))]
use crate::state::AppState;
use crate::state::MetaKey;

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
/// In test builds this consults the scripted [`mock`] instead of the
/// global-shortcut plugin — OS-level hotkey registration needs a real event
/// loop and is exercised only via the manual checklist.
///
/// # Preconditions
/// - `AppState` must already be registered via `app.manage(...)`.
///
/// # Errors
/// Returns `Err` with the offending accelerator if any registration fails
/// (e.g. the combination is already claimed by another application).
pub fn register_all<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = app;
        mock::record_register()
    }
    #[cfg(not(test))]
    {
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
}

/// Replaces every registered global shortcut with a fresh registration derived
/// from the meta key in the current settings.
///
/// Called when the meta key setting changes so the new modifier takes effect
/// immediately; calling `register_all` alone would layer the new accelerators
/// on top of the old ones, leaving both modifiers active.
///
/// In test builds this consults the scripted [`mock`] instead of the
/// global-shortcut plugin (see [`register_all`]).
///
/// # Errors
/// Returns `Err` if unregistering or any registration fails (e.g. the new
/// combination is already claimed by another application). On failure the
/// shortcut set may be left partially registered; the caller should restore a
/// known-good meta key in settings and call this again.
pub fn reregister_all<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = app;
        mock::record_reregister()
    }
    #[cfg(not(test))]
    {
        app.global_shortcut().unregister_all().map_err(|e| format!("unregister_all: {e}"))?;
        register_all(app)
    }
}

/// Scripted stand-in for OS hotkey registration in test builds.
///
/// Tests queue results with [`push_result`]; each `register_all` /
/// `reregister_all` call consumes the next queued result, defaulting to
/// `Ok(())` when the queue is empty, and is appended to the call log. State is
/// thread-local, so parallel tests (one thread per test) are isolated; test
/// helpers call [`reset`] anyway in case a runner reuses threads.
#[cfg(test)]
pub mod mock {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// One recorded (re)registration call.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Call {
        Register,
        Reregister,
    }

    #[derive(Default)]
    struct MockState {
        results: VecDeque<Result<(), String>>,
        calls: Vec<Call>,
    }

    thread_local! {
        static STATE: RefCell<MockState> = RefCell::new(MockState::default());
    }

    /// Clears the scripted results and the call log.
    pub fn reset() {
        STATE.with(|s| *s.borrow_mut() = MockState::default());
    }

    /// Queues the result for the next (re)registration call.
    pub fn push_result(result: Result<(), String>) {
        STATE.with(|s| s.borrow_mut().results.push_back(result));
    }

    /// Returns every (re)registration call recorded since the last [`reset`].
    pub fn calls() -> Vec<Call> {
        STATE.with(|s| s.borrow().calls.clone())
    }

    fn record(call: Call) -> Result<(), String> {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            state.calls.push(call);
            state.results.pop_front().unwrap_or(Ok(()))
        })
    }

    pub(super) fn record_register() -> Result<(), String> {
        record(Call::Register)
    }

    pub(super) fn record_reregister() -> Result<(), String> {
        record(Call::Reregister)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_prefix_maps_both_variants() {
        assert_eq!(meta_prefix(&MetaKey::CtrlAlt), "Ctrl+Alt");
        assert_eq!(meta_prefix(&MetaKey::CmdOpt), "CommandOrControl+Alt");
    }

    // Self-test pinning the scripted contract the settings suite builds on:
    // queued results are consumed in call order, defaulting to Ok.
    #[test]
    fn mock_consumes_queued_results_in_order_and_defaults_to_ok() {
        mock::reset();
        mock::push_result(Err("claimed".to_string()));
        assert_eq!(mock::record_reregister(), Err("claimed".to_string()));
        assert_eq!(mock::record_reregister(), Ok(()));
        assert_eq!(mock::record_register(), Ok(()));
        assert_eq!(mock::calls(), vec![mock::Call::Reregister, mock::Call::Reregister, mock::Call::Register]);
    }
}
