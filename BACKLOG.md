# Backlog

## Bugs

### Race in `do_hide_context_windows` can drop a window from tracking
**Status:** open · **Severity:** low

The three-phase hide in [commands.rs](src-tauri/src/commands.rs) minimizes the
OS window (Phase 2, no lock) *before* writing `original_position` into state
(Phase 3, under lock). If the ~2s window poll (`update_windows`) fires in that
gap, it sees the window as minimized — absent from the on-screen enumeration —
but not yet marked hidden, and removes it from every context. Narrow timing
window; surfaces as an occasional window silently dropped on hide.

**Suggested fix:** mark intent-to-hide before the OS call — e.g. set
`original_position` to an optimistic sentinel (or a dedicated "hiding" flag)
in Phase 1, so the poll's exemption
(`original_position.is_some()`) covers the window throughout Phase 2. The real
position/marker is then finalized in Phase 3 as today.

### macOS window lookup by title is fragile (title-matching)
**Status:** open · **Severity:** low

On macOS, hide/show resolve the OS window through
[`find_ax_window(pid, window_title)`](src-tauri/src/wm/macos.rs), which matches
the stored `window_title` against the live `AXTitle` **exactly and
case-sensitively**. Titles change over a window's lifetime (e.g. KeePassXC
appends its database/lock state), so a stale stored title makes the lookup
return `None` and the window silently refuses to hide/show. The ~2s poll now
refreshes `window_title` for every *enumerated* window
([`update_windows`](src-tauri/src/wm/mod.rs)), which closes almost all of the
gap, but two narrow cases remain:

- **Sub-poll race** — an app that renames its window in the <2s between the last
  poll and a hide can be missed for that one operation (self-heals on the next
  poll / retry).
- **Rename while hidden** — a window we've minimized is absent from the
  enumeration, so its title can't be refreshed; if the app changes the title
  while minimized, the subsequent *show* lookup can miss.

**Suggested fix.** Match the `AXUIElement` to its stable `CGWindowID`
(`platform_id`) instead of its title. The direct route is the **private**
`_AXUIElementGetWindow(AXUIElementRef, CGWindowID*)`, which this project avoids
(fragile across releases, App Store rejection risk — see the animation-free
hiding item below). Investigate a public-API alternative (e.g. disambiguating by
`AXPosition`/`AXSize` against the `CGWindowList` bounds for the `platform_id`);
if none is acceptable, document title-matching as an accepted limitation and
keep the poll refresh as the mitigation.

### Launch at Login does nothing
**Status:** open · **Severity:** medium

The Settings UI ([Settings.svelte](src/Settings.svelte)) has a "Launch at
Login" toggle that persists a `launch_at_login` boolean via `update_settings`,
but nothing ever registers the app with the OS login items. There is no
autostart plugin in the dependencies, so the toggle is currently a no-op that
misleads the user.

**Suggested fix:** add `tauri-plugin-autostart`, and enable/disable the OS
autostart entry in `update_settings` (or a dedicated command) whenever the flag
changes. Reconcile the stored flag against the actual OS state on startup.

---

## Features / Enhancements

### Preserve the shown Context when entering Single Context Mode from a single-Context view
**Status:** not started

Today, enabling Single Context Mode ([Settings.svelte](src/Settings.svelte) →
[`update_settings`](src-tauri/src/commands.rs)) always force-shows the Context
chosen in the dropdown, regardless of how many Contexts were visible at the
moment the mode was turned on. That's the desired behavior when several Contexts
are visible (the app has to collapse to exactly one), but when the screen is
*already* showing a single Context, switching away from it to the dropdown's
choice is surprising — the user is effectively already in a single-Context view.

Instead: when exactly one Context is visible at the moment SCM is enabled,
**preserve that Context** (don't force-show the dropdown's choice), and **gray
out the dropdown** to signal that the active Context is fixed to the one on
screen. When two or more (or zero) Contexts are visible, keep the current
behavior: force-show the dropdown's Context (defaulting to Main) and hide the
rest. Changing the dropdown *while already in SCM* should still switch to the
newly chosen Context as it does now.

