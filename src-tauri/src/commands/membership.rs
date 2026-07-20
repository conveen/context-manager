//! Window membership commands: adding a window to and removing it from a
//! Context, with the window's physical visibility reconciled against the
//! post-operation membership.

use tauri::Manager;

use super::{ctx_idx, for_each_window_copy, propagate_window_state, reconcile_window_visibility};
use crate::state::AppState;

/// Adds a window to a Context by copying its `WindowRef` metadata from
/// whichever existing Context currently tracks it.
///
/// Idempotent: returns `Ok` if the window is already a member.
///
/// After the membership change the window's physical visibility is reconciled
/// with the rule "a window is visible iff at least one of its Contexts is
/// visible", evaluated against the *post-operation* membership: if it now
/// belongs to a visible Context but is currently hidden, it is shown; if a
/// **move** leaves it only in hidden Contexts, it is hidden. The `hidden`
/// marker (and `hidden_z` on macOS) is then propagated to every copy.
///
/// When adding to a non-Main Context, the window is **moved** out of Main by
/// default (removed from Main). Pass `copy = true` to keep it in Main as well,
/// so it stays available to add to further Contexts.
///
/// When the reconciliation will hide the window, every stored copy's `hidden`
/// marker is optimistically set before the lock is released — same rationale
/// as `do_hide_context_windows`: the background poll's hidden-window
/// exemption must cover the window while the OS hide call is in flight, or a
/// poll firing in that gap would drop it from tracking. The write-back below
/// confirms the marker, or reverts it if the hide failed.
///
/// # Errors
/// Returns `Err` if the Context or the window (by `platform_id`) is not found.
#[tauri::command]
pub fn add_window_to_context<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    context_id: String,
    platform_id: u64,
    copy: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();

    // Collect what we need, then release the lock before any OS call. Compute
    // whether the window should be shown or hidden after the operation, using
    // the post-operation membership (in the target Context; out of Main if this
    // is a move) against the "visible iff any Context is visible" rule.
    let (mut win_clone, should_show, should_hide) = {
        let mut data = state.data.lock().unwrap();
        let ci = ctx_idx(&data, &context_id)?;
        if data.contexts[ci].windows.iter().any(|w| w.platform_id == platform_id) {
            return Ok(()); // already a member
        }
        let win = data
            .contexts
            .iter()
            .flat_map(|c| c.windows.iter())
            .find(|w| w.platform_id == platform_id)
            .ok_or_else(|| format!("window {platform_id} is not tracked in any context"))?
            .clone();

        // A move drops the window from Main; a copy leaves it there.
        let moving_out_of_main = !copy && !data.contexts[ci].is_main;
        let will_be_visible = data.contexts[ci].visible
            || data.contexts.iter().enumerate().any(|(i, c)| {
                i != ci
                    && !(moving_out_of_main && c.is_main)
                    && c.visible
                    && c.windows.iter().any(|w| w.platform_id == platform_id)
            });
        let was_hidden = win.hidden;
        let should_hide = !was_hidden && !will_be_visible;

        // Optimistically mark the window hidden before releasing the lock (see
        // the function doc comment); the write-back below finalizes or reverts.
        if should_hide {
            for_each_window_copy(&mut data, platform_id, |w| w.hidden = true);
        }
        (win, was_hidden && will_be_visible, should_hide)
    };

    reconcile_window_visibility(&mut win_clone, should_show, should_hide, "add_window_to_context");

    // Re-acquire to persist the membership and propagate any hidden-state
    // change. On the hide path this confirms the optimistic marker, or —
    // because a failed hide leaves the clone's `hidden` untouched (`false`) —
    // reverts it.
    let mut data = state.data.lock().unwrap();
    let ci = ctx_idx(&data, &context_id)?;
    // Propagate hidden state to all existing copies.
    propagate_window_state(&mut data, &win_clone);
    data.contexts[ci].windows.push(win_clone);

    // Remove window from Main context if moving (not copying) to a non-Main
    // context. With `copy`, the window stays in Main so it remains available to
    // add to further Contexts.
    if !copy && !data.contexts[ci].is_main {
        if let Some(main_idx) = data.contexts.iter().position(|c| c.is_main) {
            data.contexts[main_idx].windows.retain(|w| w.platform_id != platform_id);
        }
    }

    let _ = state.save_tx.send(data.clone());
    Ok(())
}

