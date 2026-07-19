import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeAppData, makeContext, makeMain, makeSettings } from "./lib/testing";

vi.mock("./lib/api", () => ({
    getAppData: vi.fn(),
    updateSettings: vi.fn(),
}));

import * as api from "./lib/api";
import Settings from "./Settings.svelte";

const mockedGetAppData = vi.mocked(api.getAppData);
const mockedUpdateSettings = vi.mocked(api.updateSettings);

describe("Settings", () => {
    beforeEach(() => {
        mockedGetAppData.mockResolvedValue(
            makeAppData([makeMain(), makeContext("a"), makeContext("b")], makeSettings()),
        );
        mockedUpdateSettings.mockResolvedValue(undefined);
    });

    afterEach(() => {
        vi.clearAllMocks();
    });

    async function renderLoaded() {
        const result = render(Settings);
        await waitFor(() => {
            expect(screen.queryByText("Loading settings…")).toBeNull();
        });
        return result;
    }

    it("marks the stored meta key active after loading", async () => {
        await renderLoaded();
        const ctrlAlt = screen.getByText("Ctrl+Alt").closest("button");
        const cmdOpt = screen.getByText("Cmd+Opt").closest("button");
        expect(ctrlAlt?.classList.contains("active")).toBe(true);
        expect(cmdOpt?.classList.contains("active")).toBe(false);
    });

    it("saves a merged settings object when the meta key is changed", async () => {
        await renderLoaded();
        const cmdOpt = screen.getByText("Cmd+Opt").closest("button");
        if (!cmdOpt) throw new Error("Cmd+Opt button not rendered");
        await fireEvent.click(cmdOpt);

        await waitFor(() => {
            expect(mockedUpdateSettings).toHaveBeenCalledWith(makeSettings({ meta_key: "CmdOpt" }));
        });
        // Optimistic update + success banner.
        expect(cmdOpt.classList.contains("active")).toBe(true);
        expect(screen.getByText("Settings saved")).toBeTruthy();
    });

    it("saves the single-context toggle and choice", async () => {
        await renderLoaded();
        await fireEvent.click(screen.getByRole("checkbox"));
        await waitFor(() => {
            expect(mockedUpdateSettings).toHaveBeenCalledWith(makeSettings({ single_context_mode: true }));
        });

        await fireEvent.change(screen.getByLabelText("Show:"), { target: { value: "b" } });
        await waitFor(() => {
            expect(mockedUpdateSettings).toHaveBeenCalledWith(
                makeSettings({ single_context_mode: true, single_context_id: "b" }),
            );
        });
    });

    it("falls back to Main in the dropdown when the stored choice is stale", async () => {
        mockedGetAppData.mockResolvedValue(
            makeAppData([makeMain(), makeContext("a")], makeSettings({ single_context_id: "deleted-context" })),
        );
        await renderLoaded();
        const select = screen.getByLabelText("Show:") as HTMLSelectElement;
        expect(select.value).toBe("main-id");
    });

    it("shows the backend error and keeps the stored settings when a save fails", async () => {
        mockedUpdateSettings.mockRejectedValue("failed to apply the new shortcut modifier: claimed");
        await renderLoaded();
        const cmdOpt = screen.getByText("Cmd+Opt").closest("button");
        if (!cmdOpt) throw new Error("Cmd+Opt button not rendered");
        await fireEvent.click(cmdOpt);

        await waitFor(() => {
            expect(screen.getByText(/failed to apply the new shortcut modifier/)).toBeTruthy();
        });
        expect(cmdOpt.classList.contains("active")).toBe(false);
        expect(screen.queryByText("Settings saved")).toBeNull();
    });

    it("shows the load error when fetching settings fails", async () => {
        mockedGetAppData.mockRejectedValue("boom");
        render(Settings);
        await waitFor(() => {
            expect(screen.getByText(/boom/)).toBeTruthy();
        });
    });
});
