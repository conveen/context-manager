# Context Manager — Design Document

## Overview

A desktop application for Windows and macOS that allows users to group application windows into named **Contexts**, then hide and show entire Contexts via the UI or keyboard shortcuts. The core value is that a single window can belong to multiple Contexts simultaneously, unlike OS-level virtual desktops/workspaces.

## Stack

| Layer | Choice | Rationale |
|---|---|---|
| Framework | Tauri v2 | Battle-tested tray, global hotkeys, and permissions; webview UI handles layout well |
| Backend | Rust | Platform window management, hotkey handling, state |
| Frontend | Svelte 5 + TypeScript | Compiles to vanilla JS (no runtime overhead), clean reactivity with runes, `svelte-dnd-action` for drag-and-drop |
| macOS windowing | `AXMinimized` via Accessibility API | Minimize to hide; un-minimize to restore (restores position/size). Public API. Moving offscreen was rejected — macOS clamps window positions so an edge stays visible. |
| Windows windowing | `ShowWindow(hwnd, SW_HIDE/SW_SHOW)` via `windows` crate | Clean, native |
| Global hotkeys | `tauri-plugin-global-shortcut` | Cross-platform, first-class Tauri support |
| Persistence | JSON file via `serde` + `tauri::AppHandle::path` | Simple; no embedded DB needed yet |

## Core Concepts

### Window
A single OS window, identified by:
- **macOS**: `CGWindowID` (u32) + process PID + bundle ID
- **Windows**: `HWND`

A window's original position is cached when it is first hidden so it can be restored on show.

### Context
A named group of windows.

```
Context {
  id: Uuid,
  name: String,          // default: "context-<n>" (0-indexed); Main context default name: "main"
  is_main: bool,         // true for exactly one Context; cannot be deleted
  thumbnail: Option<Screenshot>,
  windows: Vec<WindowRef>,
  shortcut_index: Option<u8>,  // 0–9, maps to <meta>+n
  visible: bool,
}
```

A window can appear in multiple Contexts. Membership is independent per Context.

### Main Context
- Always exists; pinned as the first entry in the sidebar and all Context lists.
- Cannot be deleted. Can be renamed.
- Shortcut index is always `0` (`<meta>+0`); this index is not assignable to other Contexts.
- All newly detected windows are **automatically added to Main** by the background window poll.
- `<meta>+H` hides all Contexts including Main, hiding all windows system-wide.
- Behaves identically to other Contexts in all other respects: appears in the sidebar with the same UI, participates in Single Context Mode, can be shown/hidden.

### WindowRef
A stable reference to a window across Context operations.

```
WindowRef {
  platform_id: PlatformWindowId,  // CGWindowID on macOS, HWND on Windows
  app_name: String,
  window_title: String,
  hidden: bool,  // set while hidden by us
}
```

## Visibility Logic

### Hide a window
1. Set `WindowRef.hidden` — the "currently hidden by us" marker. Both platforms restore geometry natively on show, so no position is captured.
2. Hide it: set `AXMinimized = true` (macOS) or `ShowWindow(SW_HIDE)` (Windows).

### Show a window
1. Un-hide it: set `AXMinimized = false` (macOS, which restores the previous position and size) or `ShowWindow(SW_SHOW)` (Windows).
2. Clear `hidden`.

**macOS z-order restoration:** when hiding, each window's front-to-back stacking
rank is captured (`WindowRef.hidden_z`) from the `CGWindowList` order (which is
front-to-back). On show, the Context's windows are un-minimized **back-to-front**
so the previously-frontmost window is un-minimized last, and it is then
explicitly raised (`AXRaise` + app `AXFrontmost`) — reinstating the window that
was on top before the Context was hidden. This restores order among the
Context's own windows; windows outside the Context keep their place.

> **macOS note:** hiding uses `AXMinimized` rather than moving the window
> offscreen, because macOS clamps window positions so a fully-offscreen window
> still shows an edge. Trade-off: minimizing plays the genie animation and
> leaves a Dock thumbnail. Animation-free hiding would require private CGS
> APIs (out of scope).

### Context visibility rule
A window is **visible** if and only if at least one of its Contexts is currently visible.

When hiding Context A:
- For each window in A, count how many of its Contexts are currently visible.
- If the count drops to 0 after hiding A, hide the window.
- If count remains > 0 (window is in another visible Context), leave it visible.

When showing Context B:
- Show all windows in B that are currently hidden.

### Single Context Mode
When enabled:
- Showing Context B immediately hides all other currently-visible Contexts (applying the rule above to each).
- No animation or transition.
- A **chosen Context** (Settings; defaults to Main) is force-shown at the moment the mode is turned on, and again whenever the choice is changed while the mode is on — so entering the mode lands on an explicit Context rather than whichever happened to be visible. After that, the usual show/hide (shortcuts, sidebar) moves the single visible Context around as normal.

