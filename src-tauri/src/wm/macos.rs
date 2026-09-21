use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};

use super::WindowInfo;
use crate::state::{ScreenRecordingStatus, WindowRef};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;

    /// Reports whether the current process has Screen Recording permission,
    /// **without** prompting. macOS 10.15+.
    fn CGPreflightScreenCaptureAccess() -> bool;

    /// Reports whether the current process has Screen Recording permission,
    /// displaying the system prompt the first time it is refused. Subsequent
    /// calls return immediately without a prompt. macOS 10.15+.
    fn CGRequestScreenCaptureAccess() -> bool;
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

/// Classifies *why* [`enumerate`] came back empty: no windows are open, or
/// their titles are unreadable because Screen Recording permission is not in
/// effect.
///
/// Two signals, because neither is sufficient alone:
/// 1. `CGPreflightScreenCaptureAccess` — authoritative for the outright
///    "not granted" case, and non-prompting, so it is safe to call on the poll.
/// 2. A pass over the raw window list looking for a window we would otherwise
///    have returned but whose `kCGWindowName` is missing. Preflight can report
///    granted while the grant is not actually applied to this process (a fresh
///    grant needs a relaunch; an ad-hoc-signed build can inherit a stale TCC
///    entry), and only the titles reveal that.
///
/// # Arguments
/// - `our_pid`: Process ID of the running application, matching [`enumerate`]'s
///   argument — our own windows are ignored, since their titles are readable
///   regardless of the permission.
///
/// # Preconditions/Assumptions
/// - Intended to be called only when [`enumerate`] returned *no* windows.
///   A single unreadable title is then evidence of the permission problem; when
///   readable windows exist, an untitled window among them is ordinary and
///   would make signal 2 a false positive.
pub fn screen_recording_status(our_pid: u32) -> ScreenRecordingStatus {
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        return ScreenRecordingStatus::Denied;
    }

    let raw = unsafe { CGWindowListCopyWindowInfo(LIST_ON_SCREEN_ONLY, NULL_WINDOW_ID) };
    if raw.is_null() {
        return ScreenRecordingStatus::Granted;
    }
    let arr: CFArray<CFDictionary<CFString, CFType>> = unsafe { CFArray::wrap_under_create_rule(raw) };

    // Mirrors `enumerate`'s filter, stopping at the title check: an entry that
    // passes every other test but has no readable title is one `enumerate`
    // silently dropped.
    let suppressed = arr.iter().any(|dict| {
        dict_i32(&dict, "kCGWindowLayer") == Some(NORMAL_WINDOW_LAYER)
            && dict_i32(&dict, "kCGWindowOwnerPID").is_some_and(|p| p as u32 != our_pid)
            && dict_string(&dict, "kCGWindowName").is_none_or(|t| t.is_empty())
    });

    if suppressed {
        ScreenRecordingStatus::NotInEffect
    } else {
        ScreenRecordingStatus::Granted
    }
}

/// Asks macOS for Screen Recording permission, showing the system prompt if it
/// has never been answered for this app.
///
/// Called once at startup. Besides prompting, this registers the app in System
/// Settings > Privacy & Security > Screen Recording — an app that has never
/// requested the permission is absent from that list, so the banner's "Open
/// System Settings" button would otherwise send the user to a list they cannot
/// find the app in.
///
/// The return value is deliberately ignored: a fresh grant does not apply to
/// the already-running process, so it says nothing useful about *this* run.
/// [`screen_recording_status`] is the authority.
pub fn request_screen_recording_access() {
    unsafe {
        let _ = CGRequestScreenCaptureAccess();
    }
}

