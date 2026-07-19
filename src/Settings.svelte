<script lang="ts">
import type { Settings, Context } from "./lib/types";
import * as api from "./lib/api";

let settings = $state<Settings | null>(null);
let contexts = $state<Context[]>([]);
let loading = $state(true);
let saving = $state(false);
let error = $state<string | null>(null);
let success = $state(false);

// The Main Context's id, and the currently-chosen single-context id resolved
// against the live context list (a null or stale choice falls back to Main).
const mainId = $derived(contexts.find((c) => c.is_main)?.id ?? "");
const selectedCtxId = $derived.by(() => {
    const id = settings?.single_context_id;
    return id && contexts.some((c) => c.id === id) ? id : mainId;
});

async function loadSettings() {
    try {
        const data = await api.getAppData();
        settings = data.settings;
        contexts = data.contexts;
        error = null;
    } catch (e) {
        error = String(e);
    } finally {
        loading = false;
    }
}

// Shared by every setting control: merges the changed field(s) into the
// current settings and persists, guarding against saves before load or while
// one is already in flight.
async function saveField(patch: Partial<Settings>) {
    if (!settings || saving) return;
    const updatedSettings = { ...settings, ...patch };
    saving = true;
    error = null;
    success = false;
    try {
        await api.updateSettings(updatedSettings);
        settings = updatedSettings;
        success = true;
        setTimeout(() => {
            success = false;
        }, 2000);
    } catch (e) {
        error = String(e);
    } finally {
        saving = false;
    }
}

$effect(() => {
    loadSettings();
});
</script>

