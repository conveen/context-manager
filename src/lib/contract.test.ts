import { describe, expect, it } from "vitest";
import appDataFixture from "../../fixtures/app_data.json";
import eventsFixture from "../../fixtures/events.json";
import { CONTEXTS_CHANGED, SHOW_SETTINGS } from "./events";
import type { AppData, Context, Settings, WindowRef } from "./types";

// The backend half lives in src-tauri/src/contract.rs; both suites assert the
// same committed fixtures, so a rename or shape change on either side of the
// IPC boundary fails one of the two.

// Compile-checked key inventories: adding or removing a field in types.ts
// breaks these records, and the tests below compare them against the fixture
// (which the backend round-trips), closing the loop. TypeScript widens JSON
// imports (e.g. "CmdOpt" becomes string), so a direct `const d: AppData =
// fixture` assignment can't serve as the compile-time check.
const WINDOW_KEYS: Record<keyof WindowRef, true> = {
    platform_id: true,
    app_name: true,
    window_title: true,
    hidden: true,
};
const CONTEXT_KEYS: Record<keyof Context, true> = {
    id: true,
    name: true,
    is_main: true,
    windows: true,
    shortcut_index: true,
    order: true,
    visible: true,
};
const SETTINGS_KEYS: Record<keyof Settings, true> = {
    meta_key: true,
    single_context_mode: true,
    single_context_id: true,
};

// Backend-only window fields the frontend deliberately doesn't model.
const MACOS_ONLY_WINDOW_KEYS = ["pid", "hidden_z"];

describe("backend contract fixtures", () => {
    it("event name constants match the shared fixture", () => {
        expect(eventsFixture.backend_to_frontend).toEqual([CONTEXTS_CHANGED, SHOW_SETTINGS]);
    });

    it("the AppData fixture carries exactly the fields types.ts models", () => {
        const windowKeys = Object.keys(appDataFixture.contexts[0].windows[0]).filter(
            (k) => !MACOS_ONLY_WINDOW_KEYS.includes(k),
        );
        expect(windowKeys.sort()).toEqual(Object.keys(WINDOW_KEYS).sort());

        const contextKeys = Object.keys(appDataFixture.contexts[0]).filter((k) => k !== "windows");
        const expectedContextKeys = Object.keys(CONTEXT_KEYS).filter((k) => k !== "windows");
        expect(contextKeys.sort()).toEqual(expectedContextKeys.sort());

        expect(Object.keys(appDataFixture.settings).sort()).toEqual(Object.keys(SETTINGS_KEYS).sort());
    });

    it("the AppData fixture parses into the shapes the UI consumes", () => {
        // Runtime cast — see the widening note above; the field values are
        // asserted here and the key sets in the previous test.
        const data = appDataFixture as unknown as AppData;
        expect(data.contexts).toHaveLength(3);
        const main = data.contexts.find((c) => c.is_main);
        expect(main?.name).toBe("main");
        expect(main?.shortcut_index).toBe(0);
        expect(main?.windows.map((w) => w.platform_id)).toEqual([101, 102]);
        expect(main?.windows[1].hidden).toBe(true);
        expect(data.contexts[2].shortcut_index).toBeNull();
        expect(data.settings.meta_key).toBe("CmdOpt");
        expect(data.settings.single_context_mode).toBe(true);
        expect(data.settings.single_context_id).toBe(data.contexts[1].id);
    });
});
