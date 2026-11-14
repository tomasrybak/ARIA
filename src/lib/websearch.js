// Web search from the command bar. The query goes into a URL template the
// user can change; the result opens in the default browser. Stored under
// `aria-search-url` and exported with the other settings.

import { webUrl } from "./links.js";

export const SEARCH_KEY = "aria-search-url";
export const DEFAULT_SEARCH_URL = "https://duckduckgo.com/?q={q}";

const SLOT = "{q}";

export function searchUrl(template, query) {
  return template.replace(SLOT, encodeURIComponent(String(query).trim()));
}

/** The template to save. Empty means the default; anything unusable throws. */
export function checkSearchTemplate(input) {
  const text = String(input ?? "").trim();
  if (!text) return DEFAULT_SEARCH_URL;
  if (!text.includes(SLOT)) throw new Error("Put {q} where the search words go");
  if (!webUrl(text.replace(SLOT, "x")) || !/^https?:\/\//i.test(text)) {
    throw new Error("Not a web address (start it with https://)");
  }
  return text;
}

/** The stored template, or the default if it is missing or no longer valid. */
export function parseSearchTemplate(raw) {
  try {
    return checkSearchTemplate(raw);
  } catch {
    return DEFAULT_SEARCH_URL;
  }
}