**Suggested implementation.**
- Backend ([`update_settings`](src-tauri/src/commands.rs)): on the off→on
  transition, count visible Contexts. If exactly one is visible, set
  `single_context_id` to that Context's id (so state reflects reality) and skip
  the `show_context` enforcement. Otherwise enforce as today. The
  choice-changed-while-on path is unchanged.
- Frontend ([Settings.svelte](src/Settings.svelte)): disable the dropdown when
  SCM is enabled *and* exactly one Context is currently visible. This needs the
  live visible-Context count, so Settings must read/refresh `contexts` visibility
  (e.g. subscribe to the `contexts-changed` event or poll `get_app_data`) rather
  than loading it once on mount.

### Toggle contexts from the system tray menu
**Status:** not started

The removed popover previously allowed toggling a context's visibility from the
menu bar without opening the main window. Re-add this as dynamic entries in the
native tray menu ([`setup_tray`](src-tauri/src/lib.rs)): one item per context
showing its name and current visibility (e.g. a checkmark), invoking
`show_context` / `hide_context`. Menu must rebuild when contexts change.

### Open Settings from the system tray menu
**Status:** not started

Add a "Settings" item to the tray menu that calls `open_settings` (which already
shows the main window and emits the `show-settings` event). Restores the
settings shortcut the popover's ⚙️ button used to provide.

### System tray icon: template (monochrome) coloring
**Status:** not started

The tray currently uses the app's colored PNG icon. For a native macOS menu-bar
look that adapts to light/dark menu bars, supply a template-style
(alpha-silhouette) asset and set `.icon_as_template(true)` on the
`TrayIconBuilder` ([`setup_tray`](src-tauri/src/lib.rs)). Requires a properly
designed template image — a plain colored icon rendered as a template becomes a
solid black blob.

### App-icon sidebar backgrounds for non-empty Contexts
**Status:** not started

Today every sidebar item shows a solid color swatch (`hueFor(ctx.id)` in
[color.ts](src/lib/color.ts), rendered in [Sidebar.svelte](src/components/Sidebar.svelte)).
Instead:

- **Empty Context** (no windows) → keep the current `hueFor` color.
- **Non-empty Context** → use the **app icon of the first window** by default.
  Let the user override it via an **Icon** submenu in the Context item's
  right-click menu (placed below the shortcut-number grid), listing each window
  in the Context by app name / title; selecting one sets that window's app icon
  as the background.

**Recommended design.** Persist only the user's *choice*, not image bytes: add
`icon_source: Option<u64>` to `Context` (the `platform_id` of the chosen
window; `None` = auto/first window). Resolve the actual image at runtime via a
new `get_window_icon(platform_id) -> Option<String>` command returning a
base64-encoded PNG `data:` URL, cached in the frontend by `platform_id`. This
keeps `contexts.json` small and avoids stale icons across app updates. Fallbacks:
if the chosen window has closed, use the first remaining window, then the
`hueFor` color; if retrieval returns `None` (unsupported window, transient
failure), fall back to the color so a broken image never shows.

**Icon retrieval (native, public APIs, uses the `pid` we already capture):**
- **macOS** — `NSRunningApplication(processIdentifier: pid).icon` → `NSImage`,
  converted to PNG via `NSBitmapImageRep`. Requires adding an AppKit binding
  (e.g. `objc2` / `objc2-app-kit`; we currently link only CoreGraphics/
  CoreFoundation). No extra permissions. Reliable for any running app.
- **Windows** — from the PID's executable path (`QueryFullProcessImageNameW`,
  already used for the app name in [win32.rs](src-tauri/src/wm/win32.rs)) via
  `SHGetFileInfoW`/`ExtractIconExW` → `HICON` → GDI (`GetIconInfo`/`GetDIBits`)
  → PNG. Enable the `Win32_UI_Shell` and `Win32_Graphics_Gdi` features on the
  `windows` crate.

