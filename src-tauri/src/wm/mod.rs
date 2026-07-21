use std::collections::{HashMap, HashSet};

use tauri::Manager;
use tokio::time::Duration;

use crate::state::{AppState, WindowRef};

// In test builds the platform modules are not compiled at all; every entry
// point below routes to the scripted [`mock`] instead. The real OS calls need
// a live desktop session (and, on macOS, Accessibility + Screen Recording
// permissions) and are exercised only via the manual checklist.
#[cfg(all(target_os = "macos", not(test)))]
mod macos;
#[cfg(test)]
pub mod mock;
#[cfg(all(target_os = "windows", not(test)))]
mod win32;

/// macOS-only: raise a window to the front, restoring it as the top window.
/// Re-exported directly rather than wrapped in a cross-platform dispatcher
/// (like `hide_window`/`show_window`) because it has no meaningful behavior on
/// other platforms. Used after un-minimizing a Context's windows to reinstate
/// the window that was frontmost before the Context was hidden.
#[cfg(all(target_os = "macos", not(test)))]
pub use macos::raise_window;
#[cfg(all(target_os = "macos", test))]
pub use mock::raise_window;

/// A snapshot of a single live OS window captured during enumeration.
///
/// `WindowInfo` is a transient value produced by `enumerate` and consumed
/// by `update_windows`; it is never persisted. The persisted counterpart is
/// `state::WindowRef`, which is created from a `WindowInfo` when a window is
/// first seen.
#[derive(Debug)]
pub struct WindowInfo {
    /// Stable OS-assigned identifier: CGWindowID (macOS) or HWND value (Windows).
    pub platform_id: u64,
    /// OS process ID of the owning application. Present only on macOS, where
    /// it is required to look up the `AXUIElement` for hide/show/raise
    /// (mirrors `state::WindowRef::pid`). On Windows nothing reads it back
    /// off the struct, so it stays a local variable in `win32::enumerate`.
    #[cfg(target_os = "macos")]
    pub pid: u32,
    /// Display name of the owning application (e.g. "Safari", "Slack").
    pub app_name: String,
    /// Current window title at the time of enumeration.
    pub window_title: String,
}

/// Returns all visible, user-facing windows currently open on the system,
/// excluding windows owned by this process.
///
/// Delegates to the platform-specific implementation. On unsupported platforms
/// an empty list is returned.
///
/// # Arguments
/// - `our_pid`: Process ID of the running application, used to exclude our
///   own windows from the result.
///
/// # Preconditions/Assumptions
/// - On macOS 10.15+, Screen Recording permission is required to obtain window
///   titles. Without it, windows without titles are silently omitted.
/// - On Windows, windows without `WS_CAPTION` (no title bar) are excluded.
///
/// # Invariants
/// - Every returned `WindowInfo` has a non-empty `window_title`.
/// - No returned `WindowInfo` is owned by the process identified by `our_pid`.
pub fn enumerate(our_pid: u32) -> Vec<WindowInfo> {
    #[cfg(test)]
    {
        mock::enumerate(our_pid)
    }
    #[cfg(not(test))]
    {
        #[cfg(target_os = "macos")]
        {
            macos::enumerate(our_pid)
        }
        #[cfg(target_os = "windows")]
        {
            win32::enumerate(our_pid)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = our_pid;
            vec![]
        }
    }
}

