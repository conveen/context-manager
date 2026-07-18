use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, ShowWindow,
    GWL_STYLE, SW_HIDE, SW_SHOW, WS_CAPTION,
};

use super::WindowInfo;
use crate::state::WindowRef;

/// State threaded through the `EnumWindows` callback via `LPARAM`.
///
/// Because `EnumWindows` accepts only a single `isize`-sized parameter for
/// caller data, a raw pointer to this struct is cast to `LPARAM` and back
/// inside the callback. The struct is stack-allocated in `enumerate` and its
/// address is stable for the duration of the `EnumWindows` call.
///
/// # Invariants
/// - The pointer passed as `LPARAM` must be non-null and point to a live
///   `EnumData` for the entire duration of the `EnumWindows` call.
struct EnumData {
    /// Process ID of the calling application; windows owned by this PID are skipped.
    our_pid: u32,
    /// Accumulator for windows that pass all filters.
    windows: Vec<WindowInfo>,
}

/// `EnumWindows` callback that collects visible, titled, top-level windows.
///
/// For each HWND, the following filters are applied in order; the window is
/// skipped if any fails:
/// 1. `IsWindowVisible` — must be true.
/// 2. Non-zero title length.
/// 3. Has `WS_CAPTION` window style (title bar present).
/// 4. Owner PID differs from `our_pid`.
///
/// Windows that pass all filters are appended to `EnumData::windows`.
///
/// # Arguments
/// - `hwnd`: Handle to the current window being enumerated.
/// - `lparam`: Caller-supplied value; must be a valid pointer to an `EnumData`.
///
/// # Preconditions/Assumptions
/// - `lparam.0` must be a non-null, correctly aligned pointer to a live
///   `EnumData` instance. Violating this is undefined behaviour.
///
/// # Invariants
/// - Always returns `BOOL(1)` to continue enumeration; never aborts early.
unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let data = &mut *(lparam.0 as *mut EnumData);

    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let title_len = GetWindowTextLengthW(hwnd);
    if title_len == 0 {
        return BOOL(1);
    }

    // Require a title bar — filters tooltips, popups, and tool windows
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
    if style & WS_CAPTION.0 == 0 {
        return BOOL(1);
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == data.our_pid {
        return BOOL(1);
    }

    let mut title_buf = vec![0u16; (title_len + 1) as usize];
    GetWindowTextW(hwnd, &mut title_buf);
    let window_title = String::from_utf16_lossy(&title_buf[..title_len as usize]);

    let app_name = process_name(pid).unwrap_or_else(|| "Unknown".to_string());

    data.windows.push(WindowInfo { platform_id: hwnd.0 as u64, app_name, window_title });

    BOOL(1)
}

/// Returns the executable name (without path or extension) for a given PID.
///
/// Opens the process with `PROCESS_QUERY_LIMITED_INFORMATION` (does not
/// require elevated privileges for most user-space processes) and queries the
/// full image path, then extracts the file stem.
///
/// # Arguments
/// - `pid`: The process ID to look up.
///
/// # Preconditions/Assumptions
/// - Returns `None` for PIDs that cannot be opened (e.g. system processes,
///   protected processes, or PIDs that have exited between enumeration and
///   this call). The caller falls back to `"Unknown"`.
///
/// # Invariants
/// - The process handle is closed implicitly when it goes out of scope via the
///   `windows` crate's `Drop` implementation.
fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len)
            .ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned())
    }
}

/// Windows implementation of window enumeration using `EnumWindows`.
///
/// Passes a raw pointer to an `EnumData` accumulator through `EnumWindows` as
/// an `LPARAM`. After enumeration completes the accumulated windows are
/// returned. Any error from `EnumWindows` itself is silently ignored; partial
/// results are still returned.
///
/// # Arguments
/// - `our_pid`: Process ID of the running application; windows owned by this
///   PID are excluded by the callback.
///
/// # Preconditions/Assumptions
/// - `EnumWindows` only enumerates top-level windows; child windows (e.g.
///   controls within a dialog) are not visited.
///
/// # Invariants
/// - `platform_id` is the `HWND` value cast to `u64`. HWNDs can be reused by
///   the OS after a window is destroyed, so stored `platform_id` values should
///   be treated as valid only for the current session.
/// - Every returned `WindowInfo` has a non-empty `window_title` and
///   `pid != our_pid`.
pub fn enumerate(our_pid: u32) -> Vec<WindowInfo> {
    let mut data = EnumData { our_pid, windows: Vec::new() };
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::EnumWindows(
            Some(enum_callback),
            LPARAM(&mut data as *mut _ as isize),
        );
    }
    data.windows
}

/// Windows implementation of `wm::hide_window`.
///
/// Calls `ShowWindow(hwnd, SW_HIDE)`, which hides the window without
/// destroying it, and sets `window.original_position` to a sentinel value.
///
/// The stored coordinates are meaningless and never read on Windows —
/// `SW_SHOW` restores geometry natively — but setting the field is
/// load-bearing: `original_position` doubles as the app-wide "hidden by us"
/// marker. A hidden window fails the `IsWindowVisible` filter in `enumerate`,
/// so without the marker the background poll would remove it from every
/// Context — permanently, since a window that stays `SW_HIDE`-hidden is never
/// re-enumerated — and the show path (which only targets windows with the
/// marker set) would never un-hide it.
///
/// # Errors
/// Always returns `Ok(())`. `SW_HIDE` is a fire-and-forget call; if the HWND
/// is invalid or the window has already been destroyed the OS ignores it.
pub fn hide_window(window: &mut WindowRef) -> Result<(), String> {
    // Pure hidden marker on Windows; only its presence matters (see above).
    window.original_position = Some([0.0, 0.0]);
    unsafe {
        let _ = ShowWindow(HWND(window.platform_id as *mut _), SW_HIDE);
    }
    Ok(())
}

/// Windows implementation of `wm::show_window`.
///
/// Calls `ShowWindow(hwnd, SW_SHOW)`, which makes the window visible and
/// restores it to its last known position and size as tracked by the OS.
/// `window.original_position` is cleared on success, releasing the hidden
/// marker set by `hide_window` so the window is treated as visible again.
///
/// # Errors
/// Always returns `Ok(())` for the same reason as `hide_window`.
pub fn show_window(window: &mut WindowRef) -> Result<(), String> {
    unsafe {
        let _ = ShowWindow(HWND(window.platform_id as *mut _), SW_SHOW);
    }
    window.original_position = None;
    Ok(())
}
