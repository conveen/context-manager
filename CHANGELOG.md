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

### Added

- Drag-and-drop reorder Contexts in the sidebar. Contexts with assigned shortcuts are pinned to the top, automatically ordered first, and cannot be re-ordered without changing shortcuts. Remaining (unassigned) Contexts can be freely re-ordered. ([#48](https://github.com/conveen/context-manager/pull/48))

### Fixed

- Hiding a Context no longer occasionally drops one of its windows from tracking when the background window poll runs at the same moment. ([#38](https://github.com/conveen/context-manager/pull/38))
- The background window poll no longer rewrites its saved state file every ~2 seconds when nothing has changed; state is only written when the window set, titles, or app names actually change. ([#61](https://github.com/conveen/context-manager/pull/61))
- Moving a window into a hidden Context, or removing it from its last visible one, no longer occasionally drops the window from tracking when the background window poll runs at the same moment. ([#63](https://github.com/conveen/context-manager/pull/63))
- New Contexts are assigned default names using the format `context-n` with the first available `n`, avoiding potential duplicate names. ([#64](https://github.com/conveen/context-manager/pull/64))
- Assigning a keyboard shortcut number outside 1–9 is now rejected with an error instead of being silently stored as a shortcut that never fires. ([#65](https://github.com/conveen/context-manager/pull/65))
- On Windows, hiding a Context no longer permanently drops its windows. Hidden windows are now correctly remembered as hidden, survive the background poll, and can be shown again. ([#67](https://github.com/conveen/context-manager/pull/67))

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
