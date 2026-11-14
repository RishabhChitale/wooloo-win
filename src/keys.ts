import type { KeyInput } from "./types";

export interface KeyEventLike {
  key: string; code: string;
  ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean;
}

const MODIFIER_CODES = new Set([
  "ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight",
  "MetaLeft", "MetaRight", "CapsLock", "NumLock", "ScrollLock", "OS",
]);

/** Ctrl+Shift+C / Ctrl+Shift+V are handled by the UI (copy/paste), never forwarded. */
export function clipboardShortcut(e: KeyEventLike): "copy" | "paste" | null {
  if (e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey) {
    if (e.code === "KeyC") return "copy";
    if (e.code === "KeyV") return "paste";
  }
  return null;
}

/** Map a DOM key event to the semantic key of INTERFACE.md; null if it should not be sent. */
export function mapKeyEvent(e: KeyEventLike): KeyInput | null {
  if (MODIFIER_CODES.has(e.code) || e.key === "Dead" || e.key === "Process" || e.key === "Unidentified") return null;
  if (clipboardShortcut(e)) return null;
  const k: KeyInput = { code: e.code, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, meta: e.metaKey };
  // printable text only when it is a single character and no ctrl/meta chord (those are semantic only)
  if ([...e.key].length === 1 && !e.ctrlKey && !e.metaKey) k.text = e.key;
  return k;
}
