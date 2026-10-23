// CLOCK widget: local time, a few other time zones, and a countdown that can
// run as a plain timer or a pomodoro.
//
// Pure functions; every one that depends on the time takes `now` (epoch ms),
// so tests set the clock instead of waiting for it. The widget in main.js is
// the rendering and the tick around this.
//
// A timer is stored by when it ends, not by how much is left: the time left
// is always derived as `endsAt - now`, so it survives a reload, a suspend and
// a late tick without drifting. Paused, it keeps `left` instead.
//
//   { phase: "idle" | "timer" | "focus" | "break",
//     total, rounds, endsAt, left }   (ms; endsAt xor left is set while active)

export const MAX_ZONES = 3;
export const FOCUS_MS = 25 * 60_000;
export const BREAK_MS = 5 * 60_000;
/** An end missed by more than this (ARIA closed, machine asleep) clears silently. */
export const STALE_MS = 10 * 60_000;
export const MAX_MINUTES = 600;

export const IDLE = Object.freeze({ phase: "idle", total: 0, rounds: 0, endsAt: null, left: null });
const PHASES = new Set(["timer", "focus", "break"]);

// Building an Intl formatter is far slower than using one, and the widget
// formats every second.
const formatters = new Map();
function formatter(key, make) {
  let f = formatters.get(key);
  if (!f) {
    f = make();
    formatters.set(key, f);
  }
  return f;
}

// ── time zones ─────────────────────────────────────────────

/**
 * `name` as an IANA zone with its proper casing, or null if the engine can't
 * format it. The name is kept as typed rather than replaced by the engine's
 * resolved one: V8 resolves current names to the legacy ones it still treats
 * as canonical (Asia/Kolkata -> Asia/Calcutta, Europe/Kyiv -> Europe/Kiev).
 */
export function canonicalZone(name) {
  const zone = String(name ?? "").trim();
  if (!zone) return null;
  let resolved;
  try {
    resolved = new Intl.DateTimeFormat("en-US", { timeZone: zone }).resolvedOptions().timeZone;
  } catch {
    return null;
  }
  const lower = zone.toLowerCase();
  if (resolved.toLowerCase() === lower) return resolved;
  const listed = Intl.supportedValuesOf?.("timeZone").find((z) => z.toLowerCase() === lower);
  if (listed) return listed;
  // An accepted name the engine doesn't list: IANA casing is capitalised words.
  return zone.replace(/[^/_-]+/g, (w) => w[0].toUpperCase() + w.slice(1).toLowerCase());
}

export function localZone() {
  return new Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** Stored zones, keeping only names this engine can format, without duplicates. */
export function parseZones(raw) {
  let data;
  try {
    data = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(data)) return [];
  const out = [];
  for (const name of data) {
    const zone = canonicalZone(name);
    if (zone && !out.includes(zone) && out.length < MAX_ZONES) out.push(zone);
  }
  return out;
}

/** `{ zones, error }`; on an error the list comes back unchanged. */
export function addZone(zones, name) {
  const zone = canonicalZone(name);
  if (!zone) return { zones, error: "unknown time zone" };
  if (zones.includes(zone)) return { zones, error: "already added" };
  if (zones.length >= MAX_ZONES) return { zones, error: `at most ${MAX_ZONES} zones` };
  return { zones: [...zones, zone], error: null };
}

export function removeZone(zones, zone) {
  return zones.filter((z) => z !== zone);
}

/** "America/Argentina/Buenos_Aires" -> "Buenos Aires". */
export function zoneLabel(zone) {
  return zone.split("/").pop().replace(/_/g, " ");
}

// The wall-clock reading in `zone` at `now`. h23 because some engines print
// midnight as "24" under the default hour cycle.
function wallClock(zone, now) {
  const f = formatter(`wall|${zone}`, () =>
    new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
    }),
  );
  const p = {};
  for (const { type, value } of f.formatToParts(now)) p[type] = Number(value);
  return p;
}

/** Minutes `zone` is ahead of UTC at `now`. */
export function utcOffset(zone, now) {
  const w = wallClock(zone, now);
  const asUtc = Date.UTC(w.year, w.month - 1, w.day, w.hour, w.minute);
  return Math.round((asUtc - Math.floor(now / 60_000) * 60_000) / 60_000);
}

/** Minutes `zone` is ahead of `home`. */
export function zoneDiff(zone, now, home = localZone()) {
  return utcOffset(zone, now) - utcOffset(home, now);
}

/** -1 / 0 / 1: the calendar day in `zone` is before, the same as, or after `home`'s. */
export function dayShift(zone, now, home = localZone()) {
  const a = wallClock(zone, now);
  const b = wallClock(home, now);
  return Math.sign(Date.UTC(a.year, a.month - 1, a.day) - Date.UTC(b.year, b.month - 1, b.day));
}

