import type { Machine, MachineSync, SpaceMeta, Workspace } from "./types";

export const NAME_MAX = 200;
export const NOTES_MAX_BYTES = 65536;

/** Workspace plus applied metadata. `order` is the merged sidebar position (or the Herdr index when none). */
export interface DisplaySpace extends Workspace {
  uuid?: string;
  color: string | null;
  notes: string | null;
  order: number;
  /** machines (labels) with the Space attached, from snapshot field `presence`. */
  presence: string[];
}

/**
 * `host` of a machine's Spaces in get_meta(): the own node id for the local machine, else the machine id.
 * (INTERFACE.md does not say how an ssh machine maps to a replica `host`; this is an open question.)
 */
export const hostOf = (m: Machine, node: string): string => (m.kind === "local" ? node : m.id);

/** Apply merged metadata to one machine's workspaces: names, categories, colours, notes; sort by order. */
export function applyMeta(host: string, spaces: Workspace[], metas: SpaceMeta[]): DisplaySpace[] {
  const byId = new Map<string, SpaceMeta>();
  for (const m of metas) if (m.host === host) byId.set(m.herdr_id, m);
  const out: DisplaySpace[] = [];
  spaces.forEach((w, i) => {
    const m = byId.get(w.id);
    const presence = Array.isArray((w as { presence?: unknown }).presence) ? ((w as { presence: unknown[] }).presence.filter((p) => typeof p === "string") as string[]) : [];
    out.push({
      ...w,
      uuid: m?.uuid,
      label: m?.name || w.label,
      category: m ? (m.category ?? undefined) : w.category,
      color: m?.color ?? null,
      notes: m?.notes ?? null,
      order: m && Number.isFinite(m.order) ? m.order : i,
      presence,
    });
  });
  // Array.prototype.sort is stable: equal orders keep Herdr order.
  return out.sort((a, b) => a.order - b.order);
}

/**
 * New order value for moving item `from` to position `to` in a list already sorted by order.
 * One fractional value is enough (protocol: a move rewrites one field).
 */
export function moveOrder(orders: number[], from: number, to: number): number {
  const rest = orders.filter((_, i) => i !== from);
  const t = Math.max(0, Math.min(to, rest.length));
  if (!rest.length) return 0;
  if (t === 0) return rest[0] - 1;
  if (t === rest.length) return rest[rest.length - 1] + 1;
  return (rest[t - 1] + rest[t]) / 2;
}

export function ago(secs: number): string {
  if (secs < 5) return "just now";
  if (secs < 60) return `${Math.floor(secs)}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

/** Badge text for a paired machine, e.g. "synced with Mac 2s ago" or "sync failed: ...". `now` is epoch ms. */
export function syncBadge(s: MachineSync, peerLabel: string, now: number): string {
  if (s.syncing) return `syncing with ${peerLabel}…`;
  if (s.last_error) return `offline: ${s.last_error}`;
  if (s.last_ok_ms === undefined) return `not synced with ${peerLabel} yet`;
  return `synced with ${peerLabel} ${ago(Math.max(0, (now - s.last_ok_ms) / 1000))}`;
}

export function presenceLabel(presence: string[]): string {
  return presence.length ? `attached on ${presence.join(", ")}` : "";
}

export const isHexColor = (c: string) => /^#[0-9a-f]{6}$/.test(c);

export function utf8Bytes(s: string): number { return new TextEncoder().encode(s).length; }

/** Validate a name per protocol 4.1: text, 1-200 chars, no control characters. */
export function validName(s: string): boolean {
  const n = [...s].length;
  return n >= 1 && n <= NAME_MAX && !/[\u0000-\u001f\u007f]/.test(s);
}

/** Notes: at most 64 KiB, only \n \r \t as control characters. */
export function validNotes(s: string): boolean {
  return utf8Bytes(s) <= NOTES_MAX_BYTES && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(s);
}

export const PALETTE = ["#f38ba8", "#fab387", "#f9e2af", "#a6e3a1", "#94e2d5", "#89b4fa", "#cba6f7", "#f5c2e7"];
