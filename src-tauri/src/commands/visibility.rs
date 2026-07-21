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
