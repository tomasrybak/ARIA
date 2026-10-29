// Launchers: a command line typed in the settings, stored by Rust as argv
// ({ name, program, args }) and run without a shell, so the line is only
// split here: quotes group words, nothing is expanded. Backslashes are plain
// characters (Windows paths).

/** "kitty --directory {dir} claude" → ["kitty", "--directory", "{dir}", "claude"]. */
export function splitCommand(line) {
  const out = [];
  let word = null;
  let quote = null;
  for (const ch of line) {
    if (quote) {
      if (ch === quote) quote = null;
      else word += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
      word ??= "";
    } else if (/\s/.test(ch)) {
      if (word !== null) out.push(word);
      word = null;
    } else {
      word = (word ?? "") + ch;
    }
  }
  if (quote) throw new Error("A quote isn't closed");
  if (word !== null) out.push(word);
  return out;
}

/** Back to one line for the settings list; splitCommand(joinCommand(a)) equals a. */
export function joinCommand(argv) {
  return argv
    .map((a) => {
      if (a && !/[\s"']/.test(a)) return a;
      return a.includes('"') ? `'${a}'` : `"${a}"`;
    })
    .join(" ");
}

export function launcherFromLine(name, line) {
  const argv = splitCommand(line);
  if (!name.trim()) throw new Error("Give the launcher a name");
  if (!argv.length) throw new Error("Type the command to run");
  return { name: name.trim(), program: argv[0], args: argv.slice(1) };
}

/**
 * The words after the last "in", searched on their own: in "open claude code
 * in aria" the launcher's words would otherwise crowd the folder out of the hits.
 */
export function folderQuery(text) {
  const words = text.trim().split(/\s+/);
  const i = words.map((w) => w.toLowerCase()).lastIndexOf("in");
  return i > 0 && i < words.length - 1 ? words.slice(i + 1).join(" ") : null;
}

/** The launcher named before the last "in" ("open claude code in xyz"), when nothing ran. */
export function launcherBefore(text, launchers) {
  const words = text.trim().toLowerCase().split(/\s+/);
  const i = words.lastIndexOf("in");
  if (i < 1) return null;
  const head = ` ${words.slice(0, i).join(" ")}`;
  return launchers.find((l) => head.endsWith(` ${l.name.toLowerCase()}`)) ?? null;
}
