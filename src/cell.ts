/** Cell metrics snapped to whole device pixels, in CSS px. A fractional cell size (the font's measured "M" width is
 *  rarely whole) makes the canvas get resampled at fractional Windows display scaling, which blurs every glyph. */
export function snapCellSize(rawW: number, fontPx: number, dpr: number): { w: number; h: number } {
  const d = dpr > 0 ? dpr : 1;
  return { w: Math.max(1, Math.round(rawW * d)) / d, h: Math.ceil(fontPx * 1.25 * d) / d };
}
