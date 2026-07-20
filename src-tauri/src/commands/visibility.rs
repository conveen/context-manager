//! Context visibility commands and global-hotkey dispatch.
//!
//! The `#[tauri::command]` handlers are thin wrappers over `show`/`hide`,
//! which take a borrowed `AppHandle` so the Rust-side callers (hotkey
//! dispatch here, Single Context Mode enforcement in `update_settings`)
//! don't have to clone the handle.

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::Code;

use super::{ctx_idx, do_hide_context_windows, do_show_context_windows};
use crate::state::AppState;

/// Backing logic for [`show_context`]: shows the Context, and in Single
/// Context Mode hides all other currently-visible Contexts first (applying
/// the multi-Context visibility rule to each).
///
/// # Errors
/// Returns `Err` if the Context does not exist.
pub(super) fn show<R: tauri::Runtime>(app: &tauri::AppHandle<R>, id: &str) -> Result<(), String> {
    // Validate existence and collect sibling IDs under a brief lock.
    let siblings_to_hide: Vec<String> = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        ctx_idx(&data, id)?; // validate
        if data.settings.single_context_mode {
            data.contexts.iter().filter(|c| c.id != id && c.visible).map(|c| c.id.clone()).collect()
        } else {
            vec![]
        }
    }; // lock released

    for sibling_id in siblings_to_hide {
        do_hide_context_windows(app, &sibling_id);
    }
    do_show_context_windows(app, id);
    Ok(())
}

/// Backing logic for [`hide_context`]: validates the Context exists, then
/// hides it.
///
/// # Errors
/// Returns `Err` if the Context does not exist.
fn hide<R: tauri::Runtime>(app: &tauri::AppHandle<R>, id: &str) -> Result<(), String> {
    // Validate existence before delegating.
    {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        ctx_idx(&data, id)?;
    }
    do_hide_context_windows(app, id);
    Ok(())
}

/// Shows a Context, making its hidden windows visible.
///
/// In Single Context Mode, all other currently-visible Contexts are hidden
/// first (applying the multi-Context visibility rule to each).
///
/// # Errors
/// Returns `Err` if the Context does not exist.
#[tauri::command]
pub fn show_context<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String) -> Result<(), String> {
    show(&app, &id)
}

/// Hides a Context. Windows that have no other visible Context are minimized
/// (macOS) or hidden via `SW_HIDE` (Windows).
///
/// # Errors
/// Returns `Err` if the Context does not exist.
#[tauri::command]
pub fn hide_context<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String) -> Result<(), String> {
    hide(&app, &id)
}

/// Hides all currently-visible Contexts. Reached only from the `<meta>+H`
/// global shortcut via `handle_shortcut`; the frontend has no hide-all
/// affordance, so this is deliberately not a Tauri command.
///
/// Visible Context IDs are collected under a single lock acquisition; then
/// each is hidden in turn (each call to `do_hide_context_windows` re-acquires
/// the lock internally, so windows hidden by an earlier call are not re-hidden
/// by later calls — the not-`hidden` filter ensures this).
fn hide_all<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let visible_ids: Vec<String> = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        data.contexts.iter().filter(|c| c.visible).map(|c| c.id.clone()).collect()
    };
    for id in visible_ids {
        do_hide_context_windows(app, &id);
    }
}

/// Toggles the Context assigned to `shortcut_index`: hides it if visible,
/// shows it if hidden. No-op if no Context has that index.
///
/// The data lock is acquired briefly to read state, then released before
/// calling `show`/`hide` (which re-acquire it), preventing deadlocks.
fn toggle_context_by_shortcut<R: tauri::Runtime>(app: &tauri::AppHandle<R>, shortcut_index: u8) {
    let (ctx_id, is_visible) = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        match data.contexts.iter().find(|c| c.shortcut_index == Some(shortcut_index)) {
            Some(c) => (c.id.clone(), c.visible),
            None => return,
        }
    };
    if is_visible {
        let _ = hide(app, &ctx_id);
    } else {
        let _ = show(app, &ctx_id);
    }
}

