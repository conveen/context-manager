import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CONTEXTS_CHANGED, SHOW_SETTINGS } from "./lib/events";
import { makeAppData, makeContext, makeMain } from "./lib/testing";

// ── Fake webview window ────────────────────────────────────────────────────
// Records event listeners so tests can fire backend events, and stubs the
// size/focus APIs the resize-on-blur logic uses.
const fake = vi.hoisted(() => {
    type Handler = (event: { payload: unknown }) => void;
    const listeners = new Map<string, Handler[]>();
    const focusHandlers: Handler[] = [];
    const setSizeCalls: Array<{ width: number; height: number }> = [];
    return {
        listeners,
        focusHandlers,
        setSizeCalls,
        reset() {
            listeners.clear();
            focusHandlers.length = 0;
            setSizeCalls.length = 0;
        },
        async emit(event: string, payload: unknown = null) {
            for (const handler of listeners.get(event) ?? []) {
                handler({ payload });
            }
        },
        window: {
            listen: (event: string, handler: Handler) => {
                const list = fake.listeners.get(event) ?? [];
                list.push(handler);
                fake.listeners.set(event, list);
                return Promise.resolve(() => {});
            },
            onFocusChanged: (handler: Handler) => {
                fake.focusHandlers.push(handler);
                return Promise.resolve(() => {});
            },
            scaleFactor: () => Promise.resolve(1),
            outerSize: () =>
                Promise.resolve({
                    toLogical: () => ({ width: 400, height: 600 }),
                }),
            setSize: (size: { width: number; height: number }) => {
                fake.setSizeCalls.push({ width: size.width, height: size.height });
                return Promise.resolve();
            },
        },
    };
});

vi.mock("@tauri-apps/api/webviewWindow", () => ({
    getCurrentWebviewWindow: () => fake.window,
}));

vi.mock("./lib/api", () => ({
    getAppData: vi.fn(),
    createContext: vi.fn(),
    openDevtools: vi.fn(),
}));

import App from "./App.svelte";
import * as api from "./lib/api";

const mockedGetAppData = vi.mocked(api.getAppData);

function defaultData() {
    return makeAppData([
        makeMain({ order: 0 }),
        // Unassigned tier order deliberately disagrees with array order.
        makeContext("b", { order: 2 }),
        makeContext("c", { order: 1 }),
        makeContext("s9", { shortcut_index: 9, order: 3 }),
    ]);
}

describe("App", () => {
    beforeEach(() => {
        fake.reset();
        mockedGetAppData.mockResolvedValue(defaultData());
    });

    afterEach(() => {
        vi.clearAllMocks();
    });

    it("renders the sidebar in two-tier order after loading", async () => {
        const { container } = render(App);
        await waitFor(() => {
            expect(container.querySelectorAll(".ctx-item").length).toBe(4);
        });
        const names = [...container.querySelectorAll(".ctx-name")].map((el) => el.textContent?.trim());
        // Shortcut tier by index (main=0, then 9), then unassigned by `order`.
        expect(names).toEqual(["main", "name-s9", "name-c", "name-b"]);
    });

    it("switches to the settings pane when the backend emits show-settings", async () => {
        render(App);
        await waitFor(() => {
            expect(fake.listeners.has(SHOW_SETTINGS)).toBe(true);
        });
        await fake.emit(SHOW_SETTINGS);
        await waitFor(() => {
            expect(screen.getByText("← Back")).toBeTruthy();
        });
        // Leaving settings restores the main view.
        await fireEvent.click(screen.getByText("← Back"));
        expect(screen.queryByText("← Back")).toBeNull();
    });

    it("opens settings on Ctrl+, and Cmd+, but not on other combinations", async () => {
        // Regression for #73: the native menu accelerator never fires on
        // Windows, so the webview handles the shortcut itself.
        render(App);
        await fireEvent.keyDown(window, { key: ",", altKey: true, ctrlKey: true });
        await fireEvent.keyDown(window, { key: "," });
        expect(screen.queryByText("← Back")).toBeNull();

        await fireEvent.keyDown(window, { key: ",", ctrlKey: true });
        await waitFor(() => {
            expect(screen.getByText("← Back")).toBeTruthy();
        });

        await fireEvent.click(screen.getByText("← Back"));
        await fireEvent.keyDown(window, { key: ",", metaKey: true });
        await waitFor(() => {
            expect(screen.getByText("← Back")).toBeTruthy();
        });
    });

    it("refreshes immediately when the backend emits contexts-changed", async () => {
        render(App);
        await waitFor(() => {
            expect(fake.listeners.has(CONTEXTS_CHANGED)).toBe(true);
        });
        const callsBefore = mockedGetAppData.mock.calls.length;
        await fake.emit(CONTEXTS_CHANGED);
        await waitFor(() => {
            expect(mockedGetAppData.mock.calls.length).toBeGreaterThan(callsBefore);
        });
    });

    it("collapses on blur and restores the captured width on focus", async () => {
        render(App);
        await waitFor(() => {
            expect(fake.focusHandlers.length).toBeGreaterThan(0);
        });

        for (const handler of fake.focusHandlers) {
            handler({ payload: false });
        }
        await waitFor(() => {
            expect(fake.setSizeCalls.at(-1)?.width).toBe(84);
        });

        for (const handler of fake.focusHandlers) {
            handler({ payload: true });
        }
        await waitFor(() => {
            // Restored to the width captured before collapsing (fake: 400).
            expect(fake.setSizeCalls.at(-1)?.width).toBe(400);
        });
    });

    it("clears the selection when the selected context disappears", async () => {
        const { container } = render(App);
        await waitFor(() => {
            expect(container.querySelectorAll(".ctx-item").length).toBe(4);
        });
        // Select context b, then have the backend report it deleted.
        const items = [...container.querySelectorAll(".ctx-item")];
        const b = items.find((el) => el.textContent?.includes("name-b"));
        if (!b) throw new Error("context b not rendered");
        await fireEvent.click(b);
        expect(container.querySelector(".ctx-item.selected")?.textContent).toContain("name-b");

        mockedGetAppData.mockResolvedValue(makeAppData([makeMain()]));
        await fake.emit(CONTEXTS_CHANGED);
        await waitFor(() => {
            expect(container.querySelector(".ctx-item.selected")).toBeNull();
        });
    });
});
