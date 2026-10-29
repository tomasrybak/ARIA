import { test } from "node:test";
import assert from "node:assert/strict";
import { folderQuery, joinCommand, launcherBefore, launcherFromLine, splitCommand } from "../src/lib/launchers.js";

test("a command line splits on spaces, quotes group and nothing expands", () => {
  assert.deepEqual(splitCommand("kitty --directory {dir} claude"), ["kitty", "--directory", "{dir}", "claude"]);
  assert.deepEqual(splitCommand(`  "C:\\Program Files\\WezTerm\\wezterm.exe" start --cwd {dir} `), [
    "C:\\Program Files\\WezTerm\\wezterm.exe", "start", "--cwd", "{dir}",
  ]);
  assert.deepEqual(splitCommand(`echo 'say "hi"' $HOME ~ *.js ""`), ["echo", 'say "hi"', "$HOME", "~", "*.js", ""]);
  assert.deepEqual(splitCommand(`--title="ARIA dev"`), ["--title=ARIA dev"]);
  assert.throws(() => splitCommand(`kitty "unclosed`), /quote/);
});

test("joining quotes what needs it and splits back the same", () => {
  for (const argv of [["kitty", "--directory", "{dir}", "claude"], ["a b", 'say "hi"', "it's", ""]]) {
    assert.deepEqual(splitCommand(joinCommand(argv)), argv);
  }
  assert.equal(joinCommand(["kitty", "My Folder"]), 'kitty "My Folder"');
});

test("a launcher needs a name and a command", () => {
  assert.deepEqual(launcherFromLine(" claude code ", "kitty claude"), { name: "claude code", program: "kitty", args: ["claude"] });
  assert.throws(() => launcherFromLine("", "kitty"), /name/);
  assert.throws(() => launcherFromLine("x", "  "), /command/);
});

test("the folder query is what follows the last 'in'", () => {
  assert.equal(folderQuery("open claude code in aria folder"), "aria folder");
  assert.equal(folderQuery("open code in the Dev in ARIA"), "ARIA");
  assert.equal(folderQuery("open thesis"), null);
  assert.equal(folderQuery("open claude code in"), null);
  assert.equal(folderQuery("in aria"), null);
});

test("a launcher is recognised before the last 'in'", () => {
  const launchers = [{ name: "Claude Code" }, { name: "code" }];
  assert.equal(launcherBefore("open claude code in xyz", launchers)?.name, "Claude Code");
  assert.equal(launcherBefore("start code in the Dev", launchers)?.name, "code");
  assert.equal(launcherBefore("open vscode in xyz", launchers), null);
  assert.equal(launcherBefore("open claude code", launchers), null);
});
