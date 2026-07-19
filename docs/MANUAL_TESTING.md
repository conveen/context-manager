# Manual Testing Checklist

The behaviors the automated suites deliberately can't cover (see DESIGN.md →
Testing → "Not covered"): everything here needs a live desktop session, real
foreign windows, OS permissions, or real input. Run through this before
cutting a release, on **both** platforms unless marked otherwise.

## Setup

- [ ] Fresh install / first launch starts with only the `main` Context and it
      collects every open window within ~2s.
- [ ] **macOS:** first use prompts for Accessibility and Screen Recording;
      after granting both and restarting, windows appear with titles.
- [ ] Quit and relaunch: Contexts, shortcuts, sidebar order, and settings are
      restored (`data.json` round-trip against the real app-data dir).

## Global shortcuts (real OS delivery)

- [ ] `<meta>+<n>` toggles the Context assigned to shortcut *n* while another
      app has focus (that's what makes the shortcut *global*).
- [ ] `<meta>+H` hides every Context, clearing the screen.
- [ ] Switching the modifier in Settings takes effect immediately: the new
      combo works, the old combo is dead — no restart.
- [ ] Choosing a modifier already claimed by another app shows the error and
      the previous modifier keeps working (rollback).

## Hide/show against real windows

- [ ] Hiding a Context hides exactly its exclusive windows; a window shared
      with a visible Context stays on screen.
- [ ] Showing restores position and size.
- [ ] **macOS:** windows un-minimize back-to-front and the previously
      frontmost window ends up on top (genie animation + Dock thumbnail are
      expected artifacts).
- [ ] **Windows:** hidden windows disappear from the taskbar (`SW_HIDE`) and
      come back with `SW_SHOW`.
- [ ] Quit an app whose windows are in a Context: they drop out of the UI
      within ~2s.

## Native menu & tray

- [ ] Tray / menu-bar icon shows the menu; "Open Context Manager" reopens and
      focuses the window after it was closed; "Quit" exits.
- [ ] Closing the main window keeps the app alive in the tray.
- [ ] **macOS:** App → Settings menu item and the native `Cmd+,` key
      equivalent both open the settings pane.
- [ ] **Windows:** File → Settings opens the settings pane by click; `Ctrl+,`
      opens it via the webview keydown fallback (#73 — the native accelerator
      never fires under WebView2).

## Drag-and-drop gestures

- [ ] Dragging a window card from Available into the Context zone moves it
      (gone from Available); holding **Shift** while dropping copies it
      (stays in Available).
- [ ] Dragging a card out of the Context zone removes it from the Context.
- [ ] Reordering the unassigned sidebar tier by drag sticks after a refresh;
      the shortcut tier refuses to reorder.
- [ ] A drop the backend rejects snaps back visually and shows the error toast.

## Single Context Mode (end-to-end feel)

- [ ] Enabling it lands on the chosen Context and hides everything else;
      switching Contexts afterwards keeps exactly one visible.
- [ ] The main window collapses on blur and restores to its previous width on
      focus.
