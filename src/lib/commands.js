// ARIA's actions, the content of the registry (mechanism in actions.js).
// Everything that touches the app goes through `ctx`, so each action can be
// tested with a fake one:
//   ctx.registry          the registry itself (help lists it)
//   ctx.openUrl(url)      system opener
//   ctx.music()           latest "music" collector state, or null
//   ctx.musicControl(op)  "playpause" | "next" | "previous"
//   ctx.showBoard(name)   apply a board preset, as a click in the menu would
//   ctx.startTimer(secs)  ctx.stopTimer() → false when no timer was running
//
// Name lists for slots (see commandLists in main.js):
//   game  [{ name, id }]   installed Steam games
//   board [{ name }]       board presets
//   link  [{ name, url }]  saved links

import { defineActions, describe } from "./actions.js";
import { match } from "./matcher.js";

const HELP_EXAMPLES = 8;

/** "1 hour 20 minutes", "90 seconds" — for answers that may be spoken. */
export function spokenDuration(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const unit = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
  const parts = [];
  if (h) parts.push(unit(h, "hour"));
  if (m) parts.push(unit(m, "minute"));
  if (s && !h) parts.push(unit(s, "second"));
  return parts.join(" ") || unit(0, "second");
}

function isPlaying(ctx) {
  return ctx.music()?.status === "playing";
}

export function buildActions() {
  return [
    {
      id: "game.launch",
      phrases: ["launch {game}", "play {game}", "start {game}"],
      bare: "{game}",
      widget: "steam",
      label: (a) => `Launch ${a.game.name}`,
      run: (ctx, a) => {
        ctx.openUrl(`steam://rungameid/${a.game.id}`);
        return `Launching ${a.game.name}`;
      },
    },
    {
      id: "media.next",
      phrases: ["next song", "next track", "skip [this] [song]", "next"],
      widget: "music",
      run: (ctx) => ctx.musicControl("next"),
    },
    {
      id: "media.previous",
      phrases: ["previous song", "previous track", "last song", "previous"],
      widget: "music",
      run: (ctx) => ctx.musicControl("previous"),
    },
    {
      id: "media.pause",
      phrases: ["pause [the] music", "stop [the] music", "pause"],
      widget: "music",
      run: async (ctx) => {
        if (!isPlaying(ctx)) return "Nothing is playing";
        await ctx.musicControl("playpause");
      },
    },
    {
      id: "media.play",
      phrases: ["play [the] music", "resume [the] music", "resume", "play"],
      widget: "music",
      run: async (ctx) => {
        if (isPlaying(ctx)) return "Already playing";
        if (!ctx.music()) return "No player is open";
        await ctx.musicControl("playpause");
      },
    },
    {
      id: "music.search",
      phrases: ["search apple music for {query:text}", "find {query:text} on apple music", "apple music {query:text}"],
      label: (a) => `Search Apple Music for "${a.query}"`,
      run: (ctx, a) => {
        ctx.openUrl(`https://music.apple.com/search?term=${encodeURIComponent(a.query)}`);
      },
    },
    {
      id: "board.show",
      phrases: ["switch to {board}", "go to {board}", "show [the] {board} board", "{board} board", "{board} mode"],
      label: (a) => `Switch to ${a.board.name}`,
      run: (ctx, a) => {
        ctx.showBoard(a.board.name);
      },
    },
    {
      id: "timer.start",
      phrases: [
        "[set] [a] timer for {length:duration}",
        "start [a] {length:duration} timer",
        "{length:duration} timer",
        "timer {length:duration}",
      ],
      widget: "clock",
      label: (a) => `Timer for ${spokenDuration(a.length)}`,
      run: (ctx, a) => {
        ctx.startTimer(a.length);
        return `Timer set for ${spokenDuration(a.length)}`;
      },
    },
    {
      id: "timer.stop",
      phrases: ["stop [the] timer", "cancel [the] timer"],
      widget: "clock",
      run: (ctx) => (ctx.stopTimer() ? "Timer stopped" : "No timer is running"),
    },
    {
      id: "link.open",
      phrases: ["open {link}"],
      bare: "{link}",
      label: (a) => `Open ${a.link.name}`,
      run: (ctx, a) => {
        ctx.openUrl(a.link.url);
      },
    },
    {
      id: "meta.help",
      phrases: ["what can i say", "what can you do", "help", "commands"],
      run: (ctx) => {
        const examples = ctx.registry.actions
          .filter((a) => a.id !== "meta.help" && a.phrases.length)
          .slice(0, HELP_EXAMPLES)
          .map((a) => a.phrases[0].replace(/\[[^\]]*\]\s*/g, "").replace(/\{(\w+)(:\w+)?\}/g, "<$1>"));
        return examples.length ? `Try: ${examples.join(" · ")}` : "Nothing to do yet.";
      },
    },
  ];
}

export function createRegistry() {
  return defineActions(buildActions());
}

/**
 * Palette rows for what is typed so far: the steps that would run, or the
 * "did you mean" options. `run: true` marks a confident match.
 */
export function suggest(registry, text, lists) {
  const m = match(text, registry, lists);
  if (m.kind === "run") {
    return { run: true, items: [{ label: m.steps.map((s) => describe(registry, s)).join(" → "), steps: m.steps }] };
  }
  if (m.kind === "choose") {
    return { run: false, items: m.options.map((o) => ({ label: describe(registry, o), steps: [o] })) };
  }
  return { run: false, items: [] };
}
