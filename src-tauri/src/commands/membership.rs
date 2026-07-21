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
