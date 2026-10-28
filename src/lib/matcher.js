// Command matcher: turns what the user typed or said into registry actions and
// their arguments, with no language model.
//
// Pure functions over plain data. Name lists (games, apps, boards, links) are
// passed in at match time, so the matcher never knows where they come from.
//
// An action declares phrases such as "launch {game}" or "[set] [a] timer for
// {minutes:duration}": plain words must match (with a little typo tolerance for
// misheard speech), `[word]` may be left out, and a `{name:type}` slot captures
// the words in between and resolves them by type.

// Starting points, to be tuned once the matcher has met real speech.
export const RUN_SCORE = 0.8;
export const SUGGEST_SCORE = 0.5;
export const MAX_SUGGESTIONS = 3;
// Running also needs a lead over the runner-up: "rock" fits two games equally
// well, and picking one of them silently is worse than asking.
export const CLEAR_LEAD = 0.05;

// A bare name ("overwatch") ranks just below the same name said with its verb.
const BARE_FACTOR = 0.95;
// A misheard word still counts as the word if it is this close.
const WORD_SIMILARITY = 0.8;

const LEADING_FILLER = /^(hey |ok |okay )?(aria |please |can you |could you |would you )+/;
const TRAILING_FILLER = / (please|for me|thanks|thank you)$/;

const SMALL = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
  "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
const TENS = { twenty: 20, thirty: 30, forty: 40, fifty: 50, sixty: 60, seventy: 70, eighty: 80, ninety: 90 };

/** Lowercase, no diacritics or punctuation, number words as digits, filler dropped. */
export function normalize(text) {
  let s = String(text)
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[’']/g, "")
    .replace(/(?<=\d)\.(?=\d)/g, "\u0000")
    .replace(/[^a-z0-9\u0000]+/g, " ")
    .replace(/\u0000/g, ".")
    .trim();
  s = numberWords(s);
  let prev;
  do {
    prev = s;
    s = s.replace(LEADING_FILLER, "").replace(TRAILING_FILLER, "").trim();
  } while (s !== prev);
  return s;
}

function numberWords(s) {
  const out = [];
  const words = s.split(" ");
  for (let i = 0; i < words.length; i++) {
    const w = words[i];
    if (w in TENS) {
      const unit = SMALL.indexOf(words[i + 1]);
      if (unit > 0 && unit < 10) {
        out.push(String(TENS[w] + unit));
        i++;
      } else out.push(String(TENS[w]));
    } else if (SMALL.includes(w)) out.push(String(SMALL.indexOf(w)));
    else out.push(w);
  }
  return out.join(" ");
}

const UNITS = [
  [/^(s|sec|secs|second|seconds)$/, 1],
  [/^(m|min|mins|minute|minutes)$/, 60],
  [/^(h|hr|hrs|hour|hours)$/, 3600],
];

function unitSeconds(word) {
  for (const [re, secs] of UNITS) if (re.test(word)) return secs;
  return null;
}

/**
 * Seconds in a normalized duration: "5 minutes", "an hour", "half an hour",
 * "1.5 hours", "1 hour 20" (a bare number after hours is minutes),
 * "1 hour and 20 minutes". Null if any word is left over.
 */
export function parseDuration(text) {
  const w = normalize(text).split(" ").filter((x) => x && x !== "and");
  let total = 0;
  let lastUnit = null;
  let i = 0;
  if (w.length === 0) return null;
  while (i < w.length) {
    if (w[i] === "half" && (w[i + 1] === "an" || w[i + 1] === "a") && unitSeconds(w[i + 2])) {
      total += unitSeconds(w[i + 2]) / 2;
      lastUnit = unitSeconds(w[i + 2]);
      i += 3;
      continue;
    }
    const n = w[i] === "a" || w[i] === "an" ? 1 : Number(w[i]);
    if (!Number.isFinite(n) || w[i] === "") return null;
    const unit = unitSeconds(w[i + 1]);
    if (unit) {
      total += n * unit;
      lastUnit = unit;
      i += 2;
    } else if (i === w.length - 1 && lastUnit && lastUnit > 1) {
      total += n * (lastUnit / 60);
      i += 1;
    } else return null;
  }
  return total > 0 ? Math.round(total) : null;
}

/** A single whole number, or null. */
export function parseNumber(text) {
  const s = normalize(text);
  return /^\d+$/.test(s) ? Number(s) : null;
}

export function levenshtein(a, b) {
  if (a === b) return 0;
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    for (let j = 1; j <= b.length; j++) {
      cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    }
    prev = cur;
  }
  return prev[b.length];
}

