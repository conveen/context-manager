<script lang="ts">
import { getCurrentWebviewWindow, type WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { currentMonitor, LogicalSize } from "@tauri-apps/api/window";
import * as api from "./lib/api";
import type { AppData, ScreenRecordingStatus } from "./lib/types";
import Sidebar from "./components/Sidebar.svelte";
import DetailPanel from "./components/DetailPanel.svelte";
import ErrorToast from "./components/ErrorToast.svelte";
import PermissionBanner from "./components/PermissionBanner.svelte";
import Settings from "./Settings.svelte";
import { showError } from "./lib/toast.svelte";

let appData = $state<AppData | null>(null);
let selectedId = $state<string | null>(null);
let showDetailPanel = $state(true);
let showSettings = $state(false);
// Assume the permission is fine until the backend says otherwise, so the
// banner never flashes during the first poll.
let screenRecording = $state<ScreenRecordingStatus>("Granted");

async function refresh() {
    try {
        const [data, status] = await Promise.all([api.getAppData(), api.getScreenRecordingStatus()]);
        appData = data;
        screenRecording = status;
        // Clear selection if the selected context was deleted.
        if (selectedId && !appData.contexts.find((c) => c.id === selectedId)) {
            selectedId = null;
        }
    } catch (e) {
        if (import.meta.env.DEV) console.error("Failed to load app data:", e);
    }
}

$effect(() => {
    refresh();
    const id = setInterval(refresh, 2500);
    return () => clearInterval(id);
});

// Global shortcuts are registered per-accelerator and the OS can refuse any of
// them (on Windows, whenever another process already owns the combination).
// Report the dead ones once, through the same toast as any other failure.
function reportFailedShortcuts(accelerators: string[]) {
    if (accelerators.length === 0) return;
    showError(
        `These shortcuts couldn't be registered: ${accelerators.join(", ")}. ` +
            "Another application is likely using them — try a different modifier in Settings.",
    );
}

// Shortcuts are registered during backend startup, before this webview exists,
// so the "shortcuts-failed" event for that registration is never delivered
// here. Ask for the recorded list instead, once, on mount.
$effect(() => {
    api.getFailedShortcuts().then(reportFailedShortcuts, (e) => {
        if (import.meta.env.DEV) console.error("Failed to read shortcut failures:", e);
    });
});

// The subscribe/cleanup dance shared by the window-event effects below:
// subscribe to the current webview window when the component mounts, and call
// the returned unlisten function on teardown. If teardown wins the race
// against the async subscription, the subscription is undone as soon as it
// resolves. Must be called during component initialization ($effect rule).
function windowSubscription(subscribe: (appWindow: WebviewWindow) => Promise<() => void>) {
    $effect(() => {
        let unlisten: (() => void) | null = null;
        let cancelled = false;
        (async () => {
            const un = await subscribe(getCurrentWebviewWindow());
            if (cancelled) un();
            else unlisten = un;
        })();
        return () => {
            cancelled = true;
            unlisten?.();
        };
    });
}

// Listen for window focus/blur to resize and show/hide the detail panel.
// On blur we capture the current width before collapsing, and on focus we
// restore to that width — so the app respects whatever width the user or
// window manager chose instead of snapping to a hard-coded value.
const COLLAPSED_WIDTH = 84;
// Logical px. Seeded with a sensible default and overwritten with the real
// width whenever the window loses focus.
let expandedWidth = 900;

windowSubscription((appWindow) =>
    appWindow.onFocusChanged(async (event) => {
        const focused = event.payload;
        const factor = await appWindow.scaleFactor();
        // Read innerSize() because the window is decorated (title bar and border)
        // Reading outerSize() can result in the bottom of the window below the task bar/off screen
        // Must convert physical pixel measurement to logical for setSize
        const size = (await appWindow.innerSize()).toLogical(factor);

        // Protect against setting height taller than the monitor's work area
        const monitor = await currentMonitor();
        const workAreaHeight = monitor ? monitor.workArea.size.toLogical(monitor.scaleFactor).height : size.height;
        const height = Math.min(size.height, workAreaHeight);

        if (focused) {
            showDetailPanel = true;
            appWindow.setSize(new LogicalSize(expandedWidth, height));
        } else {
            // Capture the current width before collapsing so we can restore it.
            // These handlers await before writing, so a rapid focus/blur pair
            // (Windows Snap Assist produces one) can deliver a second blur
            // after we've already collapsed — ignore widths at the collapsed
            // width, which would otherwise leave the window stuck narrow.
            if (size.width > COLLAPSED_WIDTH + 8) expandedWidth = size.width;
            showDetailPanel = false;
            appWindow.setSize(new LogicalSize(COLLAPSED_WIDTH, height));
        }
    }),
);

// Listen for the backend's request to show the settings panel.
windowSubscription((appWindow) =>
    appWindow.listen("show-settings", () => {
        showSettings = true;
    }),
);

// Open settings from the keyboard. The native menu item carries the same
// accelerator, but on Windows WebView2 owns focus and handles accelerator
// keys in its own pipeline before the Win32 accelerator table sees them, so
// the menu accelerator never fires there (tauri-apps/wry#451). Handling it in
// the webview works on both platforms. On macOS the native Cmd+, still fires
// too, but since this only sets showSettings = true it's an idempotent no-op —
// not worth a platform check to suppress.
$effect(() => {
    const onKey = (e: KeyboardEvent) => {
        if (e.key === "," && (e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
            e.preventDefault();
            showSettings = true;
        }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
});

// Refresh immediately when the backend changes context visibility
// (e.g. via global shortcuts) instead of waiting for the periodic poll.
windowSubscription((appWindow) => appWindow.listen("contexts-changed", () => refresh()));

// Emitted when re-registering under a new modifier leaves shortcuts dead —
// the case the on-mount fetch above cannot see, since it happens later.
windowSubscription((appWindow) =>
    appWindow.listen<string[]>("shortcuts-failed", (event) => reportFailedShortcuts(event.payload)),
);

const mainContext = $derived(appData?.contexts.find((c) => c.is_main) ?? null);
// Two-tier sidebar order: shortcut-assigned contexts first (auto-ordered by
// shortcut number, so Main — always shortcut 0 — leads), then the unassigned
// contexts in their manual `order`. Only the second tier is drag-reorderable.
const sidebarContexts = $derived.by(() => {
    const all = appData?.contexts ?? [];
    const assigned = all
        .filter((c) => c.shortcut_index !== null)
        .sort((a, b) => (a.shortcut_index ?? 0) - (b.shortcut_index ?? 0));
    const unassigned = all.filter((c) => c.shortcut_index === null).sort((a, b) => a.order - b.order);
    return [...assigned, ...unassigned];
});
const selectedContext = $derived(appData?.contexts.find((c) => c.id === selectedId) ?? null);

async function handleCreate() {
    try {
        const ctx = await api.createContext();
        await refresh();
        selectedId = ctx.id;
    } catch (e) {
        showError(String(e));
    }
}

async function handleOpenDevtools() {
    try {
        await api.openDevtools();
    } catch (e) {
        if (import.meta.env.DEV) console.error("Failed to open devtools:", e);
    }
}

function closeSettings() {
    showSettings = false;
}
</script>

<div class="app" role="main">
    <ErrorToast />
    {#if showSettings}
        <div class="settings-view">
            <button class="back-btn" onclick={closeSettings} title="Back">← Back</button>
            <Settings contexts={appData?.contexts ?? []} />
        </div>
    {:else}
        {#if import.meta.env.DEV && showDetailPanel}
            <button class="devtools-btn" onclick={handleOpenDevtools} title="Open DevTools">⚙️</button>
        {/if}
        <!-- Spans both panels, so it reads as a statement about the window list
             as a whole rather than about the selected Context. Suppressed while
             the window is collapsed to the sidebar strip on blur, where there
             is no room to read it. -->
        {#if showDetailPanel}
            <PermissionBanner status={screenRecording} />
        {/if}
        <div class="panels">
            {#if appData}
                <Sidebar
                    contexts={sidebarContexts}
                    {selectedId}
                    onSelect={(id) => { selectedId = id; }}
                    onCreate={handleCreate}
                    onRefresh={refresh}
                />
                {#if showDetailPanel}
                    <DetailPanel
                        context={selectedContext}
                        {mainContext}
                        onRefresh={refresh}
                    />
                {/if}
            {:else}
                <div class="loading">Loading…</div>
            {/if}
        </div>
    {/if}
</div>

<style>
    /* Column so the permission banner can sit above both panels; `.panels`
       restores the original side-by-side row for the panels themselves. */
    .app {
        display: flex;
        flex-direction: column;
        height: 100vh;
        overflow: hidden;
        position: relative;
    }

    .panels {
        display: flex;
        flex: 1;
        min-height: 0;
        overflow: hidden;
    }

    .devtools-btn {
        position: absolute;
        bottom: 10px;
        right: 10px;
        z-index: 1000;
        background: #333;
        border: 1px solid #555;
        border-radius: 4px;
        padding: 4px 8px;
        color: #aaa;
        font-size: 16px;
        cursor: pointer;
        transition: background 0.15s, color 0.15s;
    }

    .devtools-btn:hover {
        background: #444;
        color: #fff;
    }

    .loading {
        flex: 1;
        display: flex;
        align-items: center;
        justify-content: center;
        color: #444;
        font-family: system-ui, sans-serif;
        font-size: 14px;
    }

    .settings-view {
        position: relative;
        width: 100%;
        flex: 1;
        min-height: 0;
        overflow-y: auto;
    }

    .back-btn {
        position: sticky;
        top: 0;
        left: 20px;
        margin: 16px 0 0 0;
        padding: 6px 12px;
        border: 1px solid #333;
        border-radius: 4px;
        background: #161616;
        color: #aaa;
        font-size: 13px;
        cursor: pointer;
        z-index: 100;
        transition: all 0.15s;
    }

    .back-btn:hover {
        background: #1d1d1d;
        color: #fff;
    }
</style>
