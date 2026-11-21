/** Text to type into a terminal for files dropped onto the window: paths with spaces or shell-special characters are
 *  double-quoted, joined by spaces, with one trailing space so the user can keep typing. */
export function pathsToPasteText(paths: string[]): string {
  const clean = paths.filter((p) => p.length > 0);
  if (clean.length === 0) return "";
  return clean.map((p) => (/[\s&()\[\]{}^=;!'+,~]/.test(p) ? `"${p}"` : p)).join(" ") + " ";
}
