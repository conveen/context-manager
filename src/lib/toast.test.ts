import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dismissError, showError, toast } from "./toast.svelte";

// The toast module is a singleton; reset its state and timers around each test.
describe("toast", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        dismissError();
        vi.useRealTimers();
    });

    it("shows a message and auto-dismisses after the delay", () => {
        showError("boom");
        expect(toast.message).toBe("boom");
        vi.advanceTimersByTime(5999);
        expect(toast.message).toBe("boom");
        vi.advanceTimersByTime(1);
        expect(toast.message).toBeNull();
    });

    it("replaces the current message and restarts the auto-dismiss timer", () => {
        showError("first");
        vi.advanceTimersByTime(3000);
        showError("second");
        expect(toast.message).toBe("second");
        // 5999ms after "second" (8999ms after "first"): still visible.
        vi.advanceTimersByTime(5999);
        expect(toast.message).toBe("second");
        vi.advanceTimersByTime(1);
        expect(toast.message).toBeNull();
    });

    it("dismisses immediately and cancels the pending timer", () => {
        showError("boom");
        dismissError();
        expect(toast.message).toBeNull();
        // The cancelled timer must not resurrect or clear anything later.
        showError("again");
        vi.advanceTimersByTime(3000);
        expect(toast.message).toBe("again");
    });
});