// ---------------------------------------------------------------------------
// Hide / show via a position move ("corner hide")
// ---------------------------------------------------------------------------
//
// Technique ported from the AeroSpace tiling window manager's `hideInCorner`/
// `unhideFromCorner` (`Sources/AppBundle/tree/MacWindow.swift`) and its
// multi-monitor "optimal corner" selection (`layoutWorkspaces`,
// `Sources/AppBundle/layout/refresh.swift`), upstream commit
// `0431b6b4cfe8ec9afa6cac72f08777b667f00efc`:
// https://raw.githubusercontent.com/nikitabobko/AeroSpace/0431b6b4cfe8ec9afa6cac72f08777b667f00efc/Sources/AppBundle/tree/MacWindow.swift
// https://raw.githubusercontent.com/nikitabobko/AeroSpace/0431b6b4cfe8ec9afa6cac72f08777b667f00efc/Sources/AppBundle/layout/refresh.swift
// simplified for this project's needs (see the doc comments on
// `hide_window`/`show_window`/`hide_target` below for the full rationale and
// trade-offs).

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

    /// Reads the boxed value out of an `AXValueRef` (e.g. a `kAXValueCGPointType`
    /// box) into `value_ptr`. Returns nonzero on success. `the_type` is an
    /// `AXValueType` (a C enum, passed as `u32`); `value` is an `AXValueRef`.
    ///
    /// Bound as `u8`, not `bool`: the real signature returns Apple's
    /// `Boolean`, which is C99 `unsigned char`, not `_Bool`. Rust's `bool` has
    /// exactly two valid bit patterns (0/1); receiving anything else from C
    /// would be undefined behaviour, so callers compare the `u8` against `0`
    /// instead of trusting it to already be a valid `bool`. (This is distinct
    /// from `CGPreflightScreenCaptureAccess`/`CGRequestScreenCaptureAccess`
    /// above, which are legitimately declared `bool` in the CoreGraphics
    /// headers.)
    fn AXValueGetValue(value: CFTypeRef, the_type: u32, value_ptr: *mut std::ffi::c_void) -> u8;

    /// Boxes a value (e.g. a `CGPoint`) into a new `AXValueRef` of the given
    /// `AXValueType`. The caller owns the returned object (create rule).
    fn AXValueCreate(the_type: u32, value_ptr: *const std::ffi::c_void) -> CFTypeRef;
}

/// `AXValueType` for a boxed `CGPoint`, from `ApplicationServices/HIServices/AXValue.h`.
/// Used with [`AXValueGetValue`]/[`AXValueCreate`] to marshal `AXPosition`.
const K_AX_VALUE_CG_POINT_TYPE: u32 = 1;

/// `AXValueType` for a boxed `CGSize`, from `ApplicationServices/HIServices/AXValue.h`.
/// Used with [`AXValueGetValue`] to marshal `AXSize`.
const K_AX_VALUE_CG_SIZE_TYPE: u32 = 2;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// Returns the `CGDirectDisplayID` of the main (primary) display — the one
    /// holding the menu bar. The window-hiding corner is always one of this
    /// display's two bottom corners; [`choose_hide_corner`] picks between them
    /// by checking the other active displays (see [`hide_target`]).
    fn CGMainDisplayID() -> u32;

    /// Returns the bounds (in global screen coordinates) of the given display.
    fn CGDisplayBounds(display: u32) -> CGRect;

    /// Fills `active_displays` (capacity `max_displays`) with the
    /// `CGDirectDisplayID` of every active display and writes the count found
    /// to `*display_count` (always `<= max_displays`). Returns a `CGError`
    /// integer; 0 (`kCGErrorSuccess`) on success.
    fn CGGetActiveDisplayList(max_displays: u32, active_displays: *mut u32, display_count: *mut u32) -> i32;
}

/// Mirrors Apple's `CGPoint` (`ApplicationServices`/`CoreGraphics`): two `f64`s.
/// `#[repr(C)]` makes this layout-compatible with the real struct for FFI.
#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}

/// Mirrors Apple's `CGSize`: two `f64`s.
#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}

/// Mirrors Apple's `CGRect`: an origin `CGPoint` and a `CGSize`.
#[repr(C)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

