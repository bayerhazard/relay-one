import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import SidebarSearch from "$lib/components/SidebarSearch.svelte";

describe("SidebarSearch", () => {
  it("renders the input with placeholder and aria-label", () => {
    render(SidebarSearch, { placeholder: "Suchen…", ariaLabel: "Suche" });
    const input = screen.getByRole("textbox", { name: "Suche" }) as HTMLInputElement;
    expect(input).toBeTruthy();
    expect(input.getAttribute("placeholder")).toBe("Suchen…");
  });

  it("does not show the clear button when empty", () => {
    render(SidebarSearch, { value: "", clearLabel: "Löschen" });
    expect(screen.queryByRole("button", { name: "Löschen" })).toBeNull();
  });

  it("shows the clear button when there is a value", () => {
    render(SidebarSearch, { value: "abc", clearLabel: "Löschen" });
    expect(screen.getByRole("button", { name: "Löschen" })).toBeTruthy();
  });

  it("clears the value when the clear button is clicked", async () => {
    const { container } = render(SidebarSearch, { value: "abc", clearLabel: "Löschen" });
    const input = container.querySelector(".ss-input") as HTMLInputElement;
    const btn = screen.getByRole("button", { name: "Löschen" });
    await fireEvent.click(btn);
    expect(input.value).toBe("");
  });

  it("calls onInput when typing", async () => {
    const onInput = vi.fn();
    render(SidebarSearch, { value: "", onInput });
    const input = screen.getByRole("textbox");
    await fireEvent.input(input, { target: { value: "x" } });
    expect(onInput).toHaveBeenCalled();
  });
});
