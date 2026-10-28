// ARIA's actions, the content of the registry (mechanism in actions.js).
// Everything that touches the app goes through `ctx`, so each action can be
// tested with a fake one:
//   ctx.registry          the registry itself (help lists it)
//   ctx.openUrl(url)      system opener
//   ctx.music()           latest "music" collector state, or null
//   ctx.musicControl(op)  "playpause" | "next" | "previous"
//   ctx.showBoard(name)   apply a board preset, as a click in the menu would
//   ctx.startTimer(secs)  ctx.stopTimer() → false when no timer was running
//   ctx.facts()           snapshot of collector data for questions: { now, hw,
//                         gpu, music, classes, email, freegames, weather,
//                         crypto, github, steam, screentime }
//
// `widget` may be a list: the first of them that is on the board lights up.
//
// Name lists for slots (see commandLists in main.js):
//   game  [{ name, id }]   installed Steam games
//   board [{ name }]       board presets
//   link  [{ name, url }]  saved links
//   coin  [{ name, symbol }] coins on the crypto widget, by symbol and full name

import { defineActions, describe } from "./actions.js";
import { match } from "./matcher.js";
import * as answers from "./answers.js";

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
    ...questions(),
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

/** Read-only answers over data the widgets already have. */
function questions() {
  const ask = (id, widget, phrases, answer) => ({ id, widget, phrases, run: (ctx, a) => answer(ctx.facts(), a) });
  return [
    ask("ask.gpu", ["gpu", "thermals", "hardware"],
      ["how hot is my gpu", "gpu temp", "gpu temperature", "gpu usage", "gpu load", "how is my gpu"],
      (f) => answers.gpuAnswer(f.gpu)),
    ask("ask.cpu", ["thermals", "hardware"],
      ["how hot is my cpu", "cpu temp", "cpu temperature", "cpu usage", "cpu load", "how is my cpu"],
      (f) => answers.cpuAnswer(f.hw)),
    ask("ask.system", "hardware",
      ["system status", "how is my pc", "how is my computer", "system load"],
      (f) => answers.systemAnswer(f.hw, f.gpu)),
    ask("ask.song", "music",
      ["whats this song", "what song is this", "whats playing", "what is playing", "current song"],
      (f) => answers.songAnswer(f.music)),
    ask("ask.class", "stag",
      ["next class", "next lecture", "when is my next class", "when is my next lecture", "whats next", "what class do i have"],
      (f) => answers.nextClassAnswer(f.classes, f.now)),
    ask("ask.mail", "email",
      ["any new mail", "any new email", "unread mail", "do i have mail", "check [my] mail", "how many emails"],
      (f) => answers.mailAnswer(f.email)),
    ask("ask.freegames", "freegames",
      ["free games", "any free games", "whats free", "what games are free"],
      (f) => answers.freeGamesAnswer(f.freegames, f.now)),
    ask("ask.weather", "weather",
      ["weather", "whats the weather", "hows the weather", "how warm is it", "how cold is it", "is it cold", "temperature outside"],
      (f) => answers.weatherAnswer(f.weather)),
    {
      ...ask("ask.crypto", "crypto",
        ["{coin} price", "price of {coin}", "how much is {coin}", "whats {coin} at", "{coin}"],
        (f, a) => answers.cryptoAnswer(f.crypto, a.coin.symbol)),
      label: (a) => `${a.coin.name} price`,
    },
    ask("ask.github", "github",
      ["any new prs", "pull requests", "github", "github notifications", "anything on github"],
      (f) => answers.githubAnswer(f.github)),
    ask("ask.screentime", "screentime",
      ["screen time", "how long have i played today", "how long was i on the computer", "how long have i been on the pc"],
      (f) => answers.screenTimeAnswer(f.screentime)),
    ask("ask.friends", "steam",
      ["whos online", "who is online", "which friends are online", "whos playing", "friends online"],
      (f) => answers.friendsAnswer(f.steam)),
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
