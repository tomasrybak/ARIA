import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BREAK_MS,
  FOCUS_MS,
  IDLE,
  MAX_ZONES,
  STALE_MS,
  addZone,
  canonicalZone,
  dayShift,
  fmtDiff,
  fmtLeft,
  fmtTime,
  isPaused,
  isRunning,
  parseMinutes,
  parseTimer,
  parseZones,
  pause,
  progress,
  remaining,
  resume,
  startPomodoro,
  startTimer,
  tick,
  utcOffset,
  zoneDiff,
  zoneLabel,
} from "../src/lib/clock.js";

// Friday 23 Oct 2026, 10:00 UTC: Prague is still on summer time (CEST) until
// the 25th, New York until 1 Nov.
const NOW = Date.UTC(2026, 9, 23, 10, 0);
const MIN = 60_000;

// ── zones ──────────────────────────────────────────────────

test("a zone name is canonicalised, a made-up one rejected", () => {
  assert.equal(canonicalZone(" europe/prague "), "Europe/Prague");
  assert.equal(canonicalZone("Mars/Olympus_Mons"), null);
  assert.equal(canonicalZone(""), null);
});

test("a renamed zone keeps its current name, not the engine's legacy one", () => {
  // V8 resolves both of these to Asia/Calcutta and Europe/Kiev.
  assert.equal(canonicalZone("asia/kolkata"), "Asia/Kolkata");
  assert.equal(canonicalZone("Europe/Kyiv"), "Europe/Kyiv");
  assert.equal(canonicalZone("america/argentina/buenos_aires"), "America/Argentina/Buenos_Aires");
});

test("stored zones drop junk and duplicates and stop at the maximum", () => {
  const raw = JSON.stringify(["Asia/Tokyo", "nope", "asia/tokyo", "UTC", "Europe/London", "America/New_York"]);
  assert.deepEqual(parseZones(raw), ["Asia/Tokyo", "UTC", "Europe/London"]);
  assert.equal(parseZones(raw).length, MAX_ZONES);
  assert.deepEqual(parseZones("not json"), []);
  assert.deepEqual(parseZones('{"a":1}'), []);
});

test("adding a zone says why it was refused", () => {
  assert.deepEqual(addZone([], "Asia/Tokyo"), { zones: ["Asia/Tokyo"], error: null });
  assert.equal(addZone(["Asia/Tokyo"], "asia/tokyo").error, "already added");
  assert.equal(addZone([], "Nowhere/Town").error, "unknown time zone");
  const full = ["UTC", "Asia/Tokyo", "Europe/London"];
  assert.deepEqual(addZone(full, "America/New_York").zones, full);
});

test("the label is the city, readable", () => {
  assert.equal(zoneLabel("America/Argentina/Buenos_Aires"), "Buenos Aires");
  assert.equal(zoneLabel("UTC"), "UTC");
});

test("offsets follow daylight saving", () => {
  assert.equal(utcOffset("Europe/Prague", NOW), 120);
  // After the switch back on Sunday 25 Oct.
  assert.equal(utcOffset("Europe/Prague", Date.UTC(2026, 9, 26, 10, 0)), 60);
  assert.equal(utcOffset("Asia/Kolkata", NOW), 330);
});

test("the difference is relative to home, half hours included", () => {
  assert.equal(zoneDiff("Asia/Kolkata", NOW, "Europe/Prague"), 210);
  assert.equal(zoneDiff("America/New_York", NOW, "Europe/Prague"), -360);
  assert.equal(fmtDiff(210), "+3 h 30");
  assert.equal(fmtDiff(-360), "−6 h");
  assert.equal(fmtDiff(0), "same time");
});

test("a zone already in tomorrow or still in yesterday is marked", () => {
  // 00:30 on the 24th in Prague.
  const late = Date.UTC(2026, 9, 23, 22, 30);
  assert.equal(dayShift("America/New_York", late, "Europe/Prague"), -1);
  assert.equal(dayShift("Asia/Tokyo", late, "Europe/Prague"), 0);
  assert.equal(dayShift("Asia/Tokyo", late, "America/New_York"), 1);
});

