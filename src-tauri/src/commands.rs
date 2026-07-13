use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::Code;

use crate::state::{AppData, AppState, Context};
use crate::wm;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Returns the index of the context with the given `id` in `data.contexts`.
fn ctx_idx(data: &AppData, id: &str) -> Result<usize, String> {
    data.contexts.iter().position(|c| c.id == id).ok_or_else(|| format!("context '{id}' not found"))
}

/// Physically hides windows that would have no remaining visible Context after
/// Context `ctx_id` is hidden, then marks that Context `visible = false`.
///
/// Three-phase design to satisfy the borrow checker and keep cross-context
/// `original_position` in sync:
///
/// **Phase 1 (immutable, under lock)** — Collect the `(platform_id, WindowRef)`
/// of every window in the target Context that is currently physically visible
/// (`original_position == None`) and has no other visible Context.
///
/// **Phase 2 (OS calls, lock released)** — Call `wm::hide_window` on each
/// clone. Per-window errors are printed and skipped.
///
/// **Phase 3 (mutation, under lock)** — Mark the Context `visible = false`,
/// then write the `original_position` captured in Phase 2 back to every copy
/// of each hidden window across all Contexts. Saves state.
fn do_hide_context_windows(app: &tauri::AppHandle, ctx_id: &str) {
    let state = app.state::<AppState>();

    // Capture the current front-to-back stacking order (macOS) before minimizing,
    // so show can restore it. Done outside the lock — it is an OS enumeration call.
    #[cfg(target_os = "macos")]
    let z_map: std::collections::HashMap<u64, u32> =
        wm::enumerate(std::process::id()).iter().enumerate().map(|(i, w)| (w.platform_id, i as u32)).collect();

    // Phase 1 — collect under lock
    let hide_targets: Vec<(u64, crate::state::WindowRef)> = {
        let data = state.data.lock().unwrap();
        let ci = match ctx_idx(&data, ctx_id) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("do_hide_context_windows: {e}");
                return;
            },
        };
        data.contexts[ci]
            .windows
            .iter()
            .filter(|w| {
                w.original_position.is_none()
                    && !data.contexts.iter().enumerate().any(|(i, c)| {
                        i != ci && c.visible && c.windows.iter().any(|cw| cw.platform_id == w.platform_id)
                    })
            })
            .map(|w| (w.platform_id, w.clone()))
            .collect()
    };

    // Phase 2 — OS calls outside the lock
    let mut hidden: Vec<(u64, Option<[f64; 2]>)> = Vec::with_capacity(hide_targets.len());
    for (platform_id, mut win_clone) in hide_targets {
        match wm::hide_window(&mut win_clone) {
            Ok(()) => hidden.push((platform_id, win_clone.original_position)),
            Err(e) => eprintln!("hide_window({platform_id}): {e}"),
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
    for (platform_id, orig_pos) in &hidden {
        for ctx in &mut data.contexts {
            for w in &mut ctx.windows {
                if w.platform_id == *platform_id {
                    w.original_position = *orig_pos;
                    #[cfg(target_os = "macos")]
                    {
                        w.hidden_z = z_map.get(platform_id).copied();
                    }
                }
            }
        }
    }
    let _ = state.save_tx.send(data.clone());
}

/// Physically shows all windows in Context `ctx_id` that are currently hidden,
/// then marks that Context `visible = true`.
///
/// Mirrors the three-phase structure of `do_hide_context_windows`:
/// collect hidden windows under lock → call `wm::show_window` outside lock
/// → clear `original_position` on all copies and mark visible under lock.
fn do_show_context_windows(app: &tauri::AppHandle, ctx_id: &str) {
    let state = app.state::<AppState>();

    // Phase 1
    // `mut` is used only on macOS, to reorder for z-order restoration.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut show_targets: Vec<(u64, crate::state::WindowRef)> = {
        let data = state.data.lock().unwrap();
        let ci = match ctx_idx(&data, ctx_id) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("do_show_context_windows: {e}");
                return;
            },
        };
        data.contexts[ci]
            .windows
            .iter()
            .filter(|w| w.original_position.is_some())
            .map(|w| (w.platform_id, w.clone()))
            .collect()
    };

    // On macOS, restore stacking order: un-minimize back-to-front (highest rank
    // first) so the window that was frontmost when hidden is un-minimized last.
    // Windows with an unknown rank sort to the back (un-minimized first).
    #[cfg(target_os = "macos")]
    show_targets.sort_by(|a, b| b.1.hidden_z.unwrap_or(u32::MAX).cmp(&a.1.hidden_z.unwrap_or(u32::MAX)));

    // Remember the frontmost (lowest rank) window to explicitly raise once all
    // windows are un-minimized, reinstating it as the top window.
    #[cfg(target_os = "macos")]
    let frontmost: Option<crate::state::WindowRef> =
        show_targets.iter().min_by_key(|(_, w)| w.hidden_z.unwrap_or(u32::MAX)).map(|(_, w)| w.clone());

    // Phase 2
    let mut shown: Vec<u64> = Vec::with_capacity(show_targets.len());
    for (platform_id, mut win_clone) in show_targets {
        match wm::show_window(&mut win_clone) {
            Ok(()) => shown.push(platform_id),
            Err(e) => eprintln!("show_window({platform_id}): {e}"),
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
    for platform_id in &shown {
        for ctx in &mut data.contexts {
            for w in &mut ctx.windows {
                if w.platform_id == *platform_id {
                    w.original_position = None;
                    #[cfg(target_os = "macos")]
                    {
                        w.hidden_z = None;
                    }
                }
            }
        }
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

/// Creates a new (non-Main) Context with an auto-generated name, no shortcut,
/// and `visible = true`. Returns the newly created `Context`.
#[tauri::command]
pub fn create_context(app: tauri::AppHandle) -> Context {
    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();
    let n = data.contexts.len();
    let ctx = Context {
        id: uuid::Uuid::new_v4().to_string(),
        name: format!("context-{n}"),
        is_main: false,
        windows: vec![],
        shortcut_index: None,
        visible: true,
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
/// Returns `Err` if the Context does not exist or if index 0 is requested for
/// a non-Main Context.
#[tauri::command]
pub fn assign_shortcut(app: tauri::AppHandle, id: String, index: Option<u8>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();
    let ci = ctx_idx(&data, &id)?;
    if let Some(idx) = index {
        if idx == 0 && !data.contexts[ci].is_main {
            return Err("shortcut index 0 is reserved for the Main Context".to_string());
        }
        // Release this index from any other Context currently holding it.
        for i in 0..data.contexts.len() {
            if i != ci && data.contexts[i].shortcut_index == Some(idx) {
                data.contexts[i].shortcut_index = None;
            }
        }
    }
    data.contexts[ci].shortcut_index = index;
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
/// **move** leaves it only in hidden Contexts, it is hidden. `original_position`
/// (and `hidden_z` on macOS) is then propagated to every copy.
///
/// When adding to a non-Main Context, the window is **moved** out of Main by
/// default (removed from Main). Pass `copy = true` to keep it in Main as well,
/// so it stays available to add to further Contexts.
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
        let data = state.data.lock().unwrap();
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
        let was_hidden = win.original_position.is_some();
        (win, was_hidden && will_be_visible, !was_hidden && !will_be_visible)
    };

    // Reconcile the window's physical visibility outside the lock.
    if should_show {
        if let Err(e) = wm::show_window(&mut win_clone) {
            eprintln!("add_window_to_context show_window({platform_id}): {e}");
        }
        #[cfg(target_os = "macos")]
        {
            win_clone.hidden_z = None;
        }
    } else if should_hide {
        // Capture the front-to-back stacking rank while still visible so a later
        // show can restore it (matches `do_hide_context_windows`).
        #[cfg(target_os = "macos")]
        {
            win_clone.hidden_z =
                wm::enumerate(std::process::id()).iter().position(|w| w.platform_id == platform_id).map(|i| i as u32);
        }
        if let Err(e) = wm::hide_window(&mut win_clone) {
            eprintln!("add_window_to_context hide_window({platform_id}): {e}");
        }
    }

    // Re-acquire to persist the membership and propagate any position change.
    let mut data = state.data.lock().unwrap();
    let ci = ctx_idx(&data, &context_id)?;
    // Propagate position state to all existing copies.
    let new_pos = win_clone.original_position;
    #[cfg(target_os = "macos")]
    let new_z = win_clone.hidden_z;
    for ctx in &mut data.contexts {
        for w in &mut ctx.windows {
            if w.platform_id == platform_id {
                w.original_position = new_pos;
                #[cfg(target_os = "macos")]
                {
                    w.hidden_z = new_z;
                }
            }
        }
    }
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
/// Idempotent: returns `Ok` if the window is not a member.
///
/// # Errors
/// Returns `Err` if the Context does not exist.
#[tauri::command]
pub fn remove_window_from_context(app: tauri::AppHandle, context_id: String, platform_id: u64) -> Result<(), String> {
    let state = app.state::<AppState>();

    // Collect info before OS call: window's current state and its contexts after removal.
    let (mut win_clone, should_show, should_hide, readd_to_main) = {
        let data = state.data.lock().unwrap();
        let ci = ctx_idx(&data, &context_id)?;

        // Find the window across all contexts.
        let win = data
            .contexts
            .iter()
            .flat_map(|c| c.windows.iter())
            .find(|w| w.platform_id == platform_id)
            .ok_or_else(|| format!("window {platform_id} not found in any context"))?
            .clone();

        let was_hidden = win.original_position.is_some();
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

        (win, should_show, should_hide, readd_to_main)
    };

    // Reconcile the window's physical visibility outside the lock.
    if should_show {
        if let Err(e) = wm::show_window(&mut win_clone) {
            eprintln!("remove_window_from_context show_window({platform_id}): {e}");
        }
        #[cfg(target_os = "macos")]
        {
            win_clone.hidden_z = None;
        }
    } else if should_hide {
        // Capture the front-to-back stacking rank while still visible so a later
        // show can restore it (matches `do_hide_context_windows`).
        #[cfg(target_os = "macos")]
        {
            win_clone.hidden_z =
                wm::enumerate(std::process::id()).iter().position(|w| w.platform_id == platform_id).map(|i| i as u32);
        }
        if let Err(e) = wm::hide_window(&mut win_clone) {
            eprintln!("remove_window_from_context hide_window({platform_id}): {e}");
        }
    }

    // Re-acquire lock and apply the removal.
    let mut data = state.data.lock().unwrap();
    let ci = ctx_idx(&data, &context_id)?;
    data.contexts[ci].windows.retain(|w| w.platform_id != platform_id);

    // Propagate position state to all remaining copies.
    let new_pos = win_clone.original_position;
    #[cfg(target_os = "macos")]
    let new_z = win_clone.hidden_z;
    for ctx in &mut data.contexts {
        for w in &mut ctx.windows {
            if w.platform_id == platform_id {
                w.original_position = new_pos;
                #[cfg(target_os = "macos")]
                {
                    w.hidden_z = new_z;
                }
            }
        }
    }

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

/// Hides all currently-visible Contexts.
///
/// Visible Context IDs are collected under a single lock acquisition; then
/// each is hidden in turn (each call to `do_hide_context_windows` re-acquires
/// the lock internally, so windows hidden by an earlier call are not re-hidden
/// by later calls — the `original_position.is_none()` filter ensures this).
#[tauri::command]
pub fn hide_all(app: tauri::AppHandle) {
    let visible_ids: Vec<String> = {
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        data.contexts.iter().filter(|c| c.visible).map(|c| c.id.clone()).collect()
    };
    for id in visible_ids {
        do_hide_context_windows(&app, &id);
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

/// Dispatches a pressed global shortcut to the appropriate Context action.
///
/// - `<meta>+0`–`9` — toggle the Context assigned to that `shortcut_index`.
/// - `<meta>+H` — hide all Contexts.
///
/// Called from the `with_handler` closure in `lib.rs`.
pub fn handle_shortcut(app: &tauri::AppHandle, shortcut: &tauri_plugin_global_shortcut::Shortcut) {
    match shortcut.key {
        Code::Digit0 => toggle_context_by_shortcut(app, 0),
        Code::Digit1 => toggle_context_by_shortcut(app, 1),
        Code::Digit2 => toggle_context_by_shortcut(app, 2),
        Code::Digit3 => toggle_context_by_shortcut(app, 3),
        Code::Digit4 => toggle_context_by_shortcut(app, 4),
        Code::Digit5 => toggle_context_by_shortcut(app, 5),
        Code::Digit6 => toggle_context_by_shortcut(app, 6),
        Code::Digit7 => toggle_context_by_shortcut(app, 7),
        Code::Digit8 => toggle_context_by_shortcut(app, 8),
        Code::Digit9 => toggle_context_by_shortcut(app, 9),
        Code::KeyH => hide_all(app.clone()),
        _ => return,
    }
    // Notify the frontend so visibility indicators update immediately rather
    // than waiting for the next periodic poll.
    let _ = app.emit("contexts-changed", ());
}

/// Shows and focuses the main window.
#[tauri::command]
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

/// Returns the current application settings.
#[tauri::command]
pub fn get_settings(app: tauri::AppHandle) -> Result<crate::state::Settings, String> {
    let state = app.state::<AppState>();
    let data = state.data.lock().unwrap();
    Ok(data.settings.clone())
}

/// Updates application settings and saves to disk.
///
/// If the update turns Single Context Mode **on**, or changes the chosen Context
/// while it is on, the chosen Context is force-shown — which, because the mode is
/// now on, causes `show_context` to hide every other Context. Unrelated settings
/// edits (meta key, launch-at-login, toggling the mode off) don't move any
/// windows. The chosen Context is resolved from `single_context_id`, falling back
/// to Main when it is unset or names a Context that no longer exists.
///
/// Also reconciles the OS login-item registration with `settings.launch_at_login`
/// via the autostart plugin. Done outside the `AppState` lock (it is an OS call);
/// a failure here is returned as an error so the Settings UI surfaces it rather
/// than silently leaving the toggle as a no-op.
#[tauri::command]
pub fn update_settings(app: tauri::AppHandle, settings: crate::state::Settings) -> Result<(), String> {
    let want_autostart = settings.launch_at_login;

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

    let autolaunch = app.autolaunch();
    let autostart_result = if want_autostart { autolaunch.enable() } else { autolaunch.disable() };
    autostart_result.map_err(|e| e.to_string())?;

    if let Some(id) = enforce_id {
        show_context(app.clone(), id)?;
        // Nudge the frontend to refresh visibility indicators immediately.
        let _ = app.emit("contexts-changed", ());
    }
    Ok(())
}

/// Opens the settings view: shows/focuses the main window and emits the
/// `show-settings` event so the frontend switches to the settings panel.
#[tauri::command]
pub fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("show-settings", ());
    }
    Ok(())
}
