import { invoke } from "@tauri-apps/api/core";
import type { AppData, Context, ScreenRecordingStatus, Settings } from "./types";

export const getAppData = () => invoke<AppData>("get_app_data");

export const createContext = () => invoke<Context>("create_context");

export const renameContext = (id: string, name: string) => invoke<void>("rename_context", { id, name });

export const deleteContext = (id: string) => invoke<void>("delete_context", { id });

// Tauri v2 renames snake_case Rust params to camelCase for the JS side.
export const assignShortcut = (id: string, index: number | null) => invoke<void>("assign_shortcut", { id, index });

// orderedIds must list exactly the unassigned (no-shortcut) contexts, top to bottom.
export const reorderContexts = (orderedIds: string[]) => invoke<void>("reorder_contexts", { orderedIds });

export const addWindowToContext = (contextId: string, platformId: number, copy = false) =>
    invoke<void>("add_window_to_context", { contextId, platformId, copy });

export const removeWindowFromContext = (contextId: string, platformId: number) =>
    invoke<void>("remove_window_from_context", { contextId, platformId });

export const showContext = (id: string) => invoke<void>("show_context", { id });

export const hideContext = (id: string) => invoke<void>("hide_context", { id });

export const updateSettings = (settings: Settings) => invoke<void>("update_settings", { settings });

export const getScreenRecordingStatus = () => invoke<ScreenRecordingStatus>("get_screen_recording_status");

// macOS only; rejects elsewhere. Only reachable from the permission banner,
// which is never shown on other platforms.
export const openScreenRecordingSettings = () => invoke<void>("open_screen_recording_settings");

export const openDevtools = () => invoke<void>("open_devtools");
