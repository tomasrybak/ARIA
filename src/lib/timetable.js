// TIMETABLE: the STAG week merged with timetables the user keeps by hand
// (another school, a job), laid out on a time axis.
//
// Pure functions so the rules — which entries a given week shows, how
// overlapping entries share a day row, what time range the grid spans — are
// tested without a DOM. The widget in main.js renders around this.
//
// Stored shape (localStorage, JSON):
//
//   sources  [{ id, name, color, alternating, flip }]
//            `color` indexes the base.css source palette. An `alternating`
//            source has different odd and even weeks; `flip` records how the
//            school counts them against ISO week numbers, set once by the user
//            saying "this week is odd/even".
//   entries  [{ id, sourceId, day, start, end, title, room, week }]
//            `day` 0 = Monday, `start`/`end` "HH:MM", `week` "all" | "odd" |
//            "even". An odd/even entry only shows while its source alternates.

export const SRC_COLORS = 6;
export const MAX_SOURCES = 8;
export const MAX_ENTRIES = 200;
export const MAX_TEXT = 60;
export const DAY_NAMES = ["Po", "Út", "St", "Čt", "Pá", "So", "Ne"];

const WEEKS = new Set(["all", "odd", "even"]);
const WEEK_MS = 7 * 86_400_000;

/** "HH:MM" → minutes after midnight, or null when malformed. */
export function toMinutes(hhmm) {
  const m = /^(\d{1,2}):(\d{2})$/.exec(String(hhmm ?? "").trim());
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h < 24 && min < 60 ? h * 60 + min : null;
}

export function fmtMinutes(mins) {
  return `${String(Math.floor(mins / 60)).padStart(2, "0")}:${String(mins % 60).padStart(2, "0")}`;
}

const text = (v) => String(v ?? "").trim().slice(0, MAX_TEXT);

/** Parse stored JSON, keeping only well-formed sources and entries. */
export function parseCustom(raw) {
  let data;
  try {
    data = JSON.parse(raw);
  } catch {
    return { sources: [], entries: [] };
  }
  const sources = [];
  const ids = new Set();
  for (const s of Array.isArray(data?.sources) ? data.sources : []) {
    const id = String(s?.id ?? "");
    const name = text(s?.name);
    if (!id || !name || ids.has(id)) continue;
    ids.add(id);
    const color = Number.isInteger(s.color) ? Math.abs(s.color) % SRC_COLORS : 0;
    sources.push({ id, name, color, alternating: !!s.alternating, flip: !!s.flip });
    if (sources.length >= MAX_SOURCES) break;
  }
  const entries = [];
  const entryIds = new Set();
  for (const e of Array.isArray(data?.entries) ? data.entries : []) {
    const id = String(e?.id ?? "");
    const start = toMinutes(e?.start);
    const end = toMinutes(e?.end);
    const title = text(e?.title);
    if (!id || entryIds.has(id) || !ids.has(String(e.sourceId))) continue;
    if (!title || start == null || end == null || end <= start) continue;
    if (!Number.isInteger(e.day) || e.day < 0 || e.day > 6) continue;
    entryIds.add(id);
    entries.push({
      id,
      sourceId: String(e.sourceId),
      day: e.day,
      start: fmtMinutes(start),
      end: fmtMinutes(end),
      title,
      room: text(e.room),
      week: WEEKS.has(e.week) ? e.week : "all",
    });
    if (entries.length >= MAX_ENTRIES) break;
  }
  return { sources, entries };
}

/** ISO 8601 week number of the local date `t` (weeks start Monday). */
export function isoWeek(t) {
  const d = new Date(t);
  const day = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  // The Thursday of this week decides which year (and week) it belongs to.
  day.setUTCDate(day.getUTCDate() + 4 - (day.getUTCDay() || 7));
  const yearStart = Date.UTC(day.getUTCFullYear(), 0, 1);
  return Math.ceil(((day - yearStart) / 86_400_000 + 1) / 7);
}

const isoParity = (t) => (isoWeek(t) % 2 === 1 ? "odd" : "even");
const other = (p) => (p === "odd" ? "even" : "odd");

/** "odd" | "even" for a source in the week of `t`, as that school counts it. */
export function sourceParity(source, t) {
  const p = isoParity(t);
  return source.flip ? other(p) : p;
}

/** The `flip` that makes the week of `now` read as `parity` for this source. */
export function flipFor(parity, now) {
  return isoParity(now) !== parity;
}

/**
 * Everything on the timetable in the week of `t`, one shape for both kinds:
 * `{ day, start, end, time, subject, kind, room, color, sourceName }`. STAG
 * entries keep their kind and have `color` null; custom ones are coloured by
 * source. STAG odd/even classes follow ISO week parity.
 */
export function weekEntries(stagClasses, custom, t) {
  const out = [];
  const stagParity = isoParity(t);
  for (const c of stagClasses ?? []) {
    if (c.week && c.week !== "all" && c.week !== stagParity) continue;
    if (toMinutes(c.start) == null || toMinutes(c.end) == null) continue;
    out.push({
      day: c.day,
      start: c.start,
      end: c.end,
      time: `${c.start}–${c.end}`,
      subject: c.subject,
      kind: c.kind ?? "",
      room: c.room ?? "",
      color: null,
      sourceName: "STAG",
    });
  }
  const byId = new Map((custom?.sources ?? []).map((s) => [s.id, s]));
  for (const e of custom?.entries ?? []) {
    const src = byId.get(e.sourceId);
    if (!src) continue;
    if (e.week !== "all" && !(src.alternating && e.week === sourceParity(src, t))) continue;
    out.push({
      day: e.day,
      start: e.start,
      end: e.end,
      time: `${e.start}–${e.end}`,
      subject: e.title,
      kind: "",
      room: e.room,
      color: src.color,
      sourceName: src.name,
    });
  }
  return out.sort((a, b) => a.day - b.day || toMinutes(a.start) - toMinutes(b.start));
}

