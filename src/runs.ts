import type { Cell, RGB } from "./types";

/** A horizontal run of cells sharing one style, drawn with one fillRect + one fillText. */
export interface Run {
  col: number;       // first column
  cells: number;     // columns covered (wide glyphs count 2)
  text: string;
  fg: RGB | null;    // already inverse-resolved: null means "default foreground"
  bg: RGB | null;    // null means "default background"
  bold: boolean;
  italic: boolean;
  underline: boolean;
  hasWide: boolean;  // draw per glyph so wide glyphs stay on the grid
}

const sameRgb = (a: RGB | null, b: RGB | null) =>
  a === b || (!!a && !!b && a[0] === b[0] && a[1] === b[1] && a[2] === b[2]);

/** Effective colours for a cell. Inverse swaps fg/bg; a null side becomes `defaults` so the swap is visible. */
export function effectiveColors(c: Cell, defFg: RGB, defBg: RGB): { fg: RGB | null; bg: RGB | null } {
  if (!c.inverse) return { fg: c.fg, bg: c.bg };
  return { fg: c.bg ?? defBg, bg: c.fg ?? defFg };
}

/**
 * Batch a row of cells into style runs. A spacer cell (empty ch) directly after a wide cell is
 * the wide glyph's second column and is skipped. Blank cells keep their column (space).
 */
export function batchRow(row: Cell[], defFg: RGB, defBg: RGB): Run[] {
  const runs: Run[] = [];
  let cur: Run | null = null;
  let prevWide = false;
  for (let x = 0; x < row.length; x++) {
    const c = row[x];
    if (prevWide && c.ch === "" && !c.wide) { prevWide = false; continue; }
    prevWide = c.wide;
    const { fg, bg } = effectiveColors(c, defFg, defBg);
    const width = c.wide ? 2 : 1;
    const text = c.ch === "" ? " " : c.ch;
    if (cur && cur.col + cur.cells === x && sameRgb(cur.fg, fg) && sameRgb(cur.bg, bg)
        && cur.bold === c.bold && cur.italic === c.italic && cur.underline === c.underline) {
      cur.text += text;
      cur.cells += width;
      cur.hasWide ||= c.wide;
    } else {
      cur = { col: x, cells: width, text, fg, bg, bold: c.bold, italic: c.italic, underline: c.underline, hasWide: c.wide };
      runs.push(cur);
    }
  }
  return runs;
}

export function fontFor(run: Pick<Run, "bold" | "italic">, px: number, family: string): string {
  return `${run.italic ? "italic " : ""}${run.bold ? "bold " : ""}${px}px ${family}`;
}

export const rgbCss = (c: RGB) => `rgb(${c[0]},${c[1]},${c[2]})`;
