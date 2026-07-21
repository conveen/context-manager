//! Tauri command handlers, grouped by concern, plus the shared window-state
//! helpers they build on:
//! - [`context`] — Context CRUD, shortcut assignment, and sidebar ordering.
//! - [`membership`] — moving/copying windows between Contexts.
//! - [`visibility`] — showing/hiding Contexts and global-hotkey dispatch.
//! - [`settings`] — settings updates and the app-window helpers reached from
//!   the native menu and tray.
//!
//! lib.rs registers the `#[tauri::command]` handlers by their full module
//! path (`generate_handler!` needs the macro-generated companion items next
//! to each command, so a plain re-export is not enough). The Rust-only entry
//! points called from lib.rs (hotkey dispatch, tray, and menu handlers) are
//! re-exported below.

pub mod context;
pub mod membership;
pub mod settings;
pub mod visibility;

pub use settings::{open_main_window, open_settings};
pub use visibility::handle_shortcut;

use tauri::Manager;

use crate::state::{AppData, AppState, WindowRef};
use crate::wm;

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
fn do_hide_context_windows<R: tauri::Runtime>(app: &tauri::AppHandle<R>, ctx_id: &str) {
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
fn do_show_context_windows<R: tauri::Runtime>(app: &tauri::AppHandle<R>, ctx_id: &str) {
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
