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

export type MetaKey = "CtrlAlt" | "CmdOpt";

export interface Settings {
    meta_key: MetaKey;
    single_context_mode: boolean;
    // Context forced to be the only visible one in Single Context Mode.
    // null (or a stale id) resolves to the Main Context on the backend.
    single_context_id: string | null;
}

export interface AppData {
    contexts: Context[];
    settings: Settings;
}
