// ARIA's actions, the content of the registry (mechanism in actions.js).
// Everything that touches the app goes through `ctx`, so each action can be
// tested with a fake one.

import { defineActions, describe } from "./actions.js";
import { match } from "./matcher.js";

const HELP_EXAMPLES = 8;

export function buildActions() {
  return [
    {
      id: "meta.help",
      phrases: ["what can i say", "what can you do", "help", "commands"],
      run: (ctx) => {
        const examples = ctx.registry.actions
          .filter((a) => a.id !== "meta.help" && a.phrases.length)
          .slice(0, HELP_EXAMPLES)
          .map((a) => a.phrases[0]);
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
