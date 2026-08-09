//! Commands backing the permission banner: reading the OS permission state
//! that window enumeration depends on, and sending the user where they can
//! change it.

use tauri::Manager;

use crate::state::{AppState, ScreenRecordingStatus};

/// macOS deep link to System Settings > Privacy & Security > Screen Recording.
#[cfg(target_os = "macos")]
const SCREEN_RECORDING_PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Returns whether the OS is currently letting us read window titles.
///
/// The value is maintained by the background window poll (see
/// [`crate::wm::update_windows`]) rather than probed here, so polling this
/// command costs nothing beyond a lock. Off macOS it is always
/// [`ScreenRecordingStatus::Granted`].
#[tauri::command]
pub fn get_screen_recording_status(app: tauri::AppHandle) -> ScreenRecordingStatus {
    *app.state::<AppState>().screen_recording.lock().unwrap()
}

/// Opens the OS settings pane where Screen Recording permission is granted.
///
/// Uses `open(1)` with the System Settings deep link rather than a shell/opener
/// plugin, which the app does not otherwise depend on. The child is not waited
/// on: `open` hands off to System Settings and exits immediately.
///
/// # Errors
/// - Returns `Err` on any platform other than macOS, which has no equivalent
///   pane (the banner that invokes this is never shown there).
/// - Returns `Err` if `open` cannot be spawned.
#[tauri::command]
pub fn open_screen_recording_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(SCREEN_RECORDING_PANE)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("failed to open System Settings: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Screen Recording permission is a macOS-only concept".to_string())
    }
}