/// Reads a window's current `AXPosition` (its top-left corner, in global
/// screen coordinates).
///
/// Returns `None` if the attribute cannot be read or is not the expected
/// `CGPoint`-boxed `AXValue` (e.g. the element does not support `AXPosition`).
///
/// # Safety
/// Calls into the macOS Accessibility C API. `ax_win` must be a live
/// `AXUIElement` (as returned by `find_ax_window`).
unsafe fn get_ax_position(ax_win: &CFType) -> Option<(f64, f64)> {
    let attr_pos = CFString::new("AXPosition");
    let mut pos_raw: CFTypeRef = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(ax_win.as_CFTypeRef(), attr_pos.as_CFTypeRef(), &mut pos_raw);
    if err != 0 || pos_raw.is_null() {
        return None;
    }
    // wrap_under_create_rule: we own the returned AXValue.
    let pos_val = CFType::wrap_under_create_rule(pos_raw);

    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let ok = AXValueGetValue(
        pos_val.as_CFTypeRef(),
        K_AX_VALUE_CG_POINT_TYPE,
        &mut point as *mut CGPoint as *mut std::ffi::c_void,
    );
    if ok == 0 {
        return None;
    }

    Some((point.x, point.y))
}

/// Sets a window's `AXPosition` (its top-left corner, in global screen
/// coordinates).
///
/// # Errors
/// Returns `Err` if the `CGPoint` cannot be boxed into an `AXValue`, or if
/// `AXUIElementSetAttributeValue` reports an `AXError`.
///
/// # Safety
/// Calls into the macOS Accessibility C API. `ax_win` must be a live
/// `AXUIElement`.
unsafe fn set_ax_position(ax_win: &CFType, x: f64, y: f64) -> Result<(), String> {
    let point = CGPoint { x, y };
    let value_raw = AXValueCreate(K_AX_VALUE_CG_POINT_TYPE, &point as *const CGPoint as *const std::ffi::c_void);
    if value_raw.is_null() {
        return Err("AXValueCreate(kAXValueCGPointType) returned null".to_string());
    }
    // wrap_under_create_rule: we own the returned AXValue.
    let value = CFType::wrap_under_create_rule(value_raw);

    let attr_pos = CFString::new("AXPosition");
    let err = AXUIElementSetAttributeValue(ax_win.as_CFTypeRef(), attr_pos.as_CFTypeRef(), value.as_CFTypeRef());
    if err != 0 {
        return Err(format!("AXUIElementSetAttributeValue(AXPosition) failed with AXError {err}"));
    }

    Ok(())
}

/// Reads a window's current `AXSize` (width and height).
///
/// Returns `None` if the attribute cannot be read or is not the expected
/// `CGSize`-boxed `AXValue`. Used by [`hide_target`] to compute the
/// bottom-left hiding corner, whose position depends on the window's width.
///
/// # Safety
/// Calls into the macOS Accessibility C API. `ax_win` must be a live
/// `AXUIElement` (as returned by `find_ax_window`).
unsafe fn get_ax_size(ax_win: &CFType) -> Option<(f64, f64)> {
    let attr_size = CFString::new("AXSize");
    let mut size_raw: CFTypeRef = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(ax_win.as_CFTypeRef(), attr_size.as_CFTypeRef(), &mut size_raw);
    if err != 0 || size_raw.is_null() {
        return None;
    }
    // wrap_under_create_rule: we own the returned AXValue.
    let size_val = CFType::wrap_under_create_rule(size_raw);

    let mut size = CGSize { width: 0.0, height: 0.0 };
    let ok = AXValueGetValue(
        size_val.as_CFTypeRef(),
        K_AX_VALUE_CG_SIZE_TYPE,
        &mut size as *mut CGSize as *mut std::ffi::c_void,
    );
    if ok == 0 {
        return None;
    }

    Some((size.width, size.height))
}

