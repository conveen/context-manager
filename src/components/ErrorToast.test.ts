import { fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import { dismissError, showError } from "../lib/toast.svelte";
import ErrorToast from "./ErrorToast.svelte";

describe("ErrorToast", () => {
    afterEach(() => {
        dismissError();
    });

    it("renders nothing while there is no message", () => {
        render(ErrorToast);
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("shows the message and dismisses via the close button", async () => {
        showError("something failed");
        render(ErrorToast);
        expect(screen.getByRole("alert").textContent).toContain("something failed");

        await fireEvent.click(screen.getByTitle("Dismiss"));
        expect(screen.queryByRole("alert")).toBeNull();
    });
});
