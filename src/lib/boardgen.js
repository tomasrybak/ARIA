// Board generator: a complete board from a list of picked widgets, for the
// "Build a board" flow. Pure, so the packing can be tested without a DOM.
//
// Greedy and deterministic rather than an optimal packer. Exact packing is
// NP-hard, and on a 12×6 grid a board that is the same for the same answers,
// and whose every step can be explained, matters more than the last few cells.

import { GRID_COLS, GRID_ROWS, findFreeSpot } from "./layout.js";

const CELLS = GRID_COLS * GRID_ROWS;

/**
 * Starting points for the flow: picks in priority order (first matters most).
 * `builtin` names the shipped preset whose auto-switch trigger the purpose
 * offers when saved.
 */
export const PURPOSES = [
  {
    key: "gaming",
    label: "Gaming",
    builtin: "Gaming",
    picks: ["gpu", "perf", "thermals", "hardware", "discord", "music", "steam", "latency", "freegames"],
  },
  {
    key: "coding",
    label: "Coding",
    builtin: "Coding",
    picks: ["terminal", "github", "notes", "perf", "hardware", "music", "email", "discord"],
  },
  {
    key: "study",
    label: "Study",
    builtin: "Study",
    picks: ["stag", "calendar", "notes", "clock", "email", "weather", "music"],
  },
  {
    key: "everyday",
    label: "Everyday",
    picks: ["weather", "clock", "email", "calendar", "music", "github", "crypto", "screentime"],
  },
  { key: "empty", label: "Start empty", picks: [] },
];

const area = ([w, h]) => w * h;

/** A widget's sizes from smallest to largest footprint. */
function ladder(id, widgets) {
  const sizes = widgets[id].sizes;
  return Object.keys(sizes).sort((a, b) => area(sizes[a]) - area(sizes[b]));
}

function step(id, size, by, widgets) {
  const order = ladder(id, widgets);
  return order[order.indexOf(size) + by] ?? null;
}

function startSize(id, density, count, widgets) {
  const sizes = widgets[id].sizes;
  if (density === "glanceable") return ladder(id, widgets)[0];
  if (count <= 4 && sizes.l) return "l";
  return sizes.m ? "m" : ladder(id, widgets)[0];
}

/**
 * Place the picks at the wanted sizes: largest footprint first, ties in
 * priority order, each at the first free cell. A widget that does not fit
 * tries its smaller sizes; one that still does not is dropped, never overlapped.
 */
function pack(picks, want, widgets) {
  const layout = {};
  for (const id of Object.keys(widgets)) layout[id] = { c: 1, r: 1, size: widgets[id].home[2], hidden: true };
  const order = picks
    .map((id, rank) => ({ id, rank }))
    .sort((a, b) => area(widgets[b.id].sizes[want[b.id]]) - area(widgets[a.id].sizes[want[a.id]]) || a.rank - b.rank);
  const dropped = [];
  let covered = 0;
  for (const { id } of order) {
    const tries = ladder(id, widgets).filter((s) => area(widgets[id].sizes[s]) <= area(widgets[id].sizes[want[id]]));
    let placed = false;
    for (const size of tries.reverse()) {
      const [w, h] = widgets[id].sizes[size];
      const spot = findFreeSpot(w, h, id, layout, widgets);
      if (!spot) continue;
      layout[id] = { c: spot[0], r: spot[1], size, hidden: false };
      covered += w * h;
      placed = true;
      break;
    }
    if (!placed) dropped.push(id);
  }
  dropped.sort((a, b) => picks.indexOf(a) - picks.indexOf(b));
  return { layout, dropped, covered };
}

/** Higher is better: fewer drops, more of the board used, bigger where it matters. */
function score(result, picks, widgets) {
  let weighted = 0;
  picks.forEach((id, rank) => {
    const p = result.layout[id];
    if (!p.hidden) weighted += area(widgets[id].sizes[p.size]) * (picks.length - rank);
  });
  return [-result.dropped.length, result.covered, weighted];
}

function better(a, b) {
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

/**
 * A complete board for `picks` (ids in priority order).
 *
 * - "glanceable": every widget at its smallest size, nothing grown, and
 *   the tiles moved down to the bottom edge.
 * - "detailed": M (L with four picks or fewer), then hill-climb: try growing
 *   one widget, or growing one while shrinking a lower-priority one, keep the
 *   best move that covers more of the board, repeat until none does. Packing
 *   lets a widget that no longer fits fall back to a smaller size, and the
 *   climb keeps those fallbacks: that is how it reaches the L/M/S mixes a
 *   6-row grid needs, where growing alone strands a one-row strip.
 *
 * Before either, the lowest-priority widgets step down until the total fits
 * the grid's area. Returns `{ board, dropped }`: every widget listed (unpicked
 * ones hidden), so it applies like a shipped preset; `dropped` are picks that
 * found no room, in priority order.
 */
export function generateBoard(picks, density, widgets) {
  picks = [...new Set(picks)].filter((id) => widgets[id]);
  const want = {};
  for (const id of picks) want[id] = startSize(id, density, picks.length, widgets);

  const total = () => picks.reduce((sum, id) => sum + area(widgets[id].sizes[want[id]]), 0);
  for (let i = picks.length - 1; i >= 0 && total() > CELLS; ) {
    const smaller = step(picks[i], want[picks[i]], -1, widgets);
    if (smaller) want[picks[i]] = smaller;
    else i--;
  }

  let best = pack(picks, want, widgets);
  if (density === "detailed") {
    let bestScore = score(best, picks, widgets);
    for (;;) {
      let move = null;
      for (const [gi, grow] of picks.entries()) {
        const up = step(grow, want[grow], 1, widgets);
        if (!up) continue;
        const shrinks = [null, ...picks.slice(gi + 1)];
        for (const shrink of shrinks) {
          const down = shrink && step(shrink, want[shrink], -1, widgets);
          if (shrink && !down) continue;
          const trial = { ...want, [grow]: up, ...(shrink && { [shrink]: down }) };
          const result = pack(picks, trial, widgets);
          if (result.dropped.length > best.dropped.length || result.covered <= best.covered) continue;
          const s = score(result, picks, widgets);
          if (!move || better(s, move.score)) move = { result, score: s };
        }
      }
      if (!move || !better(move.score, bestScore)) break;
      // Continue from the sizes the pack actually used, fallbacks included.
      for (const id of picks) if (!move.result.layout[id].hidden) want[id] = move.result.layout[id].size;
      best = move.result;
      bestScore = move.score;
    }
  }
  if (density === "glanceable") anchorToBottom(best.layout, picks, widgets);
  return { board: best.layout, dropped: best.dropped };
}

/**
 * Slide a board that leaves empty rows down to the bottom edge: a few small
 * tiles read as a strip along the bottom, with the wallpaper above them,
 * rather than a cluster stuck to the header.
 */
function anchorToBottom(layout, picks, widgets) {
  const placed = picks.filter((id) => !layout[id].hidden);
  if (!placed.length) return;
  const bottom = Math.max(...placed.map((id) => layout[id].r + widgets[id].sizes[layout[id].size][1] - 1));
  for (const id of placed) layout[id].r += GRID_ROWS - bottom;
}
