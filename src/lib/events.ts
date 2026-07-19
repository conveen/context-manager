// Names of the events the backend emits to the frontend.
//
// Mirrored in src-tauri/src/events.rs; tests on both sides assert their
// constants against the shared contract fixture so the two can't silently
// drift.

/** Backend Context visibility changed outside a frontend-initiated command
 * (global shortcuts, Single Context Mode enforcement); refresh immediately. */
export const CONTEXTS_CHANGED = "contexts-changed";

/** The native menu's Settings item was activated; switch to the settings panel. */
export const SHOW_SETTINGS = "show-settings";
