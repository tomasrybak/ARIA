import { test } from "node:test";
import assert from "node:assert/strict";
import { checkSearchTemplate, DEFAULT_SEARCH_URL, parseSearchTemplate, SEARCH_KEY, searchUrl } from "../src/lib/websearch.js";
import { STORAGE_KEYS } from "../src/lib/settings.js";

test("searchUrl encodes the whole query into the slot", () => {
  assert.equal(searchUrl(DEFAULT_SEARCH_URL, "  rust & tauri?#1 "), "https://duckduckgo.com/?q=rust%20%26%20tauri%3F%231");
  assert.equal(searchUrl("https://www.google.com/search?q={q}&hl=en", "příliš"),
    "https://www.google.com/search?q=p%C5%99%C3%ADli%C5%A1&hl=en");
});

test("checkSearchTemplate keeps usable templates, empty means the default", () => {
  assert.equal(checkSearchTemplate(" https://kagi.com/search?q={q} "), "https://kagi.com/search?q={q}");
  assert.equal(checkSearchTemplate(""), DEFAULT_SEARCH_URL);
  assert.equal(checkSearchTemplate(null), DEFAULT_SEARCH_URL);
});

test("checkSearchTemplate refuses templates that can't work", () => {
  assert.throws(() => checkSearchTemplate("https://duckduckgo.com/"), /\{q\}/);
  assert.throws(() => checkSearchTemplate("javascript:alert('{q}')"), /web address/);
  assert.throws(() => checkSearchTemplate("file:///tmp/{q}"), /web address/);
  assert.throws(() => checkSearchTemplate("duckduckgo.com/?q={q}"), /https:\/\//);
});

test("parseSearchTemplate falls back to the default", () => {
  assert.equal(parseSearchTemplate(null), DEFAULT_SEARCH_URL);
  assert.equal(parseSearchTemplate("ftp://x.org/{q}"), DEFAULT_SEARCH_URL);
  assert.equal(parseSearchTemplate("https://www.startpage.com/do/search?q={q}"), "https://www.startpage.com/do/search?q={q}");
});

test("the search address travels with the settings export", () => {
  assert.ok(STORAGE_KEYS.includes(SEARCH_KEY));
});
