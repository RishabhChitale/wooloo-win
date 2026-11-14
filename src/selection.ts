import type { Cell } from "./types";

export interface CellPos { x: number; y: number }

/** Order two positions in reading order. */
export function normalizeSel(a: CellPos, b: CellPos): [CellPos, CellPos] {
  return a.y < b.y || (a.y === b.y && a.x <= b.x) ? [a, b] : [b, a];
}

export function isSelected(sel: [CellPos, CellPos] | null, x: number, y: number): boolean {
  if (!sel) return false;
  const [s, e] = sel;
  if (y < s.y || y > e.y) return false;
  if (y === s.y && x < s.x) return false;
  if (y === e.y && x > e.x) return false;
  return true;
}

/** Plain text of a stream selection (inclusive ends); trailing blanks trimmed per line. */
export function selectionText(rows: Cell[][], a: CellPos, b: CellPos): string {
  const [s, e] = normalizeSel(a, b);
  const lines: string[] = [];
  for (let y = Math.max(0, s.y); y <= Math.min(rows.length - 1, e.y); y++) {
    const row = rows[y];
    const from = y === s.y ? s.x : 0;
    const to = y === e.y ? e.x : row.length - 1;
    let line = "";
    for (let x = Math.max(0, from); x <= Math.min(row.length - 1, to); x++) line += row[x].ch === "" ? "" : row[x].ch;
    lines.push(line.replace(/\s+$/, ""));
  }
  return lines.join("\n");
}
