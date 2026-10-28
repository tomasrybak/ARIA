import { test } from "node:test";
import assert from "node:assert/strict";
import { defineActions, runAction } from "../src/lib/actions.js";
import { buildActions, createRegistry, suggest } from "../src/lib/commands.js";

test("help lists the first phrase of other actions", async () => {
  const registry = defineActions([
    ...buildActions(),
    { id: "media.next", phrases: ["next song"], run: () => {} },
    { id: "hidden", run: () => {} },
  ]);
  const out = await runAction(registry, { action: "meta.help" }, { registry });
  assert.equal(out.text, "Try: next song");
});

test("suggest: a confident match is one runnable row", () => {
  const out = suggest(createRegistry(), "what can I say", {});
  assert.equal(out.run, true);
  assert.equal(out.items.length, 1);
  assert.deepEqual(out.items[0].steps.map((s) => s.action), ["meta.help"]);
});

test("suggest: nothing matched is no rows", () => {
  assert.deepEqual(suggest(createRegistry(), "make me a sandwich", {}), { run: false, items: [] });
});
