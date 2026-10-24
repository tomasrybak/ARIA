import { test } from "node:test";
import assert from "node:assert/strict";
import { WIDGETS } from "../src/lib/widgets.js";
import { PURPOSES, generateBoard } from "../src/lib/boardgen.js";
import { GRID_COLS, GRID_ROWS, normalizeLayout, overlaps, rectOf } from "../src/lib/layout.js";
import { BUILTIN_PRESETS } from "../src/lib/presets.js";

const DENSITIES = ["glanceable", "detailed"];
const visible = (board) => Object.keys(board).filter((id) => !board[id].hidden);
const cells = (board) =>
  visible(board).reduce((sum, id) => {
    const r = rectOf(id, board, WIDGETS);
    return sum + r.w * r.h;
  }, 0);

function assertValid(board, label) {
  const placed = visible(board);
  for (const id of placed) {
    const r = rectOf(id, board, WIDGETS);
    assert.ok(r.c >= 1 && r.c + r.w <= GRID_COLS + 1, `${label}: ${id} runs off the side`);
    assert.ok(r.r >= 1 && r.r + r.h <= GRID_ROWS + 1, `${label}: ${id} runs off the bottom`);
  }
  for (let i = 0; i < placed.length; i++) {
    for (let j = i + 1; j < placed.length; j++) {
      const [a, b] = [placed[i], placed[j]];
      assert.equal(overlaps(rectOf(a, board, WIDGETS), rectOf(b, board, WIDGETS)), false, `${label}: ${a} overlaps ${b}`);
    }
  }
}

test("every purpose names real widgets, and builtins name shipped presets", () => {
  for (const p of PURPOSES) {
    for (const id of p.picks) assert.ok(WIDGETS[id], `${p.key} picks unknown widget ${id}`);
    assert.equal(new Set(p.picks).size, p.picks.length, `${p.key} picks a widget twice`);
    if (p.builtin) assert.ok(BUILTIN_PRESETS.some((b) => b.name === p.builtin), `${p.key} names ${p.builtin}`);
  }
});

test("every purpose fits without drops, inside the grid, at both densities", () => {
  for (const p of PURPOSES) {
    for (const d of DENSITIES) {
      const { board, dropped } = generateBoard(p.picks, d, WIDGETS);
      assert.deepEqual(dropped, [], `${p.key}/${d} dropped ${dropped}`);
      assertValid(board, `${p.key}/${d}`);
      assert.deepEqual(visible(board).sort(), [...p.picks].sort(), `${p.key}/${d} shows other widgets`);
    }
  }
});

test("a detailed board leaves less than an M tile unused", () => {
  // Only where the picks could fill it: Study's seven are 3 cells short.
  for (const p of PURPOSES.filter((p) => p.picks.length >= 7)) {
    const { board } = generateBoard(p.picks, "detailed", WIDGETS);
    assert.ok(GRID_COLS * GRID_ROWS - cells(board) <= 6, `${p.key} leaves ${72 - cells(board)} cells`);
  }
});

test("the board lists every widget, so it applies like a shipped preset", () => {
  const { board } = generateBoard(["clock"], "detailed", WIDGETS);
  assert.deepEqual(Object.keys(board).sort(), Object.keys(WIDGETS).sort());
  for (const [id, p] of Object.entries(board)) assert.ok(WIDGETS[id].sizes[p.size], `${id} at size ${p.size}`);
  // normalizeLayout has nothing to repair on a generated board.
  assert.equal(normalizeLayout(structuredClone(board), WIDGETS), false);
});

test("every pick is either placed or dropped, and drops hit the lowest priorities", () => {
  const all = Object.keys(WIDGETS);
  for (const d of DENSITIES) {
    const { board, dropped } = generateBoard(all, d, WIDGETS);
    assertValid(board, `all/${d}`);
    for (const id of all) assert.ok(!board[id].hidden !== dropped.includes(id), `${d}: ${id} lost`);
    // All 19 cannot fit even at their smallest; the ones left out are the
    // tail of the list, never a widget ranked above one that was kept.
    assert.ok(dropped.length > 0, `${d}: expected the full catalogue to overflow`);
    const lastPlaced = Math.max(...all.filter((id) => !board[id].hidden).map((id) => all.indexOf(id)));
    for (const id of dropped) assert.ok(all.indexOf(id) > lastPlaced, `${d}: ${id} dropped above a kept widget`);
  }
});

test("glanceable gives smaller tiles than detailed", () => {
  for (const p of PURPOSES.filter((p) => p.picks.length)) {
    const g = generateBoard(p.picks, "glanceable", WIDGETS).board;
    const d = generateBoard(p.picks, "detailed", WIDGETS).board;
    assert.ok(cells(g) < cells(d), `${p.key}: ${cells(g)} vs ${cells(d)} cells`);
  }
});

test("a glanceable board sits on the bottom edge", () => {
  for (const p of PURPOSES.filter((p) => p.picks.length)) {
    const { board } = generateBoard(p.picks, "glanceable", WIDGETS);
    const bottom = Math.max(...visible(board).map((id) => rectOf(id, board, WIDGETS)).map((r) => r.r + r.h - 1));
    assert.equal(bottom, GRID_ROWS, `${p.key} ends on row ${bottom}`);
  }
});

test("a few picks get the large size", () => {
  const { board } = generateBoard(["weather", "github"], "detailed", WIDGETS);
  assert.equal(board.weather.size, "l");
  assert.equal(board.github.size, "l");
});

test("the same answers always give the same board", () => {
  for (const p of PURPOSES) {
    for (const d of DENSITIES) {
      assert.deepEqual(generateBoard(p.picks, d, WIDGETS), generateBoard(p.picks, d, WIDGETS));
    }
  }
});

test("unknown and repeated picks are ignored", () => {
  const { board, dropped } = generateBoard(["clock", "nope", "clock"], "detailed", WIDGETS);
  assert.deepEqual(visible(board), ["clock"]);
  assert.deepEqual(dropped, []);
});

test("no picks give an empty board", () => {
  const { board, dropped } = generateBoard([], "detailed", WIDGETS);
  assert.deepEqual(visible(board), []);
  assert.deepEqual(dropped, []);
});
