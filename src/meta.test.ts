import { describe, expect, it } from "vitest";
import { applyMeta, ago, hostOf, isHexColor, moveOrder, presenceLabel, syncBadge, validName, validNotes } from "./meta";
import type { SpaceMeta, Workspace } from "./types";

const ws = (id: string, label: string): Workspace => ({ id, label, tabs: [] });
const meta = (herdr_id: string, o: Partial<SpaceMeta> = {}): SpaceMeta => ({
  uuid: "u" + herdr_id, host: "local", herdr_id, name: "n" + herdr_id, category: null, color: null, order: 0, notes: null, pinned_tabs: [], ...o,
});

describe("applyMeta", () => {
  it("applies names, colours, notes and sorts by order", () => {
    const out = applyMeta("local", [ws("a", "A"), ws("b", "B")], [meta("a", { name: "Alpha", order: 2, color: "#ff0000" }), meta("b", { name: "Beta", order: 1, notes: "hi" })]);
    expect(out.map((s) => s.label)).toEqual(["Beta", "Alpha"]);
    expect(out[1].color).toBe("#ff0000");
    expect(out[0].notes).toBe("hi");
  });
  it("keeps Herdr label/order when no meta, and ignores other machines' meta", () => {
    const out = applyMeta("local", [ws("a", "A"), ws("b", "B")], [meta("a", { host: "mac", name: "X" })]);
    expect(out.map((s) => s.label)).toEqual(["A", "B"]);
    expect(out[0].uuid).toBeUndefined();
  });
  it("maps null category to automatic", () => {
    const w = { ...ws("a", "A"), category: "claude" };
    expect(applyMeta("local", [w], [meta("a", { category: null })])[0].category).toBeUndefined();
    expect(applyMeta("local", [w], [meta("a", { category: "school" })])[0].category).toBe("school");
  });
  it("carries presence from the snapshot", () => {
    expect(applyMeta("local", [{ ...ws("a", "A"), presence: ["PC", "Mac"] }], [])[0].presence).toEqual(["PC", "Mac"]);
  });
});

describe("moveOrder", () => {
  it("places between neighbours, at the start and at the end", () => {
    expect(moveOrder([0, 1, 2], 2, 0)).toBe(-1);
    expect(moveOrder([0, 1, 2], 0, 2)).toBe(3);
    expect(moveOrder([0, 1, 2], 2, 1)).toBe(0.5);
    expect(moveOrder([5], 0, 0)).toBe(0);
  });
});

describe("badges", () => {
  it("formats synced / offline / presence", () => {
    const now = 100_000;
    expect(syncBadge({ machine_id: "m", syncing: false, last_ok_ms: now - 2000 }, "Mac", now)).toBe("synced with Mac just now");
    expect(syncBadge({ machine_id: "m", syncing: false, last_ok_ms: now - 12000 }, "Mac", now)).toBe("synced with Mac 12s ago");
    expect(syncBadge({ machine_id: "m", syncing: false, last_error: "ssh timed out" }, "Mac", now)).toBe("offline: ssh timed out");
    expect(syncBadge({ machine_id: "m", syncing: true }, "Mac", now)).toBe("syncing with Mac…");
    expect(hostOf({ id: "mac", label: "Mac", kind: "ssh" }, "n1")).toBe("mac");
    expect(hostOf({ id: "local", label: "PC", kind: "local" }, "n1")).toBe("n1");
    expect(ago(125)).toBe("2m ago");
    expect(presenceLabel(["Mac"])).toBe("attached on Mac");
    expect(presenceLabel([])).toBe("");
  });
});

describe("validation (protocol 4.1)", () => {
  it("colour is lowercase #rrggbb", () => {
    expect(isHexColor("#a1b2c3")).toBe(true);
    expect(isHexColor("#A1B2C3")).toBe(false);
    expect(isHexColor("red")).toBe(false);
  });
  it("name 1-200 chars, no control chars", () => {
    expect(validName("")).toBe(false);
    expect(validName("x".repeat(200))).toBe(true);
    expect(validName("x".repeat(201))).toBe(false);
    expect(validName("a\nb")).toBe(false);
  });
  it("notes <= 64 KiB bytes, only \\n \\r \\t controls", () => {
    expect(validNotes("line1\n\tline2\r")).toBe(true);
    expect(validNotes("a\u0000b")).toBe(false);
    expect(validNotes("é".repeat(32768))).toBe(true);
    expect(validNotes("é".repeat(32769))).toBe(false);
  });
});
