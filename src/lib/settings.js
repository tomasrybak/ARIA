// Settings export / import, the frontend half (src-tauri/src/settings.rs does
// the encryption and the config files). Pure: storage is passed in as a getter
// so the rules can be tested without a webview.

// localStorage keys that travel. An allowlist, so a key added later stays on
// this machine until someone decides it should move.
export const STORAGE_KEYS = [
  "aria-theme",
  "aria-layout",
  "aria-presets",
  "aria-presets-seeded",
  "aria-automation",
  "aria-notes",
  "aria-timetable",
  "aria-clock-zones",
  "aria-ping-host",
  "aria-crypto-period",
  "aria-term-fontsize",
];

// Window placement travels too, but importing it is the user's choice: it
// names monitors that may not exist on this machine.
export const PLACEMENT_KEY = "aria-window";

// Deliberately absent: aria-automation-state (which rule is active right now)
// and aria-clock-timer (a running countdown). Both are moments, not settings.

export const MIN_PASSPHRASE = 10; // keep in step with settings.rs

export function collectStorage(get) {
  const out = {};
  for (const key of [...STORAGE_KEYS, PLACEMENT_KEY]) {
    const value = get(key);
    if (value !== null && value !== undefined) out[key] = value;
  }
  return out;
}

export function passphraseProblem(pass, repeat) {
  if ([...pass].length < MIN_PASSPHRASE) {
    return `use at least ${MIN_PASSPHRASE} characters`;
  }
  if (repeat !== undefined && pass !== repeat) return "the passphrases differ";
  return null;
}

function parsePlacement(raw) {
  try {
    const p = JSON.parse(raw);
    return p && typeof p === "object" ? p : null;
  } catch {
    return null;
  }
}

/**
 * Whether to ask about the imported window placement.
 * monitors: [{x, y, w, h}] connected here.
 * - "none": the file carries no usable placement
 * - "same": it equals this machine's, nothing to ask
 * - "elsewhere": its monitor is not connected here, so it cannot apply
 * - "ask": both are plausible, the user picks
 */
export function placementChoice(importedRaw, localRaw, monitors) {
  const imported = parsePlacement(importedRaw);
  if (!imported) return "none";
  if (importedRaw === localRaw) return "same";
  const mon = imported.mon;
  const here =
    mon && monitors.some((m) => m.x === mon.x && m.y === mon.y && m.w === mon.w && m.h === mon.h);
  return here ? "ask" : "elsewhere";
}

/**
 * What applying an imported storage map does. Import replaces: an allowlisted
 * key the file lacks is removed, so this machine ends up matching the
 * exporter. Keys outside the allowlist are ignored, never written.
 */
export function planImport(storage, { placement = false } = {}) {
  const set = {};
  const remove = [];
  for (const key of STORAGE_KEYS) {
    if (typeof storage[key] === "string") set[key] = storage[key];
    else remove.push(key);
  }
  if (placement && typeof storage[PLACEMENT_KEY] === "string") {
    set[PLACEMENT_KEY] = storage[PLACEMENT_KEY];
  }
  const known = new Set([...STORAGE_KEYS, PLACEMENT_KEY]);
  const ignored = Object.keys(storage).filter((k) => !known.has(k));
  return { set, remove, ignored };
}

function count(raw, pick) {
  try {
    const n = pick(JSON.parse(raw));
    return Number.isFinite(n) ? n : 0;
  } catch {
    return 0;
  }
}

/** One-line description of an opened bundle, for the confirmation step. */
export function describeBundle(bundle) {
  const s = bundle.storage || {};
  const parts = [];
  const boards = count(s["aria-presets"], (p) => p.length);
  if (boards) parts.push(`${boards} board${boards === 1 ? "" : "s"}`);
  const timetables = count(s["aria-timetable"], (t) => t.sources.length);
  if (timetables) parts.push(`${timetables} timetable${timetables === 1 ? "" : "s"}`);
  if (s["aria-notes"]) parts.push("notes");
  if (s["aria-theme"]) parts.push(`${s["aria-theme"]} theme`);
  const configs = Object.keys(bundle.config || {});
  if (configs.length) parts.push(configs.join(", "));
  const date = (bundle.exported || "").slice(0, 10);
  const from = [date, bundle.app && `ARIA ${bundle.app}`].filter(Boolean).join(", ");
  return `${parts.join(" · ") || "no settings"}${from ? ` (${from})` : ""}`;
}
