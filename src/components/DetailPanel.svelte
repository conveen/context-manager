<script lang="ts">
import { dndzone, TRIGGERS } from "svelte-dnd-action";
import type { DndEvent } from "svelte-dnd-action";
import type { Context, WindowRef } from "../lib/types";
import * as api from "../lib/api";
import { showError } from "../lib/toast.svelte";

interface Props {
    context: Context | null;
    mainContext: Context | null;
    onRefresh: () => Promise<void>;
}

const { context, mainContext, onRefresh }: Props = $props();

// ── DnD item type ─────────────────────────────────────────────────────────
// svelte-dnd-action requires an `id` field; we map platform_id → id.
// isDndShadowItem is injected by the library during drag.
type DndItem = WindowRef & { id: number; isDndShadowItem?: boolean };

function toItems(windows: WindowRef[]): DndItem[] {
    return windows.map((w) => ({ ...w, id: w.platform_id }));
}

// ── Local DnD state ───────────────────────────────────────────────────────
let ctxItems: DndItem[] = $state([]);
let availItems: DndItem[] = $state([]);

// skipSync prevents the $effect from overwriting optimistic DnD state while
// a drag is in progress or an API call + refresh is pending.
let skipSync = $state(false);

// Tracks whether Shift is held so a drop can copy (keep in Main) rather than
// move. Read at finalize time; not used in the template, so a plain let.
let shiftHeld = false;

