export type SkewSeverity = "none" | "warn" | "error";

export interface SkewResult {
  ok: boolean;
  severity: SkewSeverity;
  message: string;
}

function parse(v: string): [number, number, number] | null {
  const m = /^\s*v?(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?\s*$/.exec(v ?? "");
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** Compare local and remote Herdr versions. Equal = ok, patch differs = warn, minor/major differs = error. */
export function compareHerdrVersions(local: string, remote: string): SkewResult {
  const l = parse(local);
  const r = parse(remote);
  if (!l || !r) {
    return {
      ok: false,
      severity: "warn",
      message: `Could not parse Herdr versions (local "${local}", remote "${remote}").`,
    };
  }
  if (l[0] !== r[0] || l[1] !== r[1]) {
    return {
      ok: false,
      severity: "error",
      message: `Herdr version mismatch: local ${local}, remote ${remote}. Update one side so major.minor match.`,
    };
  }
  if (l[2] !== r[2]) {
    return {
      ok: false,
      severity: "warn",
      message: `Herdr patch versions differ: local ${local}, remote ${remote}.`,
    };
  }
  return { ok: true, severity: "none", message: `Herdr versions match (${local}).` };
}

export interface SkewBanner {
  machine_id: string;
  severity: "warn" | "error";
  message: string;
}

interface SkewRow {
  machine_id: string;
  peer_label: string;
  local_herdr_version: string | null;
  peer_herdr_version: string | null;
}

/** One banner per machine, only when both versions are known and differ. null on either side = none. */
export function skewBanners(rows: SkewRow[]): SkewBanner[] {
  const out: SkewBanner[] = [];
  for (const r of rows) {
    if (!r.local_herdr_version || !r.peer_herdr_version) continue;
    if (r.local_herdr_version === r.peer_herdr_version) continue;
    const c = compareHerdrVersions(r.local_herdr_version, r.peer_herdr_version);
    if (c.ok || c.severity === "none") continue;
    out.push({ machine_id: r.machine_id, severity: c.severity, message: `${r.peer_label}: ${c.message}` });
  }
  return out;
}
