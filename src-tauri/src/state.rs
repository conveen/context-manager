use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// A reference to an OS window that is tracked within one or more Contexts.
///
/// `hidden` is set while the window is hidden by us and cleared when it is
/// shown again; both platforms restore geometry natively on show, so no
/// position needs to be remembered.
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
    /// Hidden-by-us marker: `true` while the window is hidden (minimized on
    /// macOS, `SW_HIDE` on Windows), `false` while it is visible. Keeps the
    /// background poll from dropping the (no longer enumerable) window and
    /// gates the show path. Defaults to `false` for entries persisted before
    /// this field replaced the old `original_position` marker.
    #[serde(default)]
    pub hidden: bool,
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
/// The Main Context (identified by `is_main == true`) is always present and is
/// the fallback destination for newly detected windows; see
/// [`AppData::additions_target`] for the full resolution.
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
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum MetaKey {
    /// `Ctrl+Alt` — works on Windows and Linux; available on macOS. Default
    /// off Windows, where it is also the AltGr combination (see
    /// [`MetaKey::default`]).
    CtrlAlt,
    /// `Command+Option` — macOS-native feel; maps to `CommandOrControl+Alt` in Tauri.
    CmdOpt,
    /// `Ctrl+Alt+Super`, where Super is the Windows key on Windows and Command
    /// on macOS. Default on Windows: adding a third modifier takes the
    /// combination out of AltGr's range, which is what makes `Ctrl+Alt+<digit>`
    /// so contested there.
    CtrlAltSuper,
}

impl Default for MetaKey {
    /// `Ctrl+Alt+Super` on Windows, `Ctrl+Alt` everywhere else.
    ///
    /// Windows synthesises `Ctrl+Alt` from AltGr, so resident software
    /// (keyboard-layout and IME tools, vendor control panels) commonly claims
    /// `Ctrl+Alt+<digit>` — and `RegisterHotKey` refuses a combination another
    /// process already owns, leaving the shortcut dead. macOS's Carbon hotkey
    /// API has no such contention, so `Ctrl+Alt` stays the default there.
    ///
    /// Only affects fresh installs: an existing `data.json` carries whatever
    /// modifier it was last saved with.
    fn default() -> Self {
        #[cfg(target_os = "windows")]
        {
            MetaKey::CtrlAltSuper
        }
        #[cfg(not(target_os = "windows"))]
        {
            MetaKey::CtrlAlt
        }
    }
}

/// Whether the OS is currently letting us read the window list.
///
/// Runtime-only (never persisted): it describes the environment the app is
/// running in, not user data. macOS gates `kCGWindowName` behind Screen
/// Recording permission, so without it every window's title comes back as
/// `CFNull` and enumeration yields nothing — indistinguishable, from the
/// window list alone, from having no windows open. This enum is what makes the
/// two distinguishable in the UI.
///
/// # Invariants
/// - Platforms that do not gate window enumeration (Windows) always report
///   [`Granted`](Self::Granted).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
// Only the macOS backend ever constructs the two failure variants; elsewhere
// the enum is inhabited solely by `Granted`, which reads as dead code.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum ScreenRecordingStatus {
    /// Window titles are readable: the permission is in effect, or the platform
    /// does not gate them at all.
    Granted,
    /// macOS reports the permission is not granted to this process. The user
    /// has to grant it in System Settings and relaunch the app.
    Denied,
    /// macOS reports the permission *is* granted, yet titles are still
    /// unreadable. Two known causes, with the same fix (quit and relaunch):
    /// a grant does not apply to an already-running process, and for
    /// unsigned/ad-hoc-signed builds a TCC entry from an earlier build can read
    /// as granted while binding to a different code-signature identity.
    NotInEffect,
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

    /// Index in `contexts` of the only currently-visible Context, or `None` if
    /// zero or several are visible.
    ///
    /// "Exactly one Context on screen" is the signal that the user is working
    /// inside a single Context, which both the additions target below and
    /// Single Context Mode's off→on transition treat as *the* current Context.
    pub fn single_visible(&self) -> Option<usize> {
        let mut visible = self.contexts.iter().enumerate().filter(|(_, c)| c.visible);
        match (visible.next(), visible.next()) {
            (Some((i, _)), None) => Some(i),
            _ => None,
        }
    }

    /// Index in `contexts` of the Context that newly detected windows should be
    /// added to.
    ///
    /// Newly detected windows belong to whatever Context the user is currently
    /// working in. That is only unambiguous when a single Context is on screen:
    /// exactly one visible Context wins outright; failing that, Single Context
    /// Mode's configured Context is used (nothing may be visible, but the mode
    /// still pins which Context is current). Everything else falls back to Main,
    /// the catch-all pool.
    ///
    /// The visible-Context rule deliberately outranks the Single Context Mode
    /// rule so that toggling Contexts by hotkey tracks what is actually on
    /// screen rather than the Settings dropdown's choice, which only pins the
    /// Context at the moment the mode (or the choice) changes.
    ///
    /// # Invariants
    /// - Always returns a valid index (Main always exists).
    pub fn additions_target(&self) -> usize {
        if let Some(i) = self.single_visible() {
            return i;
        }
        if self.settings.single_context_mode {
            if let Some(i) =
                self.settings.single_context_id.as_deref().and_then(|id| self.contexts.iter().position(|c| c.id == id))
            {
                return i;
            }
        }
        self.contexts.iter().position(|c| c.is_main).expect("Main Context always exists")
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
            settings: Settings { meta_key: MetaKey::default(), single_context_mode: false, single_context_id: None },
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
/// - `screen_recording` is never locked while `data` is held (and vice versa),
///   so the two mutexes cannot deadlock against each other.
pub struct AppState {
    /// Mutex-protected application data; the single source of truth at runtime.
    pub data: Mutex<AppData>,
    /// Channel used to signal the persistence worker that `data` has changed.
    pub save_tx: watch::Sender<AppData>,
    /// Whether window titles are currently readable. Refreshed by the
    /// background window poll and read by the `get_screen_recording_status`
    /// command. Deliberately outside `data`: it is derived from the OS at
    /// runtime and must not be written to `data.json`.
    pub screen_recording: Mutex<ScreenRecordingStatus>,
    /// Accelerators the OS refused at the last (re)registration, e.g.
    /// `"Ctrl+Alt+3"` — empty when every shortcut is live. Rewritten in full by
    /// [`crate::hotkeys::register_all`] and read by the `get_failed_shortcuts`
    /// command, which is how the frontend can report dead shortcuts that were
    /// registered before it finished loading. Runtime-only, like
    /// `screen_recording`.
    pub failed_shortcuts: Mutex<Vec<String>>,
}