/// Reconciles the live set of OS windows against `AppState`.
///
/// Three mutations are applied atomically under the `AppState` data lock:
/// 1. **Refresh**: for every tracked `WindowRef` that is still present in the
///    live enumeration, its `window_title` and `app_name` are updated to the
///    current values. Window titles change over a window's lifetime (e.g.
///    KeePassXC appends its database/lock state), and the macOS hide/show path
///    looks a window up by its *current* `AXTitle` — a stale stored title makes
///    that lookup fail, so the window silently refuses to hide. Hidden windows
///    (absent from the enumeration) keep the title captured when they were hidden.
/// 2. **Removals**: any `WindowRef` whose `platform_id` is no longer present
///    in the live window list is removed from every Context it belongs to,
///    *unless* it is currently hidden by us (`hidden`) — a hidden window is
///    minimized and thus absent from the on-screen enumeration, but still
///    exists and must remain tracked.
/// 3. **Additions**: any live window whose `platform_id` is not tracked in any
///    Context is added to the Main Context as a new `WindowRef`.
///
/// After reconciliation, the updated `AppData` is sent to the persistence
/// worker via the save channel — but only if one of the three mutations
/// actually changed something. A quiet tick (stable window set, unchanged
/// titles) must not signal the channel: `watch::Sender::send` marks the
/// channel changed unconditionally, so an unconditional send would wake the
/// debounced saver and rewrite `data.json` every ~2s for the lifetime of the
/// application.
///
/// # Arguments
/// - `app`: Handle to the running Tauri application; used to access `AppState`.
///
/// # Preconditions/Assumptions
/// - `AppState` must be registered with `app.manage(...)` before calling.
/// - The live window enumeration is performed *before* acquiring the lock to
///   minimise lock-hold time.
///
/// # Invariants
/// - The Main Context always exists in `AppData::contexts` (panics otherwise).
/// - The `AppState` data lock is not held across any async `.await` point.
///
/// On macOS 10.15+, window enumeration requires Screen Recording permission
/// (System Preferences > Security & Privacy > Screen Recording). Without it,
/// application windows will not appear in the available windows list.
pub fn update_windows<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let our_pid = std::process::id();
    let current = enumerate(our_pid);

    let current_ids: HashSet<u64> = current.iter().map(|w| w.platform_id).collect();

    let state = app.state::<AppState>();
    let mut data = state.data.lock().unwrap();

    // IDs tracked across all contexts before this update
    let known_ids: HashSet<u64> = data.contexts.iter().flat_map(|c| c.windows.iter().map(|w| w.platform_id)).collect();

    // Whether this tick mutated anything; gates the save-channel send below.
    let mut changed = false;

    // Refresh the title/app-name of tracked windows that are still live. A
    // window's title can change after it is added to a Context; the macOS
    // hide/show path resolves the OS window by its current `AXTitle`, so a stale
    // stored title would make hide (and show) silently fail to find the window.
    // Windows absent from `current` (i.e. hidden/minimized by us) are left as-is.
    let current_by_id: HashMap<u64, &WindowInfo> = current.iter().map(|w| (w.platform_id, w)).collect();
    for ctx in &mut data.contexts {
        for w in &mut ctx.windows {
            if let Some(info) = current_by_id.get(&w.platform_id) {
                if w.window_title != info.window_title {
                    w.window_title = info.window_title.clone();
                    changed = true;
                }
                if w.app_name != info.app_name {
                    w.app_name = info.app_name.clone();
                    changed = true;
                }
            }
        }
    }

    // Remove closed windows from every context. Windows we have intentionally
    // hidden are exempt: a hidden window is minimized and therefore absent
    // from the on-screen enumeration, but it still exists and must stay
    // tracked so it can be shown again.
    for ctx in &mut data.contexts {
        let before = ctx.windows.len();
        ctx.windows.retain(|w| current_ids.contains(&w.platform_id) || w.hidden);
        changed |= ctx.windows.len() != before;
    }

    // Add windows not yet tracked in any context to Main
    let main_ctx = data.contexts.iter_mut().find(|c| c.is_main).unwrap();
    for w in current.iter().filter(|w| !known_ids.contains(&w.platform_id)) {
        main_ctx.windows.push(WindowRef {
            platform_id: w.platform_id,
            #[cfg(target_os = "macos")]
            pid: w.pid,
            app_name: w.app_name.clone(),
            window_title: w.window_title.clone(),
            hidden: false,
            #[cfg(target_os = "macos")]
            hidden_z: None,
        });
        changed = true;
    }

    if changed {
        let _ = state.save_tx.send(data.clone());
    }
}

/// Hides the given window by minimizing it (macOS) or calling
/// `ShowWindow(SW_HIDE)` (Windows).
///
/// On macOS the window is minimized via the Accessibility API
/// (`AXMinimized = true`); un-minimizing restores its position and size. On
/// Windows, `SW_HIDE` preserves the window's position internally. Both set
/// `window.hidden` — the marker that keeps the background poll from dropping
/// the (no longer enumerable) window and lets the show path find it.
///
/// # Arguments
/// - `window`: Mutable reference to the tracked window. On macOS, `pid` must
///   be non-zero and `window_title` must match the current AX title exactly.
///
/// # Errors
/// Returns an `Err` string if the window cannot be found or the OS call fails
/// (e.g. Accessibility permission not granted on macOS).
pub fn hide_window(window: &mut WindowRef) -> Result<(), String> {
    #[cfg(test)]
    {
        mock::hide_window(window)
    }
    #[cfg(not(test))]
    {
        #[cfg(target_os = "macos")]
        {
            macos::hide_window(window)
        }
        #[cfg(target_os = "windows")]
        {
            win32::hide_window(window)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = window;
            Err("hide_window is not supported on this platform".to_string())
        }
    }
}

/// Shows the given window by un-minimizing it (macOS) or calling
/// `ShowWindow(SW_SHOW)` (Windows).
///
/// On macOS, the window is un-minimized (`AXMinimized = false`, which restores
/// its previous position and size) and the `hidden` marker is cleared on
/// success; if the marker is already clear the window is assumed visible and
/// `Ok(())` is returned immediately. On Windows, `SW_SHOW` restores the window
/// to its last known position and the marker is cleared unconditionally.
///
/// # Arguments
/// - `window`: Mutable reference to the tracked window.
///
/// # Errors
/// Returns an `Err` string if the window cannot be found or the OS call fails.
pub fn show_window(window: &mut WindowRef) -> Result<(), String> {
    #[cfg(test)]
    {
        mock::show_window(window)
    }
    #[cfg(not(test))]
    {
        #[cfg(target_os = "macos")]
        {
            macos::show_window(window)
        }
        #[cfg(target_os = "windows")]
        {
            win32::show_window(window)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = window;
            Err("show_window is not supported on this platform".to_string())
        }
    }
}

/// Spawns a background task that calls `update_windows` on a fixed interval.
///
/// The first reconciliation runs immediately on spawn (before the first sleep),
/// ensuring the application state reflects the current window set as soon as
/// possible after startup.
///
/// # Arguments
/// - `app`: Handle to the running Tauri application, passed through to
///   `update_windows` on each tick.
///
/// # Preconditions/Assumptions
/// - Must be called after `AppState` is registered via `app.manage(...)`.
///
/// # Invariants
/// - Runs indefinitely for the lifetime of the application.
/// - Each tick calls `update_windows` synchronously before sleeping, so ticks
///   do not overlap even if `update_windows` takes longer than the interval.
pub fn start_poll<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            update_windows(&app);
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}
