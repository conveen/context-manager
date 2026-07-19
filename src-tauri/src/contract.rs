//! Cross-layer contract tests against the shared fixtures in `fixtures/`.
//!
//! The frontend half lives in `src/lib/contract.test.ts`; both suites assert
//! the same committed files, so a rename or shape change on either side of
//! the IPC boundary fails one of the two.

use serde::Deserialize;

use crate::events;
use crate::state::{AppData, MetaKey};

const EVENTS_JSON: &str = include_str!("../../fixtures/events.json");
const APP_DATA_JSON: &str = include_str!("../../fixtures/app_data.json");

#[derive(Deserialize)]
struct EventsFixture {
    backend_to_frontend: Vec<String>,
}

#[test]
fn event_name_constants_match_the_shared_fixture() {
    let fixture: EventsFixture = serde_json::from_str(EVENTS_JSON).expect("events.json must parse");
    assert_eq!(
        fixture.backend_to_frontend,
        vec![events::CONTEXTS_CHANGED, events::SHOW_SETTINGS],
        "fixtures/events.json and src-tauri/src/events.rs disagree — update both together"
    );
}

#[test]
fn app_data_fixture_deserializes_and_round_trips() {
    let data: AppData = serde_json::from_str(APP_DATA_JSON).expect("app_data.json must deserialize as AppData");

    // Spot-check representative values so a silently-defaulted field (e.g.
    // after a rename, serde would fall back to the default) is caught.
    assert_eq!(data.contexts.len(), 3);
    let main = &data.contexts[0];
    assert!(main.is_main);
    assert_eq!(main.shortcut_index, Some(0));
    let hidden_win = &main.windows[1];
    assert_eq!(hidden_win.platform_id, 102);
    assert!(hidden_win.hidden);
    #[cfg(target_os = "macos")]
    {
        assert_eq!(hidden_win.pid, 4243);
        assert_eq!(hidden_win.hidden_z, Some(1));
    }
    assert_eq!(data.contexts[2].shortcut_index, None);
    assert_eq!(data.settings.meta_key, MetaKey::CmdOpt);
    assert!(data.settings.single_context_mode);
    assert_eq!(data.settings.single_context_id.as_deref(), Some("0b0b0b0b-1111-4222-8333-555555555555"));

    // Round trip: what the backend writes, the backend reads back identically.
    let serialized = serde_json::to_string(&data).expect("serialize");
    let again: AppData = serde_json::from_str(&serialized).expect("re-deserialize");
    assert_eq!(again, data);
}
