import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dismissError, toast } from "../lib/toast.svelte";
import { makeContext, makeMain } from "../lib/testing";

vi.mock("../lib/api", () => ({
    reorderContexts: vi.fn(),
    assignShortcut: vi.fn(),
    renameContext: vi.fn(),
    deleteContext: vi.fn(),
}));

import * as api from "../lib/api";
import Sidebar from "./Sidebar.svelte";

function contexts() {
    return [
        makeMain({ visible: true }),
        makeContext("s2", { shortcut_index: 2 }),
        makeContext("a", { order: 0, visible: false }),
        makeContext("b", { order: 1 }),
    ];
}

function renderSidebar(overrides: Partial<Parameters<typeof render>[1]> = {}) {
    const onSelect = vi.fn();
    const onCreate = vi.fn();
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    const result = render(Sidebar, {
        props: { contexts: contexts(), selectedId: null, onSelect, onCreate, onRefresh, ...overrides },
    });
    return { ...result, onSelect, onCreate, onRefresh };
}

describe("Sidebar", () => {
    beforeEach(() => {
        vi.mocked(api.reorderContexts).mockResolvedValue(undefined);
        vi.mocked(api.assignShortcut).mockResolvedValue(undefined);
        vi.mocked(api.renameContext).mockResolvedValue(undefined);
        vi.mocked(api.deleteContext).mockResolvedValue(undefined);
    });

    afterEach(() => {
        dismissError();
        vi.clearAllMocks();
    });

    it("renders both tiers with shortcut badges and visibility dots", () => {
        const { container } = renderSidebar();
        const items = [...container.querySelectorAll(".ctx-item")];
        expect(items.map((el) => el.querySelector(".ctx-name")?.textContent?.trim())).toEqual([
            "main",
            "name-s2",
            "name-a",
            "name-b",
        ]);
        // Badges only on the shortcut tier.
        expect(items[0].querySelector(".sc-badge")?.textContent).toBe("0");
        expect(items[1].querySelector(".sc-badge")?.textContent).toBe("2");
        expect(items[2].querySelector(".sc-badge")).toBeNull();
        // Visibility indicator reflects `visible`.
        expect(items[0].querySelector(".vis-dot")?.classList.contains("vis-on")).toBe(true);
        expect(items[2].querySelector(".vis-dot")?.classList.contains("vis-on")).toBe(false);
    });

    it("selects a context on click", async () => {
        const { container, onSelect } = renderSidebar();
        const b = [...container.querySelectorAll(".ctx-item")].find((el) => el.textContent?.includes("name-b"));
        if (!b) throw new Error("context b not rendered");
        await fireEvent.click(b);
        expect(onSelect).toHaveBeenCalledWith("b");
    });

    it("opens the context menu on right-click, but never for Main", async () => {
        const { container } = renderSidebar();
        const items = [...container.querySelectorAll(".ctx-item")];
        await fireEvent.contextMenu(items[0]);
        expect(container.querySelector(".ctx-menu")).toBeNull();

        await fireEvent.contextMenu(items[2]);
        expect(container.querySelector(".ctx-menu")).toBeTruthy();
    });

    it("assigns a shortcut from the menu and refreshes", async () => {
        const { container, onRefresh } = renderSidebar();
        const a = [...container.querySelectorAll(".ctx-item")][2];
        await fireEvent.contextMenu(a);
        await fireEvent.click(screen.getByTitle("Assign shortcut 3"));

        await waitFor(() => {
            expect(api.assignShortcut).toHaveBeenCalledWith("a", 3);
        });
        expect(onRefresh).toHaveBeenCalled();
        expect(container.querySelector(".ctx-menu")).toBeNull();
    });

    it("renames via double-click and Enter, trimming the input", async () => {
        const { container, onRefresh } = renderSidebar();
        const a = [...container.querySelectorAll(".ctx-item")][2];
        const nameEl = a.querySelector(".ctx-name");
        if (!nameEl) throw new Error("name element missing");
        await fireEvent.dblClick(nameEl);

        const input = container.querySelector(".rename-input") as HTMLInputElement;
        expect(input.value).toBe("name-a");
        await fireEvent.input(input, { target: { value: "  focus  " } });
        await fireEvent.keyDown(input, { key: "Enter" });

        await waitFor(() => {
            expect(api.renameContext).toHaveBeenCalledWith("a", "focus");
        });
        expect(onRefresh).toHaveBeenCalled();
    });

    it("cancels a rename with Escape", async () => {
        const { container } = renderSidebar();
        const a = [...container.querySelectorAll(".ctx-item")][2];
        const nameEl = a.querySelector(".ctx-name");
        if (!nameEl) throw new Error("name element missing");
        await fireEvent.dblClick(nameEl);
        const input = container.querySelector(".rename-input") as HTMLInputElement;
        await fireEvent.keyDown(input, { key: "Escape" });

        expect(container.querySelector(".rename-input")).toBeNull();
        expect(api.renameContext).not.toHaveBeenCalled();
    });

    it("deletes from the menu and surfaces failures as a toast", async () => {
        vi.mocked(api.deleteContext).mockRejectedValue("cannot delete");
        const { container, onRefresh } = renderSidebar();
        const a = [...container.querySelectorAll(".ctx-item")][2];
        await fireEvent.contextMenu(a);
        await fireEvent.click(screen.getByText("Delete"));

        await waitFor(() => {
            expect(toast.message).toContain("cannot delete");
        });
        // Unlike a rejected drag (which must snap optimistic state back), a
        // failed delete changed nothing — no refresh is issued.
        expect(onRefresh).not.toHaveBeenCalled();
    });

    it("submits the new order on drag finalize and snaps back on rejection", async () => {
        vi.mocked(api.reorderContexts).mockRejectedValue("context 'b' is not an unassigned (reorderable) context");
        const { container, onRefresh } = renderSidebar();
        const zone = container.querySelector(".free-zone");
        if (!zone) throw new Error("free zone not rendered");

        // svelte-dnd-action reports drags via consider/finalize custom events;
        // the gesture itself can't run in jsdom, so dispatch the events the
        // library would emit.
        const items = [makeContext("b", { order: 1 }), makeContext("a", { order: 0 })];
        zone.dispatchEvent(new CustomEvent("finalize", { detail: { items, info: { trigger: "droppedIntoZone" } } }));

        await waitFor(() => {
            expect(api.reorderContexts).toHaveBeenCalledWith(["b", "a"]);
        });
        await waitFor(() => {
            expect(toast.message).toContain("not an unassigned");
        });
        expect(onRefresh).toHaveBeenCalled();
    });
});
