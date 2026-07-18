use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// A reference to an OS window that is tracked within one or more Contexts.
///
/// `original_position` is populated the first time the window is hidden and
/// cleared when it is shown again, allowing the window to be restored to
/// exactly where it was before it was hidden.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WindowRef {
    /// Stable OS-assigned window identifier: CGWindowID on macOS, HWND value on Windows.
    pub platform_id: u64,
    /// OS process ID of the owning application. Present only on macOS, where it
    /// is required to look up the `AXUIElement` for position manipulation.
    /// Defaults to 0 for entries loaded from state persisted before this field
    /// was introduced (those entries are cleaned up by the background poll on
    /// next launch).
    #[cfg(target_os = "macos")]
    #[serde(default)]
    pub pid: u32,
    /// Display name of the owning application (e.g. "Safari", "Visual Studio Code").
    pub app_name: String,
    /// Title of the window at the time it was added to a Context.
    pub window_title: String,
    /// Hidden-by-us marker: `Some` while the window is hidden, `None` while it
    /// is visible. On macOS the value is the screen coordinates `[x, y]`
    /// captured just before hiding; on Windows it is a meaningless sentinel
    /// (`SW_SHOW` restores geometry natively). Only the presence of the value
    /// may be relied on cross-platform.
    pub original_position: Option<[f64; 2]>,
    /// Front-to-back stacking rank captured when the window was hidden (`0` =
    /// frontmost, larger = further back), used to restore z-order on show by
    /// un-minimizing back-to-front. macOS-only: hiding elsewhere preserves the OS
    /// z-order natively. `None` while visible or if the rank is unknown.
    #[cfg(target_os = "macos")]
    #[serde(default)]
    pub hidden_z: Option<u32>,
}

/// A named group of windows that can be shown or hidden together.
///
/// A window may belong to more than one Context simultaneously. A window is
/// considered visible on screen when at least one of its Contexts is visible.
/// The Main Context (identified by `is_main == true`) is always present and
/// receives newly detected windows automatically.
///
/// # Invariants
/// - Exactly one `Context` in `AppData::contexts` has `is_main == true`.
/// - The Main Context always has `shortcut_index == Some(0)`.
/// - `id` is a UUID v4 string and is unique across all Contexts.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Context {
    /// UUID v4 string uniquely identifying this Context.
    pub id: String,
    /// User-visible name. Defaults to `"context-<n>"` for new Contexts; `"main"` for Main.
    pub name: String,
    /// Whether this is the Main Context. The Main Context cannot be deleted.
    pub is_main: bool,
    /// Windows currently assigned to this Context.
    pub windows: Vec<WindowRef>,
    /// Index used to derive the `<meta>+n` keyboard shortcut. `None` means no
    /// shortcut is assigned. Main is always `Some(0)`; other Contexts may use 1–9.
    pub shortcut_index: Option<u8>,
    /// Manual sort key for the sidebar's *unassigned* tier (Contexts with no
    /// `shortcut_index`), ascending. Ignored for Contexts that have a shortcut,
    /// which the sidebar auto-orders by `shortcut_index` instead. Kept globally
    /// unique and dense by [`AppData::normalize_order`]. Defaults to `0` for
    /// entries loaded from state persisted before this field existed; the
    /// normalization pass on load rewrites those by their array position so the
    /// pre-existing display order is preserved.
    #[serde(default)]
    pub order: u32,
    /// Whether this Context (and its exclusive windows) is currently shown on screen.
    pub visible: bool,
}

/// The modifier key combination used as the hotkey prefix for all Context shortcuts.
///
/// # Invariants
/// - The chosen combination must not conflict with common system shortcuts.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum MetaKey {
    /// `Ctrl+Alt` — default; works on Windows and Linux; available on macOS.
    CtrlAlt,
    /// `Command+Option` — macOS-native feel; maps to `CommandOrControl+Alt` in Tauri.
    CmdOpt,
}

