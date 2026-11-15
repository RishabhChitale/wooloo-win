import { describe, expect, it } from "vitest";
import { diagnosticsText, failureView, parseSessions } from "./diagnostics";
import type { Machine, MachineStatus } from "./types";

const local: Machine = { id: "l", label: "This PC", kind: "local" };
const ssh: Machine = { id: "s", label: "Mac", kind: "ssh", ssh_target: "me@mac" };
const failed: MachineStatus = { machine_id: "l", state: "failed", reason: "no pipe", tried: ["\\\\.\\pipe\\herdr", ""], hint: "start herdr" };
const env = { appVersion: "0.1.0", userAgent: "UA" };

describe("failureView", () => {
  it("is null unless failed", () => {
    expect(failureView(local, undefined)).toBeNull();
    expect(failureView(local, { machine_id: "l", state: "connected" })).toBeNull();
  });
  it("lists tried paths, drops blanks, offers Start only for local", () => {
    const v = failureView(local, failed)!;
    expect(v.tried).toEqual(["\\\\.\\pipe\\herdr"]);
    expect(v.canStart).toBe(true);
    expect(failureView(ssh, { ...failed, machine_id: "s" })!.canStart).toBe(false);
  });
  it("falls back when no reason", () => {
    expect(failureView(ssh, { machine_id: "s", state: "failed" })!.reason).toContain("no reason");
  });
});

describe("diagnosticsText", () => {
  it("includes all fields", () => {
    const t = diagnosticsText(ssh, { ...failed, machine_id: "s" }, env);
    for (const x of ["app version: 0.1.0", "userAgent: UA", "machine: Mac", "kind: ssh", "target: me@mac", "state: failed", "reason: no pipe", "hint: start herdr", "  - \\\\.\\pipe\\herdr"]) expect(t).toContain(x);
  });
  it("copes with missing status", () => {
    expect(diagnosticsText(local, undefined, env)).toContain("state: unknown");
  });
});

describe("parseSessions", () => {
  it("parses the discovery reason", () => {
    expect(parseSessions("found sessions: default, work; choose one")).toEqual(["default", "work"]);
    expect(parseSessions("found sessions: a")).toEqual(["a"]);
  });
  it("is empty otherwise", () => {
    expect(parseSessions("no Herdr server found")).toEqual([]);
    expect(parseSessions(undefined)).toEqual([]);
  });
  it("only offered for local failed machines", () => {
    const st: MachineStatus = { machine_id: "l", state: "failed", reason: "found sessions: a, b; choose one" };
    expect(failureView(local, st)!.sessions).toEqual(["a", "b"]);
    expect(failureView(ssh, st)!.sessions).toEqual([]);
  });
});
