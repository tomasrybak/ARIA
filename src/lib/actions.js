// Action registry: the one list of things ARIA can do. The command bar, voice
// and any later language model all pick from it, and nothing outside it runs.
//
// An action is plain data plus a `run(ctx, args)`:
//   id       unique, stable (routines refer to it)
//   phrases  matcher templates, see matcher.js
//   bare     optional "{slot:type}" a name alone may stand for ("overwatch")
//   widget   optional widget id(s) the action is about (unused by the UI for
//            now; meant for widgets that light up on what matters)
//   label    optional (args) → text for "did you mean" options
//   feedback false for actions that do nothing visible to confirm ("cancel")
//   run      does the work through `ctx` (injected, so actions are testable
//            without Tauri); returns an answer string, `{ text, widget }` or nothing

import { parseTemplate } from "./matcher.js";

export function defineActions(list) {
  const byId = new Map();
  const templates = [];
  const actions = list.map((a) => {
    if (!a.id || typeof a.run !== "function") throw new Error(`action needs an id and run(): ${a.id}`);
    if (byId.has(a.id)) throw new Error(`duplicate action id: ${a.id}`);
    const bare = a.bare ? parseTemplate(a.bare)[0] : null;
    if (a.bare && !bare?.slot) throw new Error(`bare must be a single slot: ${a.id}`);
    const action = { ...a, phrases: a.phrases || [], bare };
    byId.set(a.id, action);
    for (const phrase of action.phrases) templates.push({ action, parts: parseTemplate(phrase) });
    return action;
  });
  return { actions, byId, templates };
}

/** Text for one matched step in a "did you mean" list. */
export function describe(registry, step) {
  const action = registry.byId.get(step.action);
  if (action?.label) return action.label(step.args);
  return action?.phrases[0] ?? step.action;
}

/** Run one step; failures come back as an answer, never as a throw. */
export async function runAction(registry, step, ctx) {
  const action = registry.byId.get(step.action);
  if (!action) return { text: `Unknown action ${step.action}`, feedback: true, error: true };
  try {
    const out = await action.run(ctx, step.args || {});
    const answer = typeof out === "string" ? { text: out } : out || {};
    return {
      text: answer.text ?? "",
      widget: answer.widget ?? action.widget ?? null,
      feedback: action.feedback !== false,
      error: false,
    };
  } catch (e) {
    return { text: String(e?.message ?? e), widget: action.widget ?? null, feedback: true, error: true };
  }
}

/** Run steps in order and stop at the first failure (compound commands, routines). */
export async function runSteps(registry, steps, ctx) {
  const answers = [];
  for (const step of steps) {
    const answer = await runAction(registry, step, ctx);
    answers.push(answer);
    if (answer.error) break;
  }
  return answers;
}
