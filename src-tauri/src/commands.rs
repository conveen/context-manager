use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::Code;

use crate::state::{AppData, AppState, Context, WindowRef};
use crate::wm;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Returns the index of the context with the given `id` in `data.contexts`.
fn ctx_idx(data: &AppData, id: &str) -> Result<usize, String> {
    data.contexts.iter().position(|c| c.id == id).ok_or_else(|| format!("context '{id}' not found"))
}

/// Applies `f` to every stored copy of the window identified by `platform_id`
/// across all Contexts. A window can belong to several Contexts at once, each
/// holding its own `WindowRef` copy, and per-window state like `hidden` (and
/// `hidden_z` on macOS) must stay in sync across all of them.
fn for_each_window_copy(data: &mut AppData, platform_id: u64, mut f: impl FnMut(&mut WindowRef)) {
    for ctx in &mut data.contexts {
        for w in &mut ctx.windows {
            if w.platform_id == platform_id {
                f(w);
            }
        }
    }
}

/// Writes `src`'s hidden-state fields (`hidden`, plus `hidden_z` on macOS) to
/// every stored copy of the same window across all Contexts, keeping
/// per-window state consistent no matter which Context it is read from.
fn propagate_window_state(data: &mut AppData, src: &WindowRef) {
    let hidden = src.hidden;
    #[cfg(target_os = "macos")]
    let z = src.hidden_z;
    for_each_window_copy(data, src.platform_id, |w| {
        w.hidden = hidden;
        #[cfg(target_os = "macos")]
        {
            w.hidden_z = z;
        }
    });
}

/// Front-to-back stacking rank of `platform_id` in the current on-screen
/// enumeration (`0` = frontmost), or `None` if the window is not enumerable.
/// Captured just before hiding a window so a later show can restore the
/// z-order.
#[cfg(target_os = "macos")]
fn current_z_rank(platform_id: u64) -> Option<u32> {
    wm::enumerate(std::process::id()).iter().position(|w| w.platform_id == platform_id).map(|i| i as u32)
}

/// Reconciles a window's physical visibility outside the lock, shared by the
/// two membership commands: shows the window when `should_show`, hides it
/// (capturing the current stacking rank first, on macOS) when `should_hide`.
/// Mutates `win_clone`'s hidden-state fields for the caller to propagate; OS
/// errors are printed with `caller` for context and otherwise ignored.
fn reconcile_window_visibility(win_clone: &mut WindowRef, should_show: bool, should_hide: bool, caller: &str) {
    if should_show {
        if let Err(e) = wm::show_window(win_clone) {
            eprintln!("{caller} show_window({}): {e}", win_clone.platform_id);
        }
        #[cfg(target_os = "macos")]
        {
            win_clone.hidden_z = None;
        }
    } else if should_hide {
        // Capture the front-to-back stacking rank while still visible so a
        // later show can restore it (matches `do_hide_context_windows`).
        #[cfg(target_os = "macos")]
        {
            win_clone.hidden_z = current_z_rank(win_clone.platform_id);
        }
        if let Err(e) = wm::hide_window(win_clone) {
            eprintln!("{caller} hide_window({}): {e}", win_clone.platform_id);
        }
    }
}

