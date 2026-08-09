<script lang="ts">
import type { ScreenRecordingStatus } from "../lib/types";
import * as api from "../lib/api";
import { showError } from "../lib/toast.svelte";

interface Props {
    // "Granted" renders nothing; the parent still mounts this component so the
    // banner appears the moment the poll reports a problem.
    status: ScreenRecordingStatus;
}

const { status }: Props = $props();

// Both failing states need the same recovery (grant, then relaunch); they
// differ only in what the user is told is wrong, since someone who has already
// granted the permission would otherwise be told to do what they just did.
const DENIED = "Context Manager needs Screen Recording permission to see your windows.";
const NOT_IN_EFFECT = "Screen Recording permission is granted, but not in effect for this build.";
const message = $derived(status === "Denied" ? DENIED : NOT_IN_EFFECT);

async function openSettings() {
    try {
        await api.openScreenRecordingSettings();
    } catch (e) {
        showError(String(e));
    }
}
</script>

{#if status !== "Granted"}
    <div class="banner" role="alert">
        <span class="banner-icon">⚠️</span>
        <div class="banner-body">
            <p class="banner-message">{message}</p>
            <p class="banner-detail">Quit and relaunch the app after granting it — macOS doesn't apply the permission to an already-running process.</p>
        </div>
        <button class="banner-btn" onclick={openSettings}>Open System Settings</button>
    </div>
{/if}

<style>
    /* Palette matches ErrorToast.svelte / Settings.svelte's error banner. */
    .banner {
        display: flex;
        align-items: center;
        gap: 12px;
        flex-shrink: 0;
        padding: 10px 14px;
        background: #3a1a1a;
        color: #ff6b6b;
        border-bottom: 1px solid #5a2a2a;
        font-family: system-ui, sans-serif;
    }

    .banner-icon {
        font-size: 15px;
        flex-shrink: 0;
    }

    .banner-body {
        flex: 1;
        min-width: 0;
    }

    .banner-message {
        margin: 0;
        font-size: 13px;
        font-weight: 500;
    }

    .banner-detail {
        margin: 2px 0 0 0;
        font-size: 11px;
        color: #c98a8a;
    }

    .banner-btn {
        flex-shrink: 0;
        padding: 6px 12px;
        border: 1px solid #5a2a2a;
        border-radius: 4px;
        background: #4a2020;
        color: #ff9b9b;
        font-family: inherit;
        font-size: 12px;
        font-weight: 500;
        cursor: pointer;
        transition: background 0.15s, color 0.15s;
    }

    .banner-btn:hover {
        background: #5a2626;
        color: #fff;
    }
</style>
