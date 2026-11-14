import type { PaneSurface, RGB } from "./types";
import { batchRow, fontFor, rgbCss } from "./runs";
import { isSelected, normalizeSel, selectionText, type CellPos } from "./selection";
import { theme } from "./theme";

export const FONT_FAMILY = '"Cascadia Mono", Consolas, "SF Mono", Menlo, monospace';
export const FONT_PX = 14;

const hex = (h: string): RGB => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
export const DEF_FG = hex(theme.foreground);
export const DEF_BG = hex(theme.background);

export interface CellSize { w: number; h: number }

let measured: CellSize | null = null;
export function measureCell(): CellSize {
  if (measured) return measured;
  const c = document.createElement("canvas").getContext("2d")!;
  c.font = fontFor({ bold: false, italic: false }, FONT_PX, FONT_FAMILY);
  measured = { w: Math.max(1, c.measureText("M").width), h: Math.ceil(FONT_PX * 1.25) };
  return measured;
}

export interface PaneCallbacks {
  focus(): void;
  paste(text: string): void;
  scroll(lines: number): void;
}

/** One pane: a canvas that draws the latest cell grid, with selection, cursor and wheel scrollback. */
export class PaneView {
  readonly el: HTMLDivElement;
  private canvas = document.createElement("canvas");
  private ctx = this.canvas.getContext("2d")!;
  private surf: PaneSurface | null = null;
  private sel: [CellPos, CellPos] | null = null;
  private anchor: CellPos | null = null;
  private wheelAcc = 0;
  focused = false;

  constructor(readonly paneId: string, private cb: PaneCallbacks) {
    this.el = document.createElement("div");
    this.el.className = "pane";
    this.el.appendChild(this.canvas);
    this.el.addEventListener("mousedown", (e) => this.onDown(e));
    window.addEventListener("mousemove", this.onMove);
    window.addEventListener("mouseup", this.onUp);
    this.el.addEventListener("wheel", (e) => this.onWheel(e), { passive: false });
  }

  destroy() {
    window.removeEventListener("mousemove", this.onMove);
    window.removeEventListener("mouseup", this.onUp);
    this.el.remove();
  }

  setFocused(f: boolean) { this.focused = f; this.el.classList.toggle("focused", f); this.draw(); }

  update(s: PaneSurface, totalCols: number, totalRows: number) {
    this.surf = s;
    const st = this.el.style;
    st.left = `${(s.x / totalCols) * 100}%`;
    st.top = `${(s.y / totalRows) * 100}%`;
    st.width = `${(s.cols / totalCols) * 100}%`;
    st.height = `${(s.rows / totalRows) * 100}%`;
    this.draw();
  }

  private cellAt(e: MouseEvent): CellPos {
    const r = this.canvas.getBoundingClientRect();
    const { w, h } = measureCell();
    const s = this.surf;
    const x = Math.floor((e.clientX - r.left) / w), y = Math.floor((e.clientY - r.top) / h);
    return { x: Math.max(0, Math.min((s?.cols ?? 1) - 1, x)), y: Math.max(0, Math.min((s?.rows ?? 1) - 1, y)) };
  }

  private onDown(e: MouseEvent) {
    if (e.button !== 0) return;
    this.cb.focus();
    this.anchor = this.cellAt(e);
    this.sel = null;
    this.draw();
  }
  private onMove = (e: MouseEvent) => {
    if (!this.anchor) return;
    const p = this.cellAt(e);
    this.sel = p.x === this.anchor.x && p.y === this.anchor.y ? null : normalizeSel(this.anchor, p);
    this.draw();
  };
  private onUp = () => { this.anchor = null; };

  private onWheel(e: WheelEvent) {
    e.preventDefault();
    const { h } = measureCell();
    const unit = e.deltaMode === 1 ? 1 : e.deltaMode === 2 ? (this.surf?.rows ?? 24) : h;
    this.wheelAcc += (-e.deltaY / unit); // wheel up (negative deltaY) => positive = older output
    const lines = Math.trunc(this.wheelAcc);
    if (lines !== 0) { this.wheelAcc -= lines; this.cb.scroll(lines); }
  }

  hasSelection(): boolean { return this.sel !== null; }
  selectedText(): string {
    if (!this.sel || !this.surf) return "";
    return selectionText(this.surf.rows_data, this.sel[0], this.sel[1]);
  }
  clearSelection() { this.sel = null; this.draw(); }

  draw() {
    const s = this.surf;
    if (!s) return;
    const { w, h } = measureCell();
    const dpr = window.devicePixelRatio || 1;
    const pw = Math.round(s.cols * w), ph = s.rows * h;
    if (this.canvas.width !== Math.round(pw * dpr) || this.canvas.height !== Math.round(ph * dpr)) {
      this.canvas.width = Math.round(pw * dpr); this.canvas.height = Math.round(ph * dpr);
      this.canvas.style.width = `${pw}px`; this.canvas.style.height = `${ph}px`;
    }
    const g = this.ctx;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.textBaseline = "alphabetic";
    g.fillStyle = theme.background;
    g.fillRect(0, 0, pw, ph);
    const baseline = Math.round(h * 0.78);
    s.rows_data.forEach((row, y) => {
      const top = y * h;
      for (const run of batchRow(row, DEF_FG, DEF_BG)) {
        const x = run.col * w;
        if (run.bg) { g.fillStyle = rgbCss(run.bg); g.fillRect(x, top, run.cells * w, h); }
        g.font = fontFor(run, FONT_PX, FONT_FAMILY);
        g.fillStyle = run.fg ? rgbCss(run.fg) : theme.foreground;
        if (run.hasWide) {
          let cx = run.col;
          for (const ch of run.text) {
            const wide = /[ᄀ-ᅟ⺀-꓏가-힣豈-﫿︰-﹯＀-｠￠-￦]|[\u{1f300}-\u{1faff}\u{20000}-\u{3fffd}]/u.test(ch);
            g.fillText(ch, cx * w, top + baseline);
            cx += wide ? 2 : 1;
          }
        } else if (run.text.trim() !== "") {
          g.fillText(run.text, x, top + baseline);
        }
        if (run.underline) { g.fillRect(x, top + h - 2, run.cells * w, 1); }
      }
      if (this.sel) {
        g.fillStyle = "rgba(88,91,112,0.65)";
        for (let x = 0; x < s.cols; x++) if (isSelected(this.sel, x, y)) g.fillRect(x * w, top, w, h);
      }
    });
    const c = s.cursor;
    if (c.visible && c.y < s.rows && c.x < s.cols) {
      g.fillStyle = theme.cursor;
      const cx = c.x * w, cy = c.y * h;
      if (!this.focused) { g.strokeStyle = theme.cursor; g.strokeRect(cx + 0.5, cy + 0.5, w - 1, h - 1); }
      else if (c.shape === "bar") g.fillRect(cx, cy, 2, h);
      else if (c.shape === "underline") g.fillRect(cx, cy + h - 2, w, 2);
      else {
        g.globalAlpha = 0.85; g.fillRect(cx, cy, w, h); g.globalAlpha = 1;
        const cell = s.rows_data[c.y]?.[c.x];
        if (cell && cell.ch.trim()) { g.fillStyle = theme.background; g.font = fontFor(cell, FONT_PX, FONT_FAMILY); g.fillText(cell.ch, cx, cy + baseline); }
      }
    }
  }
}
