import { test } from "node:test";
import assert from "node:assert/strict";
import {
  PLACEMENT_KEY,
  STORAGE_KEYS,
  collectStorage,
  describeBundle,
  passphraseProblem,
  placementChoice,
  planImport,
} from "../src/lib/settings.js";

const getter = (obj) => (k) => (k in obj ? obj[k] : null);

test("export collects only allowlisted keys, placement included", () => {
  const out = collectStorage(
    getter({
      "aria-theme": "nord",
      "aria-window": "{}",
      "aria-automation-state": "{}",
      "aria-clock-timer": "{}",
      unrelated: "x",
    }),
  );
  assert.deepEqual(out, { "aria-theme": "nord", "aria-window": "{}" });
});

test("machine moments never travel", () => {
  for (const k of ["aria-automation-state", "aria-clock-timer"]) {
    assert.ok(!STORAGE_KEYS.includes(k), k);
  }
  assert.ok(!STORAGE_KEYS.includes(PLACEMENT_KEY), "placement is opt-in on import");
});

test("passphrase rules", () => {
  assert.match(passphraseProblem("short"), /at least 10/);
  assert.equal(passphraseProblem("long enough pass"), null);
  assert.match(passphraseProblem("long enough pass", "long enough pasS"), /differ/);
  // Counted in characters, not UTF-16 units or bytes.
  assert.match(passphraseProblem("žluťoučký"), /at least/);
});

test("import replaces: missing allowlisted keys are removed", () => {
  const plan = planImport({ "aria-theme": "nord", "aria-timetable": "{}" });
  assert.deepEqual(plan.set, { "aria-theme": "nord", "aria-timetable": "{}" });
  assert.ok(plan.remove.includes("aria-notes"));
  assert.ok(!plan.remove.includes("aria-theme"));
});

test("import never writes keys outside the allowlist", () => {
  const plan = planImport({ "aria-theme": "nord", "aria-evil": "x", "aria-clock-timer": "{}" });
  assert.deepEqual(Object.keys(plan.set), ["aria-theme"]);
  assert.deepEqual(plan.ignored, ["aria-evil", "aria-clock-timer"]);
});

test("placement is applied only when chosen, and never removed", () => {
  const storage = { [PLACEMENT_KEY]: "{\"x\":0}" };
  assert.ok(!(PLACEMENT_KEY in planImport(storage).set));
  assert.equal(planImport(storage, { placement: true }).set[PLACEMENT_KEY], "{\"x\":0}");
  assert.ok(!planImport({}).remove.includes(PLACEMENT_KEY));
});

test("placement choice", () => {
  const mon = { x: 1920, y: 0, w: 1920, h: 1080 };
  const imported = JSON.stringify({ x: 1920, y: 0, w: 1920, h: 1080, mon });
  const local = JSON.stringify({ x: 0, y: 0, w: 1280, h: 800, mon: { x: 0, y: 0, w: 1920, h: 1080 } });
  assert.equal(placementChoice(null, local, [mon]), "none");
  assert.equal(placementChoice("not json", local, [mon]), "none");
  assert.equal(placementChoice(imported, imported, [mon]), "same");
  assert.equal(placementChoice(imported, local, [mon]), "ask");
  assert.equal(placementChoice(imported, local, [{ x: 0, y: 0, w: 2560, h: 1440 }]), "elsewhere");
});

test("bundle description", () => {
  const text = describeBundle({
    app: "0.2.0",
    exported: "2026-10-27T10:00:00+01:00",
    storage: {
      "aria-presets": JSON.stringify([{ name: "A" }, { name: "B" }]),
      "aria-timetable": JSON.stringify({ sources: [{}], entries: [] }),
      "aria-theme": "nord",
    },
    config: { mail: {}, weather: {} },
  });
  assert.equal(text, "2 boards · 1 timetable · nord theme · mail, weather (2026-10-27, ARIA 0.2.0)");
  assert.equal(describeBundle({}), "no settings");
});
