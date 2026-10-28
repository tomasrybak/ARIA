import { test } from "node:test";
import assert from "node:assert/strict";
import { parseLinks, webUrl, withLink, withoutLink } from "../src/lib/links.js";

test("webUrl accepts web addresses and adds https", () => {
  assert.equal(webUrl("stag.zcu.cz"), "https://stag.zcu.cz");
  assert.equal(webUrl("https://github.com/tomasrybak/ARIA"), "https://github.com/tomasrybak/ARIA");
  assert.equal(webUrl("http://localhost:3000/x"), "http://localhost:3000/x");
  assert.equal(webUrl("  https://example.com/  "), "https://example.com/");
});

test("webUrl refuses other schemes and non-addresses", () => {
  assert.equal(webUrl("file:///etc/passwd"), null);
  assert.equal(webUrl("javascript:alert(1)"), null);
  assert.equal(webUrl("steam://rungameid/730"), null);
  assert.equal(webUrl("notes"), null);
  assert.equal(webUrl(""), null);
});

test("withLink adds, replaces by name and keeps the list sorted", () => {
  let links = withLink([], "STAG", "stag.zcu.cz");
  links = withLink(links, "notes", "https://example.com/notes");
  links = withLink(links, "stag", "https://portal.zcu.cz");
  assert.deepEqual(links, [
    { name: "notes", url: "https://example.com/notes" },
    { name: "stag", url: "https://portal.zcu.cz" },
  ]);
  assert.throws(() => withLink(links, " ", "example.com"), /name/);
  assert.throws(() => withLink(links, "x", "ftp://example.com"), /web address/);
  assert.deepEqual(withoutLink(links, "notes"), [{ name: "stag", url: "https://portal.zcu.cz" }]);
});

test("parseLinks drops anything malformed", () => {
  const raw = JSON.stringify([{ name: "ok", url: "https://example.com" }, { name: "bad", url: "file:///x" }, 42]);
  assert.deepEqual(parseLinks(raw), [{ name: "ok", url: "https://example.com" }]);
  assert.deepEqual(parseLinks("not json"), []);
  assert.deepEqual(parseLinks(null), []);
});