/** 1 for equal strings, 0 for nothing in common. */
export function similarity(a, b) {
  const len = Math.max(a.length, b.length);
  return len === 0 ? 1 : 1 - levenshtein(a, b) / len;
}

function tokenScore(q, n) {
  const ratio = similarity(q, n);
  // "over" for "overwatch": typing or saying the start of a word is common.
  const prefix = q.length >= 3 && n.startsWith(q) ? 0.9 : 0;
  return Math.max(ratio, prefix);
}

/** Letters of the words plus whole numbers: "Counter-Strike 2" → "cs2". */
function initials(tokens) {
  return tokens.map((t) => (/^\d+$/.test(t) ? t : t[0])).join("");
}

/**
 * How well a spoken/typed query names an item, 0–1. Every query word must find
 * a close name word; name words nobody mentioned cost a little ("overwatch"
 * still finds "Overwatch 2"). Joined spellings and initials count too.
 */
export function nameScore(query, name) {
  const q = normalize(query);
  const n = normalize(name);
  if (!q || !n) return 0;
  if (q === n) return 1;
  const qt = q.split(" ");
  const nt = n.split(" ");
  const used = new Set();
  let sum = 0;
  for (const t of qt) {
    let best = 0;
    let at = -1;
    nt.forEach((u, k) => {
      const s = tokenScore(t, u);
      if (s > best) [best, at] = [s, k];
    });
    sum += best;
    if (best >= WORD_SIMILARITY) used.add(at);
  }
  const words = (sum / qt.length) * (0.85 + (0.15 * used.size) / nt.length);
  const joined = similarity(q.replaceAll(" ", ""), n.replaceAll(" ", ""));
  const acronym = nt.length > 1 && q.replaceAll(" ", "") === initials(nt) ? 0.9 : 0;
  return Math.max(words, joined, acronym);
}

/** Best items of a list for a query, best first, each `{ item, score }`. */
export function rankNames(query, items, limit = MAX_SUGGESTIONS) {
  return items
    .map((item) => ({ item, score: nameScore(query, item.name) }))
    .filter((r) => r.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, limit);
}

/** "[set] [a] timer for {minutes:duration}" → parts. */
export function parseTemplate(phrase) {
  return phrase
    .trim()
    .split(/\s+/)
    .map((tok) => {
      const slot = tok.match(/^\{(\w+)(?::(\w+))?\}$/);
      if (slot) return { slot: slot[1], type: slot[2] || slot[1] };
      const optional = tok.match(/^\[(.+)\]$/);
      return { word: normalize(optional ? optional[1] : tok), optional: Boolean(optional) };
    });
}

/** Resolutions of one slot, each `{ value, score }`; none means the words don't fit. */
function resolveSlot(type, words, lists) {
  const text = words.join(" ");
  if (type === "text") return [{ value: text, score: 1 }];
  if (type === "duration") {
    const secs = parseDuration(text);
    return secs ? [{ value: secs, score: 1 }] : [];
  }
  if (type === "number") {
    const n = parseNumber(text);
    return n === null ? [] : [{ value: n, score: 1 }];
  }
  const items = lists[type];
  if (!items) return [];
  return rankNames(text, items).map((r) => ({ value: r.item, score: r.score }));
}

