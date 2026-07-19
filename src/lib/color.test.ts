import { describe, expect, it } from "vitest";
import { hueFor } from "./color";

describe("hueFor", () => {
    it("is deterministic and stays within 0-359", () => {
        for (const id of ["", "main", "a-uuid-like-string", "另一个"]) {
            const hue = hueFor(id);
            expect(hue).toBe(hueFor(id));
            expect(hue).toBeGreaterThanOrEqual(0);
            expect(hue).toBeLessThan(360);
            expect(Number.isInteger(hue)).toBe(true);
        }
    });

    it("differs for typical distinct ids", () => {
        expect(hueFor("context-1")).not.toBe(hueFor("context-2"));
    });
});