/// Removes a window from a Context.
///
/// If this was the window's last non-Main Context, the window is returned to
/// Main (the catch-all "Available Windows" pool) so it never ends up belonging
/// to no Context. This matters on macOS in particular: a hidden window is
/// minimized and thus absent from the on-screen enumeration, so an orphaned
/// hidden window would never be re-added to Main by the poll and would vanish
/// from the UI until manually un-minimized.
///
/// Physical visibility is then reconciled with the rule "a window is visible
/// iff at least one of its Contexts is visible", using the post-removal
/// membership (including a re-add to Main):
/// - If it is now in a visible Context but currently hidden, it is shown.
/// - If it is now only in hidden Contexts, it is hidden.
///
/// When the reconciliation will hide the window, every stored copy's `hidden`
/// marker is optimistically set before the lock is released — same rationale
/// as `do_hide_context_windows`: the background poll's hidden-window
/// exemption must cover the window while the OS hide call is in flight, or a
/// poll firing in that gap would drop it from tracking. The write-back below
/// confirms the marker, or reverts it if the hide failed.
///
/// Idempotent: returns `Ok` if the window is not a member.
///
/// # Errors
/// Returns `Err` if the Context does not exist.
#[tauri::command]
pub fn remove_window_from_context<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    context_id: String,
    platform_id: u64,
) -> Result<(), String> {
    let state = app.state::<AppState>();

    // Collect info before OS call: window's current state and its contexts after removal.
    let (mut win_clone, should_show, should_hide, readd_to_main) = {
        let mut data = state.data.lock().unwrap();
        let ci = ctx_idx(&data, &context_id)?;

        // Find the window across all contexts.
        let win = data
            .contexts
            .iter()
            .flat_map(|c| c.windows.iter())
            .find(|w| w.platform_id == platform_id)
            .ok_or_else(|| format!("window {platform_id} not found in any context"))?
            .clone();

        let was_hidden = win.hidden;
        let ci_is_main = data.contexts[ci].is_main;

        // Simulate the removal to check what contexts it would be in.
        let mut remaining_visible = false;
        let mut remaining_hidden = false;
        for (i, ctx) in data.contexts.iter().enumerate() {
            if i == ci {
                // This context is being removed from
                continue;
            }
            if ctx.windows.iter().any(|w| w.platform_id == platform_id) {
                if ctx.visible {
                    remaining_visible = true;
                } else {
                    remaining_hidden = true;
                }
            }
        }
        let no_remaining = !remaining_visible && !remaining_hidden;

        // Removing a window from its last non-Main Context returns it to Main
        // (see the doc comment). If it is removed from Main itself and lands in
        // no Context, leave its physical state untouched — the poll re-adds it.
        let readd_to_main = no_remaining && !ci_is_main;
        let belongs_nowhere = no_remaining && ci_is_main;

        // Visibility of the post-removal membership (which, when re-added, is Main).
        let main_visible = data.contexts.iter().find(|c| c.is_main).map(|c| c.visible).unwrap_or(true);
        let will_be_visible = if readd_to_main { main_visible } else { remaining_visible };

        let should_show = was_hidden && will_be_visible;
        let should_hide = !was_hidden && !will_be_visible && !belongs_nowhere;

        // Optimistically mark the window hidden before releasing the lock (see
        // the function doc comment); the write-back below finalizes or reverts.
        if should_hide {
            for_each_window_copy(&mut data, platform_id, |w| w.hidden = true);
        }

        (win, should_show, should_hide, readd_to_main)
    };

    reconcile_window_visibility(&mut win_clone, should_show, should_hide, "remove_window_from_context");

    // Re-acquire lock and apply the removal.
    let mut data = state.data.lock().unwrap();
    let ci = ctx_idx(&data, &context_id)?;
    data.contexts[ci].windows.retain(|w| w.platform_id != platform_id);

    // Propagate hidden state to all remaining copies. On the hide path this
    // confirms the optimistic marker, or — because a failed hide leaves the
    // clone's `hidden` untouched (`false`) — reverts it.
    propagate_window_state(&mut data, &win_clone);

    // Return the window to Main if the removal left it in no Context.
    if readd_to_main {
        if let Some(main_idx) = data.contexts.iter().position(|c| c.is_main) {
            if !data.contexts[main_idx].windows.iter().any(|w| w.platform_id == platform_id) {
                data.contexts[main_idx].windows.push(win_clone);
            }
        }
    }

    let _ = state.save_tx.send(data.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use tauri::Manager;

    use super::*;
    use crate::state::AppData;
    use crate::test_util::{app_data, ctx, main_ctx, mock_app, win};
    use crate::wm::mock::{self, Call};

    fn windows_of<'a>(data: &'a AppData, ctx_id: &str) -> Vec<u64> {
        data.contexts.iter().find(|c| c.id == ctx_id).unwrap().windows.iter().map(|w| w.platform_id).collect()
    }

    #[test]
    fn adding_moves_the_window_out_of_main_by_default() {
        let (app, rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", true, vec![])]));
        add_window_to_context(app.handle().clone(), "a".into(), 1, false).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert_eq!(windows_of(&data, "a"), vec![1]);
        assert!(windows_of(&data, "m").is_empty(), "a move drops the window from Main");
        assert!(mock::calls().is_empty(), "both Contexts visible: no OS calls");
        drop(data);
        assert!(rx.has_changed().unwrap());
    }

    #[test]
    fn adding_with_copy_keeps_the_window_in_main() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", true, vec![])]));
        add_window_to_context(app.handle().clone(), "a".into(), 1, true).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert_eq!(windows_of(&data, "a"), vec![1]);
        assert_eq!(windows_of(&data, "m"), vec![1], "a copy stays in Main");
    }

    #[test]
    fn adding_is_idempotent_and_validates_inputs() {
        let (app, rx) =
            mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", true, vec![win(1, false)])]));
        // Already a member: Ok without touching anything.
        add_window_to_context(app.handle().clone(), "a".into(), 1, false).unwrap();
        assert!(!rx.has_changed().unwrap());
        // Unknown Context / untracked window.
        assert!(add_window_to_context(app.handle().clone(), "ghost".into(), 1, false).is_err());
        assert!(add_window_to_context(app.handle().clone(), "a".into(), 99, false).is_err());
    }

    #[test]
    fn adding_a_hidden_window_to_a_visible_context_shows_it() {
        // Window 2 is hidden inside hidden Context a; adding it to visible
        // main must show it and clear the marker on every copy.
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", false, vec![win(2, true)])]));
        add_window_to_context(app.handle().clone(), "m".into(), 2, false).unwrap();

        assert_eq!(mock::calls(), vec![Call::Show(2)]);
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(data.contexts.iter().flat_map(|c| c.windows.iter()).all(|w| !w.hidden));
    }

    #[test]
    fn moving_a_window_into_a_hidden_context_hides_it() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", false, vec![])]));
        add_window_to_context(app.handle().clone(), "a".into(), 1, false).unwrap();

        assert_eq!(mock::calls(), vec![Call::Hide(1)]);
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(data.contexts.iter().find(|c| c.id == "a").unwrap().windows[0].hidden);
        assert!(windows_of(&data, "m").is_empty());
    }

    #[test]
    fn copying_a_window_into_a_hidden_context_leaves_it_visible() {
        // With copy, the window remains in visible Main, so it must not be hidden.
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", false, vec![])]));
        add_window_to_context(app.handle().clone(), "a".into(), 1, true).unwrap();

        assert!(mock::calls().is_empty());
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(data.contexts.iter().flat_map(|c| c.windows.iter()).all(|w| !w.hidden));
    }

    #[test]
    fn failed_hide_on_move_reverts_the_optimistic_marker() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", false, vec![])]));
        mock::fail_hide(1);
        add_window_to_context(app.handle().clone(), "a".into(), 1, false).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        // Membership still applied, but the marker reflects the failed hide.
        assert_eq!(windows_of(&data, "a"), vec![1]);
        assert!(!data.contexts.iter().find(|c| c.id == "a").unwrap().windows[0].hidden);
    }

    #[test]
    fn removing_from_the_last_non_main_context_returns_the_window_to_main() {
        // Window 2 is hidden in hidden Context a and belongs nowhere else;
        // removal must re-add it to (visible) Main and show it.
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", false, vec![win(2, true)])]));
        remove_window_from_context(app.handle().clone(), "a".into(), 2).unwrap();

        assert_eq!(mock::calls(), vec![Call::Show(2)]);
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(windows_of(&data, "a").is_empty());
        assert_eq!(windows_of(&data, "m"), vec![2]);
        assert!(!data.contexts.iter().find(|c| c.id == "m").unwrap().windows[0].hidden);
    }

    #[test]
    fn removing_a_window_that_remains_visible_elsewhere_touches_nothing() {
        let (app, _rx) =
            mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", true, vec![win(1, false)])]));
        remove_window_from_context(app.handle().clone(), "a".into(), 1).unwrap();

        assert!(mock::calls().is_empty());
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(windows_of(&data, "a").is_empty());
        assert_eq!(windows_of(&data, "m"), vec![1]);
    }

    #[test]
    fn removing_a_windows_last_visible_context_hides_it() {
        // Window 1 is visible in a (visible) and also tracked in b (hidden).
        // Removing it from a leaves only hidden Contexts → it must be hidden.
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", false, vec![]),
            ctx("a", true, vec![win(1, false)]),
            ctx("b", false, vec![win(1, false)]),
        ]));
        remove_window_from_context(app.handle().clone(), "a".into(), 1).unwrap();

        assert_eq!(mock::calls(), vec![Call::Hide(1)]);
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(data.contexts.iter().find(|c| c.id == "b").unwrap().windows[0].hidden);
    }

    #[test]
    fn removing_from_main_with_no_other_context_leaves_physical_state_alone() {
        // The poll will re-add the (still live) window; nothing is hidden/shown.
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)])]));
        remove_window_from_context(app.handle().clone(), "m".into(), 1).unwrap();

        assert!(mock::calls().is_empty());
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(data.contexts.iter().all(|c| c.windows.is_empty()));
    }

    #[test]
    fn removing_validates_the_context_and_tolerates_non_members() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(1, false)]), ctx("a", true, vec![])]));
        assert!(remove_window_from_context(app.handle().clone(), "ghost".into(), 1).is_err());
        assert!(remove_window_from_context(app.handle().clone(), "a".into(), 99).is_err(), "untracked window");
    }
    #[test]
    fn poll_firing_mid_hide_during_a_removal_does_not_drop_the_window() {
        // The #43 poll-interleaving race: removing a window's last visible
        // Context hides it; the poll ticking between the OS hide and the
        // write-back (reproduced by the mock's on-hide hook) must not drop it
        // from the hidden Context that still tracks it — without the
        // optimistic marker, the poll saw a not-hidden, non-enumerable window
        // and removed it, so the write-back had nothing to return it to.
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", false, vec![]),
            ctx("a", true, vec![win(1, false)]),
            ctx("b", false, vec![win(1, false)]),
        ]));
        mock::set_windows(vec![mock::mock_win(1, "App1", "Win1")]);
        let handle = app.handle().clone();
        mock::set_on_hide(move |_| {
            mock::set_windows(vec![]);
            crate::wm::update_windows(&handle);
        });
        remove_window_from_context(app.handle().clone(), "a".into(), 1).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert!(windows_of(&data, "a").is_empty());
        assert_eq!(windows_of(&data, "b"), vec![1], "the mid-hide poll must not drop the window (#43)");
        assert!(data.contexts.iter().find(|c| c.id == "b").unwrap().windows[0].hidden);
    }
}