test("the time follows the locale's own hour convention", () => {
  const t = Date.UTC(2026, 9, 23, 12, 5);
  assert.equal(fmtTime(t, { zone: "UTC", locale: "en-GB" }), "12:05");
  assert.match(fmtTime(t, { zone: "UTC", locale: "en-US" }), /^12:05\sPM$/u);
  assert.equal(fmtTime(t, { zone: "UTC", locale: "en-GB", seconds: true }), "12:05:00");
});

// ── timer ──────────────────────────────────────────────────

test("time left rounds up so the end reads 00:00 exactly", () => {
  assert.equal(fmtLeft(5 * MIN), "05:00");
  assert.equal(fmtLeft(400), "00:01");
  assert.equal(fmtLeft(0), "00:00");
  assert.equal(fmtLeft(-5), "00:00");
  assert.equal(fmtLeft(90 * MIN), "1:30:00");
});

test("custom minutes are whole and bounded", () => {
  assert.equal(parseMinutes(" 12 "), 12 * MIN);
  assert.equal(parseMinutes("0"), null);
  assert.equal(parseMinutes("1.5"), null);
  assert.equal(parseMinutes("601"), null);
  assert.equal(parseMinutes("ten"), null);
});

test("time left is derived from the end, not counted down", () => {
  const s = startTimer(10 * MIN, NOW);
  assert.equal(remaining(s, NOW + 3 * MIN), 7 * MIN);
  // A tick that never happened costs nothing: an hour of sleep is just later.
  assert.equal(remaining(s, NOW + 60 * MIN), 0);
  assert.equal(progress(s, NOW + 5 * MIN), 0.5);
});

test("pausing freezes what is left and resuming continues from it", () => {
  let s = startTimer(10 * MIN, NOW);
  s = pause(s, NOW + 4 * MIN);
  assert.ok(isPaused(s) && !isRunning(s));
  assert.equal(remaining(s, NOW + 50 * MIN), 6 * MIN);
  s = resume(s, NOW + 50 * MIN);
  assert.equal(s.endsAt, NOW + 56 * MIN);
  // A paused timer never ends on its own.
  assert.deepEqual(tick(pause(s, NOW + 51 * MIN), NOW + 200 * MIN).ended, null);
});

test("a plain timer ends once and goes idle", () => {
  const s = startTimer(5 * MIN, NOW);
  assert.equal(tick(s, NOW + 4 * MIN).ended, null);
  const t = tick(s, NOW + 5 * MIN);
  assert.equal(t.ended, "timer");
  assert.equal(t.state, IDLE);
  assert.equal(tick(t.state, NOW + 6 * MIN).ended, null);
});

test("a pomodoro rolls focus to break to focus, counting rounds", () => {
  let s = startPomodoro(NOW);
  let t = tick(s, NOW + FOCUS_MS + 2000);
  assert.equal(t.ended, "focus");
  assert.equal(t.state.phase, "break");
  assert.equal(t.state.rounds, 1);
  // The break is timed from when it was announced, not from the missed end.
  assert.equal(t.state.endsAt, NOW + FOCUS_MS + 2000 + BREAK_MS);
  t = tick(t.state, t.state.endsAt);
  assert.equal(t.ended, "break");
  assert.equal(t.state.phase, "focus");
  assert.equal(t.state.rounds, 1);
});

test("an end missed long ago clears without an announcement", () => {
  const s = startPomodoro(NOW);
  const t = tick(s, NOW + FOCUS_MS + STALE_MS + 1);
  assert.equal(t.ended, null);
  assert.equal(t.state, IDLE);
});

test("stored timer state survives a reload and junk reads as idle", () => {
  const running = startTimer(5 * MIN, NOW);
  assert.deepEqual(parseTimer(JSON.stringify(running)), running);
  const paused = pause(running, NOW + MIN);
  assert.deepEqual(parseTimer(JSON.stringify(paused)), paused);
  assert.equal(parseTimer("garbage"), IDLE);
  assert.equal(parseTimer('{"phase":"focus","total":0,"endsAt":1}'), IDLE);
  assert.equal(parseTimer('{"phase":"lunch","total":5,"endsAt":1}'), IDLE);
});