/**
 * The week's label for the widget badge: "odd"/"even" when every alternating
 * timetable agrees on it, null when none alternates or they disagree (two
 * schools counting differently) — then the badge falls back to the week number.
 */
export function weekLabel(stagClasses, custom, t) {
  const seen = new Set();
  if ((stagClasses ?? []).some((c) => c.week && c.week !== "all")) seen.add(isoParity(t));
  for (const s of custom?.sources ?? []) if (s.alternating) seen.add(sourceParity(s, t));
  return seen.size === 1 ? [...seen][0] : null;
}

/** `t` moved by whole weeks — the badge's preview of next week. */
export const shiftWeeks = (t, n) => t + n * WEEK_MS;

/**
 * Grid geometry for a week's entries. `from`/`to` are minutes, widened to
 * whole hours so the hour lines land on the grid edges. Each day gets as many
 * lanes as it has overlapping entries at once (greedy interval partitioning);
 * every item carries its `lane`. Weekend rows appear only when used.
 */
export function layoutWeek(entries, fallback = [8 * 60, 16 * 60]) {
  const items = entries
    .map((e) => ({ ...e, startMin: toMinutes(e.start), endMin: toMinutes(e.end) }))
    .filter((e) => e.startMin != null && e.endMin > e.startMin);

  let from = fallback[0];
  let to = fallback[1];
  if (items.length) {
    from = Math.floor(Math.min(...items.map((e) => e.startMin)) / 60) * 60;
    to = Math.ceil(Math.max(...items.map((e) => e.endMin)) / 60) * 60;
  }

  const lastDay = items.some((e) => e.day === 6) ? 6 : items.some((e) => e.day === 5) ? 5 : 4;
  const days = [];
  for (let d = 0; d <= lastDay; d++) {
    const dayItems = items.filter((e) => e.day === d).sort((a, b) => a.startMin - b.startMin);
    const laneEnds = [];
    for (const e of dayItems) {
      let lane = laneEnds.findIndex((end) => end <= e.startMin);
      if (lane === -1) lane = laneEnds.length;
      laneEnds[lane] = e.endMin;
      e.lane = lane;
    }
    days.push({ day: d, lanes: Math.max(1, laneEnds.length) });
  }
  return { from, to, days, items };
}

// ── edits (each returns a new value; the widget persists it) ────────────

const newId = (now) => `${now}-${Math.random().toString(36).slice(2, 8)}`;

export function addSource(custom, name, now) {
  const clean = text(name);
  if (!clean || custom.sources.length >= MAX_SOURCES) return custom;
  // Next colour not yet taken, so two schools never start out alike.
  const used = new Set(custom.sources.map((s) => s.color));
  let color = 0;
  while (used.has(color) && color < SRC_COLORS - 1) color++;
  const source = { id: newId(now), name: clean, color, alternating: false, flip: false };
  return { ...custom, sources: [...custom.sources, source] };
}

export function updateSource(custom, id, patch) {
  return {
    ...custom,
    sources: custom.sources.map((s) => (s.id === id ? { ...s, ...patch } : s)),
  };
}

export function removeSource(custom, id) {
  return {
    sources: custom.sources.filter((s) => s.id !== id),
    entries: custom.entries.filter((e) => e.sourceId !== id),
  };
}

/** Why an entry from the form cannot be saved, or null when it can. */
export function entryProblem(custom, entry) {
  if (!text(entry.title)) return "add a subject";
  const start = toMinutes(entry.start);
  const end = toMinutes(entry.end);
  if (start == null || end == null) return "fill in both times (HH:MM)";
  if (end <= start) return "the end has to be after the start";
  if (!custom.sources.some((s) => s.id === entry.sourceId)) return "pick a timetable first";
  const isNew = !custom.entries.some((e) => e.id === entry.id);
  if (isNew && custom.entries.length >= MAX_ENTRIES) return `at most ${MAX_ENTRIES} classes`;
  return null;
}

/** Add, or replace the entry with `entry.id`. Invalid input leaves it as is. */
export function putEntry(custom, entry, now) {
  if (entryProblem(custom, entry)) return custom;
  const start = toMinutes(entry.start);
  const end = toMinutes(entry.end);
  const title = text(entry.title);
  const clean = {
    id: entry.id || newId(now),
    sourceId: entry.sourceId,
    day: Math.min(6, Math.max(0, Number(entry.day) || 0)),
    start: fmtMinutes(start),
    end: fmtMinutes(end),
    title,
    room: text(entry.room),
    week: WEEKS.has(entry.week) ? entry.week : "all",
  };
  const exists = custom.entries.some((e) => e.id === clean.id);
  return {
    ...custom,
    entries: exists
      ? custom.entries.map((e) => (e.id === clean.id ? clean : e))
      : [...custom.entries, clean],
  };
}

export function removeEntry(custom, id) {
  return { ...custom, entries: custom.entries.filter((e) => e.id !== id) };
}

/** Replace one week of an alternating source with a copy of the other. */
export function copyWeek(custom, sourceId, fromWeek, now) {
  const toWeek = other(fromWeek);
  const kept = custom.entries.filter((e) => !(e.sourceId === sourceId && e.week === toWeek));
  const copies = custom.entries
    .filter((e) => e.sourceId === sourceId && e.week === fromWeek)
    .map((e, i) => ({ ...e, id: `${newId(now)}-${i}`, week: toWeek }));
  return { ...custom, entries: [...kept, ...copies].slice(0, MAX_ENTRIES) };
}
