export interface WindowRef {
    platform_id: number;
    app_name: string;
    window_title: string;
    // True while the window is hidden by us (minimized / SW_HIDE).
    hidden: boolean;
}

export interface Context {
    id: string;
    name: string;
    is_main: boolean;
    windows: WindowRef[];
    shortcut_index: number | null;
    // Manual sidebar sort key for contexts without a shortcut (ascending).
    // Ignored for shortcut-assigned contexts, which sort by shortcut_index.
    order: number;
    visible: boolean;
}

// "CtrlAltSuper" is Ctrl+Alt+Win on Windows (the default there) and
// Ctrl+Alt+Cmd on macOS.
export type MetaKey = "CtrlAlt" | "CmdOpt" | "CtrlAltSuper";

export interface Settings {
    meta_key: MetaKey;
    single_context_mode: boolean;
    // Context forced to be the only visible one in Single Context Mode.
    // null (or a stale id) resolves to the Main Context on the backend.
    single_context_id: string | null;
}

// Whether the OS is letting the backend read window titles. Mirrors the Rust
// `ScreenRecordingStatus` enum, whose unit variants serialize to their names.
// - "Granted": titles are readable, or the platform doesn't gate them (Windows).
// - "Denied": macOS Screen Recording permission is not granted.
// - "NotInEffect": granted, but not applied to this process — needs a relaunch.
export type ScreenRecordingStatus = "Granted" | "Denied" | "NotInEffect";

export interface AppData {
    contexts: Context[];
    settings: Settings;
}
