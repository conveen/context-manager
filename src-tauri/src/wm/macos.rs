use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};

use super::WindowInfo;
use crate::state::WindowRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
}

const LIST_ON_SCREEN_ONLY: u32 = 1 << 0;
const NULL_WINDOW_ID: u32 = 0;
const NORMAL_WINDOW_LAYER: i32 = 0;

/// Extracts a `String` value from a CGWindowList entry dictionary by key.
///
/// Returns `None` if the key is absent or if the stored value is not a
/// `CFString` (e.g. a `CFNull` placeholder, which macOS uses for
/// `kCGWindowName` when Screen Recording permission is not granted).
///
/// # Arguments
/// - `dict`: A window-info dictionary produced by `CGWindowListCopyWindowInfo`.
/// - `key`: The CGWindow dictionary key name (e.g. `"kCGWindowOwnerName"`).
///
/// # Preconditions/Assumptions
/// - `dict` originates from `CGWindowListCopyWindowInfo`; the type cast inside
///   is sound for that specific call site.
fn dict_string(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<String> {
    let k = CFString::new(key);
    dict.find(&k).and_then(|v| {
        if v.type_of() == CFString::type_id() {
            Some(unsafe { CFString::wrap_under_get_rule(v.as_CFTypeRef() as _) }.to_string())
        } else {
            None
        }
    })
}

/// Extracts an `i32` value from a CGWindowList entry dictionary by key.
///
/// Returns `None` if the key is absent, if the stored value is not a
/// `CFNumber`, or if the number cannot be represented as `i32`.
///
/// # Arguments
/// - `dict`: A window-info dictionary produced by `CGWindowListCopyWindowInfo`.
/// - `key`: The CGWindow dictionary key name (e.g. `"kCGWindowLayer"`).
///
/// # Preconditions/Assumptions
/// - `dict` originates from `CGWindowListCopyWindowInfo`; the type cast inside
///   is sound for that specific call site.
fn dict_i32(dict: &CFDictionary<CFString, CFType>, key: &str) -> Option<i32> {
    let k = CFString::new(key);
    dict.find(&k).and_then(|v| {
        if v.type_of() == CFNumber::type_id() {
            unsafe { CFNumber::wrap_under_get_rule(v.as_CFTypeRef() as _) }.to_i32()
        } else {
            None
        }
    })
}

/// macOS implementation of window enumeration using `CGWindowListCopyWindowInfo`.
///
/// Queries CoreGraphics for all on-screen windows and filters to those that are:
/// - At window layer 0 (normal application windows; excludes menu bar,
///   overlays, and desktop elements).
/// - Not owned by this process.
/// - Have a non-empty `kCGWindowName` (title).
///
/// # Arguments
/// - `our_pid`: Process ID of the running application; windows owned by this
///   PID are excluded from the result.
///
/// # Preconditions/Assumptions
/// - On macOS 10.15 (Catalina) and later, `kCGWindowName` is only populated
///   for windows of other processes when Screen Recording permission has been
///   granted. Without it, those entries are silently skipped.
/// - `CGWindowListCopyWindowInfo` returns a create-rule `CFArrayRef`; we take
///   ownership via `CFArray::wrap_under_create_rule`.
///
/// # Invariants
/// - Every returned `WindowInfo` has a non-empty `window_title`.
/// - Every returned `WindowInfo` has `pid != our_pid`.
/// - `platform_id` corresponds to the `CGWindowID` (`kCGWindowNumber`), which
///   is stable for the lifetime of the window.
pub fn enumerate(our_pid: u32) -> Vec<WindowInfo> {
    let raw = unsafe { CGWindowListCopyWindowInfo(LIST_ON_SCREEN_ONLY, NULL_WINDOW_ID) };
    if raw.is_null() {
        return vec![];
    }

    let arr: CFArray<CFDictionary<CFString, CFType>> = unsafe { CFArray::wrap_under_create_rule(raw) };

    let mut windows = Vec::new();

    for dict in arr.iter() {
        // Normal app windows sit at layer 0
        if dict_i32(&dict, "kCGWindowLayer") != Some(NORMAL_WINDOW_LAYER) {
            continue;
        }

        let pid = match dict_i32(&dict, "kCGWindowOwnerPID") {
            Some(p) => p as u32,
            None => continue,
        };
        if pid == our_pid {
            continue;
        }

        let platform_id = match dict_i32(&dict, "kCGWindowNumber") {
            Some(id) => id as u64,
            None => continue,
        };

        let app_name = dict_string(&dict, "kCGWindowOwnerName").unwrap_or_default();

        // kCGWindowName is null for windows of other processes without Screen
        // Recording permission (macOS 10.15+). Skip windowless entries rather
        // than showing an unlabelled card.
        let window_title = match dict_string(&dict, "kCGWindowName") {
            Some(t) if !t.is_empty() => t,
            _ => continue,
        };

        windows.push(WindowInfo { platform_id, pid, app_name, window_title });
    }

    windows
}

// ---------------------------------------------------------------------------
// Hide / show via the macOS Accessibility API
// ---------------------------------------------------------------------------

// Bindings to the AX functions we need from `ApplicationServices.framework`.
// All AX object types (`AXUIElementRef`, `AXValueRef`) are `CFTypeRef` aliases
// at the C level, so we use `CFTypeRef` (`*const c_void`) throughout to avoid
// defining additional opaque wrapper types.
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// Creates an `AXUIElement` representing the application with the given PID.
    /// Returns null if the PID is invalid. The caller owns the returned object
    /// (create rule).
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;

    /// Copies the value of an accessibility attribute. Returns an `AXError`
    /// integer; 0 (`kAXErrorSuccess`) on success. The value written to `*value`
    /// follows the create rule (caller owns it). `attribute` is a `CFStringRef`.
    fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;

    /// Sets the value of an accessibility attribute. Returns an `AXError`
    /// integer. `attribute` is a `CFStringRef`; `value` is a `CFTypeRef`.
    fn AXUIElementSetAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: CFTypeRef) -> i32;

    /// Performs an accessibility action (e.g. `AXRaise`) on an element. Returns
    /// an `AXError` integer. `action` is a `CFStringRef`.
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFTypeRef) -> i32;
}