/// Physically hides windows that would have no remaining visible Context after
/// Context `ctx_id` is hidden, then marks that Context `visible = false`.
///
/// Three-phase design to satisfy the borrow checker and keep the
/// cross-context `hidden` marker in sync:
///
/// **Phase 1 (immutable, under lock)** — Collect a clone of every window in
/// the target Context that is currently physically visible (not `hidden`) and
/// has no other visible Context.
///
/// **Phase 2 (OS calls, lock released)** — Call `wm::hide_window` on each
/// clone. Per-window errors are printed and skipped.
///
/// **Phase 3 (mutation, under lock)** — Mark the Context `visible = false`,
/// then write each clone's hidden state back to every copy of that window
/// across all Contexts. Saves state.
///
/// Between Phase 1 and Phase 3, every target's `hidden` marker is
/// optimistically set (rather than left clear until Phase 3) so the
/// background window poll's hidden-window exemption covers it for the whole
/// window in which the OS call is in flight — otherwise a poll firing after
/// the OS-level minimize but before the write-back would see the window
/// minimized (absent from the live enumeration) yet still marked not-hidden,
/// and drop it from tracking entirely. Phase 3 confirms the marker on
/// success, or reverts it on failure.
fn do_hide_context_windows(app: &tauri::AppHandle, ctx_id: &str) {
    let state = app.state::<AppState>();

    // Capture the current front-to-back stacking order (macOS) before minimizing,
    // so show can restore it. Done outside the lock — it is an OS enumeration call.
    #[cfg(target_os = "macos")]
    let z_map: std::collections::HashMap<u64, u32> =
        wm::enumerate(std::process::id()).iter().enumerate().map(|(i, w)| (w.platform_id, i as u32)).collect();

    // Phase 1 — collect under lock, then optimistically mark the targets
    // hidden before releasing the lock (see function doc comment).
    let hide_targets: Vec<WindowRef> = {
        let mut data = state.data.lock().unwrap();
        let ci = match ctx_idx(&data, ctx_id) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("do_hide_context_windows: {e}");
                return;
            },
        };
        let targets: Vec<WindowRef> = data.contexts[ci]
            .windows
            .iter()
            .filter(|w| {
                !w.hidden
                    && !data.contexts.iter().enumerate().any(|(i, c)| {
                        i != ci && c.visible && c.windows.iter().any(|cw| cw.platform_id == w.platform_id)
                    })
            })
            .cloned()
            .collect();

        for t in &targets {
            for_each_window_copy(&mut data, t.platform_id, |w| w.hidden = true);
        }

        targets
    };

    // Phase 2 — OS calls outside the lock
    let mut hidden: Vec<WindowRef> = Vec::with_capacity(hide_targets.len());
    let mut failed: Vec<u64> = Vec::new();
    for mut win_clone in hide_targets {
        // Attach the pre-captured stacking rank so the propagate below syncs
        // it to every copy along with the hidden marker.
        #[cfg(target_os = "macos")]
        {
            win_clone.hidden_z = z_map.get(&win_clone.platform_id).copied();
        }
        match wm::hide_window(&mut win_clone) {
            Ok(()) => hidden.push(win_clone),
            Err(e) => {
                eprintln!("hide_window({}): {e}", win_clone.platform_id);
                failed.push(win_clone.platform_id);
            },
        }
    }

    // Phase 3 — write-back under lock
    let mut data = state.data.lock().unwrap();
    match ctx_idx(&data, ctx_id) {
        Ok(ci) => data.contexts[ci].visible = false,
        Err(e) => {
            eprintln!("do_hide_context_windows write-back: {e}");
            return;
        },
    }
    for w in &hidden {
        propagate_window_state(&mut data, w);
    }
    // Hide failed: revert the optimistic marker so the window isn't left
    // permanently (and incorrectly) marked hidden.
    for platform_id in &failed {
        for_each_window_copy(&mut data, *platform_id, |w| w.hidden = false);
    }
    let _ = state.save_tx.send(data.clone());
}

/// Physically shows all windows in Context `ctx_id` that are currently hidden,
/// then marks that Context `visible = true`.
///
/// Mirrors the three-phase structure of `do_hide_context_windows`:
/// collect hidden windows under lock → call `wm::show_window` outside lock
/// → clear the hidden marker on all copies and mark visible under lock.
fn do_show_context_windows(app: &tauri::AppHandle, ctx_id: &str) {
    let state = app.state::<AppState>();

    // Phase 1
    // `mut` is used only on macOS, to reorder for z-order restoration.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut show_targets: Vec<WindowRef> = {
        let data = state.data.lock().unwrap();
        let ci = match ctx_idx(&data, ctx_id) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("do_show_context_windows: {e}");
                return;
            },
        };
        data.contexts[ci].windows.iter().filter(|w| w.hidden).cloned().collect()
    };

    // On macOS, restore stacking order: un-minimize back-to-front (highest rank
    // first) so the window that was frontmost when hidden is un-minimized last.
    // Windows with an unknown rank sort to the back (un-minimized first).
    #[cfg(target_os = "macos")]
    show_targets.sort_by(|a, b| b.hidden_z.unwrap_or(u32::MAX).cmp(&a.hidden_z.unwrap_or(u32::MAX)));

    // Remember the frontmost (lowest rank) window to explicitly raise once all
    // windows are un-minimized, reinstating it as the top window.
    #[cfg(target_os = "macos")]
    let frontmost: Option<WindowRef> = show_targets.iter().min_by_key(|w| w.hidden_z.unwrap_or(u32::MAX)).cloned();

    // Phase 2
    let mut shown: Vec<WindowRef> = Vec::with_capacity(show_targets.len());
    for mut win_clone in show_targets {
        match wm::show_window(&mut win_clone) {
            Ok(()) => {
                // show_window cleared the hidden marker on the clone; clear the
                // stacking rank too so the propagate below syncs the
                // fully-visible state to every copy.
                #[cfg(target_os = "macos")]
                {
                    win_clone.hidden_z = None;
                }
                shown.push(win_clone);
            },
            Err(e) => eprintln!("show_window({}): {e}", win_clone.platform_id),
        }
    }

    // Raise the previously-frontmost window last so it ends up on top.
    #[cfg(target_os = "macos")]
    if let Some(fm) = frontmost {
        if let Err(e) = wm::raise_window(&fm) {
            eprintln!("raise_window({}): {e}", fm.platform_id);
        }
    }

    // Phase 3
    let mut data = state.data.lock().unwrap();
    match ctx_idx(&data, ctx_id) {
        Ok(ci) => data.contexts[ci].visible = true,
        Err(e) => {
            eprintln!("do_show_context_windows write-back: {e}");
            return;
        },
    }
    for w in &shown {
        propagate_window_state(&mut data, w);
    }
    let _ = state.save_tx.send(data.clone());
}

