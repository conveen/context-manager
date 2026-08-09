# Changelog

All notable changes to this project are documented here, following
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) conventions.

<!--
How to cut a release:

1. As you work, add entries under the `## [Unreleased]` section below, using
   the standard sub-headings (Added / Changed / Fixed / Removed / Security) as
   needed — omit any sub-heading you have nothing to say under.
2. When you're ready to release, rename `## [Unreleased]` to
   `## [v<major>.<minor>.<patch>] - <YYYY-MM-DD>` (matching the tag you're
   about to push exactly, e.g. `## [v1.2.3] - 2026-07-07`), and add a fresh,
   empty `## [Unreleased]` section above it for future work.
3. Commit that change to `master`, then tag the resulting commit with a
   `v`-prefixed version: `git tag v1.2.3 && git push origin v1.2.3`.
4. .github/workflows/release.yml triggers on that tag, builds the release
   bundles, and extracts the `## [v1.2.3]` section verbatim as the GitHub
   Release body — so the heading format above must match exactly, or the
   workflow fails fast with an error rather than publishing a Release with no
   notes.
-->

## [Unreleased]

### Changed

- Newly opened windows are now added to the Context you're working in rather than always to `main`. When exactly one Context is on screen, that Context gets the window; with Single Context Mode on and nothing on screen, the Context chosen in Settings does; otherwise new windows still go to `main` as before. ([#93](https://github.com/conveen/context-manager/pull/93))
- Turning on Single Context Mode while exactly one Context is already on screen now keeps that Context on screen instead of switching to whichever Context the Settings dropdown names. The dropdown shows the Context you're looking at whenever exactly one is visible, so it always previews what enabling the mode will do, and it is disabled for as long as the mode is on — show a different Context to change which one is active. Turning the mode on with nothing, or several Contexts, on screen still switches to the chosen Context. ([#93](https://github.com/conveen/context-manager/pull/93))

### Fixed

- Snapping the main window to the side of the screen with the window manager (`META`+`Left`/`Right` on Windows) no longer grows it taller than the display, which pushed the sidebar's Add Context button off the bottom edge. The window grew by the height of its title bar and border on every focus change; its height is now also clamped to the monitor's work area. A rapid focus/blur pair from Snap Assist no longer leaves the window stuck at the collapsed width either. ([#91](https://github.com/conveen/context-manager/pull/91))
- On macOS, an empty available-windows list caused by missing Screen Recording permission is no longer silent. A banner above the sidebar and detail panel explains that the permission is missing — or granted but not yet in effect for the running app — with a button that opens the System Settings pane and a reminder that Context Manager must be quit and relaunched before a new grant applies. The permission is also requested once at startup, so the app appears in that pane before the user goes looking for it. ([#96](https://github.com/conveen/context-manager/pull/96))

## [v0.1.1] - 2026-07-20

### Added

- Drag-and-drop reorder Contexts in the sidebar. Contexts with assigned shortcuts are pinned to the top, automatically ordered first, and cannot be re-ordered without changing shortcuts. Remaining (unassigned) Contexts can be freely re-ordered. ([#48](https://github.com/conveen/context-manager/pull/48))

### Changed

- Replaced the per-window "original position" hidden marker with a plain `hidden` flag — hiding a window no longer reads its position on macOS, since nothing ever restored from it. Existing saved state loads unchanged; a window that was hidden at upgrade time loses its marker once and stays minimized until shown manually. ([#70](https://github.com/conveen/context-manager/pull/70))
- Internal cleanup: deduplicated the backend's window-state propagation, z-order capture, and membership show/hide reconciliation, collapsed the hotkey digit dispatch, merged the platform menu-setup variants, removed a dead line in the persistence saver, extracted the frontend's repeated window-event listener boilerplate (also fixing a listener leak when a component tore down before its subscription resolved) and Settings save handlers, and removed the unused or Rust-only command API surface (`get_settings`, `hide_all`, `open_settings`, `open_main_window`). ([#70](https://github.com/conveen/context-manager/pull/70))
- Internal: split the backend command module into per-concern files (`context`, `membership`, `visibility`, `settings`) and switched internal show/hide and menu/tray calls to borrowed `AppHandle`s. ([#71](https://github.com/conveen/context-manager/pull/71))

### Fixed

- Hiding a Context no longer occasionally drops one of its windows from tracking when the background window poll runs at the same moment. ([#38](https://github.com/conveen/context-manager/pull/38))
- The background window poll no longer rewrites its saved state file every ~2 seconds when nothing has changed; state is only written when the window set, titles, or app names actually change. ([#61](https://github.com/conveen/context-manager/pull/61))
- Changing the Keyboard Shortcut Modifier in Settings now takes effect immediately; previously the old shortcuts stayed active and the new ones didn't work until the app was restarted. ([#62](https://github.com/conveen/context-manager/pull/62))
- Moving a window into a hidden Context, or removing it from its last visible one, no longer occasionally drops the window from tracking when the background window poll runs at the same moment. ([#63](https://github.com/conveen/context-manager/pull/63))
- New Contexts are assigned default names using the format `context-n` with the first available `n`, avoiding potential duplicate names. ([#64](https://github.com/conveen/context-manager/pull/64))
- Assigning a keyboard shortcut number outside 1–9 is now rejected with an error instead of being silently stored as a shortcut that never fires. ([#65](https://github.com/conveen/context-manager/pull/65))
- On Windows, hiding a Context no longer permanently drops its windows. Hidden windows are now correctly remembered as hidden, survive the background poll, and can be shown again. ([#67](https://github.com/conveen/context-manager/pull/67))
- Contexts created while Single Context Mode is enabled now start hidden, so the active Context remains the only visible one instead of two Contexts becoming visible at once. ([#68](https://github.com/conveen/context-manager/pull/68))
- Failed operations in the main window — renaming, deleting, shortcut assignment, reordering, creating a Context, toggling visibility, and drag-and-drop window changes — now show an error toast instead of failing silently, and rejected drag-and-drop changes snap back immediately. ([#69](https://github.com/conveen/context-manager/pull/69))
- The `Ctrl+,` / `Cmd+,` shortcut now opens the Settings page on Windows. Previously the native menu accelerator never fired there because WebView2 handles accelerator keys before the Win32 menu accelerator table sees them; the shortcut is now handled in the webview so it works on both platforms. ([#74](https://github.com/conveen/context-manager/pull/74))

### Removed

- Removed the "Launch at Login" feature. The toggle never actually registered the app with the OS login items so it didn't work, and the feature isn't necessary as of now. It may be reintroduced in a later version. ([#72](https://github.com/conveen/context-manager/pull/72))

## [v0.1.0] - 2026-07-08

### Added

- Support for macOS and Windows.
- A default `main` Context that automatically collects every open window.
- Create named Contexts and move/copy windows from the `main` Context.
- Drag-and-drop window assignment from an "Available Windows" list into a Context; hold Shift while dropping to copy a window into multiple Contexts instead of moving it out of `main`.
- Switch between Contexts via configurable keyboard shortcuts (`Ctrl+Alt+0`-`9` or `Cmd+Opt+0`-`9`), plus `Ctrl+Alt+H`/`Cmd+Opt+H` to hide everything at once.
- Single Context Mode: switching to a Context automatically hides every other Context's windows, so exactly one Context is ever on screen.
- macOS menu bar / Windows system tray integration, with the main window minimizing out of the way when not focused.
- Launch-at-login setting.
