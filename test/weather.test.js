import { test } from "node:test";
import assert from "node:assert/strict";
import { dayLabel, describe, fmtPrecip, fmtTemp, fmtWind, placeLine, rangeBar } from "../src/lib/weather.js";

test("WMO codes map to a label and icon", () => {
  assert.deepEqual(describe(0), { label: "Clear", icon: "clear" });
  assert.deepEqual(describe(63), { label: "Rain", icon: "rain" });
  assert.deepEqual(describe(95), { label: "Thunderstorm", icon: "storm" });
});

test("clear and partly cloudy switch to night icons after dark", () => {
  assert.equal(describe(0, false).icon, "night");
  assert.equal(describe(2, false).icon, "night-partly");
  // Rain at night is still rain.
  assert.equal(describe(61, false).icon, "rain");
});

test("an unknown code reads as cloud, not sun", () => {
  assert.deepEqual(describe(42), { label: "Unknown", icon: "cloud" });
});

test("temperatures round and never show minus zero", () => {
  assert.equal(fmtTemp(11.4), "11°");
  assert.equal(fmtTemp(-0.4), "0°");
  assert.equal(fmtTemp(-3.6), "-4°");
  assert.equal(fmtTemp(undefined), "—");
});

test("wind carries the unit for the chosen system", () => {
  assert.equal(fmtWind(15.3, "metric"), "15 km/h");
  assert.equal(fmtWind(9.6, "imperial"), "10 mph");
});

test("precipitation below 20 % is hidden", () => {
  assert.equal(fmtPrecip(10), "");
  assert.equal(fmtPrecip(20), "20%");
  assert.equal(fmtPrecip(null), "");
});

test("day labels come from the calendar date, not the viewer's time zone", () => {
  assert.equal(dayLabel("2026-10-22", 0), "Today");
  assert.equal(dayLabel("2026-10-23", 1), "Fri");
  assert.equal(dayLabel("2026-10-25", 3), "Sun");
  assert.equal(dayLabel("garbage", 2), "");
});

test("a place without region or country is just its name", () => {
  assert.equal(placeLine({ name: "Plzeň", detail: "Plzeňský kraj, Czechia" }), "Plzeň · Plzeňský kraj, Czechia");
  assert.equal(placeLine({ name: "Nowhere", detail: "" }), "Nowhere");
});

test("range bars share the week's scale", () => {
  assert.deepEqual(rangeBar(0, 10, 0, 20), { left: 0, width: 50 });
  assert.deepEqual(rangeBar(10, 20, 0, 20), { left: 50, width: 50 });
});

test("a flat day still draws a sliver, and a flat week a full bar", () => {
  assert.equal(rangeBar(5, 5, 0, 10).width, 2);
  assert.deepEqual(rangeBar(5, 5, 5, 5), { left: 0, width: 100 });
});
