/// <reference types="svelte" />
/// <reference types="vite/client" />

interface ImportMeta {
    readonly env: Record<string, string | undefined> & {
        readonly DEV: boolean;
        readonly PROD: boolean;
    };
}
