import { test } from "node:test";
import assert from "node:assert/strict";
import {
  dayLabel,
  describe,
  fmtPrecip,
  fmtTemp,
  fmtWind,
  hourlyChart,
  monotonePath,
  placeLine,
  rainLine,
  rangeBar,
} from "../src/lib/weather.js";

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

const hours = (probs, code = 61) =>
  probs.map((p, i) => ({ time: `${String(13 + i).padStart(2, "0")}:00`, temp: 10, code, precip_prob: p }));

test("the rain line says when rain starts and how likely it gets", () => {
  assert.deepEqual(rainLine(hours([10, 30, 70, 85, 40])), { text: "Rain from 15:00 · 85%", wet: true });
});

test("the rain line says when rain already falling stops", () => {
  assert.deepEqual(rainLine(hours([80, 60, 20, 0])), { text: "Rain until 15:00", wet: true });
  assert.equal(rainLine(hours([90, 90, 90])).text, "Rain for the next 3 h");
});

test("a dry forecast says so, snow says snow", () => {
  assert.deepEqual(rainLine(hours([0, 10, 49])), { text: "Dry for the next 3 h", wet: false });
  assert.equal(rainLine(hours([0, 70], 73)).text, "Snow from 14:00 · 70%");
});

test("missing chances are not read as dry", () => {
  assert.equal(rainLine(hours([0, null, 0])), null);
  assert.equal(rainLine([]), null);
});

test("the chart puts the warmest hour highest and bars in their own band", () => {
  const h = [8, 12, 10].map((temp, i) => ({ temp, precip_prob: [0, 50, 100][i] }));
  const c = hourlyChart(h, 300, 200);
  assert.equal(c.col, 100);
  assert.deepEqual(c.points.map((p) => p.x), [50, 150, 250]);
  assert.ok(c.points[1].y < c.points[2].y && c.points[2].y < c.points[0].y);
  // Every point stays above the rain band; full chance fills that band.
  for (const p of c.points) assert.ok(p.y <= c.tempBottom);
  assert.equal(c.bars[2].y, c.rainTop);
  assert.equal(c.bars[0].h, 0);
});

test("a flat forecast draws a level line in the middle", () => {
  const c = hourlyChart([{ temp: 5 }, { temp: 5 }], 100, 100);
  assert.equal(c.points[0].y, c.points[1].y);
});

test("the curve never overshoots its neighbouring hours", () => {
  const pts = [3, 9, 9.5, 2, 2, 7, 8].map((y, i) => ({ x: i * 40, y }));
  const nums = monotonePath(pts).match(/-?\d+(\.\d+)?/g).map(Number);
  // After the M x,y pair, each segment is three x,y pairs: two controls, then the end.
  for (let i = 0; i < pts.length - 1; i++) {
    const lo = Math.min(pts[i].y, pts[i + 1].y);
    const hi = Math.max(pts[i].y, pts[i + 1].y);
    for (const k of [1, 3]) {
      const cy = nums[2 + i * 6 + k];
      assert.ok(cy >= lo - 0.05 && cy <= hi + 0.05, `segment ${i} control y ${cy} outside ${lo}..${hi}`);
    }
  }
});
