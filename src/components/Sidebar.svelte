<script lang="ts">
import type { Context } from "../lib/types";
import * as api from "../lib/api";
import { hueFor } from "../lib/color";

interface Props {
    contexts: Context[];
    selectedId: string | null;
    onSelect: (id: string) => void;
    onCreate: () => void;
    onRefresh: () => Promise<void>;
}

const { contexts, selectedId, onSelect, onCreate, onRefresh }: Props = $props();

// ── Context menu ──────────────────────────────────────────────────────────
let menu: { x: number; y: number; ctx: Context } | null = $state(null);
let renamingId: string | null = $state(null);
let renameValue = $state("");

function openMenu(e: MouseEvent, ctx: Context) {
    e.preventDefault();
    // Prevent context menu on main context
    if (ctx.is_main) return;
    menu = { x: e.clientX, y: e.clientY, ctx };
}

function closeMenu() {
    menu = null;
}

async function handleAssignShortcut(id: string, index: number | null) {
    closeMenu();
    try {
        await api.assignShortcut(id, index);
        await onRefresh();
    } catch (e) {
        console.error(e);
    }
}

function startRename(ctx: Context) {
    closeMenu();
    renamingId = ctx.id;
    renameValue = ctx.name;
}

async function submitRename(id: string) {
    const trimmed = renameValue.trim();
    renamingId = null;
    if (!trimmed) return;
    try {
        await api.renameContext(id, trimmed);
        await onRefresh();
    } catch (e) {
        console.error(e);
    }
}

async function handleDelete(id: string) {
    closeMenu();
    try {
        await api.deleteContext(id);
        await onRefresh();
    } catch (e) {
        console.error(e);
    }
}
</script>

