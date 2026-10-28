import { test } from "node:test";
import assert from "node:assert/strict";
import { defineActions } from "../src/lib/actions.js";
import {
  RUN_SCORE,
  match,
  nameScore,
  normalize,
  parseDuration,
  rankNames,
  parseTemplate,
  tokenize,
} from "../src/lib/matcher.js";

const noop = () => {};
const registry = defineActions([
  { id: "game.launch", phrases: ["launch {game}", "play {game}", "start {game}"], bare: "{game}", run: noop },
  { id: "media.next", phrases: ["next song", "skip [this] [song]"], run: noop },
  { id: "media.pause", phrases: ["pause [the] music"], run: noop },
  { id: "timer.start", phrases: ["[set] [a] timer for {length:duration}", "{length:duration} timer"], run: noop },
  { id: "note.add", phrases: ["note {body:text}", "take a note {body:text}"], run: noop },
  { id: "board.show", phrases: ["switch to {board}", "{board} mode"], run: noop },
  { id: "link.open", phrases: ["open {link}"], run: noop },
]);

const lists = {
  game: [
    { name: "Overwatch 2", value: 2357570 },
    { name: "Counter-Strike 2", value: 730 },
    { name: "Deep Rock Galactic", value: 548430 },
    { name: "Rock and Stone", value: 1 },
  ],
  board: [{ name: "Gaming" }, { name: "Study" }, { name: "Coding" }],
  link: [{ name: "notes" }, { name: "STAG" }],
};

const run = (text) => match(text, registry, lists);

test("normalize drops filler, punctuation and diacritics, and reads number words", () => {
  assert.equal(normalize("Hey ARIA, could you skip this song, please?"), "skip this song");
  assert.equal(normalize("Přepni na Studium"), "prepni na studium");
  assert.equal(normalize("twenty five minutes"), "25 minutes");
  assert.equal(normalize("1.5 hours"), "1.5 hours");
});

test("durations", () => {
  assert.equal(parseDuration("5 minutes"), 300);
  assert.equal(parseDuration("five min"), 300);
  assert.equal(parseDuration("an hour"), 3600);
  assert.equal(parseDuration("half an hour"), 1800);
  assert.equal(parseDuration("1.5 hours"), 5400);
  assert.equal(parseDuration("1 hour 20"), 4800);
  assert.equal(parseDuration("1 hour and 20 minutes"), 4800);
  assert.equal(parseDuration("90 seconds"), 90);
  assert.equal(parseDuration("5"), null);
  assert.equal(parseDuration("5 bananas"), null);
});

test("templates parse words, optional words and typed slots", () => {
  assert.deepEqual(parseTemplate("[set] timer for {length:duration}"), [
    { word: "set", optional: true },
    { word: "timer", optional: false },
    { word: "for", optional: false },
    { slot: "length", type: "duration" },
  ]);
  assert.deepEqual(parseTemplate("launch {game}")[1], { slot: "game", type: "game" });
});

test("names: exact, partial, joined spelling and initials", () => {
  assert.equal(nameScore("overwatch 2", "Overwatch 2"), 1);
  assert.ok(nameScore("overwatch", "Overwatch 2") >= RUN_SCORE);
  assert.ok(nameScore("counter strike", "Counter-Strike 2") >= RUN_SCORE);
  assert.ok(nameScore("counterstrike 2", "Counter-Strike 2") >= RUN_SCORE);
  assert.ok(nameScore("cs 2", "Counter-Strike 2") >= RUN_SCORE);
  assert.ok(nameScore("minecraft", "Overwatch 2") < 0.5);
});

test("item weight scales a name score and a bonus adds to it", () => {
  const [thesis] = rankNames("thesis", [{ name: "Thesis", weight: 0.9 }]);
  assert.ok(Math.abs(thesis.score - 0.9) < 1e-9);
  const [boosted] = rankNames("thesis", [{ name: "Thesis", weight: 0.9, bonus: 0.1 }]);
  assert.ok(Math.abs(boosted.score - 0.99) < 1e-9);
  assert.equal(rankNames("thesis", [{ name: "Thesis", bonus: 0.5 }])[0].score, 1);
  // A bonus strengthens a match but doesn't lift an unrelated name.
  const plain = nameScore("thesis", "Budget");
  const [budget] = rankNames("thesis", [{ name: "Budget", bonus: 0.1 }]);
  assert.ok((budget?.score ?? 0) <= plain * 1.1 + 1e-9);
});

test("a phrase with a name slot runs the action with the list item", () => {
  const out = run("launch overwatch");
  assert.equal(out.kind, "run");
  assert.equal(out.steps[0].action, "game.launch");
  assert.equal(out.steps[0].args.game.value, 2357570);
});

test("a bare name stands for its action", () => {
  const out = run("Overwatch");
  assert.equal(out.kind, "run");
  assert.equal(out.steps[0].action, "game.launch");
});

test("optional words and misheard words still match", () => {
  assert.equal(run("skip").steps[0].action, "media.next");
  assert.equal(run("skip this song").steps[0].action, "media.next");
  assert.equal(run("set a timmer for 10 minutes").steps[0].args.length, 600);
  assert.equal(run("ten minute timer").steps[0].args.length, 600);
});

test("text slots keep the user's own words, case and punctuation", () => {
  const out = run("Note: Email Pavel about the exam, tomorrow!");
  assert.equal(out.steps[0].action, "note.add");
  assert.equal(out.steps[0].args.body, "Email Pavel about the exam, tomorrow");
  // Filler around the command is not part of the text.
  assert.equal(run("Hey Aria, note buy milk please").steps[0].args.body, "buy milk");
  // Number words are read as numbers for matching, but the text stays as said.
  assert.equal(run("note call twenty five people").steps[0].args.body, "call twenty five people");
});

test("tokens keep their place in the original text", () => {
  const words = tokenize("Hey Aria, set timer for twenty five mins");
  assert.deepEqual(words.map((w) => w.t), ["set", "timer", "for", "25", "mins"]);
  assert.equal("Hey Aria, set timer for twenty five mins".slice(words[3].from, words[3].to), "twenty five");
});

test("compound commands split on and/then only when every part is a command", () => {
  const out = run("pause the music and switch to study");
  assert.equal(out.kind, "run");
  assert.deepEqual(out.steps.map((s) => s.action), ["media.pause", "board.show"]);
  assert.equal(out.steps[1].args.board.name, "Study");

  const name = run("play rock and stone");
  assert.equal(name.kind, "run");
  assert.equal(name.steps.length, 1);
  assert.equal(name.steps[0].args.game.name, "Rock and Stone");
});

test("unsure input offers choices, nonsense offers nothing", () => {
  const out = run("play rock");
  assert.equal(out.kind, "choose");
  assert.ok(out.options.length >= 2 && out.options.length <= 3);
  assert.ok(out.options.every((o) => o.action === "game.launch"));

  assert.deepEqual(run("what is the meaning of life"), { kind: "none" });
  assert.deepEqual(run("   "), { kind: "none" });
});
