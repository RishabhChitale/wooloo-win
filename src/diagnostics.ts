import type { Machine, MachineStatus } from "./types";

/** What the machine row shows under the label when a connection failed. */
export interface FailureView {
  reason: string;
  hint?: string;
  tried: string[];
  retry?: string;
  canStart: boolean;
}

/** null unless the machine is in the failed state. `canStart` only for local machines. */
export function failureView(m: Machine, st: MachineStatus | undefined): FailureView | null {
  if (!st || st.state !== "failed") return null;
  return {
    reason: st.reason?.trim() || "connection failed (no reason reported)",
    hint: st.hint?.trim() || undefined,
    tried: (st.tried ?? []).filter((t) => t.trim().length > 0),
    retry: st.retry_in_secs ? `retrying in ${st.retry_in_secs}s` : undefined,
    canStart: m.kind === "local",
  };
}

export interface DiagEnv { appVersion: string; userAgent: string }

/** Plain-text block the user pastes back when something is unreachable. */
export function diagnosticsText(m: Machine, st: MachineStatus | undefined, env: DiagEnv): string {
  const lines = [
    "wooloo diagnostics",
    `app version: ${env.appVersion}`,
    `userAgent: ${env.userAgent}`,
    `machine: ${m.label}`,
    `kind: ${m.kind}`,
    `target: ${m.kind === "ssh" ? (m.ssh_target ?? "(none)") : (m.pipe_path ?? "local")}`,
  ];
  if (m.session) lines.push(`session: ${m.session}`);
  lines.push(`state: ${st?.state ?? "unknown"}`);
  if (st?.reason) lines.push(`reason: ${st.reason}`);
  if (st?.hint) lines.push(`hint: ${st.hint}`);
  if (st?.retry_in_secs) lines.push(`retry in: ${st.retry_in_secs}s`);
  const tried = st?.tried ?? [];
  if (tried.length) { lines.push("tried:"); for (const t of tried) lines.push(`  - ${t}`); }
  return lines.join("\n");
}
