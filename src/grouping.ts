import type { Workspace } from "./types";

// Mirrors SpaceCategory in ~/wooloo HerdrDestinations.swift: fixed order, keyword placement.
export type CategoryId = "claude" | "school" | "creative" | "finance" | "misc";

export const CATEGORY_ORDER: CategoryId[] = ["claude", "school", "creative", "finance", "misc"];

export const CATEGORY_TITLES: Record<CategoryId, string> = {
  claude: "Claude Optimization",
  school: "School",
  creative: "Creative Projects",
  finance: "Finance",
  misc: "Miscellaneous",
};

// Checked in this order; a keyword matches the start of any word in the label.
const KEYWORDS: [CategoryId, string[]][] = [
  ["claude", ["claude", "skill", "rishabh"]],
  ["school", ["study", "studies", "school", "course", "homework", "exam", "midterm", "quiz", "lecture",
    "semester", "university", "tmu", "gpa", "ecn", "mth", "law", "cmn", "gms"]],
  ["finance", ["option", "trading", "trade", "finance", "financial", "budget", "invest", "stock", "money",
    "tax", "crypto", "portfolio"]],
  ["creative", ["playbox", "superintel", "crushon", "bpm", "upscayl", "video", "amv", "marquee", "music",
    "design", "film", "photo", "anime", "art", "creative"]],
];

export function automaticCategory(label: string): CategoryId {
  const words = label.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  for (const [cat, keys] of KEYWORDS) {
    if (keys.some((k) => words.some((w) => w.startsWith(k)))) return cat;
  }
  return "misc";
}

/** Accept a category slug ("claude") or a title ("Claude Optimization"); otherwise undefined. */
export function parseCategory(raw: string | undefined): CategoryId | undefined {
  if (!raw) return undefined;
  const s = raw.trim().toLowerCase();
  for (const id of CATEGORY_ORDER) {
    if (id === s || CATEGORY_TITLES[id].toLowerCase() === s) return id;
  }
  return undefined;
}

export function categoryOf(ws: Pick<Workspace, "label" | "category">): CategoryId {
  return parseCategory(ws.category) ?? automaticCategory(ws.label);
}

export interface CategoryGroup<T extends Workspace = Workspace> { category: CategoryId; title: string; spaces: T[] }

/** Non-empty groups in category order; each keeps the input (Herdr) order of its Spaces. */
export function groupSpaces<T extends Workspace>(spaces: T[]): CategoryGroup<T>[] {
  const buckets = new Map<CategoryId, T[]>();
  for (const s of spaces) {
    const c = categoryOf(s);
    const list = buckets.get(c) ?? [];
    list.push(s);
    buckets.set(c, list);
  }
  return CATEGORY_ORDER.filter((c) => buckets.has(c)).map((c) => ({
    category: c, title: CATEGORY_TITLES[c], spaces: buckets.get(c)!,
  }));
}
