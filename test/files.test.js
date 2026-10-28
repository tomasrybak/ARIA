import { test } from "node:test";
import assert from "node:assert/strict";
import { FILE_WEIGHT, MAX_BONUS, MAX_OPENS, fileLabel, fileLists, openBonus, parseOpens, recordOpen } from "../src/lib/files.js";

test("open counts parse defensively", () => {
  assert.deepEqual(parseOpens(null), {});
  assert.deepEqual(parseOpens("[1,2]"), {});
  assert.deepEqual(parseOpens('{"/a":{"n":2,"at":5},"/b":{"n":0,"at":5},"/c":"x"}'), { "/a": { n: 2, at: 5 } });
});

test("recording an open counts up and drops the least recently opened past the cap", () => {
  let opens = recordOpen({}, "/a", 1);
  opens = recordOpen(opens, "/a", 2);
  assert.deepEqual(opens["/a"], { n: 2, at: 2 });

  const full = Object.fromEntries(Array.from({ length: MAX_OPENS }, (_, i) => [`/f${i}`, { n: 1, at: i + 10 }]));
  const next = recordOpen(full, "/new", 9999);
  assert.equal(Object.keys(next).length, MAX_OPENS);
  assert.ok(!("/f0" in next));
  assert.ok("/new" in next);
});

test("the open bonus grows with use, is capped and keeps a file below an exact app", () => {
  assert.equal(openBonus(0), 0);
  assert.ok(openBonus(1) < openBonus(3));
  assert.equal(openBonus(1000), MAX_BONUS);
  assert.ok(FILE_WEIGHT * (1 + MAX_BONUS) < 1);
});

test("hits split into file and folder lists with weight, bonus and a label", () => {
  const hits = [
    { name: "xyz.xls", path: "/d/xyz.xls", folder: "Documents", dir: false, mtime: 1 },
    { name: "ARIA", path: "/dev/ARIA", folder: "Dev", dir: true, mtime: 1 },
  ];
  const { file, folder } = fileLists(hits, { "/d/xyz.xls": { n: 3, at: 1 } });
  assert.equal(file.length, 1);
  assert.equal(folder[0].name, "ARIA");
  assert.equal(file[0].weight, FILE_WEIGHT);
  assert.equal(file[0].bonus, openBonus(3));
  assert.equal(folder[0].bonus, 0);
  assert.equal(fileLabel(file[0]), "xyz.xls — Documents");
});
