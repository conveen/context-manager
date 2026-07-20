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

## Testing

### Principles

- **Deterministic and headless.** Every automated test runs on any dev machine and in CI with no display, no OS permissions (Accessibility/Screen Recording), and no real input injection. Everything that can't meet that bar is mocked at a seam or moved to the manual checklist.
- **Test through production entry points.** Backend tests invoke the same command functions the frontend calls; frontend tests render real components. Pure helpers additionally get direct unit tests.
- **One thin OS boundary.** The `wm` platform calls, hotkey (re)registration, and native menu/tray are the only code that touches the OS. They stay thin and are replaced by scripted mocks in tests; all state-machine logic above them is fully testable.
- **The existing static gates stay first-line**: `svelte-check`, Clippy, rustfmt, Biome already catch type and lint classes of bugs; tests target behavior.
- No hard coverage percentage to start. The bar is: behavior changes come with tests, and every fixed bug gets a regression test.

### Test layers

| Layer | Scope | Tools | Where it runs |
|---|---|---|---|
| Backend unit | Pure helpers: `normalize_order`/`next_order`, `digit_of`, `meta_prefix`, default-name generation, serde migration of old `data.json` shapes | `cargo test`, inline `#[cfg(test)]` modules | CI, both matrix legs |
| Backend command/state-machine | Full command bodies against a real `AppState` under `tauri::test::MockRuntime`, with `wm` and hotkey registration scripted | `cargo test` + `tauri` `"test"` feature (dev-dependency) | CI, both matrix legs |
| Frontend unit | `toast.svelte.ts` timers, `color.ts`, `api.ts` command-name/arg-casing mapping | Vitest (fake timers, `mockIPC`) | CI |
| Frontend component | Svelte 5 components: rendering, handlers, event listeners, keyboard shortcuts | Vitest + `@testing-library/svelte` + jsdom; Tauri APIs replaced by module mocks | CI |
| Cross-layer contract | Backend⇄frontend event names and the `AppData` JSON shape, pinned by shared fixtures asserted from **both** suites | `cargo test` + Vitest against committed fixtures | CI |
| Manual checklist | The OS-dependent behaviors listed under "Not covered" | [docs/MANUAL_TESTING.md](docs/MANUAL_TESTING.md), run before each release | Human, real desktop |

### Enabling refactors (small, no behavior change)

1. **Commands generic over the runtime.** Command signatures change from `tauri::AppHandle` (concretely Wry) to `tauri::AppHandle<R: tauri::Runtime>` so they run under `MockRuntime`; `generate_handler!` supports generic commands.
2. **`wm` test seam.** Under `cfg(test)`, the `wm::enumerate` / `hide_window` / `show_window` / `raise_window` dispatchers route to a scripted mock (thread-local: scripted enumeration results, per-window success/failure, ordered call log). The platform modules are untouched.
3. **Hotkey seam.** Same pattern for `hotkeys::register_all` / `reregister_all`: under `cfg(test)` they consult a scripted result instead of the global-shortcut plugin, so `update_settings`' rollback path is testable without OS hotkey registration.
4. **Persistence path seam.** Extract `load_from(&Path)` / `save_to(&Path)` and an async `run_saver(path, rx)`; the `AppHandle`-taking functions become thin wrappers. Tests use temp dirs and `tokio::time::pause` for instant, deterministic debounce tests.
5. **Event-name constants.** `contexts-changed` / `show-settings` string literals become mirrored constants (Rust + TS), each side asserted against the shared fixture (below).

### Backend suites

