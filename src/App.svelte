<script lang="ts">
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/window";
import * as api from "./lib/api";
import Sidebar from "./components/Sidebar.svelte";
import DetailPanel from "./components/DetailPanel.svelte";
import Settings from "./Settings.svelte";

let appData = $state<Awaited<ReturnType<typeof api.getAppData>> | null>(null);
let selectedId = $state<string | null>(null);
let showDetailPanel = $state(true);
let showSettings = $state(false);

async function refresh() {
    try {
        appData = await api.getAppData();
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

// Listen for window focus/blur to resize and show/hide the detail panel.
// On blur we capture the current width before collapsing, and on focus we
// restore to that width — so the app respects whatever width the user or
// window manager chose instead of snapping to a hard-coded value.
$effect(() => {
    let unlisten: (() => void) | null = null;
    const COLLAPSED_WIDTH = 84;
    // Logical px. Seeded with a sensible default and overwritten with the real
    // width whenever the window loses focus.
    let expandedWidth = 900;

    (async () => {
        const appWindow = getCurrentWebviewWindow();
        unlisten = await appWindow.onFocusChanged(async (event) => {
            const focused = event.payload;
            // outerSize() is in physical pixels; convert to logical so setSize
            // (which takes logical) is stable across repeated resizes on HiDPI.
            const factor = await appWindow.scaleFactor();
            const size = (await appWindow.outerSize()).toLogical(factor);
            if (focused) {
                showDetailPanel = true;
                appWindow.setSize(new LogicalSize(expandedWidth, size.height));
            } else {
                // Capture the current width before collapsing so we can restore it.
                expandedWidth = size.width;
                showDetailPanel = false;
                appWindow.setSize(new LogicalSize(COLLAPSED_WIDTH, size.height));
            }
        });
    })();

    return () => {
        if (unlisten) unlisten();
    };
});

// Listen for the backend's request to show the settings panel.
$effect(() => {
    let unlisten: (() => void) | null = null;

    (async () => {
        const appWindow = getCurrentWebviewWindow();
        unlisten = await appWindow.listen("show-settings", () => {
            showSettings = true;
        });
    })();

    return () => {
        if (unlisten) unlisten();
    };
});

// Refresh immediately when the backend changes context visibility
// (e.g. via global shortcuts) instead of waiting for the periodic poll.
$effect(() => {
    let unlisten: (() => void) | null = null;

    (async () => {
        const appWindow = getCurrentWebviewWindow();
        unlisten = await appWindow.listen("contexts-changed", () => {
            refresh();
        });
    })();

    return () => {
        if (unlisten) unlisten();
    };
});

const mainContext = $derived(appData?.contexts.find((c) => c.is_main) ?? null);
const userContexts = $derived(appData?.contexts.filter((c) => !c.is_main) ?? []);
// Pin main context at top of sidebar
const sidebarContexts = $derived(mainContext ? [mainContext, ...userContexts] : userContexts);
const selectedContext = $derived(appData?.contexts.find((c) => c.id === selectedId) ?? null);

async function handleCreate() {
    try {
        const ctx = await api.createContext();
        await refresh();
        selectedId = ctx.id;
    } catch (e) {
        if (import.meta.env.DEV) console.error(e);
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
    {#if showSettings}
        <div class="settings-view">
            <button class="back-btn" onclick={closeSettings} title="Back">← Back</button>
            <Settings />
        </div>
    {:else}
        {#if import.meta.env.DEV && showDetailPanel}
            <button class="devtools-btn" onclick={handleOpenDevtools} title="Open DevTools">⚙️</button>
        {/if}
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
    {/if}
</div>

<style>
    .app {
        display: flex;
        height: 100vh;
        overflow: hidden;
        position: relative;
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
        height: 100%;
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