<!-- Context menu backdrop + popup -->
{#if menu}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="backdrop" role="presentation" onclick={closeMenu}></div>
    <div class="ctx-menu" style="left:{menu.x}px;top:{menu.y}px" role="menu">
        <div class="sc-grid">
            {#each [1,2,3,4,5,6,7,8,9] as n (n)}
                <button
                    class="sc-btn"
                    class:sc-active={menu.ctx.shortcut_index === n}
                    onclick={() => handleAssignShortcut(menu!.ctx.id, n)}
                    title="Assign shortcut {n}"
                >{n}</button>
            {/each}
            <button
                class="sc-btn sc-clear"
                onclick={() => handleAssignShortcut(menu!.ctx.id, null)}
                title="Clear shortcut"
            >×</button>
        </div>
        <div class="menu-sep"></div>
        <button class="menu-item" onclick={() => startRename(menu!.ctx)}>Rename</button>
        <button class="menu-item danger" onclick={() => handleDelete(menu!.ctx.id)}>Delete</button>
    </div>
{/if}

<aside class="sidebar">
    {#each contexts as ctx (ctx.id)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
            class="ctx-item"
            class:selected={selectedId === ctx.id}
            role="button"
            tabindex="0"
            onclick={() => onSelect(ctx.id)}
            onkeydown={(e) => e.key === 'Enter' && onSelect(ctx.id)}
            oncontextmenu={(e) => openMenu(e, ctx)}
            title={ctx.name}
        >
            <div class="thumb" style="background:hsl({hueFor(ctx.id)},50%,36%)">
                {#if ctx.shortcut_index !== null}
                    <span class="sc-badge">{ctx.shortcut_index}</span>
                {/if}
            </div>

            {#if renamingId === ctx.id}
                <!-- svelte-ignore a11y_autofocus -->
                <input
                    class="rename-input"
                    bind:value={renameValue}
                    autofocus
                    onclick={(e) => e.stopPropagation()}
                    onblur={() => submitRename(ctx.id)}
                    onkeydown={(e) => {
                        if (e.key === 'Enter') submitRename(ctx.id);
                        else if (e.key === 'Escape') renamingId = null;
                    }}
                />
            {:else}
                <span class="ctx-name" ondblclick={!ctx.is_main ? () => startRename(ctx) : undefined}>
                    {ctx.name}
                </span>
            {/if}

            <div class="vis-dot" class:vis-on={ctx.visible}></div>
        </div>
    {/each}

    <button class="add-btn" onclick={onCreate} title="New context">+</button>
</aside>

<style>
    .sidebar {
        width: 84px;
        min-width: 84px;
        display: flex;
        flex-direction: column;
        align-items: center;
        padding: 8px 0;
        background: #111;
        border-right: 1px solid #222;
        overflow-y: auto;
        gap: 4px;
    }

    .ctx-item {
        position: relative;
        width: 68px;
        cursor: pointer;
        border-radius: 6px;
        padding: 4px;
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 3px;
        user-select: none;
    }
    .ctx-item:hover { background: #1d1d1d; }
    .ctx-item.selected { background: #252525; }

    .thumb {
        width: 56px;
        height: 46px;
        border-radius: 4px;
        position: relative;
        flex-shrink: 0;
    }

    .sc-badge {
        position: absolute;
        bottom: 3px;
        right: 3px;
        font-size: 10px;
        font-weight: 700;
        background: rgba(0,0,0,0.55);
        color: #fff;
        border-radius: 3px;
        padding: 0 3px;
        line-height: 15px;
        font-family: system-ui, sans-serif;
    }

    .ctx-name {
        font-size: 10px;
        color: #888;
        text-align: center;
        width: 100%;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-family: system-ui, sans-serif;
    }

    .rename-input {
        font-size: 10px;
        width: 100%;
        background: #252525;
        border: 1px solid #555;
        border-radius: 3px;
        color: #fff;
        padding: 1px 3px;
        font-family: system-ui, sans-serif;
        outline: none;
    }

    .vis-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: #383838;
        position: absolute;
        top: 6px;
        right: 6px;
    }
    .vis-dot.vis-on { background: #4caf50; }

    .add-btn {
        margin-top: auto;
        width: 36px;
        height: 36px;
        border-radius: 50%;
        border: 1px solid #2e2e2e;
        background: transparent;
        color: #555;
        font-size: 22px;
        line-height: 1;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
        transition: color 0.1s, border-color 0.1s;
    }
    .add-btn:hover { color: #aaa; border-color: #555; }

    /* ── Context menu ────────────────────────────────────────────────────── */
    .backdrop {
        position: fixed;
        inset: 0;
        z-index: 99;
    }

    .ctx-menu {
        position: fixed;
        z-index: 100;
        background: #1e1e1e;
        border: 1px solid #333;
        border-radius: 7px;
        padding: 7px;
        min-width: 148px;
        box-shadow: 0 6px 20px rgba(0,0,0,0.6);
        font-family: system-ui, sans-serif;
    }

    .sc-grid {
        display: grid;
        grid-template-columns: repeat(5, 1fr);
        gap: 3px;
        margin-bottom: 6px;
    }

    .sc-btn {
        height: 24px;
        border-radius: 4px;
        border: 1px solid #333;
        background: #161616;
        color: #aaa;
        font-size: 11px;
        font-weight: 500;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 0;
        transition: background 0.1s;
        font-family: system-ui, sans-serif;
    }
    .sc-btn:hover { background: #2a2a2a; color: #fff; }
    .sc-btn.sc-active { background: #0060c0; border-color: #0070dd; color: #fff; }
    .sc-clear { color: #e05; font-size: 14px; }

    .menu-sep {
        height: 1px;
        background: #2a2a2a;
        margin: 4px 0;
    }

    .menu-item {
        display: block;
        width: 100%;
        text-align: left;
        padding: 5px 8px;
        border-radius: 4px;
        border: none;
        background: transparent;
        color: #bbb;
        font-size: 12px;
        cursor: pointer;
        font-family: system-ui, sans-serif;
    }
    .menu-item:hover { background: #2a2a2a; color: #fff; }
    .menu-item.danger { color: #e05; }
    .menu-item.danger:hover { background: #2a1010; }
</style>
