import { test } from "node:test";
import assert from "node:assert/strict";
import { CYCLE_MS, cycleIndex, endingSoon, endsLabel, partition, startsLabel } from "../src/lib/freegames.js";

// Friday 23 Oct 2026, 12:00 UTC.
const NOW = Date.UTC(2026, 9, 23, 12, 0);
const H = 3_600_000;
const game = (title, starts_at, ends_at) => ({ title, starts_at, ends_at });

test("live games come ending soonest first, open-ended last", () => {
  const { live } = partition([game("a", null, NOW + 50 * H), game("b", null, null), game("c", null, NOW + 2 * H)], NOW);
  assert.deepEqual(live.map((g) => g.title), ["c", "a", "b"]);
});

test("an ended game drops out between polls", () => {
  const { live, upcoming } = partition([game("gone", null, NOW - 1), game("now", null, NOW + H)], NOW);
  assert.deepEqual(live.map((g) => g.title), ["now"]);
  assert.equal(upcoming.length, 0);
});

test("an upcoming game goes live once its start passes", () => {
  const next = game("next", NOW + H, NOW + 100 * H);
  assert.deepEqual(partition([next], NOW).upcoming.map((g) => g.title), ["next"]);
  assert.deepEqual(partition([next], NOW + H).live.map((g) => g.title), ["next"]);
});

test("an empty or missing list is just empty", () => {
  assert.deepEqual(partition(undefined, NOW), { live: [], upcoming: [] });
});

test("time left reads in the largest unit that fits", () => {
  assert.equal(endsLabel(NOW + 40 * 60_000, NOW), "ends in 40 min");
  assert.equal(endsLabel(NOW + 5 * H, NOW), "ends in 5 h");
  assert.equal(endsLabel(NOW + 75 * H, NOW), "ends in 3 d");
  assert.equal(endsLabel(null, NOW), "");
});

test("a start under a day away is relative, later ones name the day", () => {
  assert.equal(startsLabel(NOW + 3 * H, NOW, "en"), "in 3 h");
  assert.equal(startsLabel(Date.UTC(2026, 9, 29, 15, 0), NOW, "en"), "from Thu");
});

test("under a day left counts as ending soon", () => {
  assert.equal(endingSoon(game("x", null, NOW + 5 * H), NOW), true);
  assert.equal(endingSoon(game("x", null, NOW + 30 * H), NOW), false);
  assert.equal(endingSoon(game("x", null, null), NOW), false);
});

test("S steps through the games on the clock and wraps around", () => {
  const t0 = 1_000 * CYCLE_MS; // a period boundary
  assert.equal(cycleIndex(t0, 3), 1);
  assert.equal(cycleIndex(t0 + CYCLE_MS - 1, 3), 1);
  assert.equal(cycleIndex(t0 + CYCLE_MS, 3), 2);
  assert.equal(cycleIndex(t0 + 2 * CYCLE_MS, 3), 0);
  assert.equal(cycleIndex(t0, 1), 0);
  assert.equal(cycleIndex(t0, 0), 0);
});
