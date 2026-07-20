//! Scripted stand-in for the platform window layer in test builds.
//!
//! Tests script the enumeration result with [`set_windows`] and per-window
//! failures with [`fail_hide`] / [`fail_show`]; every call is appended to an
//! ordered log readable via [`calls`]. Success semantics mirror the real
//! implementations: `hide_window` sets the `hidden` marker, `show_window`
//! clears it, and a failed call leaves the marker untouched.
//!
//! State is thread-local, so parallel tests (one thread per test) are
//! isolated; test helpers call [`reset`] anyway in case a runner reuses
//! threads.

use std::cell::RefCell;
use std::collections::HashSet;

use super::WindowInfo;
use crate::state::WindowRef;

/// One recorded call into the platform layer, in call order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    Hide(u64),
    Show(u64),
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Raise(u64),
}

/// A scripted live window returned by [`enumerate`].
#[derive(Clone, Debug)]
pub struct MockWindow {
    pub platform_id: u64,
    pub app_name: String,
    pub window_title: String,
}

/// Shorthand constructor for a scripted live window.
pub fn mock_win(platform_id: u64, app_name: &str, window_title: &str) -> MockWindow {
    MockWindow { platform_id, app_name: app_name.to_string(), window_title: window_title.to_string() }
}

#[derive(Default)]
struct MockState {
    windows: Vec<MockWindow>,
    fail_hide: HashSet<u64>,
    fail_show: HashSet<u64>,
    calls: Vec<Call>,
}

thread_local! {
    static STATE: RefCell<MockState> = RefCell::new(MockState::default());
    // Held separately from MockState: a boxed closure has no Default, and the
    // hook must be taken out while it runs so it can call back into the mock
    // (set_windows/enumerate) without a double borrow.
    static ON_HIDE: RefCell<Option<Box<dyn FnMut(u64)>>> = const { RefCell::new(None) };
}

/// Clears the scripted windows, failure sets, the call log, and the mid-hide
/// hook.
pub fn reset() {
    STATE.with(|s| *s.borrow_mut() = MockState::default());
    ON_HIDE.with(|h| *h.borrow_mut() = None);
}

/// Installs a hook that runs from inside every successful [`hide_window`]
/// call, after the OS-level hide "happened" (the window would no longer be
/// enumerable) but before the caller's write-back runs — the exact gap the
/// background poll can race into. Tests use it to fire `update_windows`
/// mid-hide deterministically. The hook is removed while it runs (no
/// reentrancy) and reinstalled afterwards; [`reset`] clears it.
pub fn set_on_hide(hook: impl FnMut(u64) + 'static) {
    ON_HIDE.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
}

/// Scripts the set of "live" windows the next [`enumerate`] calls return.
pub fn set_windows(windows: Vec<MockWindow>) {
    STATE.with(|s| s.borrow_mut().windows = windows);
}

/// Makes [`hide_window`] fail for `platform_id` until [`reset`].
pub fn fail_hide(platform_id: u64) {
    STATE.with(|s| s.borrow_mut().fail_hide.insert(platform_id));
}

/// Makes [`show_window`] fail for `platform_id` until [`reset`].
pub fn fail_show(platform_id: u64) {
    STATE.with(|s| s.borrow_mut().fail_show.insert(platform_id));
}

/// Returns every platform call recorded since the last [`reset`], in order.
pub fn calls() -> Vec<Call> {
    STATE.with(|s| s.borrow().calls.clone())
}

/// Mirrors `wm::enumerate`: returns the scripted live windows. `our_pid` is
/// accepted for signature parity and ignored; scripted windows are never ours.
pub fn enumerate(_our_pid: u32) -> Vec<WindowInfo> {
    STATE.with(|s| {
        s.borrow()
            .windows
            .iter()
            .map(|w| WindowInfo {
                platform_id: w.platform_id,
                // The real macOS enumeration provides the owning pid; derive a
                // deterministic one so tests never depend on its exact value.
                #[cfg(target_os = "macos")]
                pid: w.platform_id as u32 + 1,
                app_name: w.app_name.clone(),
                window_title: w.window_title.clone(),
            })
            .collect()
    })
}

/// Mirrors the real `hide_window` contract: records the call, then either
/// fails (marker untouched) or sets the `hidden` marker. On success the
/// [`set_on_hide`] hook, if any, runs before returning — i.e. before the
/// caller's write-back.
pub fn hide_window(window: &mut WindowRef) -> Result<(), String> {
    let fail = STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.calls.push(Call::Hide(window.platform_id));
        state.fail_hide.contains(&window.platform_id)
    });
    if fail {
        return Err(format!("mock: hide_window({}) scripted to fail", window.platform_id));
    }
    window.hidden = true;
    // Run the mid-hide hook outside any STATE borrow; it may call back into
    // the mock. Taken out for the duration so it can't recurse, and only
    // reinstalled if the hook didn't replace itself.
    if let Some(mut hook) = ON_HIDE.with(|h| h.borrow_mut().take()) {
        hook(window.platform_id);
        ON_HIDE.with(|h| {
            let mut slot = h.borrow_mut();
            if slot.is_none() {
                *slot = Some(hook);
            }
        });
    }
    Ok(())
}

/// Mirrors the real `show_window` contract: records the call, then either
/// fails (marker untouched) or clears the `hidden` marker.
pub fn show_window(window: &mut WindowRef) -> Result<(), String> {
    let fail = STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.calls.push(Call::Show(window.platform_id));
        state.fail_show.contains(&window.platform_id)
    });
    if fail {
        return Err(format!("mock: show_window({}) scripted to fail", window.platform_id));
    }
    window.hidden = false;
    Ok(())
}

/// Mirrors `wm::raise_window` (macOS-only in production): records the call.
#[cfg(target_os = "macos")]
pub fn raise_window(window: &WindowRef) -> Result<(), String> {
    STATE.with(|s| s.borrow_mut().calls.push(Call::Raise(window.platform_id)));
    Ok(())
}

// Self-tests pinning the scripted contract the command/state-machine suites
// build on: success mirrors the real marker semantics, scripted failures
// leave the marker untouched, and the call log records everything in order.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::win;

    #[test]
    fn scripted_windows_are_enumerated_and_calls_are_logged_in_order() {
        reset();
        set_windows(vec![mock_win(1, "A", "a"), mock_win(2, "B", "b")]);
        let ids: Vec<u64> = enumerate(0).iter().map(|w| w.platform_id).collect();
        assert_eq!(ids, vec![1, 2]);

        let mut w1 = win(1, false);
        let mut w2 = win(2, true);
        hide_window(&mut w1).unwrap();
        show_window(&mut w2).unwrap();
        assert!(w1.hidden);
        assert!(!w2.hidden);
        assert_eq!(calls(), vec![Call::Hide(1), Call::Show(2)]);
    }

    #[test]
    fn scripted_failures_leave_the_hidden_marker_untouched() {
        reset();
        fail_hide(1);
        fail_show(2);
        let mut w1 = win(1, false);
        let mut w2 = win(2, true);
        assert!(hide_window(&mut w1).is_err());
        assert!(show_window(&mut w2).is_err());
        assert!(!w1.hidden, "failed hide must not set the marker");
        assert!(w2.hidden, "failed show must not clear the marker");
        assert_eq!(calls(), vec![Call::Hide(1), Call::Show(2)]);
    }
}