// ---------------------------------------------------------------------------
// Tauri commands — Context CRUD
// ---------------------------------------------------------------------------

/// Returns the current `AppData` snapshot (all Contexts and Settings).
#[tauri::command]
pub fn get_app_data(app: tauri::AppHandle) -> AppData {
    app.state::<AppState>().data.lock().unwrap().clone()
}

/// Creates a new (non-Main) Context with an auto-generated name and no
/// shortcut. The name is the first `context-<n>` (n ≥ 1) not already in use,
/// honoring the same uniqueness rule `rename_context` enforces. The Context
/// starts visible, except under Single Context Mode where it starts hidden so
/// the active Context remains the sole visible one. Returns the newly created
/// `Context`.
#[tauri::command]
pub fn create_context(app: tauri::AppHandle) -> Context {
    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();
    // First unused default name. Deriving <n> from the Context count alone can
    // collide after a deletion (create context-1 and context-2, delete
    // context-1, create again → a second "context-2").
    let name = (1..)
        .map(|n| format!("context-{n}"))
        .find(|candidate| !data.contexts.iter().any(|c| &c.name == candidate))
        .expect("some context-<n> name is always unused");
    // New Contexts have no shortcut, so they join the unassigned tier at its end.
    let order = data.next_order();
    let ctx = Context {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        is_main: false,
        windows: vec![],
        shortcut_index: None,
        order,
        // Under Single Context Mode exactly one Context may be visible at a
        // time, so a newly created Context must start hidden — the currently
        // active Context stays the sole visible one until the user switches.
        visible: !data.settings.single_context_mode,
    };
    data.contexts.push(ctx.clone());
    let _ = state.save_tx.send(data.clone());
    ctx
}

/// Renames a Context. Enforces non-empty, unique, and non-"main" names.
///
/// # Errors
/// - Returns `Err` if no Context with `id` exists.
/// - Returns `Err` if the new name is empty or just whitespace.
/// - Returns `Err` if the new name is "main" (reserved).
/// - Returns `Err` if the new name is already used by another Context.
#[tauri::command]
pub fn rename_context(app: tauri::AppHandle, id: String, name: String) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Context name cannot be empty".to_string());
    }
    if trimmed.eq_ignore_ascii_case("main") {
        return Err("Context name cannot be 'main' (reserved)".to_string());
    }

    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();
    let ci = ctx_idx(&data, &id)?;

    // Check for uniqueness (excluding current context)
    if data.contexts.iter().enumerate().any(|(i, c)| i != ci && c.name == trimmed) {
        return Err(format!("Context name '{}' is already in use", trimmed));
    }

    data.contexts[ci].name = trimmed.to_string();
    let _ = state.save_tx.send(data.clone());
    Ok(())
}

/// Deletes a non-Main Context.
///
/// If the Context is currently visible, its windows are hidden first (via
/// `do_hide_context_windows`, which acquires the lock internally), so no window
/// is left stranded in an untrackable hidden state.
///
/// # Errors
/// Returns `Err` if `id` refers to the Main Context or does not exist.
#[tauri::command]
pub fn delete_context(app: tauri::AppHandle, id: String) -> Result<(), String> {
    // Validate and check whether we need to hide windows.
    let needs_hide = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        let ci = ctx_idx(&data, &id)?;
        if data.contexts[ci].is_main {
            return Err("cannot delete the Main Context".to_string());
        }
        data.contexts[ci].visible
    }; // lock released here

    if needs_hide {
        do_hide_context_windows(&app, &id);
    }

    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();
    if let Ok(ci) = ctx_idx(&data, &id) {
        data.contexts.remove(ci);
        let _ = state.save_tx.send(data.clone());
    }
    Ok(())
}

