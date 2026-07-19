//! Context CRUD commands: create/rename/delete, keyboard-shortcut assignment,
//! and manual sidebar ordering.

use tauri::Manager;

use super::{ctx_idx, do_hide_context_windows};
use crate::state::{AppData, AppState, Context};

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
