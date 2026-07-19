//! Names of the events the backend emits to the frontend.
//!
//! Kept as constants (mirrored in `src/lib/events.ts` on the frontend) so the
//! two sides can't silently drift: tests on both sides assert their constants
//! against the shared contract fixture.

/// Emitted whenever Context visibility changes outside a frontend-initiated
/// command (global shortcuts, Single Context Mode enforcement), so the
/// frontend refreshes immediately instead of waiting for its periodic poll.
pub const CONTEXTS_CHANGED: &str = "contexts-changed";

/// Emitted at the main window when the native menu's Settings item is
/// activated, so the frontend switches to the settings panel.
pub const SHOW_SETTINGS: &str = "show-settings";
