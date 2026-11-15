import { describe, expect, it } from "vitest";

// INTERFACE.md is the single source of truth for command and event names.
import doc from "../INTERFACE.md?raw";
import api from "./api.ts?raw";

function section(title: string): string {
  const m = doc.match(new RegExp(`## ${title}[^\\n]*\\n([\\s\\S]*?)(?=\\n## |$)`));
  if (!m) throw new Error(`INTERFACE.md has no section "${title}"`);
  return m[1];
}
const documentedCommands = [...section("Tauri commands").matchAll(/^- ([a-z_]+)\(/gm)].map((m) => m[1]);
const documentedEvents = [...section("Tauri events").matchAll(/^- "([a-z-]+)"/gm)].map((m) => m[1]);
const used = (re: RegExp) => [...api.matchAll(re)].map((m) => m[1]);
const invoked = used(/\binvoke(?:<[^>]*>)?\(\s*"([a-z_]+)"/g);
const listened = used(/\b(?:on|listen)<[^>]*>\(\s*"([a-z-]+)"/g);

describe("INTERFACE.md contract", () => {
  it("parses a non-trivial set of names", () => {
    expect(documentedCommands.length).toBeGreaterThan(10);
    expect(documentedEvents).toContain("meta-changed");
    expect(invoked.length).toBeGreaterThan(10);
  });
  it("api.ts only invokes documented commands", () => {
    expect(invoked.filter((n) => !documentedCommands.includes(n))).toEqual([]);
  });
  it("api.ts only listens to documented events", () => {
    expect(listened.filter((n) => !documentedEvents.includes(n))).toEqual([]);
  });
  it("every documented command is used by the UI", () => {
    expect(documentedCommands.filter((n) => !invoked.includes(n))).toEqual([]);
  });
  it("every documented event is listened to", () => {
    expect(documentedEvents.filter((n) => !listened.includes(n))).toEqual([]);
  });
});
