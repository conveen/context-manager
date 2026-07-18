// Shared error-toast state. Any component can call `showError` to surface a
// failed backend call; ErrorToast.svelte (rendered once in App.svelte) displays
// the current message. Settings.svelte keeps its own inline banner because its
// errors are scoped to the settings form, not the main window.

// Auto-dismiss delay. Long enough to read a one-line backend error.
const DISMISS_MS = 6000;

let timer: ReturnType<typeof setTimeout> | undefined;

export const toast = $state<{ message: string | null }>({ message: null });

/** Shows `message` in the error toast, replacing any current one and
 * restarting the auto-dismiss timer. */
export function showError(message: string) {
    toast.message = message;
    clearTimeout(timer);
    timer = setTimeout(dismissError, DISMISS_MS);
}

/** Hides the toast immediately (also used by its close button). */
export function dismissError() {
    toast.message = null;
    clearTimeout(timer);
    timer = undefined;
}