/// Which of the primary display's two bottom corners a window is hidden in.
/// See [`choose_hide_corner`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HideCorner {
    BottomLeft,
    BottomRight,
}

/// Maximum number of active displays probed by [`active_display_bounds`].
/// Generous headroom over any realistic multi-monitor setup; a system with
/// more displays than this simply has the extras ignored by the corner-choice
/// heuristic in [`choose_hide_corner`] — irrelevant in practice at this cap,
/// though in principle an ignored display could be the one that would have
/// changed the corner choice.
const MAX_DISPLAYS: usize = 16;

/// Returns the bounds (`CGDisplayBounds`) of every currently active display.
///
/// Uses a fixed-size stack array sized by [`MAX_DISPLAYS`] rather than the
/// two-call (count-then-fill) `CGGetActiveDisplayList` idiom, since that cap
/// comfortably exceeds any real setup this project needs to handle.
///
/// # Safety
/// Calls into CoreGraphics (`CGGetActiveDisplayList`/`CGDisplayBounds`), which
/// are safe to call from any thread.
unsafe fn active_display_bounds() -> Vec<CGRect> {
    let mut ids = [0u32; MAX_DISPLAYS];
    let mut count: u32 = 0;
    let err = CGGetActiveDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count);
    if err != 0 {
        return Vec::new();
    }
    let count = (count as usize).min(MAX_DISPLAYS);
    ids[..count].iter().map(|&id| CGDisplayBounds(id)).collect()
}

/// Whether `point` lies within `bounds`, matching `CGRectContainsPoint`'s
/// half-open semantics (the min edges are inside the rect, the max edges are
/// not).
fn rect_contains(bounds: &CGRect, point: (f64, f64)) -> bool {
    point.0 >= bounds.origin.x
        && point.0 < bounds.origin.x + bounds.size.width
        && point.1 >= bounds.origin.y
        && point.1 < bounds.origin.y + bounds.size.height
}

/// Picks which of the primary display's bottom corners to hide a window in,
/// so the window's body doesn't spill onto a neighbouring display arranged to
/// the right of or below the primary one.
///
/// Only the window's top-left corner is placed at the chosen point, so
/// (almost) the entire window body extends off-screen in one direction from
/// it — rightward from the bottom-right corner, leftward from the bottom-left
/// one (see [`hide_target`]). If another display happens to sit in that
/// direction, the window renders fully visible there instead of being
/// hidden — a real bug on any multi-monitor setup with a display to the
/// right of or below the primary.
///
/// Ported from AeroSpace's `layoutWorkspaces` (see the module doc comment for
/// the pinned source reference): three probe points are cast just outside
/// each candidate corner — along the bottom edge, up the side edge, and
/// diagonally past the corner — and checked against every active display's
/// bounds. The diagonal probe is weighted `IMPORTANT` (10x) since a display
/// diagonally beyond the corner is the worst case. Whichever corner's probes
/// land on fewer other displays is picked; ties (including the common case of
/// a single display, where no probe lands on anything) favour the
/// bottom-right corner, matching this project's original single-monitor
/// behaviour exactly.
///
/// # Safety
/// Calls into CoreGraphics (`CGGetActiveDisplayList`/`CGDisplayBounds`, via
/// [`active_display_bounds`]), which is safe to call from any thread.
unsafe fn choose_hide_corner(primary: &CGRect) -> HideCorner {
    let displays = active_display_bounds();

    let x_off = primary.size.width * 0.1;
    let y_off = primary.size.height * 0.1;

    let brc = (primary.origin.x + primary.size.width, primary.origin.y + primary.size.height);
    let blc = (primary.origin.x, primary.origin.y + primary.size.height);

    // Probe points just outside each candidate corner: along the bottom edge,
    // up/along the side edge, and diagonally past the corner (index 2 — the
    // worst case, weighted `IMPORTANT`x below).
    let brc_probes = [(brc.0 + 2.0, brc.1 - y_off), (brc.0 - x_off, brc.1 + 2.0), (brc.0 + 2.0, brc.1 + 2.0)];
    let blc_probes = [(blc.0 - 2.0, blc.1 - y_off), (blc.0 + x_off, blc.1 + 2.0), (blc.0 - 2.0, blc.1 + 2.0)];

    const IMPORTANT: u32 = 10;
    const WEIGHTS: [u32; 3] = [1, 1, IMPORTANT];

    let score = |probes: [(f64, f64); 3]| -> u32 {
        probes
            .iter()
            .zip(WEIGHTS)
            .map(|(&p, w)| w * displays.iter().filter(|d| rect_contains(d, p)).count() as u32)
            .sum()
    };

    if score(blc_probes) < score(brc_probes) {
        HideCorner::BottomLeft
    } else {
        HideCorner::BottomRight
    }
}

