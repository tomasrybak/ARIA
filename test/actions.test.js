import { test } from "node:test";
import assert from "node:assert/strict";
import { defineActions, describe, runAction, runSteps } from "../src/lib/actions.js";

test("ids must be unique and bare must be one slot", () => {
  const run = () => {};
  assert.throws(() => defineActions([{ id: "a", run }, { id: "a", run }]), /duplicate/);
  assert.throws(() => defineActions([{ id: "a", bare: "launch {game}", run }]), /single slot/);
  assert.throws(() => defineActions([{ id: "a" }]), /run/);
});

test("run passes ctx and args, and normalizes the answer", async () => {
  const reg = defineActions([
    { id: "say", widget: "clock", run: (ctx, args) => `${ctx.greeting} ${args.who}` },
    { id: "quiet", run: () => {} },
    { id: "elsewhere", widget: "clock", run: () => ({ text: "ok", widget: "weather" }) },
  ]);
  assert.deepEqual(await runAction(reg, { action: "say", args: { who: "you" } }, { greeting: "hi" }),
    { text: "hi you", widget: "clock", feedback: true, error: false });
  assert.deepEqual(await runAction(reg, { action: "quiet" }, {}), { text: "", widget: null, feedback: true, error: false });
  assert.equal((await runAction(reg, { action: "elsewhere" }, {})).widget, "weather");
});

test("failures come back as answers, and a sequence stops at the first one", async () => {
  const done = [];
  const reg = defineActions([
    { id: "ok", run: async () => { done.push("ok"); return "fine"; } },
    { id: "boom", run: async () => { throw new Error("no Steam"); } },
  ]);
  const answers = await runSteps(reg, [{ action: "ok" }, { action: "boom" }, { action: "ok" }], {});
  assert.deepEqual(answers.map((a) => a.error), [false, true]);
  assert.equal(answers[1].text, "no Steam");
  assert.deepEqual(done, ["ok"]);
  assert.equal((await runAction(reg, { action: "missing" }, {})).error, true);
});

test("describe uses the label when an action has one", () => {
  const reg = defineActions([
    { id: "game", phrases: ["launch {game}"], label: (a) => `Launch ${a.game.name}`, run: () => {} },
    { id: "next", phrases: ["next song"], run: () => {} },
  ]);
  assert.equal(describe(reg, { action: "game", args: { game: { name: "Overwatch 2" } } }), "Launch Overwatch 2");
  assert.equal(describe(reg, { action: "next" }), "next song");
});

test("feedback: false marks actions with nothing to confirm", async () => {
  const reg = defineActions([{ id: "cancel", feedback: false, run: () => {} }]);
  assert.equal((await runAction(reg, { action: "cancel" }, {})).feedback, false);
});
