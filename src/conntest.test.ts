import { describe, expect, it } from "vitest";
import { formatDiag } from "./conntest";

describe("formatDiag", () => {
  it("one line per step plus the hint", () => {
    const t = formatDiag({ ok: false, hint: "Shields Up", steps: [
      { step: "tailscale", label: "Tailscale ping", status: "ok", detail: "pong" },
      { step: "tcp", label: "TCP port 22", status: "fail", detail: "timed out" },
    ] });
    expect(t).toBe("ok   Tailscale ping - pong\nFAIL TCP port 22 - timed out\nHint: Shields Up");
  });
  it("skip and no detail", () => {
    expect(formatDiag({ ok: true, hint: null, steps: [{ step: "tailscale", label: "Tailscale ping", status: "skip", detail: "" }] })).toBe("skip Tailscale ping");
  });
  it("warn step is tagged WARN and does not fail the report", () => {
    const t = formatDiag({ ok: true, hint: "a VPN/TUN adapter is capturing 100.64.0.0/10; exclude it from the VPN or pause it", steps: [
      { step: "route", label: "Route to peer", status: "warn", detail: "via Wi-Fi" },
    ] });
    expect(t).toBe("WARN Route to peer - via Wi-Fi\nHint: a VPN/TUN adapter is capturing 100.64.0.0/10; exclude it from the VPN or pause it");
  });
});