- **Visibility state machine** (`do_hide/do_show_context_windows`, `show`/`hide`): the "visible iff any Context is visible" rule with shared windows; `hidden` propagation across all copies of a window; optimistic marking + revert when a scripted OS hide fails; macOS z-order capture/restore ordering asserted via the mock's call log (`cfg(target_os = "macos")` tests).
- **Single Context Mode**: showing hides all visible siblings; `create_context` starts hidden while the mode is on; `update_settings` force-shows the chosen Context on enable/choice-change, falls back to Main on `None`/stale id, and doesn't move windows on unrelated edits.
- **Membership**: move-vs-copy semantics (Shift), return-to-Main on last removal, idempotency both directions, post-operation visibility reconciliation.
- **CRUD & ordering**: rename validation (empty/`main`/duplicate), Main-deletion rejection, shortcut rules (0 reserved for Main, >9 rejected, index stealing demotes the loser to the end of the unassigned tier), `reorder_contexts` set-coverage validation.
- **Poll reconciliation** (`update_windows`): title/app-name refresh, removal of closed windows with the hidden-window exemption, auto-add to Main, and the changed-gating (a quiet tick must not signal the save channel — regression class of #61; the hidden-exemption tests are the regression class of #38/#63/#67).
- **Persistence**: defaults on missing/corrupt file; migration fixtures (pre-`order`, pre-`hidden` state, unknown leftover keys such as `launch_at_login`); `normalize_order` on load; save/load round-trip; debounce coalescing (a burst of sends produces one write).
- **Events**: commands that must emit `contexts-changed` (shortcut dispatch, Single Context enforcement) are asserted via a Rust-side `listen_any` on the mock app — this is the backend half of the event contract.
- **Hotkey dispatch**: `handle_shortcut` digit/H routing and `toggle_context_by_shortcut` (no-op on unassigned index), below the OS boundary.

### Frontend suites

- **`toast.svelte.ts`**: auto-dismiss, replace-and-restart timer, manual dismiss (fake timers).
- **`api.ts`**: each wrapper invokes the right command name with the documented camelCase arg mapping (via `mockIPC`).
- **`App.svelte`**: `show-settings` event → settings pane; `contexts-changed` event → refresh; the `Ctrl+,`/`Cmd+,` keydown handler including modifier guards (regression for #73); focus/blur collapse/expand using a fake window object (`listen`, `setSize`, `outerSize`, `scaleFactor`); two-tier sidebar ordering derivation. Tauri's `getCurrentWebviewWindow` is module-mocked with a fake that records listeners and lets tests fire events — the frontend half of the event contract.
- **`Settings.svelte`**: load/render, `saveField` patch-merge + optimistic update, error banner on rejected save, controls disabled while saving, stale `single_context_id` falling back to Main in the dropdown.
- **`Sidebar.svelte` / `DetailPanel.svelte`**: rendering (visibility indicators, tier split, Available Windows = Main minus current members), context-menu and dnd **handler** logic invoked with synthetic consider/finalize events.

### Backend ⇄ frontend contract

Features that span the IPC boundary are covered by **paired tests plus a shared fixture**, since no automated test drives the real wire headlessly:

- **Events** (`contexts-changed`, `show-settings`): a fixture file lists every event name. The Rust suite asserts its constants match the fixture and that each trigger emits; the frontend suite asserts its constants match the same fixture and that each handler reacts. Drift on either side fails a test.
- **Data shape**: a committed representative `AppData` JSON fixture (including macOS-only fields). Rust asserts deserialize→serialize round-trips it; the frontend imports it typed as `AppData` (compile-time check via `svelte-check`) and parses it at runtime. A serde rename or a `types.ts` drift breaks one of the two.
- **Command names/args**: pinned from the frontend side by the `api.ts` tests. There is no Rust-side introspection of `generate_handler!`, so a backend rename is caught by the frontend suite, not the backend one.

### Not covered (and why)

| Area | Why it is excluded | Fallback |
|---|---|---|
| `wm/macos.rs`, `wm/win32.rs` internals (AX / Win32 calls) | Need a live desktop session, real foreign windows, and on macOS Accessibility + Screen Recording permissions that cannot be granted non-interactively in CI | Kept thin behind the mocked seam; manual checklist |
| Global hotkey delivery end-to-end (OS actually firing `Ctrl+Alt+5`) | Requires OS-level input injection into a real event loop; headless CI cannot do this | Accelerator strings and dispatch logic tested below the OS boundary; manual checklist |
| Native menu & tray (`setup_app_menu`, `setup_tray`, muda/tray-icon) | Need a real event loop; the wiring is declarative and thin. Includes the WebView2 `Ctrl+,` accelerator behavior itself (#73) | The webview keydown fallback is component-tested; manual checklist |
| WebDriver E2E (`tauri-driver`) | Upstream supports Windows/Linux only — no macOS (WKWebView) support — so E2E would cover half our platforms at disproportionate CI cost | Revisit if a Windows-only smoke E2E earns its keep |
| `svelte-dnd-action` drag gestures | The library depends on real pointer geometry and measurements jsdom does not implement | Handler logic tested with synthetic events; gesture itself on the manual checklist |
| Visual behavior (genie animation, on-screen z-order, Dock thumbnails) | Inherently visual | Manual checklist |
| `start_poll` / `spawn_saver` wrappers, `main.rs`, `run()` builder wiring, `open_devtools` | Infinite-loop spawn wrappers and glue with no logic; their bodies (`update_windows`, `run_saver`) are tested; failures here are obvious at launch | — |

### Tooling & CI

| Task | Command |
|---|---|
| Backend tests | `cargo test` in `src-tauri` (dev-dependency on `tauri` with the `"test"` feature) |
| Frontend tests | `npm run test` (Vitest; `test:watch` for development) |
| Both | `./run.sh -l test` |

CI runs both suites on the existing macOS + Windows matrix legs (platform-`cfg` tests differ per leg), alongside the current check/fmt/lint jobs.

### Rollout

1. **Harness PR** — Vitest + testing-library setup, `cargo test` scaffolding, the five enabling refactors, `run.sh`/CI wiring, one seed test per layer.
2. **Backend suites PR** — state machine, membership, CRUD, poll, persistence, events (regression tests for the #38/#61/#63/#67/#68 classes).
3. **Frontend suites PR** — unit + component tests (including the #73 shortcut regression).
4. **Contract PR** — event/data fixtures, mirrored constants, paired assertions; manual checklist doc.

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