// File search hits (from Rust `files_search`) as matcher name lists, ranked up
// by how often each file was opened from the command bar.
//
// Open counts are stored as { [path]: { n, at } } under `aria-file-opens`.
// Paths are machine-specific, so the key stays out of the settings export.

export const FILE_OPENS_KEY = "aria-file-opens";
export const MAX_OPENS = 500;
// An app, game or board with the same name as a file wins.
export const FILE_WEIGHT = 0.9;
// A few opens settle a tie between same-named files (the matcher's CLEAR_LEAD
// is 0.05); the cap keeps FILE_WEIGHT × (1 + bonus) below an exact app name.
const BONUS_PER_DOUBLING = 0.03;
export const MAX_BONUS = 0.1;

export function parseOpens(raw) {
  try {
    const data = JSON.parse(raw);
    if (!data || typeof data !== "object" || Array.isArray(data)) return {};
    return Object.fromEntries(
      Object.entries(data).filter(([, v]) => Number.isInteger(v?.n) && v.n > 0 && Number.isFinite(v.at)),
    );
  } catch {
    return {};
  }
}

/** Count one more open; the least recently opened go first past MAX_OPENS. */
export function recordOpen(opens, path, now) {
  const next = { ...opens, [path]: { n: (opens[path]?.n ?? 0) + 1, at: now } };
  const paths = Object.keys(next);
  if (paths.length <= MAX_OPENS) return next;
  paths.sort((a, b) => next[b].at - next[a].at);
  return Object.fromEntries(paths.slice(0, MAX_OPENS).map((p) => [p, next[p]]));
}

export function openBonus(n) {
  return n > 0 ? Math.min(MAX_BONUS, BONUS_PER_DOUBLING * Math.log2(1 + n)) : 0;
}

/** `file` and `folder` lists for the matcher, in the order Rust found them. */
export function fileLists(hits, opens) {
  const file = [];
  const folder = [];
  for (const h of hits) {
    const item = { name: h.name, path: h.path, folder: h.folder, weight: FILE_WEIGHT, bonus: openBonus(opens[h.path]?.n ?? 0) };
    (h.dir ? folder : file).push(item);
  }
  return { file, folder };
}

/** "xyz.xls — Documents/school": the folder tells same-named files apart. */
export function fileLabel(item) {
  return `${item.name} — ${item.folder}`;
}

/** Status line of the "Search files" settings, from Rust `files_status`. */
export function searchStatusText(status, enabled) {
  if (!enabled) return "Off: the command bar finds apps, boards and links only.";
  if (status.building || status.age == null) return "Indexing…";
  const names = `${status.entries.toLocaleString("en")} names`;
  if (status.capped) return `Stopped at ${names}: choose smaller folders.`;
  return `${names} · updated ${ago(status.age)}`;
}

function ago(secs) {
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)} min ago`;
  return `${Math.floor(secs / 3600)} h ago`;
}

/** Why picked folders are missing from the saved list (Rust drops nested and missing ones). */
export function droppedFolders(picked, saved) {
  return picked
    .filter((p) => !saved.includes(p))
    .map((p) => {
      const parent = saved.find((f) => isUnder(p, f));
      return parent ? `${p} is already covered by ${parent}.` : `${p} isn't a folder.`;
    });
}

function isUnder(path, folder) {
  if (!path.startsWith(folder)) return false;
  return /[\\/]$/.test(folder) || /[\\/]/.test(path[folder.length] ?? "");
}
