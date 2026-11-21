import { describe, expect, it } from "vitest";
import { snapCellSize } from "./cell";
import { pathsToPasteText } from "./drop";

describe("snapCellSize", () => {
  it("lands on whole device pixels at fractional scaling", () => {
    for (const dpr of [1, 1.25, 1.5, 1.75, 2, 2.5]) {
      const { w, h } = snapCellSize(8.4, 14, dpr);
      expect(Number.isInteger(Math.round(w * dpr * 1e6) / 1e6)).toBe(true);
      expect(Math.abs(w * dpr - Math.round(w * dpr))).toBeLessThan(1e-9);
      expect(Math.abs(h * dpr - Math.round(h * dpr))).toBeLessThan(1e-9);
    }
  });
  it("keeps the 100% size close to the raw size and never collapses", () => {
    expect(snapCellSize(8.4, 14, 1)).toEqual({ w: 8, h: 18 });
    expect(snapCellSize(0.01, 14, 1).w).toBeGreaterThanOrEqual(1);
    expect(snapCellSize(8, 14, 0).w).toBe(8);
  });
});

describe("pathsToPasteText", () => {
  it("quotes paths with spaces and leaves plain ones", () => {
    expect(pathsToPasteText(["C:\\Users\\risha\\a.txt", "C:\\My Files\\b c.png"])).toBe('C:\\Users\\risha\\a.txt "C:\\My Files\\b c.png" ');
  });
  it("returns nothing for no paths", () => {
    expect(pathsToPasteText([])).toBe("");
    expect(pathsToPasteText([""])).toBe("");
  });
});