**App Store / UWP caveat.** The Windows executable route is reliable for classic
desktop apps but **not** for UWP/MSIX (Microsoft Store) apps: their process is a
stub (`ApplicationFrameHost.exe` or a package host), so the exe icon is
generic/wrong. Their true icon requires a separate path —
`IShellItemImageFactory`, or reading the package manifest's logo asset. Until
that path is implemented, Store apps fall back to the `hueFor` color swatch.
(macOS has no equivalent gap — sandboxed/Store apps still resolve via their PID.)

### Auto-generated context thumbnails from added windows
**Status:** not started

The sidebar currently renders a solid color swatch per context
(`hueFor(ctx.id)` in [color.ts](src/lib/color.ts)); there is no real thumbnail
code despite the `thumbnail: Option<Screenshot>` field described in DESIGN.md.
Generate a thumbnail for each context based on the windows it contains (e.g. a
composite/screenshot of member windows), captured on context creation and
refreshed when membership changes. Note macOS Screen Recording permission is
required to capture window imagery.

### Animation-free window hiding on macOS
**Status:** not started

Hiding on macOS now uses `AXMinimized` ([wm/macos.rs](src-tauri/src/wm/macos.rs)),
which genuinely hides windows via a public API but plays the minimize genie
animation and leaves a Dock thumbnail. This conflicts with the "no transition"
goal for Single Context Mode. Truly instant, artifact-free hiding appears to
require private CGS APIs (e.g. compositor-level alpha or moving windows to an
off-screen Mission Control Space) — powerful but private, fragile across macOS
releases, and an App Store rejection risk. Investigate whether an acceptable
public-API path exists; otherwise document the animation as an accepted
trade-off.

### macOS Accessibility permission onboarding
**Status:** not started

The window-management layer ([wm/macos.rs](src-tauri/src/wm/macos.rs)) requires
Accessibility permission (and Screen Recording for window titles/enumeration)
and surfaces errors when it is not granted, but there is no onboarding flow.
Detect the missing permission (e.g. `AXIsProcessTrusted`) on startup and show a
prompt explaining the requirement, with a direct link to
System Settings → Privacy & Security → Accessibility. See DESIGN.md Error
Handling.

---

## Done

### Context picker when entering Single Context Mode
**Resolved.** [Settings.svelte](src/Settings.svelte) now shows a Context dropdown
next to the Single Context Mode toggle (defaulting to Main). The choice is
persisted as `Settings.single_context_id` (an `Option<String>`; `None`/stale →
Main). [`update_settings`](src-tauri/src/commands.rs) force-shows the chosen
Context when the mode is turned on, or when the choice changes while it is on
(reusing `show_context`, which hides all others because the mode is enabled), so
entering the mode lands on an explicit Context instead of whichever was visible.

### Main window cannot be reopened from the tray after closing
**Resolved.** The main window's close-requested event is now intercepted in
[`run`](src-tauri/src/lib.rs) (`on_window_event`): it calls `api.prevent_close()`
and hides the window instead of letting it be destroyed. Because the window is
never destroyed, the app stays alive and "Open Context Manager"
(`open_main_window`) reliably re-shows it.

**Optional follow-up (not done) — Dock icon behavior on macOS.** With the window
now hidden (not destroyed) on close, the Dock icon remains but clicking it does
nothing: macOS's default Dock-click reopen only recreates *destroyed* windows,
and there is no handler to un-hide ours. This is a UX decision, not a
correctness issue, so it was left out of the bug fix. Three ways forward:

1. **Do nothing** — keep `ActivationPolicy::Regular`. Harmless; just a Dock icon
   whose click is a no-op while the window is hidden.
2. **Keep the Dock icon and make it work** — add a `RunEvent::Reopen` handler
   (macOS) that calls `open_main_window`, so clicking the Dock icon re-shows the
   window. Most conventional macOS behavior; keeps Dock icon + ⌘-Tab.
3. **Go `Accessory`** — set `ActivationPolicy::Accessory` on macOS to drop the
   Dock icon and ⌘-Tab entry entirely, making the tray the single entry point.
   Matches the menu-bar-app model in [DESIGN.md](DESIGN.md), at the cost of
   discoverability (some users won't realize it's running).

Recommendation: option 2 for a conventional Mac-app feel, or option 3 if
committing to the menu-bar-utility model.