/// Assigns or clears the keyboard `shortcut_index` for a Context.
///
/// - Index 0 is reserved for Main and is rejected for non-Main Contexts.
/// - Indices 1–9 are available for non-Main Contexts.
/// - If another Context already holds the requested index, it is silently
///   unassigned (no error — the caller's intent wins).
/// - Pass `index: null` / `None` to remove the shortcut assignment.
///
/// # Errors
/// - Returns `Err` if the Context does not exist.
/// - Returns `Err` if index 0 is requested for a non-Main Context.
/// - Returns `Err` if the index is greater than 9 — only `<meta>+0`–`9`
///   shortcuts exist, so a larger index would be stored but never fire.
#[tauri::command]
pub fn assign_shortcut(app: tauri::AppHandle, id: String, index: Option<u8>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();
    let ci = ctx_idx(&data, &id)?;
    if let Some(idx) = index {
        if idx == 0 && !data.contexts[ci].is_main {
            return Err("shortcut index 0 is reserved for the Main Context".to_string());
        }
        if idx > 9 {
            return Err(format!("shortcut index {idx} is out of range; only 1-9 are available"));
        }
        // Release this index from any other Context currently holding it. A
        // Context that loses its shortcut this way falls into the unassigned
        // tier, so send it to the end of that tier.
        for i in 0..data.contexts.len() {
            if i != ci && data.contexts[i].shortcut_index == Some(idx) {
                data.contexts[i].shortcut_index = None;
                data.contexts[i].order = data.next_order();
            }
        }
    } else if data.contexts[ci].shortcut_index.is_some() {
        // Clearing this Context's shortcut demotes it into the unassigned tier;
        // place it at the end so it doesn't jump to an arbitrary spot.
        data.contexts[ci].order = data.next_order();
    }
    data.contexts[ci].shortcut_index = index;
    let _ = state.save_tx.send(data.clone());
    Ok(())
}

