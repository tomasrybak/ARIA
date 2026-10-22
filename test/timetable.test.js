import { test } from "node:test";
import assert from "node:assert/strict";
import {
  addSource,
  copyWeek,
  entryProblem,
  flipFor,
  isoWeek,
  layoutWeek,
  parseCustom,
  putEntry,
  removeSource,
  sourceParity,
  updateSource,
  weekEntries,
  weekLabel,
} from "../src/lib/timetable.js";

// Thursday 22 Oct 2026 is in ISO week 43 (odd); a week later is week 44.
const NOW = new Date(2026, 9, 22, 9, 0).getTime();
const NEXT = new Date(2026, 9, 29, 9, 0).getTime();

function school(alternating = false, flip = false) {
  let c = addSource({ sources: [], entries: [] }, "SPŠ", NOW);
  return updateSource(c, c.sources[0].id, { alternating, flip });
}

test("isoWeek follows ISO 8601, including year boundaries", () => {
  assert.equal(isoWeek(NOW), 43);
  assert.equal(isoWeek(new Date(2026, 0, 1).getTime()), 1); // Thursday
  assert.equal(isoWeek(new Date(2027, 0, 1).getTime()), 53); // Friday of 2026-W53
  assert.equal(isoWeek(new Date(2025, 11, 29).getTime()), 1); // Monday of 2026-W01
});

test("parseCustom drops malformed entries and orphans", () => {
  const raw = JSON.stringify({
    sources: [{ id: "s", name: " SPŠ ", color: 9 }, { id: "", name: "x" }],
    entries: [
      { id: "a", sourceId: "s", day: 0, start: "8:00", end: "08:45", title: "Mat" },
      { id: "b", sourceId: "s", day: 0, start: "09:00", end: "08:00", title: "backwards" },
      { id: "c", sourceId: "gone", day: 0, start: "08:00", end: "09:00", title: "orphan" },
      { id: "d", sourceId: "s", day: 7, start: "08:00", end: "09:00", title: "no day" },
      { id: "a", sourceId: "s", day: 1, start: "08:00", end: "09:00", title: "dup id" },
    ],
  });
  const c = parseCustom(raw);
  assert.deepEqual(c.sources, [{ id: "s", name: "SPŠ", color: 3, alternating: false, flip: false }]);
  assert.deepEqual(c.entries.map((e) => [e.id, e.start, e.week]), [["a", "08:00", "all"]]);
  assert.deepEqual(parseCustom("not json"), { sources: [], entries: [] });
});

test("an alternating source shows its odd week, then its even week", () => {
  let c = school(true);
  const sid = c.sources[0].id;
  c = putEntry(c, { sourceId: sid, day: 0, start: "08:00", end: "08:45", title: "odd", week: "odd" }, NOW);
  c = putEntry(c, { sourceId: sid, day: 0, start: "08:00", end: "08:45", title: "even", week: "even" }, NOW);
  c = putEntry(c, { sourceId: sid, day: 1, start: "08:00", end: "08:45", title: "both", week: "all" }, NOW);
  assert.deepEqual(weekEntries([], c, NOW).map((e) => e.subject), ["odd", "both"]);
  assert.deepEqual(weekEntries([], c, NEXT).map((e) => e.subject), ["even", "both"]);
});

test("flip lets a school count weeks the other way round", () => {
  const flip = flipFor("even", NOW); // the user says "this week is even"
  assert.equal(flip, true);
  assert.equal(sourceParity({ flip }, NOW), "even");
  assert.equal(sourceParity({ flip }, NEXT), "odd");
});

test("odd/even entries hide while their source does not alternate", () => {
  let c = school(false);
  const sid = c.sources[0].id;
  c = putEntry(c, { sourceId: sid, day: 0, start: "08:00", end: "08:45", title: "odd", week: "odd" }, NOW);
  assert.equal(weekEntries([], c, NOW).length, 0);
});

test("STAG odd/even classes follow ISO parity; plain ones every week", () => {
  const stag = [
    { day: 2, start: "10:15", end: "11:55", subject: "KIV/PT", kind: "Př", week: "all" },
    { day: 3, start: "08:25", end: "10:00", subject: "KIV/OS", kind: "Cv", week: "even" },
  ];
  assert.deepEqual(weekEntries(stag, null, NOW).map((e) => e.subject), ["KIV/PT"]);
  assert.deepEqual(weekEntries(stag, null, NEXT).map((e) => e.subject), ["KIV/PT", "KIV/OS"]);
  assert.equal(weekLabel(stag, null, NOW), "odd");
  assert.equal(weekLabel(stag.slice(0, 1), null, NOW), null);
});

test("weekLabel gives up when two schools disagree on the parity", () => {
  let c = school(true, false);
  c = addSource(c, "Other", NOW);
  c = updateSource(c, c.sources[1].id, { alternating: true, flip: true });
  assert.equal(weekLabel([], c, NOW), null);
});

test("overlapping entries get their own lanes; back-to-back ones share", () => {
  const e = (start, end, day = 0) => ({ day, start, end });
  const g = layoutWeek([e("08:00", "09:30"), e("09:00", "10:00"), e("09:30", "10:15"), e("13:05", "14:40", 2)]);
  assert.deepEqual(g.items.filter((i) => i.day === 0).map((i) => i.lane), [0, 1, 0]);
  assert.deepEqual(g.days.map((d) => d.lanes), [2, 1, 1, 1, 1]);
  assert.equal(g.from, 8 * 60);
  assert.equal(g.to, 15 * 60);
});

test("weekend rows appear only when something is on them", () => {
  assert.equal(layoutWeek([{ day: 0, start: "08:00", end: "09:00" }]).days.length, 5);
  assert.equal(layoutWeek([{ day: 5, start: "08:00", end: "09:00" }]).days.length, 6);
  assert.deepEqual([layoutWeek([]).from, layoutWeek([]).to], [480, 960]);
});

test("copyWeek replaces the other week instead of piling up duplicates", () => {
  let c = school(true);
  const sid = c.sources[0].id;
  c = putEntry(c, { sourceId: sid, day: 0, start: "08:00", end: "08:45", title: "A", week: "odd" }, NOW);
  c = copyWeek(c, sid, "odd", NOW);
  c = copyWeek(c, sid, "odd", NOW);
  assert.deepEqual(c.entries.map((e) => e.week).sort(), ["even", "odd"]);
});

test("putEntry edits in place and rejects a backwards time", () => {
  let c = school();
  const sid = c.sources[0].id;
  c = putEntry(c, { sourceId: sid, day: 0, start: "08:00", end: "08:45", title: "A" }, NOW);
  const id = c.entries[0].id;
  c = putEntry(c, { id, sourceId: sid, day: 1, start: "09:00", end: "09:45", title: "B" }, NOW);
  assert.deepEqual(c.entries.map((e) => [e.title, e.day]), [["B", 1]]);
  assert.equal(putEntry(c, { sourceId: sid, day: 0, start: "10:00", end: "09:00", title: "x" }, NOW), c);
  assert.equal(removeSource(c, sid).entries.length, 0);
});

test("entryProblem says what is wrong instead of failing silently", () => {
  const c = school();
  const sid = c.sources[0].id;
  const ok = { sourceId: sid, day: 1, start: "10:45", end: "11:30", title: "PVA" };
  assert.equal(entryProblem(c, ok), null);
  assert.equal(entryProblem(c, { ...ok, title: " " }), "add a subject");
  assert.equal(entryProblem(c, { ...ok, end: "" }), "fill in both times (HH:MM)");
  assert.equal(entryProblem(c, { ...ok, end: "10:00" }), "the end has to be after the start");
});