/// Maps `Code::Digit0`–`Digit9` to its numeric value; `None` for any other key.
fn digit_of(code: Code) -> Option<u8> {
    const DIGITS: [Code; 10] = [
        Code::Digit0,
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    DIGITS.iter().position(|c| *c == code).map(|i| i as u8)
}

/// Dispatches a pressed global shortcut to the appropriate Context action.
///
/// - `<meta>+0`–`9` — toggle the Context assigned to that `shortcut_index`.
/// - `<meta>+H` — hide all Contexts.
///
/// Called from the `with_handler` closure in `lib.rs`.
pub fn handle_shortcut<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
) {
    if shortcut.key == Code::KeyH {
        hide_all(app);
    } else if let Some(n) = digit_of(shortcut.key) {
        toggle_context_by_shortcut(app, n);
    } else {
        return;
    }
    // Notify the frontend so visibility indicators update immediately rather
    // than waiting for the next periodic poll.
    let _ = app.emit(crate::events::CONTEXTS_CHANGED, ());
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tauri::Listener;
    use tauri_plugin_global_shortcut::Shortcut;

    use super::*;
    use crate::state::{AppData, Context};
    use crate::test_util::{app_data, ctx, main_ctx, mock_app, win};
    use crate::wm::mock::{self, Call};

    fn ctx_by<'a>(data: &'a AppData, id: &str) -> &'a Context {
        data.contexts.iter().find(|c| c.id == id).unwrap()
    }

    fn win_in<'a>(data: &'a AppData, ctx_id: &str, platform_id: u64) -> &'a crate::state::WindowRef {
        ctx_by(data, ctx_id).windows.iter().find(|w| w.platform_id == platform_id).unwrap()
    }

    #[test]
    fn hiding_hides_only_windows_with_no_other_visible_context() {
        // main (visible) shares window 1 with a; a exclusively holds window 2.
        let (app, rx) = mock_app(app_data(vec![
            main_ctx("m", true, vec![win(1, false)]),
            ctx("a", true, vec![win(1, false), win(2, false)]),
        ]));
        hide_context(app.handle().clone(), "a".into()).unwrap();

        assert_eq!(mock::calls(), vec![Call::Hide(2)], "the shared window must not be hidden");
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(!ctx_by(&data, "a").visible);
        assert!(win_in(&data, "a", 2).hidden);
        // The shared window stays visible in every copy.
        assert!(!win_in(&data, "a", 1).hidden);
        assert!(!win_in(&data, "m", 1).hidden);
        drop(data);
        assert!(rx.has_changed().unwrap());
    }

    #[test]
    fn hidden_marker_is_propagated_to_every_copy_of_the_window() {
        // Window 2 lives in both a and b; b is already hidden, so hiding a
        // hides window 2 — and b's copy must pick up the marker too.
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", true, vec![]),
            ctx("a", true, vec![win(2, false)]),
            ctx("b", false, vec![win(2, false)]),
        ]));
        hide_context(app.handle().clone(), "a".into()).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(win_in(&data, "a", 2).hidden);
        assert!(win_in(&data, "b", 2).hidden);
    }

    #[test]
    fn failed_hide_reverts_the_optimistic_marker() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![win(2, false)])]));
        mock::fail_hide(2);
        hide_context(app.handle().clone(), "a".into()).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(!win_in(&data, "a", 2).hidden, "marker must be reverted after a failed hide");
        assert!(!ctx_by(&data, "a").visible, "the Context is still marked hidden");
    }

    #[test]
    fn showing_shows_hidden_windows_and_clears_markers_on_all_copies() {
        let (app, _rx) =
            mock_app(app_data(vec![main_ctx("m", false, vec![win(1, true)]), ctx("a", false, vec![win(1, true)])]));
        show_context(app.handle().clone(), "a".into()).unwrap();

        // On macOS the frontmost shown window is additionally raised, even
        // when no stacking rank was captured for it.
        #[cfg(target_os = "macos")]
        assert_eq!(mock::calls(), vec![Call::Show(1), Call::Raise(1)]);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(mock::calls(), vec![Call::Show(1)]);
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(ctx_by(&data, "a").visible);
        assert!(!win_in(&data, "a", 1).hidden);
        assert!(!win_in(&data, "m", 1).hidden, "marker cleared on every copy");
        assert!(!ctx_by(&data, "m").visible, "other Contexts' visibility untouched");
    }

    #[test]
    fn unknown_context_errors() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![])]));
        assert!(show_context(app.handle().clone(), "nope".into()).is_err());
        assert!(hide_context(app.handle().clone(), "nope".into()).is_err());
    }

    #[test]
    fn single_context_mode_show_hides_all_visible_siblings() {
        let mut data = app_data(vec![
            main_ctx("m", true, vec![win(1, false)]),
            ctx("a", true, vec![win(2, false)]),
            ctx("b", false, vec![win(3, true)]),
        ]);
        data.settings.single_context_mode = true;
        let (app, _rx) = mock_app(data);
        show_context(app.handle().clone(), "b".into()).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(ctx_by(&data, "b").visible);
        assert!(!ctx_by(&data, "m").visible);
        assert!(!ctx_by(&data, "a").visible);
        assert!(win_in(&data, "m", 1).hidden);
        assert!(win_in(&data, "a", 2).hidden);
        assert!(!win_in(&data, "b", 3).hidden);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn show_unminimizes_back_to_front_and_raises_the_previously_frontmost_window() {
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", false, vec![]),
            ctx("a", true, vec![win(1, false), win(2, false), win(3, false)]),
        ]));
        // Enumeration order is front-to-back: window 2 is frontmost.
        mock::set_windows(vec![
            mock::mock_win(2, "App2", "Win2"),
            mock::mock_win(1, "App1", "Win1"),
            mock::mock_win(3, "App3", "Win3"),
        ]);
        hide_context(app.handle().clone(), "a".into()).unwrap();
        show_context(app.handle().clone(), "a".into()).unwrap();

        assert_eq!(
            mock::calls(),
            vec![
                Call::Hide(1),
                Call::Hide(2),
                Call::Hide(3),
                // Back-to-front: deepest window first, frontmost last…
                Call::Show(3),
                Call::Show(1),
                Call::Show(2),
                // …and the previously-frontmost window is explicitly raised.
                Call::Raise(2),
            ]
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_keeps_the_stored_window_order_without_raising() {
        // Off macOS the OS preserves z-order natively; windows are shown in
        // stored order and nothing is raised.
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", false, vec![]),
            ctx("a", true, vec![win(1, false), win(2, false), win(3, false)]),
        ]));
        hide_context(app.handle().clone(), "a".into()).unwrap();
        show_context(app.handle().clone(), "a".into()).unwrap();

        assert_eq!(
            mock::calls(),
            vec![Call::Hide(1), Call::Hide(2), Call::Hide(3), Call::Show(1), Call::Show(2), Call::Show(3)]
        );
    }

    #[test]
    fn shortcut_toggles_the_assigned_context_and_emits() {
        let mut data = app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![win(2, false)])]);
        data.contexts[1].shortcut_index = Some(1);
        let (app, _rx) = mock_app(data);
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        app.listen_any(crate::events::CONTEXTS_CHANGED, move |_| {
            h.fetch_add(1, Ordering::SeqCst);
        });

        let digit1 = Shortcut::new(None, Code::Digit1);
        handle_shortcut(app.handle(), &digit1); // visible → hide
        {
            let state = app.state::<AppState>();
            let data = state.data.lock().unwrap();
            assert!(!ctx_by(&data, "a").visible);
        }
        handle_shortcut(app.handle(), &digit1); // hidden → show
        {
            let state = app.state::<AppState>();
            let data = state.data.lock().unwrap();
            assert!(ctx_by(&data, "a").visible);
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2, "each dispatch notifies the frontend");
    }

    #[test]
    fn shortcut_h_hides_every_visible_context() {
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", true, vec![win(1, false)]),
            ctx("a", true, vec![win(2, false)]),
            ctx("b", false, vec![]),
        ]));
        handle_shortcut(app.handle(), &Shortcut::new(None, Code::KeyH));

        assert_eq!(mock::calls(), vec![Call::Hide(1), Call::Hide(2)]);
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(data.contexts.iter().all(|c| !c.visible));
    }

    #[test]
    fn unassigned_digit_and_unmapped_key_change_nothing() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)])]));
        handle_shortcut(app.handle(), &Shortcut::new(None, Code::Digit5));
        handle_shortcut(app.handle(), &Shortcut::new(None, Code::KeyA));

        assert!(mock::calls().is_empty());
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(data.contexts[0].visible);
    }
    // ── Poll-interleaving races (the #19/#43/#59 class) ───────────────────
    // The bug: the background poll fires while an OS hide is in flight — the
    // window is already gone from the live enumeration, but the hide's
    // write-back hasn't run. Without the optimistic hidden marker set under
    // the Phase-1 lock, the poll saw a not-hidden, non-enumerable window and
    // dropped it from tracking. The mock's on-hide hook reproduces that
    // interleaving deterministically.

    #[test]
    fn poll_firing_mid_hide_does_not_drop_the_window() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![win(2, false)])]));
        mock::set_windows(vec![mock::mock_win(2, "App2", "Win2")]);
        let handle = app.handle().clone();
        mock::set_on_hide(move |_| {
            // The just-minimized window disappears from the live enumeration…
            mock::set_windows(vec![]);
            // …and the poll ticks before the hide's write-back runs.
            crate::wm::update_windows(&handle);
        });
        hide_context(app.handle().clone(), "a".into()).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        let a = ctx_by(&data, "a");
        assert_eq!(a.windows.len(), 1, "the mid-hide poll must not drop the window (#19)");
        assert!(a.windows[0].hidden);
    }

    #[test]
    fn poll_firing_during_hide_all_keeps_every_window() {
        // #59: <meta>+H hides every Context in sequence; each hide is a
        // separate poll-race window. The hook removes each window from the
        // scripted enumeration as it is minimized, polling every time.
        let (app, _rx) =
            mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", true, vec![win(2, false)])]));
        mock::set_windows(vec![mock::mock_win(1, "App1", "Win1"), mock::mock_win(2, "App2", "Win2")]);
        let handle = app.handle().clone();
        let mut live: std::collections::HashSet<u64> = [1, 2].into();
        mock::set_on_hide(move |id| {
            live.remove(&id);
            mock::set_windows(
                live.iter().map(|&i| mock::mock_win(i, &format!("App{i}"), &format!("Win{i}"))).collect(),
            );
            crate::wm::update_windows(&handle);
        });
        handle_shortcut(app.handle(), &Shortcut::new(None, Code::KeyH));

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert_eq!(ctx_by(&data, "m").windows.len(), 1, "window 1 must survive the mid-hide polls (#59)");
        assert_eq!(ctx_by(&data, "a").windows.len(), 1, "window 2 must survive the mid-hide polls (#59)");
        assert!(win_in(&data, "m", 1).hidden);
        assert!(win_in(&data, "a", 2).hidden);
        assert!(data.contexts.iter().all(|c| !c.visible));
    }
}
