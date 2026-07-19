//! Context CRUD commands: create/rename/delete, keyboard-shortcut assignment,
//! and manual sidebar ordering.

use tauri::Manager;

use super::{ctx_idx, do_hide_context_windows};
use crate::state::{AppData, AppState, Context};

/// Returns the current `AppData` snapshot (all Contexts and Settings).
#[tauri::command]
pub fn get_app_data<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> AppData {
    app.state::<AppState>().data.lock().unwrap().clone()
}

/// Creates a new (non-Main) Context with an auto-generated name and no
/// shortcut. The name is the first `context-<n>` (n ≥ 1) not already in use,
/// honoring the same uniqueness rule `rename_context` enforces. The Context
/// starts visible, except under Single Context Mode where it starts hidden so
/// the active Context remains the sole visible one. Returns the newly created
/// `Context`.
#[tauri::command]
pub fn create_context<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Context {
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
pub fn rename_context<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String, name: String) -> Result<(), String> {
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
pub fn delete_context<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String) -> Result<(), String> {
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
pub fn assign_shortcut<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    id: String,
    index: Option<u8>,
) -> Result<(), String> {
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
pub fn reorder_contexts<R: tauri::Runtime>(app: tauri::AppHandle<R>, ordered_ids: Vec<String>) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use tauri::Manager;

    use super::*;
    use crate::test_util::{app_data, ctx, main_ctx, mock_app, win};

    // Seed test proving the MockRuntime harness: a real command body runs
    // against a real AppState, and the save channel is signalled.
    #[test]
    fn create_context_uses_first_unused_default_name_and_signals_save() {
        let (app, rx) = mock_app(app_data(vec![main_ctx("m", true, vec![])]));
        assert!(!rx.has_changed().unwrap());

        let first = create_context(app.handle().clone());
        assert_eq!(first.name, "context-1");
        assert!(!first.is_main);
        assert_eq!(first.shortcut_index, None);
        assert!(first.visible);
        assert!(rx.has_changed().unwrap());

        let second = create_context(app.handle().clone());
        assert_eq!(second.name, "context-2");

        let state = app.state::<crate::state::AppState>();
        assert_eq!(state.data.lock().unwrap().contexts.len(), 3);
    }

    #[test]
    fn rename_context_rejects_reserved_and_duplicate_names() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![])]));
        let handle = app.handle();
        assert!(rename_context(handle.clone(), "a".into(), "  ".into()).is_err());
        assert!(rename_context(handle.clone(), "a".into(), "Main".into()).is_err());
        assert!(rename_context(handle.clone(), "a".into(), "main".into()).is_err());
        assert!(rename_context(handle.clone(), "missing".into(), "x".into()).is_err());
        assert!(rename_context(handle.clone(), "a".into(), "focus".into()).is_ok());
    }

    #[test]
    fn rename_rejects_names_held_by_other_contexts_and_trims_whitespace() {
        let (app, _rx) =
            mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![]), ctx("b", true, vec![])]));
        let handle = app.handle();
        // ctx() names Contexts "name-<id>".
        assert!(rename_context(handle.clone(), "a".into(), "name-b".into()).is_err());
        assert!(rename_context(handle.clone(), "a".into(), "  focus  ".into()).is_ok());
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert_eq!(data.contexts[1].name, "focus");
    }

    #[test]
    fn create_context_reuses_freed_default_names() {
        let mut data = app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![])]);
        data.contexts[1].name = "context-2".to_string();
        let (app, _rx) = mock_app(data);
        // context-2 is taken but context-1 is free — the gap is filled first.
        assert_eq!(create_context(app.handle().clone()).name, "context-1");
        assert_eq!(create_context(app.handle().clone()).name, "context-3");
    }

    #[test]
    fn create_context_starts_hidden_under_single_context_mode() {
        let mut data = app_data(vec![main_ctx("m", true, vec![])]);
        data.settings.single_context_mode = true;
        let (app, _rx) = mock_app(data);
        let created = create_context(app.handle().clone());
        assert!(!created.visible, "the active Context must remain the sole visible one");
    }

    #[test]
    fn new_contexts_join_the_end_of_the_unassigned_tier() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![])]));
        let created = create_context(app.handle().clone());
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        let max_other = data.contexts.iter().filter(|c| c.id != created.id).map(|c| c.order).max().unwrap();
        let created_order = data.contexts.iter().find(|c| c.id == created.id).unwrap().order;
        assert!(created_order > max_other);
    }

    #[test]
    fn delete_context_rejects_main_and_unknown_ids() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![])]));
        assert!(delete_context(app.handle().clone(), "m".into()).is_err());
        assert!(delete_context(app.handle().clone(), "ghost".into()).is_err());
    }

    #[test]
    fn deleting_a_visible_context_hides_its_exclusive_windows_first() {
        use crate::wm::mock::{self, Call};
        // Window 1 is shared with visible main; window 2 is exclusive to a.
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", true, vec![win(1, false)]),
            ctx("a", true, vec![win(1, false), win(2, false)]),
        ]));
        delete_context(app.handle().clone(), "a".into()).unwrap();

        assert_eq!(mock::calls(), vec![Call::Hide(2)]);
        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        assert_eq!(data.contexts.len(), 1);
        assert!(!data.contexts[0].windows[0].hidden, "the shared window stays visible in main");
    }

    #[test]
    fn deleting_a_hidden_context_touches_no_windows() {
        use crate::wm::mock;
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", false, vec![win(2, true)])]));
        delete_context(app.handle().clone(), "a".into()).unwrap();
        assert!(mock::calls().is_empty());
    }

    #[test]
    fn assign_shortcut_validates_range_and_the_reserved_zero() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![])]));
        let handle = app.handle();
        assert!(assign_shortcut(handle.clone(), "a".into(), Some(0)).is_err(), "0 is reserved for Main");
        assert!(assign_shortcut(handle.clone(), "a".into(), Some(10)).is_err(), "only 0-9 shortcuts exist");
        assert!(assign_shortcut(handle.clone(), "ghost".into(), Some(1)).is_err());
        assert!(assign_shortcut(handle.clone(), "m".into(), Some(0)).is_ok(), "Main may keep 0");
        assert!(assign_shortcut(handle.clone(), "a".into(), Some(9)).is_ok());
    }

    #[test]
    fn assign_shortcut_steals_the_index_and_demotes_the_previous_holder() {
        let mut data = app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![]), ctx("b", true, vec![])]);
        data.contexts[1].shortcut_index = Some(1);
        let (app, _rx) = mock_app(data);
        assign_shortcut(app.handle().clone(), "b".into(), Some(1)).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        let a = data.contexts.iter().find(|c| c.id == "a").unwrap();
        let b = data.contexts.iter().find(|c| c.id == "b").unwrap();
        assert_eq!(b.shortcut_index, Some(1), "the caller's intent wins");
        assert_eq!(a.shortcut_index, None, "the previous holder is unassigned");
        let max_other = data.contexts.iter().filter(|c| c.id != "a").map(|c| c.order).max().unwrap();
        assert!(a.order > max_other, "the demoted Context lands at the end of the unassigned tier");
    }

    #[test]
    fn clearing_a_shortcut_demotes_the_context_to_the_end_of_the_unassigned_tier() {
        let mut data = app_data(vec![main_ctx("m", true, vec![]), ctx("a", true, vec![]), ctx("b", true, vec![])]);
        data.contexts[1].shortcut_index = Some(1);
        let (app, _rx) = mock_app(data);
        assign_shortcut(app.handle().clone(), "a".into(), None).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        let a = data.contexts.iter().find(|c| c.id == "a").unwrap();
        assert_eq!(a.shortcut_index, None);
        let max_other = data.contexts.iter().filter(|c| c.id != "a").map(|c| c.order).max().unwrap();
        assert!(a.order > max_other);
    }

    #[test]
    fn reorder_contexts_applies_the_new_unassigned_order() {
        let (app, _rx) = mock_app(app_data(vec![
            main_ctx("m", true, vec![]),
            ctx("b", true, vec![]),
            ctx("c", true, vec![]),
            ctx("d", true, vec![]),
        ]));
        reorder_contexts(app.handle().clone(), vec!["d".into(), "b".into(), "c".into()]).unwrap();

        let state = app.state::<AppState>();
        let data = state.data.lock().unwrap();
        let order_of = |id: &str| data.contexts.iter().find(|c| c.id == id).unwrap().order;
        assert!(order_of("d") < order_of("b"));
        assert!(order_of("b") < order_of("c"));
    }

    #[test]
    fn reorder_contexts_rejects_invalid_sets() {
        let mut data = app_data(vec![main_ctx("m", true, vec![]), ctx("b", true, vec![]), ctx("c", true, vec![])]);
        data.contexts[1].shortcut_index = Some(1); // b is shortcut-assigned
        let (app, _rx) = mock_app(data);
        let handle = app.handle();
        // Shortcut-assigned Contexts are auto-ordered and off-limits.
        assert!(reorder_contexts(handle.clone(), vec!["b".into(), "c".into()]).is_err());
        assert!(reorder_contexts(handle.clone(), vec!["ghost".into()]).is_err());
        assert!(reorder_contexts(handle.clone(), vec!["c".into(), "c".into()]).is_err());
        // Must cover exactly the unassigned set (here: just c).
        assert!(reorder_contexts(handle.clone(), vec![]).is_err());
        assert!(reorder_contexts(handle.clone(), vec!["c".into()]).is_ok());
    }

    #[test]
    fn get_app_data_returns_the_current_snapshot() {
        let (app, _rx) = mock_app(app_data(vec![main_ctx("m", true, vec![win(7, false)])]));
        let data = get_app_data(app.handle().clone());
        assert_eq!(data.contexts.len(), 1);
        assert_eq!(data.contexts[0].windows[0].platform_id, 7);
    }
}
