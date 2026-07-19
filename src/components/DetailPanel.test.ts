import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dismissError, toast } from "../lib/toast.svelte";
import { makeContext, makeMain, makeWindow } from "../lib/testing";

vi.mock("../lib/api", () => ({
    addWindowToContext: vi.fn(),
    removeWindowFromContext: vi.fn(),
    showContext: vi.fn(),
    hideContext: vi.fn(),
}));

import * as api from "../lib/api";
import DetailPanel from "./DetailPanel.svelte";

// Main tracks windows 1-3; the selected context holds window 1, so Available
// must list exactly 2 and 3.
function props() {
    const context = makeContext("a", { windows: [makeWindow(1)] });
    const mainContext = makeMain({ windows: [makeWindow(1), makeWindow(2), makeWindow(3)] });
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    return { context, mainContext, onRefresh };
}

// svelte-dnd-action reports drops via consider/finalize custom events; the
// gesture itself can't run in jsdom, so tests dispatch what the library would
// emit. Trigger names mirror TRIGGERS in svelte-dnd-action.
function finalize(zone: Element, id: number, trigger: string) {
    zone.dispatchEvent(
        new CustomEvent("finalize", {
            detail: { items: [], info: { trigger, id: String(id) } },
        }),
    );
}

describe("DetailPanel", () => {
    beforeEach(() => {
        vi.mocked(api.addWindowToContext).mockResolvedValue(undefined);
        vi.mocked(api.removeWindowFromContext).mockResolvedValue(undefined);
        vi.mocked(api.showContext).mockResolvedValue(undefined);
        vi.mocked(api.hideContext).mockResolvedValue(undefined);
    });

    afterEach(() => {
        dismissError();
        vi.clearAllMocks();
    });

    it("prompts for a selection when no context is chosen", () => {
        const { onRefresh } = props();
        render(DetailPanel, { props: { context: null, mainContext: makeMain(), onRefresh } });
        expect(screen.getByText(/Select a context/)).toBeTruthy();
    });

    it("lists the context's windows and derives Available as Main minus members", () => {
        const { container } = render(DetailPanel, { props: props() });
        const zones = container.querySelectorAll(".drop-zone");
        const ctxTitles = [...zones[0].querySelectorAll(".win-title")].map((el) => el.textContent);
        const availTitles = [...zones[1].querySelectorAll(".win-title")].map((el) => el.textContent);
        expect(ctxTitles).toEqual(["Win1"]);
        expect(availTitles).toEqual(["Win2", "Win3"]);
        expect(screen.getByText("Context windows (1)")).toBeTruthy();
        expect(screen.getByText("Available windows (2)")).toBeTruthy();
    });

    it("toggles visibility through the header button", async () => {
        const p = props();
        const { container } = render(DetailPanel, { props: p });
        const btn = container.querySelector(".vis-btn");
        if (!btn) throw new Error("visibility button missing");
        await fireEvent.click(btn);
        await waitFor(() => {
            expect(api.hideContext).toHaveBeenCalledWith("a");
        });
        expect(p.onRefresh).toHaveBeenCalled();

        vi.clearAllMocks();
        const hidden = { ...p, context: makeContext("a", { visible: false }) };
        const second = render(DetailPanel, { props: hidden });
        const showBtn = second.container.querySelector(".vis-btn");
        if (!showBtn) throw new Error("visibility button missing");
        await fireEvent.click(showBtn);
        await waitFor(() => {
            expect(api.showContext).toHaveBeenCalledWith("a");
        });
    });

    it("adds a window dropped into the context zone (move by default)", async () => {
        const p = props();
        const { container } = render(DetailPanel, { props: p });
        finalize(container.querySelectorAll(".drop-zone")[0], 2, "droppedIntoZone");
        await waitFor(() => {
            expect(api.addWindowToContext).toHaveBeenCalledWith("a", 2, false);
        });
        expect(p.onRefresh).toHaveBeenCalled();
    });

    it("copies instead of moving while Shift is held", async () => {
        const { container } = render(DetailPanel, { props: props() });
        await fireEvent.keyDown(window, { key: "Shift", shiftKey: true });
        finalize(container.querySelectorAll(".drop-zone")[0], 3, "droppedIntoZone");
        await waitFor(() => {
            expect(api.addWindowToContext).toHaveBeenCalledWith("a", 3, true);
        });
        // Releasing Shift reverts to move semantics.
        await fireEvent.keyUp(window, { key: "Shift", shiftKey: false });
        finalize(container.querySelectorAll(".drop-zone")[0], 2, "droppedIntoZone");
        await waitFor(() => {
            expect(api.addWindowToContext).toHaveBeenCalledWith("a", 2, false);
        });
    });

    it("removes a window dropped into the available zone", async () => {
        const { container } = render(DetailPanel, { props: props() });
        finalize(container.querySelectorAll(".drop-zone")[1], 1, "droppedIntoZone");
        await waitFor(() => {
            expect(api.removeWindowFromContext).toHaveBeenCalledWith("a", 1);
        });
    });

    it("surfaces a rejected drop as a toast and refreshes to snap back", async () => {
        vi.mocked(api.addWindowToContext).mockRejectedValue("window 99 is not tracked in any context");
        const p = props();
        const { container } = render(DetailPanel, { props: p });
        finalize(container.querySelectorAll(".drop-zone")[0], 99, "droppedIntoZone");
        await waitFor(() => {
            expect(toast.message).toContain("not tracked");
        });
        expect(p.onRefresh).toHaveBeenCalled();
    });

    it("ignores cancelled drags without calling the backend", async () => {
        const { container } = render(DetailPanel, { props: props() });
        finalize(container.querySelectorAll(".drop-zone")[0], 2, "dragStopped");
        await new Promise((resolve) => setTimeout(resolve, 0));
        expect(api.addWindowToContext).not.toHaveBeenCalled();
        expect(api.removeWindowFromContext).not.toHaveBeenCalled();
    });
});