/// Returns the `AXUIElement` for the first window whose `AXTitle` matches
/// `title` in the application with the given `pid`.
///
/// Enumerates the application's `AXWindows` attribute and compares each
/// window's `AXTitle` against `title`. Returns `None` if the process cannot
/// be accessed (Accessibility permission not granted), has no windows, or no
/// title matches.
///
/// The returned `CFType` owns one Accessibility retain on the element; it is
/// released when dropped.
///
/// # Limitations
/// Title matching is exact and case-sensitive. If the window title has changed
/// since the `WindowRef` was recorded, the lookup will fail. This is a known
/// limitation to be addressed in a later milestone.
///
/// # Safety
/// Calls into the macOS Accessibility C API.
unsafe fn find_ax_window(pid: u32, title: &str) -> Option<CFType> {
    let app_raw = AXUIElementCreateApplication(pid as i32);
    if app_raw.is_null() {
        return None;
    }
    // wrap_under_create_rule: we own this reference.
    let app_el = CFType::wrap_under_create_rule(app_raw);

    let attr_windows = CFString::new("AXWindows");
    let mut windows_raw: CFTypeRef = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(app_el.as_CFTypeRef(), attr_windows.as_CFTypeRef(), &mut windows_raw);
    drop(app_el);

    if err != 0 || windows_raw.is_null() {
        return None;
    }

    // The returned value is a CFArray (create rule — we own it).
    let windows_arr: CFArray<CFType> = CFArray::wrap_under_create_rule(windows_raw as _);
    let attr_title = CFString::new("AXTitle");

    for win_cftype in windows_arr.iter() {
        let mut title_raw: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(win_cftype.as_CFTypeRef(), attr_title.as_CFTypeRef(), &mut title_raw);
        if err != 0 || title_raw.is_null() {
            continue;
        }
        // wrap_under_create_rule: we own the returned CFString.
        let win_title = CFString::wrap_under_create_rule(title_raw as _).to_string();
        if win_title == title {
            // ItemRef borrows from the array; wrap_under_get_rule adds a
            // CFRetain so the returned CFType remains valid after the array
            // is released.
            return Some(CFType::wrap_under_get_rule((*win_cftype).as_CFTypeRef()));
        }
    }

    None
}

