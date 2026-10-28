// The widget catalogue: what size presets each widget offers, and where it
// sits on a fresh board. Extracted from main.js so the placement can be
// checked by `npm test` without a DOM — a default board that overlaps or
// overflows would otherwise only show up as a scrambled HUD on first run.

/**
 * Size presets, on a strict column rhythm: every preset is 3, 6 or 12 columns
 * wide — a third, a half, or the whole 12-column board. Nothing is 4, 5 or 8
 * wide any more, because those never line up with anything else: a 5-wide
 * widget beside a 3-wide one leaves a 4-column slot that only fits a widget
 * nobody has. Heights stay free; differing heights just stack.
 *
 *   S = [3,1]  one-line headline    M = [3,2]  the standard tile
 *   L = [6,3]  double-wide feature
 *
 * STAG and TERMINAL are the two exceptions, and only on the width they cannot
 * give up: a five-day timetable and an 80-column shell genuinely do not fit in
 * a third of the board. They are still 6 or 12 wide, so they tile with the
 * rest — STAG just takes a taller M, since a week compressed into two rows is
 * unreadable.
 *
 * `home` = [col, row, size] on a fresh board. Those coordinates fill the grid
 * exactly (see the test), so the defaults are a finished board rather than an
 * arbitrary scattering.
 */
export const WIDGETS = {
  // STAG and TERMINAL share a footprint so they sit level side by side, and
  // so an L of either is the top two-thirds of the board with a strip of
  // widgets still visible underneath — a full-board terminal would leave
  // nothing to glance at, which is the whole point of the HUD.
  stag:       { sizes: { s: [3, 2], m: [6, 4], l: [12, 4] }, home: [1, 1, "m"],
              cat: "work", blurb: "Your week of lectures from STAG, with rooms" },
  terminal:   { sizes: { m: [6, 4], l: [12, 4] }, home: [7, 1, "m"],
              cat: "work", blurb: "A real shell, right on the board" },
  hardware:   { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 5, "m"],
              cat: "system", blurb: "CPU, RAM and GPU load as gauges" },
  music:      { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [4, 5, "m"],
              cat: "media", blurb: "What is playing, with playback controls" },
  // The bottom-right strip is one row tall, which is exactly what S is for:
  // the glanceable headline (unread count, who is in voice, the current
  // price) with the list dropped.
  email:      { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [7, 5, "s"],
              cat: "work", blurb: "Unread mail across your IMAP accounts" },
  github:     { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [10, 5, "s"],
              cat: "work", blurb: "Notifications, open PRs and your contribution wall" },
  discord:    { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [7, 6, "s"],
              cat: "media", blurb: "Who is in voice, and what friends are playing" },
  crypto:     { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [10, 6, "s"],
              cat: "daily", blurb: "BTC and ETH price with a sparkline" },
  // The default board is full, so everything below starts in the tray; drag
  // one in (or shrink something) to place it. The four telemetry widgets are
  // deliberately not on the default board: they are a gaming loadout, not
  // everyday chrome, and the "Gaming" preset places all four at once.
  screentime: { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "daily", blurb: "Time on screen today and your top apps" },
  calendar:   { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "work", blurb: "Upcoming events from your calendar feeds" },
  weather:    { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "daily", blurb: "Conditions now, rain ahead and the week" },
  // No L: a clock and a timer have nothing to fill a half-board with.
  clock:      { sizes: { s: [3, 1], m: [3, 2] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "daily", blurb: "Time zones and a pomodoro timer" },
  freegames:  { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "media", blurb: "Games free to keep on Epic, Steam and GOG" },
  steam:      { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "media", blurb: "Friends online, what they play and your recent games" },
  gpu:        { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "system", blurb: "Load, VRAM, temperature, power and clocks" },
  thermals:   { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "system", blurb: "CPU, GPU and drive temperatures" },
  perf:       { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "system", blurb: "CPU, GPU and memory over the last minutes" },
  latency:    { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "system", blurb: "Ping, jitter and packet loss to any host" },
  notes:      { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "work", blurb: "To-dos with pins and reminders" },
  // The one widget that looks backwards: what today is like next to the days
  // before it. Same footprint as SCREEN TIME, which it reads alongside.
  trends:     { sizes: { s: [3, 1], m: [3, 2], l: [6, 3] }, home: [1, 1, "m"], defaultHidden: true,
              cat: "daily", blurb: "Today next to your usual days" },
};

/** Edit-panel groups, in display order; every widget's `cat` is one of these keys. */
export const CATEGORIES = [
  ["system", "System"],
  ["work", "Work & study"],
  ["media", "Games & media"],
  ["daily", "Daily info"],
];

/** A fresh board: every widget at its home cell and size. */
export function defaultLayout() {
  const layout = {};
  for (const [id, w] of Object.entries(WIDGETS)) {
    layout[id] = { c: w.home[0], r: w.home[1], size: w.home[2], hidden: !!w.defaultHidden };
  }
  return layout;
}