/// User-configurable application settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Settings {
    /// Modifier key prefix used for all Context keyboard shortcuts.
    pub meta_key: MetaKey,
    /// When `true`, showing any Context immediately hides all other visible Contexts.
    pub single_context_mode: bool,
    /// The Context forced to be the single visible one when Single Context Mode is
    /// enabled (and each time the choice changes while enabled). Holds the chosen
    /// Context's `id`; `None` — or a stale id whose Context no longer exists —
    /// resolves to the Main Context. Only meaningful while `single_context_mode`.
    #[serde(default)]
    pub single_context_id: Option<String>,
    /// Whether the application should start automatically at user login.
    pub launch_at_login: bool,
}

/// The full persisted application state: all Contexts and user settings.
///
/// `AppData` is serialized to `data.json` in the platform app-data directory
/// and deserialized on startup. Any field missing from the JSON (e.g. after
/// an upgrade) falls back to `serde` defaults.
///
/// # Invariants
/// - `contexts` is never empty; it always contains at least the Main Context.
/// - Exactly one element of `contexts` has `is_main == true`.
/// - Window membership is ephemeral: on startup the poll reconciles live
///   windows against the loaded state, removing any stale `WindowRef` entries.
///
/// # Examples
/// ```no_run
/// let data = AppData::default();
/// assert_eq!(data.contexts.len(), 1);
/// assert!(data.contexts[0].is_main);
/// assert_eq!(data.contexts[0].shortcut_index, Some(0));
/// ```
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppData {
    /// Ordered list of Contexts. Main is always first (`index 0`).
    pub contexts: Vec<Context>,
    /// User-configurable settings.
    pub settings: Settings,
}

impl AppData {
    /// Rewrites every Context's `order` to a dense, globally-unique `0..n`
    /// ranking derived from a stable sort on the current `order` values.
    ///
    /// This does **not** move elements within `contexts`; it only reassigns the
    /// `order` field. Because the sort is stable, ties keep their existing array
    /// order — so state persisted before `order` existed (every value defaulting
    /// to `0`) is renumbered by array position, exactly preserving the previous
    /// sidebar order. Once orders are already distinct the operation is a no-op
    /// beyond re-densifying any gaps, making it idempotent and safe to run on
    /// every load.
    pub fn normalize_order(&mut self) {
        let mut idx: Vec<usize> = (0..self.contexts.len()).collect();
        idx.sort_by_key(|&i| self.contexts[i].order);
        for (rank, i) in idx.into_iter().enumerate() {
            self.contexts[i].order = rank as u32;
        }
    }

    /// Returns the smallest `order` value strictly greater than every Context's,
    /// i.e. the value to give a Context that should sort to the end of the
    /// unassigned tier. Assumes [`normalize_order`](Self::normalize_order) keeps
    /// orders dense, so this equals `contexts.len()`.
    pub fn next_order(&self) -> u32 {
        self.contexts.iter().map(|c| c.order).max().map_or(0, |m| m + 1)
    }
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            contexts: vec![Context {
                id: uuid::Uuid::new_v4().to_string(),
                name: "main".to_string(),
                is_main: true,
                windows: vec![],
                shortcut_index: Some(0),
                order: 0,
                visible: true,
            }],
            settings: Settings {
                meta_key: MetaKey::CtrlAlt,
                single_context_mode: false,
                single_context_id: None,
                launch_at_login: false,
            },
        }
    }
}

/// Runtime application state registered with Tauri's managed-state system.
///
/// `data` is the authoritative in-memory state. Any mutation must be followed
/// by a send on `save_tx` to trigger the debounced persistence worker.
///
/// # Invariants
/// - `data` must never be held locked across an `.await` point; doing so will
///   deadlock the async runtime.
/// - `save_tx`'s corresponding receiver is owned by the saver task spawned in
///   `persistence::spawn_saver` and lives for the duration of the application.
pub struct AppState {
    /// Mutex-protected application data; the single source of truth at runtime.
    pub data: Mutex<AppData>,
    /// Channel used to signal the persistence worker that `data` has changed.
    pub save_tx: watch::Sender<AppData>,
}
