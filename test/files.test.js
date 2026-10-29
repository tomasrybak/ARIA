import { test } from "node:test";
import assert from "node:assert/strict";
import {
  FILE_WEIGHT,
  MAX_BONUS,
  MAX_OPENS,
  droppedFolders,
  fileLabel,
  fileLists,
  openBonus,
  parseOpens,
  recordOpen,
  searchStatusText,
} from "../src/lib/files.js";

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

test("the search status line covers off, indexing, built and capped", () => {
  const built = { entries: 12345, age: 190, building: false, capped: false };
  assert.match(searchStatusText(built, false), /^Off/);
  assert.equal(searchStatusText({ ...built, building: true }, true), "Indexing…");
  assert.equal(searchStatusText({ ...built, age: null }, true), "Indexing…");
  assert.equal(searchStatusText(built, true), "12,345 names · updated 3 min ago");
  assert.equal(searchStatusText({ ...built, age: 20 }, true), "12,345 names · updated just now");
  assert.equal(searchStatusText({ ...built, age: 7300 }, true), "12,345 names · updated 2 h ago");
  assert.equal(searchStatusText({ ...built, entries: 100000, capped: true }, true), "Stopped at 100,000 names: choose smaller folders.");
});

test("dropped folders name the folder that covers them", () => {
  const saved = ["/home/t/Dev", "C:\\Users\\t\\Documents", "D:\\"];
  assert.deepEqual(droppedFolders(["/home/t/Dev"], saved), []);
  assert.deepEqual(droppedFolders(["/home/t/Dev/ARIA"], saved), ["/home/t/Dev/ARIA is already covered by /home/t/Dev."]);
  assert.deepEqual(droppedFolders(["/home/t/Devices"], saved), ["/home/t/Devices isn't a folder."]);
  assert.deepEqual(droppedFolders(["C:\\Users\\t\\Documents\\school"], saved), [
    "C:\\Users\\t\\Documents\\school is already covered by C:\\Users\\t\\Documents.",
  ]);
  assert.deepEqual(droppedFolders(["D:\\games"], saved), ["D:\\games is already covered by D:\\."]);
});
