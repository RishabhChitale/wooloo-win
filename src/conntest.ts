import type { DiagReport } from "./types";

/** One line per step, same layout as the Rust format_report: "ok   Label - detail". */
export function formatDiag(r: DiagReport): string {
  const tag = (s: string) => (s === "ok" ? "ok  " : s === "skip" ? "skip" : "FAIL");
  let out = r.steps.map((s) => `${tag(s.status)} ${s.label}${s.detail ? ` - ${s.detail}` : ""}`).join("\n");
  if (r.hint) out += `\nHint: ${r.hint}`;
  return out;
}
