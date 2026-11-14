import { describe, expect, it } from "vitest";
import { batchRow, effectiveColors } from "./runs";
import { clipboardShortcut, mapKeyEvent, type KeyEventLike } from "./keys";
import { automaticCategory, groupSpaces, parseCategory } from "./grouping";
import { isSelected, normalizeSel, selectionText } from "./selection";
import { snakeKeys } from "./api";
import type { Cell, Workspace } from "./types";

const cell = (ch: string, o: Partial<Cell> = {}): Cell =>
  ({ ch, fg: null, bg: null, bold: false, italic: false, underline: false, inverse: false, wide: false, ...o });
const row = (s: string) => [...s].map((c) => cell(c));
const DF: [number, number, number] = [200, 200, 200], DB: [number, number, number] = [0, 0, 0];

describe("batchRow", () => {
  it("merges same-style cells into one run", () => {
    const r = batchRow(row("hello"), DF, DB);
    expect(r).toHaveLength(1);
    expect(r[0]).toMatchObject({ col: 0, cells: 5, text: "hello" });
  });
  it("splits on style change", () => {
    const r = batchRow([cell("a"), cell("b", { bold: true }), cell("c", { fg: [1, 2, 3] })], DF, DB);
    expect(r.map((x) => x.text)).toEqual(["a", "b", "c"]);
    expect(r.map((x) => x.col)).toEqual([0, 1, 2]);
  });
  it("counts wide cells as 2 columns and skips the spacer", () => {
    const r = batchRow([cell("日", { wide: true }), cell(""), cell("x")], DF, DB);
    expect(r).toHaveLength(1);
    expect(r[0]).toMatchObject({ cells: 3, text: "日x", hasWide: true });
  });
  it("handles wide cells with the spacer absent without double-advancing", () => {
    const withSpacer = batchRow([cell("日", { wide: true }), cell(""), cell("x"), cell("y", { bold: true })], DF, DB);
    const noSpacer = batchRow([cell("日", { wide: true }), cell("x"), cell("y", { bold: true })], DF, DB);
    expect(noSpacer).toEqual(withSpacer);
    expect(noSpacer.map((r) => r.col)).toEqual([0, 3]);
    expect(noSpacer[0]).toMatchObject({ cells: 3, text: "日x" });
  });
  it("keeps an empty cell after a non-wide cell as a blank column", () => {
    const r = batchRow([cell("a"), cell(""), cell("b")], DF, DB);
    expect(r[0]).toMatchObject({ cells: 3, text: "a b" });
  });
  it("resolves inverse", () => {
    const r = batchRow([cell("a", { inverse: true, fg: [9, 9, 9] })], DF, DB);
    expect(r[0].fg).toEqual(DB);
    expect(r[0].bg).toEqual([9, 9, 9]);
  });
});

describe("effectiveColors", () => {
  it("passes through when not inverse", () => {
    expect(effectiveColors(cell("a", { fg: [1, 1, 1] }), DF, DB)).toEqual({ fg: [1, 1, 1], bg: null });
  });
  it("uses defaults for null sides when inverse", () => {
    expect(effectiveColors(cell("a", { inverse: true }), DF, DB)).toEqual({ fg: DB, bg: DF });
  });
});

const ev = (o: Partial<KeyEventLike>): KeyEventLike =>
  ({ key: "a", code: "KeyA", ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...o });

describe("keys", () => {
  it("maps printable text", () => {
    expect(mapKeyEvent(ev({}))).toEqual({ text: "a", code: "KeyA", ctrl: false, alt: false, shift: false, meta: false });
  });
  it("omits text for ctrl chords and named keys", () => {
    expect(mapKeyEvent(ev({ ctrlKey: true }))!.text).toBeUndefined();
    expect(mapKeyEvent(ev({ key: "Enter", code: "Enter" }))!.text).toBeUndefined();
  });
  it("ignores modifiers, dead keys", () => {
    expect(mapKeyEvent(ev({ key: "Shift", code: "ShiftLeft" }))).toBeNull();
    expect(mapKeyEvent(ev({ key: "Dead" }))).toBeNull();
  });
  it("detects clipboard shortcuts and does not forward them", () => {
    const c = ev({ key: "C", code: "KeyC", ctrlKey: true, shiftKey: true });
    expect(clipboardShortcut(c)).toBe("copy");
    expect(clipboardShortcut(ev({ key: "V", code: "KeyV", ctrlKey: true, shiftKey: true }))).toBe("paste");
    expect(mapKeyEvent(c)).toBeNull();
    expect(clipboardShortcut(ev({ ctrlKey: true, code: "KeyC" }))).toBeNull();
  });
});

const ws = (label: string, category?: string): Workspace => ({ id: label, label, category, tabs: [] });

describe("grouping", () => {
  it("categorises by keyword prefix", () => {
    expect(automaticCategory("Study Central")).toBe("school");
    expect(automaticCategory("options-trading")).toBe("finance");
    expect(automaticCategory("Marquee player")).toBe("creative");
    expect(automaticCategory("scratch")).toBe("misc");
  });
  it("parses slugs and titles", () => {
    expect(parseCategory("claude")).toBe("claude");
    expect(parseCategory("Creative Projects")).toBe("creative");
    expect(parseCategory("nope")).toBeUndefined();
  });
  it("groups in category order, non-empty only, keeping input order", () => {
    const g = groupSpaces([ws("scratch"), ws("b-budget"), ws("a-study"), ws("explicit", "claude"), ws("study2")]);
    expect(g.map((x) => x.category)).toEqual(["claude", "school", "finance", "misc"]);
    expect(g[1].spaces.map((s) => s.label)).toEqual(["a-study", "study2"]);
  });
});

describe("selection", () => {
  it("normalizes order", () => {
    const [a, b] = normalizeSel({ x: 5, y: 2 }, { x: 1, y: 1 });
    expect(a).toEqual({ x: 1, y: 1 }); expect(b).toEqual({ x: 5, y: 2 });
  });
  it("isSelected covers a stream", () => {
    const s = normalizeSel({ x: 2, y: 0 }, { x: 1, y: 1 });
    expect(isSelected(s, 1, 0)).toBe(false);
    expect(isSelected(s, 2, 0)).toBe(true);
    expect(isSelected(s, 9, 0)).toBe(true);
    expect(isSelected(s, 1, 1)).toBe(true);
    expect(isSelected(s, 2, 1)).toBe(false);
    expect(isSelected(null, 0, 0)).toBe(false);
  });
  it("extracts text, trimming trailing blanks", () => {
    const rows = [row("abc   "), row("def   ")];
    expect(selectionText(rows, { x: 1, y: 0 }, { x: 1, y: 1 })).toBe("bc\nde");
  });
});

describe("snakeKeys", () => {
  it("normalises camelCase event payload keys and leaves snake_case alone", () => {
    expect(snakeKeys({ machineId: "m", tabId: "t", panes: [{ paneId: "p", rowsData: [[{ ch: "a" }]], dirty_rows: [1] }] }))
      .toEqual({ machine_id: "m", tab_id: "t", panes: [{ pane_id: "p", rows_data: [[{ ch: "a" }]], dirty_rows: [1] }] });
  });
});
