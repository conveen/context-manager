import type { DndEvent } from "svelte-dnd-action";

// Type augmentation for svelte-dnd-action's custom DOM events.
// In Svelte 5 runes mode, old on:consider/on:finalize directives cause
// "mixing syntaxes" errors. Use onconsider/onfinalize with these declarations.
declare module "svelte/elements" {
    interface HTMLAttributes<T extends EventTarget> {
        // biome-ignore lint/suspicious/noExplicitAny: svelte-dnd-action is generic, need any to accept all item types
        onconsider?: (event: CustomEvent<DndEvent<any>>) => void;
        // biome-ignore lint/suspicious/noExplicitAny: svelte-dnd-action is generic, need any to accept all item types
        onfinalize?: (event: CustomEvent<DndEvent<any>>) => void;
    }
}