/// macOS implementation of `wm::hide_window`.
///
/// Minimizes the window by setting its `AXMinimized` attribute to `true` and
/// marks it hidden. Minimizing genuinely removes the window from the screen,
/// unlike moving it offscreen (which macOS clamps so an edge stays visible),
/// and un-minimizing restores its geometry natively — so no position needs to
/// be captured here.
///
/// # Errors
/// - Window not found via the Accessibility API (wrong PID/title, or
///   Accessibility permission not granted).
/// - `AXUIElementSetAttributeValue(AXMinimized)` fails (e.g. a window that
///   cannot be minimized, is fullscreen, or is a system window). The hidden
///   marker is reverted so a retry is possible.
pub fn hide_window(window: &mut WindowRef) -> Result<(), String> {
    unsafe {
        let ax_win = find_ax_window(window.pid, &window.window_title).ok_or_else(|| {
            format!(
                "window '{}' (pid {}) not found via Accessibility API — \
                 ensure Accessibility permission is granted in System Settings",
                window.window_title, window.pid
            )
        })?;

        // Mark hidden so the visibility logic and show_window treat it as such.
        window.hidden = true;

        // Minimize (AXMinimized = true).
        let attr_min = CFString::new("AXMinimized");
        let err = AXUIElementSetAttributeValue(
            ax_win.as_CFTypeRef(),
            attr_min.as_CFTypeRef(),
            CFBoolean::true_value().as_CFTypeRef(),
        );
        if err != 0 {
            window.hidden = false;
            return Err(format!(
                "AXUIElementSetAttributeValue(AXMinimized=true) failed with AXError {err} — \
                 window may not support minimizing, be fullscreen, or be a system window"
            ));
        }

        Ok(())
    }
}

/// macOS implementation of `wm::show_window`.
///
/// If `window.hidden` is `false` the window is already visible; returns
/// `Ok(())` immediately. Otherwise un-minimizes the window by setting
/// `AXMinimized` to `false` (which restores its previous position and size)
/// and clears the hidden marker.
///
/// # Errors
/// - Window not found via the Accessibility API.
/// - `AXUIElementSetAttributeValue(AXMinimized)` fails.
pub fn show_window(window: &mut WindowRef) -> Result<(), String> {
    if !window.hidden {
        return Ok(()); // already visible
    }

    unsafe {
        let ax_win = find_ax_window(window.pid, &window.window_title).ok_or_else(|| {
            format!("window '{}' (pid {}) not found via Accessibility API", window.window_title, window.pid)
        })?;

        let attr_min = CFString::new("AXMinimized");
        let err = AXUIElementSetAttributeValue(
            ax_win.as_CFTypeRef(),
            attr_min.as_CFTypeRef(),
            CFBoolean::false_value().as_CFTypeRef(),
        );
        if err != 0 {
            return Err(format!("AXUIElementSetAttributeValue(AXMinimized=false) failed with AXError {err}"));
        }

        // Clear only after the OS call succeeds so a retry is possible.
        window.hidden = false;

        Ok(())
    }
}

/// macOS implementation of `wm::raise_window`.
///
/// Brings the window's owning application to the front (`AXFrontmost = true`)
/// and raises the window to the top within that application (`AXRaise`),
/// making it the frontmost window on screen. Called after un-minimizing a
/// Context's windows to restore the window that was on top before hiding.
///
/// # Errors
/// - Window not found via the Accessibility API.
/// - The `AXRaise` action fails.
pub fn raise_window(window: &WindowRef) -> Result<(), String> {
    unsafe {
        let ax_win = find_ax_window(window.pid, &window.window_title).ok_or_else(|| {
            format!("window '{}' (pid {}) not found via Accessibility API", window.window_title, window.pid)
        })?;

        // Activate the owning application so its windows can come to the front.
        let app_raw = AXUIElementCreateApplication(window.pid as i32);
        if !app_raw.is_null() {
            let app_el = CFType::wrap_under_create_rule(app_raw);
            let attr_frontmost = CFString::new("AXFrontmost");
            let _ = AXUIElementSetAttributeValue(
                app_el.as_CFTypeRef(),
                attr_frontmost.as_CFTypeRef(),
                CFBoolean::true_value().as_CFTypeRef(),
            );
        }

        // Raise the window to the top within its application.
        let action_raise = CFString::new("AXRaise");
        let err = AXUIElementPerformAction(ax_win.as_CFTypeRef(), action_raise.as_CFTypeRef());
        if err != 0 {
            return Err(format!("AXUIElementPerformAction(AXRaise) failed with AXError {err}"));
        }

        Ok(())
    }
}