/** "+6 h", "−3 h 30", "same time". */
export function fmtDiff(minutes) {
  if (minutes === 0) return "same time";
  const sign = minutes > 0 ? "+" : "−";
  const abs = Math.abs(minutes);
  const h = Math.floor(abs / 60);
  const m = abs % 60;
  return m ? `${sign}${h} h ${m}` : `${sign}${h} h`;
}

// ── formatting ─────────────────────────────────────────────

/** Time in the system locale's own 12/24 h convention; `locale` is for tests. */
export function fmtTime(now, { zone, seconds = false, locale } = {}) {
  const f = formatter(`time|${zone}|${seconds}|${locale}`, () =>
    new Intl.DateTimeFormat(locale, {
      timeZone: zone,
      hour: "numeric",
      minute: "2-digit",
      second: seconds ? "2-digit" : undefined,
    }),
  );
  return f.format(now);
}

export function fmtDate(now, { zone, locale, short = false } = {}) {
  const f = formatter(`date|${zone}|${locale}|${short}`, () =>
    new Intl.DateTimeFormat(locale, {
      timeZone: zone,
      weekday: short ? "short" : "long",
      day: "numeric",
      month: short ? "short" : "long",
    }),
  );
  return f.format(now);
}

/** "04:59", "1:30:00". Rounds up, so it reads 00:00 exactly when the timer ends. */
export function fmtLeft(ms) {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

export function phaseLabel(phase) {
  return { timer: "Timer", focus: "Focus", break: "Break" }[phase] ?? "";
}

// ── timer ──────────────────────────────────────────────────

/** Whole minutes typed into the custom field, as ms; null if out of range. */
export function parseMinutes(input) {
  const n = Number(String(input ?? "").trim());
  if (!Number.isInteger(n) || n < 1 || n > MAX_MINUTES) return null;
  return n * 60_000;
}

/** Stored timer state, or IDLE if anything about it is off. */
export function parseTimer(raw) {
  let s;
  try {
    s = JSON.parse(raw);
  } catch {
    return IDLE;
  }
  if (!s || !PHASES.has(s.phase) || !(s.total > 0)) return IDLE;
  const rounds = Number.isInteger(s.rounds) && s.rounds >= 0 ? s.rounds : 0;
  if (Number.isFinite(s.endsAt)) return { phase: s.phase, total: s.total, rounds, endsAt: s.endsAt, left: null };
  if (Number.isFinite(s.left) && s.left >= 0) return { phase: s.phase, total: s.total, rounds, endsAt: null, left: s.left };
  return IDLE;
}

function run(phase, total, rounds, now) {
  return { phase, total, rounds, endsAt: now + total, left: null };
}

export function startTimer(ms, now) {
  return run("timer", ms, 0, now);
}

export function startPomodoro(now) {
  return run("focus", FOCUS_MS, 0, now);
}

export const isRunning = (s) => s.phase !== "idle" && s.endsAt != null;
export const isPaused = (s) => s.phase !== "idle" && s.left != null;

export function pause(s, now) {
  if (!isRunning(s)) return s;
  return { ...s, endsAt: null, left: Math.max(0, s.endsAt - now) };
}

export function resume(s, now) {
  if (!isPaused(s)) return s;
  return { ...s, endsAt: now + s.left, left: null };
}

/** Time left in ms, or null when idle. */
export function remaining(s, now) {
  if (isRunning(s)) return Math.max(0, s.endsAt - now);
  if (isPaused(s)) return s.left;
  return null;
}

/** Share of the current phase already gone, 0..1. */
export function progress(s, now) {
  const left = remaining(s, now);
  if (left == null) return 0;
  return Math.min(1, Math.max(0, 1 - left / s.total));
}

/**
 * Advance past an end that has arrived. `ended` is the phase that just
 * finished (what to announce), or null when nothing did.
 *
 * A pomodoro rolls focus -> break -> focus on its own, the next phase timed
 * from now rather than from the missed end. An end missed by more than
 * STALE_MS clears without an announcement: a pomodoro that ran out while
 * ARIA was closed is over, not owed a chime hours later.
 */
export function tick(s, now) {
  if (!isRunning(s) || now < s.endsAt) return { state: s, ended: null };
  if (now - s.endsAt > STALE_MS) return { state: IDLE, ended: null };
  if (s.phase === "focus") return { state: run("break", BREAK_MS, s.rounds + 1, now), ended: "focus" };
  if (s.phase === "break") return { state: run("focus", FOCUS_MS, s.rounds, now), ended: "break" };
  return { state: IDLE, ended: "timer" };
}
