// Shared factories for frontend tests. Not imported by production code and
// not matched by the Vitest include pattern (only *.test.ts files are).

import type { AppData, Context, Settings, WindowRef } from "./types";

export function makeWindow(platformId: number, overrides: Partial<WindowRef> = {}): WindowRef {
    return {
        platform_id: platformId,
        app_name: `App${platformId}`,
        window_title: `Win${platformId}`,
        hidden: false,
        ...overrides,
    };
}

export function makeContext(id: string, overrides: Partial<Context> = {}): Context {
    return {
        id,
        name: `name-${id}`,
        is_main: false,
        windows: [],
        shortcut_index: null,
        order: 0,
        visible: true,
        ...overrides,
    };
}

export function makeSettings(overrides: Partial<Settings> = {}): Settings {
    return {
        meta_key: "CtrlAlt",
        single_context_mode: false,
        single_context_id: null,
        ...overrides,
    };
}

export function makeAppData(contexts: Context[], settings: Settings = makeSettings()): AppData {
    return { contexts, settings };
}

export function makeMain(overrides: Partial<Context> = {}): Context {
    return makeContext("main-id", { name: "main", is_main: true, shortcut_index: 0, ...overrides });
}
