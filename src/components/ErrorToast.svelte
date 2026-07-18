<script lang="ts">
import { dismissError, toast } from "../lib/toast.svelte";
</script>

{#if toast.message}
    <div class="toast" role="alert">
        <span class="toast-icon">⚠️</span>
        <span class="toast-text">{toast.message}</span>
        <button class="toast-close" onclick={dismissError} title="Dismiss">×</button>
    </div>
{/if}

<style>
    /* Matches Settings.svelte's error banner palette. */
    .toast {
        position: fixed;
        bottom: 16px;
        left: 50%;
        transform: translateX(-50%);
        z-index: 1000;
        display: flex;
        align-items: center;
        gap: 10px;
        max-width: min(520px, calc(100vw - 32px));
        padding: 10px 14px;
        border-radius: 6px;
        background: #3a1a1a;
        color: #ff6b6b;
        border: 1px solid #5a2a2a;
        box-shadow: 0 6px 20px rgba(0, 0, 0, 0.6);
        font-family: system-ui, sans-serif;
        font-size: 13px;
        font-weight: 500;
    }

    .toast-icon {
        font-size: 15px;
        flex-shrink: 0;
    }

    .toast-text {
        overflow-wrap: anywhere;
    }

    .toast-close {
        flex-shrink: 0;
        background: none;
        border: none;
        color: #ff6b6b;
        font-size: 16px;
        line-height: 1;
        padding: 2px 4px;
        cursor: pointer;
        border-radius: 3px;
    }
    .toast-close:hover {
        background: #4a2020;
    }
</style>