function wordScore(want, got) {
  if (want === got) return 1;
  if (want.length < 4) return 0;
  const s = similarity(want, got);
  return s >= WORD_SIMILARITY ? s : 0;
}

/**
 * Every way `parts` can consume all of `tokens`. Score = average of the plain
 * words × the weakest slot, so one poorly matched name sinks the whole reading.
 */
function readings(parts, tokens, lists) {
  const out = [];
  const walk = (pi, ti, words, slotMin, args) => {
    if (pi === parts.length) {
      if (ti === tokens.length) {
        const wordAvg = words.length ? words.reduce((a, b) => a + b, 0) / words.length : 1;
        out.push({ score: wordAvg * slotMin, args });
      }
      return;
    }
    const p = parts[pi];
    if (p.slot) {
      const last = pi === parts.length - 1;
      for (let end = last ? tokens.length : ti + 1; end <= tokens.length; end++) {
        for (const r of resolveSlot(p.type, tokens.slice(ti, end), lists)) {
          walk(pi + 1, end, words, Math.min(slotMin, r.score), { ...args, [p.slot]: r.value });
        }
      }
      return;
    }
    if (p.optional) walk(pi + 1, ti, words, slotMin, args);
    if (ti < tokens.length) {
      const s = wordScore(p.word, tokens[ti]);
      if (s > 0) walk(pi + 1, ti + 1, [...words, s], slotMin, args);
    }
  };
  walk(0, 0, [], 1, {});
  return out;
}

/** All candidate readings of one command, best first, each `{ action, args, score }`. */
export function candidates(text, registry, lists = {}) {
  const tokens = normalize(text).split(" ").filter(Boolean);
  if (!tokens.length) return [];
  const out = [];
  for (const { action, parts } of registry.templates) {
    for (const r of readings(parts, tokens, lists)) out.push({ action: action.id, args: r.args, score: r.score });
  }
  for (const action of registry.actions) {
    if (!action.bare || !lists[action.bare.type]) continue;
    for (const r of rankNames(tokens.join(" "), lists[action.bare.type])) {
      out.push({ action: action.id, args: { [action.bare.slot]: r.item }, score: r.score * BARE_FACTOR });
    }
  }
  out.sort((a, b) => b.score - a.score);
  // One entry per action + args: two phrases reading the same way are one option.
  const seen = new Set();
  return out.filter((c) => {
    const key = c.action + JSON.stringify(c.args);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

const JOINERS = new Set(["and", "then"]);

/** The best reading, if it is confident and clearly ahead of the next one. */
function clearWinner(list) {
  const [best, next] = list;
  if (!best || best.score < RUN_SCORE) return null;
  return !next || best.score - next.score >= CLEAR_LEAD ? best : null;
}

/**
 * What to do with a command:
 *   { kind: "run", steps: [{ action, args, score }, ...] }
 *   { kind: "choose", options: [...] }   — "did you mean", best first
 *   { kind: "none" }
 *
 * It runs only a confident reading that is clearly ahead of the next one.
 * "pause music and open notes" runs as two steps, but only when every part is
 * a confident command on its own, so a name like "Rock and Stone" stays whole.
 */
export function match(text, registry, lists = {}) {
  const whole = candidates(text, registry, lists);
  const winner = clearWinner(whole);
  if (winner) return { kind: "run", steps: [winner] };

  const tokens = normalize(text).split(" ");
  if (tokens.some((t) => JOINERS.has(t))) {
    const parts = [[]];
    for (const t of tokens) JOINERS.has(t) ? parts.push([]) : parts.at(-1).push(t);
    if (parts.every((p) => p.length)) {
      const steps = parts.map((p) => clearWinner(candidates(p.join(" "), registry, lists)));
      if (steps.every(Boolean)) return { kind: "run", steps };
    }
  }

  const options = whole.filter((c) => c.score >= SUGGEST_SCORE).slice(0, MAX_SUGGESTIONS);
  return options.length ? { kind: "choose", options } : { kind: "none" };
}