<div class="settings-container">
    <h1>Settings</h1>

    {#if loading}
        <div class="status loading-status">Loading settings…</div>
    {:else if error}
        <div class="status error-status">
            <span class="status-icon">⚠️</span>
            <span>{error}</span>
        </div>
    {/if}

    {#if success}
        <div class="status success-status">
            <span class="status-icon">✓</span>
            <span>Settings saved</span>
        </div>
    {/if}

    {#if settings && !loading}
        <div class="settings-content">
            <!-- Meta Key Setting -->
            <div class="setting-group">
                <div class="setting-header">
                    <h3>Keyboard Shortcut Modifier</h3>
                    <p class="description">Choose the modifier key for Context shortcuts (0-9, H)</p>
                </div>
                <div class="button-group">
                    <button
                        class="option-btn"
                        class:active={settings.meta_key === "CtrlAlt"}
                        disabled={saving}
                        onclick={() => saveField({ meta_key: "CtrlAlt" })}
                    >
                        <span class="option-name">Ctrl+Alt</span>
                        <span class="option-desc">Windows & Linux style</span>
                    </button>
                    <button
                        class="option-btn"
                        class:active={settings.meta_key === "CmdOpt"}
                        disabled={saving}
                        onclick={() => saveField({ meta_key: "CmdOpt" })}
                    >
                        <span class="option-name">Cmd+Opt</span>
                        <span class="option-desc">macOS native</span>
                    </button>
                </div>
            </div>

            <!-- Single Context Mode -->
            <div class="setting-group">
                <div class="setting-header">
                    <h3>Single Context Mode</h3>
                    <p class="description">Only show one Context at a time</p>
                </div>
                <div class="single-ctx-controls">
                    <label class="toggle-wrapper">
                        <input
                            type="checkbox"
                            checked={settings.single_context_mode}
                            disabled={saving}
                            onchange={(e) => saveField({ single_context_mode: e.currentTarget.checked })}
                        />
                        <span class="toggle-box"></span>
                        <span class="toggle-text">
                            {#if settings.single_context_mode}
                                Enabled
                            {:else}
                                Disabled
                            {/if}
                        </span>
                    </label>
                    <div class="ctx-select-wrap">
                        <label class="ctx-select-label" for="single-ctx-select">Show:</label>
                        <select
                            id="single-ctx-select"
                            class="ctx-select"
                            value={selectedCtxId}
                            disabled={saving}
                            onchange={(e) => saveField({ single_context_id: e.currentTarget.value })}
                        >
                            {#each contexts as ctx (ctx.id)}
                                <option value={ctx.id}>{ctx.name}</option>
                            {/each}
                        </select>
                    </div>
                </div>
                <p class="detail-text">
                    When enabled, showing a Context automatically hides all others. Turning it
                    on (or changing the choice above) switches to the selected Context and hides
                    the rest. Defaults to <strong>main</strong>.
                </p>
            </div>

        </div>
    {/if}
</div>

<style>
    .settings-container {
        background: #0f0f0f;
        color: #bbb;
        font-family: system-ui, sans-serif;
        min-height: 100vh;
        padding: 32px 24px;
        max-width: 600px;
        margin: 0 auto;
    }

    h1 {
        font-size: 28px;
        color: #fff;
        margin: 0 0 8px 0;
        font-weight: 600;
    }

    h3 {
        font-size: 15px;
        color: #fff;
        margin: 0;
        font-weight: 600;
    }

    .status {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 12px 16px;
        border-radius: 6px;
        margin-bottom: 20px;
        font-size: 13px;
        font-weight: 500;
    }

    .status-icon {
        font-size: 16px;
    }

    .loading-status {
        background: #1a2a3a;
        color: #6ba3d0;
        border: 1px solid #2a4a5a;
    }

    .error-status {
        background: #3a1a1a;
        color: #ff6b6b;
        border: 1px solid #5a2a2a;
    }

    .success-status {
        background: #1a3a2a;
        color: #4caf50;
        border: 1px solid #2a5a3a;
    }

    .settings-content {
        display: flex;
        flex-direction: column;
        gap: 28px;
    }

    .setting-group {
        padding: 20px;
        background: #1a1a1a;
        border: 1px solid #2a2a2a;
        border-radius: 8px;
    }

    .setting-header {
        margin-bottom: 16px;
    }

    .description {
        font-size: 12px;
        color: #666;
        margin: 4px 0 0 0;
    }

    .button-group {
        display: flex;
        gap: 12px;
    }

    .option-btn {
        flex: 1;
        padding: 12px 16px;
        border: 1.5px solid #333;
        border-radius: 6px;
        background: #111;
        color: #aaa;
        font-size: 13px;
        font-weight: 500;
        cursor: pointer;
        transition: all 0.2s;
        display: flex;
        flex-direction: column;
        gap: 4px;
        text-align: left;
    }

    .option-btn:hover:not(:disabled) {
        background: #1d1d1d;
        border-color: #444;
        color: #fff;
    }

    .option-btn.active {
        background: #0060c0;
        border-color: #0080ff;
        color: #fff;
    }

    .option-btn:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .option-name {
        font-weight: 600;
    }

    .option-desc {
        font-size: 11px;
        color: inherit;
        opacity: 0.8;
    }

    .single-ctx-controls {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
        margin-bottom: 12px;
    }

    .single-ctx-controls .toggle-wrapper {
        margin-bottom: 0;
    }

    .ctx-select-wrap {
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .ctx-select-label {
        font-size: 12px;
        color: #888;
    }

    .ctx-select {
        background: #111;
        color: #ddd;
        border: 1.5px solid #333;
        border-radius: 6px;
        padding: 8px 10px;
        font-size: 13px;
        font-family: inherit;
        cursor: pointer;
        min-width: 140px;
        transition: border-color 0.2s;
    }

    .ctx-select:hover:not(:disabled) {
        border-color: #444;
    }

    .ctx-select:focus {
        outline: none;
        border-color: #0080ff;
    }

    .ctx-select:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .toggle-wrapper {
        display: flex;
        align-items: center;
        gap: 12px;
        cursor: pointer;
        margin-bottom: 12px;
    }

    .toggle-wrapper input {
        display: none;
    }

    .toggle-box {
        width: 40px;
        height: 24px;
        background: #2a2a2a;
        border: 1.5px solid #3a3a3a;
        border-radius: 12px;
        transition: all 0.2s;
        position: relative;
        flex-shrink: 0;
    }

    .toggle-box::after {
        content: "";
        position: absolute;
        width: 20px;
        height: 20px;
        background: #666;
        border-radius: 10px;
        top: 2px;
        left: 2px;
        transition: all 0.2s;
    }

    .toggle-wrapper input:checked ~ .toggle-box {
        background: #0060c0;
        border-color: #0080ff;
    }

    .toggle-wrapper input:checked ~ .toggle-box::after {
        left: 18px;
        background: #fff;
    }

    .toggle-wrapper:has(input:disabled) {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .toggle-text {
        font-weight: 500;
        color: #bbb;
        min-width: 70px;
    }

    .detail-text {
        font-size: 12px;
        color: #666;
        margin: 0;
        line-height: 1.5;
    }
</style>