## Keyboard Shortcuts

| Action | Default |
|---|---|
| Show/hide Context n (0-indexed) | `Ctrl+Alt+n` |
| Hide all Contexts | `Ctrl+Alt+H` |

- **Meta key** is configurable in Settings; defaults to `Ctrl+Alt`.
- Shortcuts are registered globally via `tauri-plugin-global-shortcut`.
- A Context must have a `shortcut_index` assigned to be reachable by shortcut.

## Application UI

### Tray / Menu Bar
- Windows: system tray icon
- macOS: menu bar icon
- Click opens a native menu with:
  - "Open Context Manager" — shows and focuses the main window
  - "Quit"

> **Note:** An earlier design had the tray open a compact **popover** window
> (Context list + quick toggles). This was removed; the main Context Manager
> window and the Settings window cover these needs for now. If a quick-access
> surface is wanted later, reintroduce it as a dedicated webview window.

### Context Manager Window
The primary window. Two panels:

#### Left panel — Context sidebar
Styled like the Slack workspace switcher: a slim vertical bar. Each Context is represented by:
- A thumbnail screenshot (captured at Context creation or last update)
- Its name below the thumbnail
- A visual indicator if currently visible
- Right-click menu: Rename, Delete, Assign shortcut

Clicking a Context selects it in the right panel.

#### Right panel — Context detail
Shows the windows assigned to the current Context as a grid/list (app icon + window title). From here:
- **Drag a window card** from the "Available Windows" section (bottom) into the Context window list (top) to add it. By default this **moves** the window: it is added to the target Context and removed from Main (when the target is not Main).
- **Shift+drag** **copies** instead of moves: the window is added to the target Context but kept in Main, so it stays in Available and can be added to further Contexts. This is how a window comes to belong to multiple Contexts.
- **Drag a window card** out of the Context list to remove it.
- Available Windows = Main's windows minus those already in this Context. Because every newly detected window is auto-added to Main, Available initially lists everything; once a window is *moved* (non-Shift) out of Main into another Context, it leaves Available. Shift-copy preserves it in Main and therefore in Available.

No live window dragging from the desktop. Everything happens within the app.

## Platform Window Enumeration

### macOS
- `CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID)` to get all windows.
- Filter: `kCGWindowLayer == 0` (normal windows), exclude our own app.
- Enrich with AX API to get the `AXWindow` element for position manipulation.

### Windows
- `EnumWindows` callback filtering for `WS_VISIBLE`, non-tool windows, non-our-own.

### Refresh
Window list is refreshed:
- On app focus
- When the Context Manager window is opened
- On a periodic background poll (every ~2s) to catch newly opened windows

When a **new window appears**: auto-add it to Main.

When a **window disappears** (app quit):
- Its `WindowRef` is removed from all Contexts it belongs to.

## Persistence

State is persisted as JSON to the platform's app data directory (`AppHandle::path().app_data_dir()`):
- `contexts.json` — all Context definitions and window memberships
- `settings.json` — meta key, single context mode toggle, other preferences

Writes are debounced (250ms) after any state change to avoid thrashing.

## Settings

| Setting | Type | Default |
|---|---|---|
| Meta key | Enum (Ctrl+Alt, Cmd+Opt, etc.) | Ctrl+Alt |
| Single Context Mode | bool | false |
| Single Context (which Context is force-shown when the mode is enabled) | Context id (`None` → Main) | Main |

## Error Handling

- If a window cannot be hidden (e.g., system window, fullscreen app), surface a visible error notification: *"[Window title] cannot be added to a Context — this window type is not supported."*
- Do not add unsupported windows silently.
- On macOS, if Accessibility permissions are not granted, show an onboarding prompt with a direct link to System Settings > Privacy & Security > Accessibility.

## Out of Scope (for now)

- Window re-association after app relaunch (window membership is ephemeral with the process)
- Per-monitor Contexts
- Transition animations
- CGS private API for compositor-level hiding
- Dragging live windows from the desktop into the app
- Window thumbnail live preview (static screenshot only)

## Open Questions

- Should Main's thumbnail auto-update, or use a static generated icon? (Leaning toward static icon, deferred to polish milestone.)

## Milestones

1. **Foundation** — Tauri v2 scaffold, tray icon, global hotkey registration, JSON persistence
2. **Window enumeration** — list all open windows on macOS and Windows
3. **Hide/show** — implement offscreen move (macOS) and SW_HIDE (Windows); verify with edge cases
4. **Context data model** — CRUD for Contexts, window membership, visibility logic
5. **Context Manager UI** — sidebar + detail panel, drag-and-drop, available windows list
6. **Tray menu** — menu bar / system tray with "Open Context Manager" and "Quit"
7. **Settings** — meta key config, single context mode
8. **Polish** — thumbnails (auto-refresh on Context change), error handling, accessibility permissions onboarding