$effect(() => {
    const onKey = (e: KeyboardEvent) => {
        shiftHeld = e.shiftKey;
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    return () => {
        window.removeEventListener("keydown", onKey);
        window.removeEventListener("keyup", onKey);
    };
});

$effect(() => {
    const ctx = context;
    const main = mainContext;
    if (!ctx || !main || skipSync) return;

    const ctxIds = new Set(ctx.windows.map((w) => w.platform_id));
    ctxItems = toItems(ctx.windows);
    availItems = toItems(main.windows.filter((w) => !ctxIds.has(w.platform_id)));
});

// ── DnD handlers ─────────────────────────────────────────────────────────

function onCtxConsider(e: CustomEvent<DndEvent<DndItem>>) {
    skipSync = true;
    ctxItems = e.detail.items;
}

function onAvailConsider(e: CustomEvent<DndEvent<DndItem>>) {
    skipSync = true;
    availItems = e.detail.items;
}

function onCtxFinalize(e: CustomEvent<DndEvent<DndItem>>) {
    ctxItems = e.detail.items;
    const { trigger, id } = e.detail.info;
    if (trigger === TRIGGERS.DROPPED_INTO_ZONE && context) {
        const ctxId = context.id;
        const copy = shiftHeld;
        doWindowOp(() => api.addWindowToContext(ctxId, Number(id), copy));
    } else if (trigger !== TRIGGERS.DROPPED_INTO_ANOTHER) {
        // Drag cancelled (DRAG_STOPPED, DROPPED_OUTSIDE_OF_ANY, etc.)
        skipSync = false;
    }
    // DROPPED_INTO_ANOTHER: the avail zone's DROPPED_INTO_ZONE handler
    // will call doWindowOp, which releases skipSync when done.
}

function onAvailFinalize(e: CustomEvent<DndEvent<DndItem>>) {
    availItems = e.detail.items;
    const { trigger, id } = e.detail.info;
    if (trigger === TRIGGERS.DROPPED_INTO_ZONE && context) {
        const ctxId = context.id;
        doWindowOp(() => api.removeWindowFromContext(ctxId, Number(id)));
    } else if (trigger !== TRIGGERS.DROPPED_INTO_ANOTHER) {
        skipSync = false;
    }
}

// Runs the given API operation, then refreshes and releases skipSync.
// skipSync is assumed to already be true (set by the first consider event).
async function doWindowOp(op: () => Promise<void>) {
    try {
        await op();
    } catch (e) {
        showError(String(e));
    } finally {
        // Refresh on failure too, so a rejected drop snaps the optimistic
        // zone contents back to the backend's instead of lingering until the poll.
        await onRefresh();
        skipSync = false;
    }
}

// ── Visibility toggle ─────────────────────────────────────────────────────
async function toggleVisibility() {
    if (!context) return;
    try {
        if (context.visible) {
            await api.hideContext(context.id);
        } else {
            await api.showContext(context.id);
        }
        await onRefresh();
    } catch (e) {
        showError(String(e));
    }
}

// Count non-shadow items for the zone labels.
function realCount(items: DndItem[]): number {
    return items.filter((i) => !i.isDndShadowItem).length;
}
</script>

<main class="detail">
    {#if !context}
        <div class="empty">
            <p>Select a context from the sidebar,<br>or create a new one with <kbd>+</kbd>.</p>
        </div>
    {:else}
        <header class="detail-header">
            <span class="detail-title">{context.name}</span>
            {#if context.shortcut_index !== null}
                <span class="detail-sc">[{context.shortcut_index}]</span>
            {/if}
            <button
                class="vis-btn"
                class:vis-on={context.visible}
                onclick={toggleVisibility}
                title={context.visible ? 'Hide context' : 'Show context'}
            >{context.visible ? '●' : '○'}</button>
        </header>

        <div class="zones">
            <!-- ── Context windows zone ── -->
            <section class="zone">
                <div class="zone-label">Context windows ({realCount(ctxItems)})</div>
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                    class="drop-zone"
                    use:dndzone={{ items: ctxItems, flipDurationMs: 150, type: 'window', dropTargetStyle: {} }}
                    onconsider={onCtxConsider}
                    onfinalize={onCtxFinalize}
                >
                    {#each ctxItems as item (item.id)}
                        <div class="win-card" class:shadow={item.isDndShadowItem}>
                            <div class="win-app">{item.app_name}</div>
                            <div class="win-title">{item.window_title}</div>
                        </div>
                    {/each}
                    {#if ctxItems.length === 0}
                        <div class="zone-hint">Drag windows here to add them to this context</div>
                    {/if}
                </div>
            </section>

            <div class="zone-sep"></div>

            <!-- ── Available windows zone ── -->
            <section class="zone">
                <div class="zone-label">
                    <span>Available windows ({realCount(availItems)})</span>
                    {#if realCount(availItems) > 0}
                        <span class="zone-hint-inline">Hold <kbd>Shift</kbd> to copy</span>
                    {/if}
                </div>
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div
                    class="drop-zone"
                    use:dndzone={{ items: availItems, flipDurationMs: 150, type: 'window', dropTargetStyle: {} }}
                    onconsider={onAvailConsider}
                    onfinalize={onAvailFinalize}
                >
                    {#each availItems as item (item.id)}
                        <div class="win-card" class:shadow={item.isDndShadowItem}>
                            <div class="win-app">{item.app_name}</div>
                            <div class="win-title">{item.window_title}</div>
                        </div>
                    {/each}
                    {#if availItems.length === 0}
                        <div class="zone-hint">No other windows available</div>
                    {/if}
                </div>
            </section>
        </div>
    {/if}
</main>

<style>
    .detail {
        flex: 1;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        font-family: system-ui, sans-serif;
        min-width: 0;
    }

    .empty {
        flex: 1;
        display: flex;
        align-items: center;
        justify-content: center;
        color: #444;
        text-align: center;
        line-height: 1.7;
        font-size: 13px;
    }

    kbd {
        background: #252525;
        border: 1px solid #3a3a3a;
        border-radius: 3px;
        padding: 0 4px;
        font-size: 11px;
        font-family: system-ui, sans-serif;
        color: #888;
    }

    /* ── Header ─────────────────────────────────────────────────────────────── */
    .detail-header {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 11px 14px;
        border-bottom: 1px solid #222;
        flex-shrink: 0;
    }

    .detail-title {
        font-size: 15px;
        font-weight: 600;
        color: #ddd;
        flex: 1;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .detail-sc {
        font-size: 11px;
        color: #666;
        background: #1e1e1e;
        border: 1px solid #2e2e2e;
        padding: 1px 6px;
        border-radius: 4px;
        white-space: nowrap;
        font-family: monospace;
    }

    .vis-btn {
        background: none;
        border: none;
        cursor: pointer;
        font-size: 16px;
        color: #444;
        padding: 3px 5px;
        border-radius: 4px;
        line-height: 1;
        transition: color 0.15s, background 0.15s;
    }
    .vis-btn:hover { background: #222; color: #aaa; }
    .vis-btn.vis-on { color: #4caf50; }

    /* ── Zones ───────────────────────────────────────────────────────────────── */
    .zones {
        flex: 1;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        min-height: 0;
    }

    .zone {
        flex: 1;
        display: flex;
        flex-direction: column;
        padding: 10px 12px;
        min-height: 0;
        overflow: hidden;
    }

    .zone-label {
        font-size: 10px;
        font-weight: 700;
        color: #555;
        text-transform: uppercase;
        letter-spacing: 0.07em;
        margin-bottom: 6px;
        flex-shrink: 0;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }

    .zone-hint-inline {
        font-weight: 500;
        color: #444;
        text-transform: none;
        letter-spacing: 0;
        display: flex;
        align-items: center;
        gap: 4px;
    }

    .zone-hint-inline kbd {
        background: #202020;
        border: 1px solid #333;
        border-radius: 3px;
        padding: 0 4px;
        font-size: 10px;
        color: #888;
    }

    .drop-zone {
        flex: 1;
        overflow-y: auto;
        border: 1px dashed #262626;
        border-radius: 6px;
        padding: 5px;
        display: flex;
        flex-direction: column;
        gap: 3px;
        min-height: 48px;
        transition: border-color 0.15s;
    }
    .drop-zone:global(.dnd-drop-target) {
        border-color: #0060c0;
    }

    .zone-hint {
        color: #333;
        font-size: 12px;
        text-align: center;
        padding: 14px 0;
        flex-shrink: 0;
    }

    .zone-sep {
        height: 1px;
        background: #1a1a1a;
        margin: 0 12px;
        flex-shrink: 0;
    }

    /* ── Window card ─────────────────────────────────────────────────────────── */
    .win-card {
        background: #1c1c1c;
        border: 1px solid #272727;
        border-radius: 5px;
        padding: 6px 10px;
        cursor: grab;
        flex-shrink: 0;
        transition: background 0.1s, border-color 0.1s;
    }
    .win-card:hover { background: #222; border-color: #333; }
    .win-card:active { cursor: grabbing; }
    .win-card.shadow { opacity: 0.35; }

    .win-app {
        font-size: 10px;
        font-weight: 700;
        color: #777;
        margin-bottom: 1px;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }

    .win-title {
        font-size: 12px;
        color: #ccc;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
</style>