/// Sets the manual sidebar order of the unassigned tier (Contexts with no
/// `shortcut_index`) to the given sequence of Context ids.
///
/// `ordered_ids` must list exactly the Contexts that currently have no
/// shortcut, in the desired top-to-bottom order; each is assigned an ascending
/// `order` starting at 0. Shortcut-assigned Contexts (including Main) are
/// auto-ordered by `shortcut_index` and are never affected — passing one of
/// them, or omitting an unassigned Context, is rejected so the frontend and
/// backend can't silently drift out of sync.
///
/// # Errors
/// - Returns `Err` if any id is unknown, refers to a shortcut-assigned Context,
///   appears more than once, or if `ordered_ids` does not cover exactly the set
///   of currently-unassigned Contexts.
#[tauri::command]
pub fn reorder_contexts(app: tauri::AppHandle, ordered_ids: Vec<String>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();

    // The set this call is allowed to (and must fully) reorder.
    let unassigned: std::collections::HashSet<&str> =
        data.contexts.iter().filter(|c| c.shortcut_index.is_none()).map(|c| c.id.as_str()).collect();

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for id in &ordered_ids {
        if !unassigned.contains(id.as_str()) {
            return Err(format!("context '{id}' is not an unassigned (reorderable) context"));
        }
        if !seen.insert(id.as_str()) {
            return Err(format!("context '{id}' appears more than once in the new order"));
        }
    }
    if seen.len() != unassigned.len() {
        return Err("new order must cover exactly the unassigned contexts".to_string());
    }

    // Apply: position in `ordered_ids` becomes the new `order`. Shortcut-assigned
    // Contexts keep their (ignored) order; normalize_order re-densifies globally.
    let rank: std::collections::HashMap<&str, u32> =
        ordered_ids.iter().enumerate().map(|(i, id)| (id.as_str(), i as u32)).collect();
    for ctx in &mut data.contexts {
        if let Some(&r) = rank.get(ctx.id.as_str()) {
            ctx.order = r;
        }
    }
    data.normalize_order();
    let _ = state.save_tx.send(data.clone());
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands — Window membership
// ---------------------------------------------------------------------------

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
pub fn add_window_to_context(
    app: tauri::AppHandle,
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
pub fn remove_window_from_context(app: tauri::AppHandle, context_id: String, platform_id: u64) -> Result<(), String> {
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

// ---------------------------------------------------------------------------
// Tauri commands — Visibility
// ---------------------------------------------------------------------------

/// Shows a Context, making its hidden windows visible.
///
/// In Single Context Mode, all other currently-visible Contexts are hidden
/// first (applying the multi-Context visibility rule to each).
///
/// # Errors
/// Returns `Err` if the Context does not exist.
#[tauri::command]
pub fn show_context(app: tauri::AppHandle, id: String) -> Result<(), String> {
    // Validate existence and collect sibling IDs under a brief lock.
    let siblings_to_hide: Vec<String> = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        ctx_idx(&data, &id)?; // validate
        if data.settings.single_context_mode {
            data.contexts.iter().filter(|c| c.id != id && c.visible).map(|c| c.id.clone()).collect()
        } else {
            vec![]
        }
    }; // lock released

    for sibling_id in siblings_to_hide {
        do_hide_context_windows(&app, &sibling_id);
    }
    do_show_context_windows(&app, &id);
    Ok(())
}

/// Hides a Context. Windows that have no other visible Context are moved
/// offscreen (macOS) or hidden via `SW_HIDE` (Windows).
///
/// # Errors
/// Returns `Err` if the Context does not exist.
#[tauri::command]
pub fn hide_context(app: tauri::AppHandle, id: String) -> Result<(), String> {
    // Validate existence before delegating.
    {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        ctx_idx(&data, &id)?;
    }
    do_hide_context_windows(&app, &id);
    Ok(())
}

/// Hides all currently-visible Contexts. Reached only from the `<meta>+H`
/// global shortcut via `handle_shortcut`; the frontend has no hide-all
/// affordance, so this is deliberately not a Tauri command.
///
/// Visible Context IDs are collected under a single lock acquisition; then
/// each is hidden in turn (each call to `do_hide_context_windows` re-acquires
/// the lock internally, so windows hidden by an earlier call are not re-hidden
/// by later calls — the not-`hidden` filter ensures this).
fn hide_all(app: &tauri::AppHandle) {
    let visible_ids: Vec<String> = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        data.contexts.iter().filter(|c| c.visible).map(|c| c.id.clone()).collect()
    };
    for id in visible_ids {
        do_hide_context_windows(app, &id);
    }
}

// ---------------------------------------------------------------------------
// Hotkey dispatch (called from the global-shortcut handler in lib.rs)
// ---------------------------------------------------------------------------

/// Toggles the Context assigned to `shortcut_index`: hides it if visible,
/// shows it if hidden. No-op if no Context has that index.
///
/// The data lock is acquired briefly to read state, then released before
/// calling `show_context`/`hide_context` (which re-acquire it), preventing
/// deadlocks.
fn toggle_context_by_shortcut(app: &tauri::AppHandle, shortcut_index: u8) {
    let (ctx_id, is_visible) = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        match data.contexts.iter().find(|c| c.shortcut_index == Some(shortcut_index)) {
            Some(c) => (c.id.clone(), c.visible),
            None => return,
        }
    };
    if is_visible {
        let _ = hide_context(app.clone(), ctx_id);
    } else {
        let _ = show_context(app.clone(), ctx_id);
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
pub fn handle_shortcut(app: &tauri::AppHandle, shortcut: &tauri_plugin_global_shortcut::Shortcut) {
    if shortcut.key == Code::KeyH {
        hide_all(app);
    } else if let Some(n) = digit_of(shortcut.key) {
        toggle_context_by_shortcut(app, n);
    } else {
        return;
    }
    // Notify the frontend so visibility indicators update immediately rather
    // than waiting for the next periodic poll.
    let _ = app.emit("contexts-changed", ());
}

/// Shows and focuses the main window. Reached only from the tray menu's
/// "Open Context Manager" item; the frontend runs inside this window, so this
/// is deliberately not a Tauri command.
pub fn open_main_window(app: tauri::AppHandle) {
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

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Updates application settings and saves to disk.
///
/// If the update turns Single Context Mode **on**, or changes the chosen Context
/// while it is on, the chosen Context is force-shown — which, because the mode is
/// now on, causes `show_context` to hide every other Context. Unrelated settings
/// edits (meta key, launch-at-login, toggling the mode off) don't move any
/// windows. The chosen Context is resolved from `single_context_id`, falling back
/// to Main when it is unset or names a Context that no longer exists.
#[tauri::command]
pub fn update_settings(app: tauri::AppHandle, settings: crate::state::Settings) -> Result<(), String> {
    // Store settings and decide whether to enforce single-context visibility,
    // all under a brief lock. `show_context` (which re-acquires the lock) is
    // called only after the lock is released.
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
        show_context(app.clone(), id)?;
        // Nudge the frontend to refresh visibility indicators immediately.
        let _ = app.emit("contexts-changed", ());
    }
    Ok(())
}

/// Opens the settings view: shows/focuses the main window and emits the
/// `show-settings` event so the frontend switches to the settings panel.
/// Reached only from the application menu's Settings item; deliberately not a
/// Tauri command — the frontend opens its settings panel directly.
pub fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("show-settings", ());
    }
    Ok(())
}