/// The point a window is moved to in order to hide it: 1px inside a corner of
/// the primary display's bounds, chosen by [`choose_hide_corner`] so the
/// window's body doesn't spill onto a neighbouring display.
///
/// Ported from AeroSpace's `hideInCorner` — see the module doc comment for
/// the pinned source reference:
/// - Bottom-right corner: `monitor.visibleRect.bottomRightCorner - CGPoint(x:
///   1, y: 1)`. As before, virtually the entire window extends past the
///   display's right and bottom edges; macOS's clamp (which keeps *some*
///   pixel of an "on-screen" window inside a display) then guarantees only a
///   1px sliver remains visible, at a location nobody looks.
/// - Bottom-left corner: `monitor.visibleRect.bottomLeftCorner + CGPoint(x:
///   1, y: -1) + CGPoint(x: -windowWidth, y: 0)` — the window's *width* is
///   subtracted so its body extends left off-screen instead, which requires
///   reading `AXSize` via `ax_win`. If that read fails, this falls back to
///   the bottom-right corner (which needs only the display bounds),
///   mirroring AeroSpace's own `fallthrough` on that path.
///
/// Unlike `AXMinimized`, this never resizes the window and is a pure
/// `AXPosition` write.
///
/// # Safety
/// Calls into CoreGraphics (`CGMainDisplayID`/`CGDisplayBounds`, via
/// [`choose_hide_corner`]) and the Accessibility API (via [`get_ax_size`]),
/// all safe to call from any thread. `ax_win` must be a live `AXUIElement`.
unsafe fn hide_target(ax_win: &CFType) -> (f64, f64) {
    let primary = CGDisplayBounds(CGMainDisplayID());
    let bottom_right = (primary.origin.x + primary.size.width - 1.0, primary.origin.y + primary.size.height - 1.0);

    match choose_hide_corner(&primary) {
        HideCorner::BottomRight => bottom_right,
        HideCorner::BottomLeft => match get_ax_size(ax_win) {
            Some((width, _height)) => (primary.origin.x + 1.0 - width, primary.origin.y + primary.size.height - 1.0),
            None => bottom_right,
        },
    }
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
/// Hides the window by moving it, not by minimizing it: captures its current
/// `AXPosition` (so `show_window` can restore the exact point later), then
/// sets `AXPosition` to [`hide_target`] — 1px inside a corner of the primary
/// display, chosen so the window's body doesn't spill onto a neighbouring
/// display. See that function's doc comment for why this reliably pushes the
/// window almost entirely off-screen.
///
/// Idempotent for the same reason `show_window` is (see its guard below): if
/// `window.hidden` is already `true`, this returns `Ok(())` immediately
/// without re-capturing `AXPosition`. Capturing again would read back the
/// *hiding corner* as the "original" position, permanently stranding the
/// window there with no way to restore it — AeroSpace's `hideInCorner` guards
/// against the same thing, for the same reason (see its `isHiddenInCorner`
/// check).
///
/// This replaces the previous `AXMinimized`-based mechanism. Trade-offs versus
/// minimizing:
/// - No genie animation and no Dock thumbnail — the goal of this change (see
///   issue #28). Pure public-API `AXPosition` write, no private CGS calls.
/// - A ~1px sliver of the window technically remains on-screen at the target
///   corner (not truly invisible, though visually unnoticeable in practice).
/// - The window is never minimized, so it keeps its normal (non-minimized)
///   window-server state — an unexpected activation path for its owning app
///   (e.g. Cmd-Tab, or the app raising one of its own windows) could still
///   bring it to the front. This project does not attempt to guard against
///   that; see the PR description for #28.
///
/// # Errors
/// - Window not found via the Accessibility API (wrong PID/title, or
///   Accessibility permission not granted).
/// - The current `AXPosition` cannot be read, or the new `AXPosition` cannot
///   be set (e.g. a fullscreen or system window that doesn't support
///   repositioning). Either failure reverts the hidden marker so a retry is
///   possible.
pub fn hide_window(window: &mut WindowRef) -> Result<(), String> {
    if window.hidden {
        return Ok(()); // already hidden
    }

    unsafe {
        let ax_win = find_ax_window(window.pid, &window.window_title).ok_or_else(|| {
            format!(
                "window '{}' (pid {}) not found via Accessibility API — \
                 ensure Accessibility permission is granted in System Settings",
                window.window_title, window.pid
            )
        })?;

        let original_pos = get_ax_position(&ax_win).ok_or_else(|| {
            format!("could not read AXPosition of window '{}' (pid {})", window.window_title, window.pid)
        })?;

        // Mark hidden and stash the position so the visibility logic and
        // show_window treat it as such, before attempting the OS call.
        window.hidden = true;
        window.hidden_pos = Some(original_pos);

        let (target_x, target_y) = hide_target(&ax_win);
        if let Err(e) = set_ax_position(&ax_win, target_x, target_y) {
            window.hidden = false;
            window.hidden_pos = None;
            return Err(format!(
                "failed to move window into hiding corner: {e} — \
                 window may be fullscreen or a system window that doesn't support repositioning"
            ));
        }

        Ok(())
    }
}

/// macOS implementation of `wm::show_window`.
///
/// If `window.hidden` is `false` the window is already visible; returns
/// `Ok(())` immediately. Otherwise restores the window's `AXPosition` to the
/// point captured by `hide_window` (`window.hidden_pos`) and clears both the
/// hidden marker and the stored position.
///
/// If `hidden_pos` is `None` — state persisted from before this mechanism
/// replaced `AXMinimized`, or from before the position was captured — the
/// restore is skipped (there is nothing to restore *to*) and only the hidden
/// marker is cleared; the window is left wherever it currently is rather than
/// erroring.
///
/// # Errors
/// - Window not found via the Accessibility API.
/// - A stored `hidden_pos` exists but `AXUIElementSetAttributeValue(AXPosition)`
///   fails restoring it.
pub fn show_window(window: &mut WindowRef) -> Result<(), String> {
    if !window.hidden {
        return Ok(()); // already visible
    }

    unsafe {
        let ax_win = find_ax_window(window.pid, &window.window_title).ok_or_else(|| {
            format!("window '{}' (pid {}) not found via Accessibility API", window.window_title, window.pid)
        })?;

        if let Some((x, y)) = window.hidden_pos {
            set_ax_position(&ax_win, x, y)?;
        }

        // Clear only after the OS call succeeds (or was skipped) so a
        // failed restore can be retried.
        window.hidden = false;
        window.hidden_pos = None;

        Ok(())
    }
}

/// macOS implementation of `wm::raise_window`.
///
/// Brings the window's owning application to the front (`AXFrontmost = true`)
/// and raises the window to the top within that application (`AXRaise`),
/// making it the frontmost window on screen. Called after showing a Context's
/// windows to restore the window that was on top before hiding.
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
