import { test } from "node:test";
import assert from "node:assert/strict";
import { defineActions, runAction, runSteps } from "../src/lib/actions.js";
import { buildActions, createRegistry, spokenDuration, suggest } from "../src/lib/commands.js";
import { match } from "../src/lib/matcher.js";

const registry = createRegistry();
const lists = {
  game: [
    { name: "Overwatch 2", id: 2357570 },
    { name: "Counter-Strike 2", id: 730 },
    { name: "Deep Rock Galactic", id: 548430 },
  ],
  board: [{ name: "Gaming" }, { name: "Study" }, { name: "Coding" }],
  link: [{ name: "notes", url: "https://example.com/notes" }, { name: "STAG", url: "https://stag.zcu.cz" }],
};

/** A ctx that records what the actions asked for. */
function fakeCtx({ music = null, timer = false, last = null } = {}) {
  const calls = [];
  return {
    calls,
    registry,
    openUrl: (url) => calls.push(["open", url]),
    music: () => music,
    musicControl: async (op) => calls.push(["music", op]),
    showBoard: (name) => calls.push(["board", name]),
    startTimer: (secs) => calls.push(["timer", secs]),
    stopTimer: () => (calls.push(["stop"]), timer),
    addNote: (text) => (calls.push(["note", text]), text.trim() ? { text: text.trim().slice(0, 140) } : null),
    lastAnswer: () => last,
  };
}

async function say(text, ctx = fakeCtx()) {
  const m = match(text, registry, lists);
  assert.equal(m.kind, "run", `"${text}" should run, got ${JSON.stringify(m)}`);
  const answers = await runSteps(registry, m.steps, ctx);
  return { ctx, answers, text: answers.at(-1).text };
}

test("spoken durations", () => {
  assert.equal(spokenDuration(600), "10 minutes");
  assert.equal(spokenDuration(4800), "1 hour 20 minutes");
  assert.equal(spokenDuration(90), "1 minute 30 seconds");
  assert.equal(spokenDuration(3601), "1 hour");
});

test("launch a game by phrase or by bare name", async () => {
  for (const text of ["launch overwatch", "play counter strike", "Deep Rock Galactic"]) {
    const { ctx } = await say(text);
    assert.match(ctx.calls[0][1], /^steam:\/\/rungameid\/\d+$/);
  }
  const { ctx, text } = await say("overwatch");
  assert.deepEqual(ctx.calls, [["open", "steam://rungameid/2357570"]]);
  assert.equal(text, "Launching Overwatch 2");
});

test("media controls respect what is playing", async () => {
  assert.deepEqual((await say("next song")).ctx.calls, [["music", "next"]]);
  assert.deepEqual((await say("skip")).ctx.calls, [["music", "next"]]);
  assert.deepEqual((await say("previous track")).ctx.calls, [["music", "previous"]]);

  const playing = { status: "playing" };
  assert.deepEqual((await say("pause the music", fakeCtx({ music: playing }))).ctx.calls, [["music", "playpause"]]);
  assert.equal((await say("pause", fakeCtx({ music: { status: "paused" } }))).text, "Nothing is playing");
  assert.equal((await say("resume", fakeCtx({ music: playing }))).text, "Already playing");
  assert.equal((await say("play")).text, "No player is open");
  assert.deepEqual((await say("play music", fakeCtx({ music: { status: "paused" } }))).ctx.calls, [["music", "playpause"]]);
});

test("boards, timers and links", async () => {
  assert.deepEqual((await say("switch to study")).ctx.calls, [["board", "Study"]]);
  assert.deepEqual((await say("gaming mode")).ctx.calls, [["board", "Gaming"]]);

  const timer = await say("set a timer for 25 minutes");
  assert.deepEqual(timer.ctx.calls, [["timer", 1500]]);
  assert.equal(timer.text, "Timer set for 25 minutes");
  assert.equal((await say("stop the timer")).text, "No timer is running");
  assert.equal((await say("cancel timer", fakeCtx({ timer: true }))).text, "Timer stopped");

  assert.deepEqual((await say("open notes")).ctx.calls, [["open", "https://example.com/notes"]]);
  assert.deepEqual((await say("stag")).ctx.calls, [["open", "https://stag.zcu.cz"]]);
});

test("apple music search keeps the query as typed and encodes it", async () => {
  const { ctx } = await say("search apple music for Daft Punk & friends");
  assert.deepEqual(ctx.calls, [["open", "https://music.apple.com/search?term=Daft%20Punk%20%26%20friends"]]);
});

test("compound commands run every part", async () => {
  const { ctx } = await say("pause the music and switch to study", fakeCtx({ music: { status: "playing" } }));
  assert.deepEqual(ctx.calls, [["music", "playpause"], ["board", "Study"]]);
});

test("help lists example phrases without template syntax", async () => {
  const small = defineActions([
    ...buildActions().filter((a) => a.id === "meta.help"),
    { id: "media.next", phrases: ["next song"], run: () => {} },
    { id: "timer", phrases: ["[set] [a] timer for {length:duration}"], run: () => {} },
    { id: "hidden", run: () => {} },
  ]);
  const out = await runAction(small, { action: "meta.help" }, { registry: small });
  assert.equal(out.text, "Try: next song · timer for <length>");
});

test("suggest: one runnable row, choices, or nothing", () => {
  const run = suggest(registry, "what can I say", lists);
  assert.equal(run.run, true);
  assert.deepEqual(run.items[0].steps.map((s) => s.action), ["meta.help"]);

  const pick = suggest(registry, "launch 2", lists);
  assert.equal(pick.run, false);
  assert.ok(pick.items.length >= 2);
  assert.ok(pick.items.every((i) => i.label.startsWith("Launch ")));

  assert.deepEqual(suggest(registry, "make me a sandwich", lists), { run: false, items: [] });
});

test("dictated notes keep the user's words, typed reminders pass through", async () => {
  const { ctx, text } = await say("Note: Email Pavel about the exam, tomorrow!");
  assert.deepEqual(ctx.calls, [["note", "Email Pavel about the exam, tomorrow"]]);
  assert.equal(text, "Noted");
  assert.deepEqual((await say("remember to buy milk")).ctx.calls, [["note", "buy milk"]]);
  assert.deepEqual((await say("note call Pavel @17:00")).ctx.calls, [["note", "call Pavel @17:00"]]);
  assert.equal((await say(`note ${"x".repeat(150)}`)).text, "Noted, shortened to 140 characters");
});

test("repeat and cancel", async () => {
  assert.equal((await say("repeat that", fakeCtx({ last: "GPU at 13%" }))).text, "GPU at 13%");
  assert.equal((await say("say that again")).text, "Nothing to repeat yet");
  const cancel = await say("never mind");
  assert.equal(cancel.answers[0].feedback, false);
  assert.equal((await say("stop")).ctx.calls.length, 0);
  // "stop" alone cancels; with an object it is that object's action.
  assert.equal(match("stop the music", registry, lists).steps[0].action, "media.pause");
  assert.equal(match("stop the timer", registry, lists).steps[0].action, "timer.stop");
});
