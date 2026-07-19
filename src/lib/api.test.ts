import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import * as api from "./api";
import { makeSettings } from "./testing";

// Pins the command names and the snake_case→camelCase argument mapping the
// backend's #[tauri::command] handlers expect. A rename on either side breaks
// this suite.
describe("api command mapping", () => {
    afterEach(() => {
        clearMocks();
    });

    it("invokes each command with the expected name and arguments", async () => {
        const calls: Array<{ cmd: string; args: unknown }> = [];
        mockIPC((cmd, args) => {
            // No-arg commands surface as undefined or {} depending on the
            // invoke path; normalize — only the arg *mapping* is under test.
            const normalized = args && Object.keys(args).length > 0 ? args : undefined;
            calls.push({ cmd, args: normalized });
            return null;
        });

        const settings = makeSettings();
        await api.getAppData();
        await api.createContext();
        await api.renameContext("ctx-1", "focus");
        await api.deleteContext("ctx-1");
        await api.assignShortcut("ctx-1", 3);
        await api.assignShortcut("ctx-1", null);
        await api.reorderContexts(["a", "b"]);
        await api.addWindowToContext("ctx-1", 42, true);
        await api.removeWindowFromContext("ctx-1", 42);
        await api.showContext("ctx-1");
        await api.hideContext("ctx-1");
        await api.updateSettings(settings);

        expect(calls).toEqual([
            { cmd: "get_app_data", args: undefined },
            { cmd: "create_context", args: undefined },
            { cmd: "rename_context", args: { id: "ctx-1", name: "focus" } },
            { cmd: "delete_context", args: { id: "ctx-1" } },
            { cmd: "assign_shortcut", args: { id: "ctx-1", index: 3 } },
            { cmd: "assign_shortcut", args: { id: "ctx-1", index: null } },
            { cmd: "reorder_contexts", args: { orderedIds: ["a", "b"] } },
            { cmd: "add_window_to_context", args: { contextId: "ctx-1", platformId: 42, copy: true } },
            { cmd: "remove_window_from_context", args: { contextId: "ctx-1", platformId: 42 } },
            { cmd: "show_context", args: { id: "ctx-1" } },
            { cmd: "hide_context", args: { id: "ctx-1" } },
            { cmd: "update_settings", args: { settings } },
        ]);
    });

    it("propagates backend rejections", async () => {
        mockIPC(() => {
            throw "context 'x' not found";
        });
        await expect(api.showContext("x")).rejects.toBe("context 'x' not found");
    });
});
