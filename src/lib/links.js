// Saved links: short names the command bar and voice open ("open notes",
// "STAG"). Stored as [{ name, url }] under `aria-links` and exported with the
// other settings. Web addresses only; the opener would refuse anything else.

export const LINKS_KEY = "aria-links";

export function parseLinks(raw) {
  try {
    const list = JSON.parse(raw);
    if (!Array.isArray(list)) return [];
    return list.filter((l) => typeof l?.name === "string" && webUrl(l.url) === l.url);
  } catch {
    return [];
  }
}

/** The URL if it is a valid http(s) address, else null. "stag.zcu.cz" gets https://. */
export function webUrl(input) {
  const text = String(input ?? "").trim();
  if (!text) return null;
  const withScheme = /^[a-z][a-z0-9+.-]*:/i.test(text) ? text : `https://${text}`;
  try {
    const url = new URL(withScheme);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (!url.hostname.includes(".") && url.hostname !== "localhost") return null;
    return url.href === `${url.origin}/` && !text.endsWith("/") ? url.origin : url.href;
  } catch {
    return null;
  }
}

/** Add or replace by name (case-insensitive); the list stays sorted by name. */
export function withLink(links, name, url) {
  const clean = name.trim();
  const href = webUrl(url);
  if (!clean) throw new Error("Give the link a name");
  if (!href) throw new Error("Not a web address");
  const key = clean.toLowerCase();
  return [...links.filter((l) => l.name.toLowerCase() !== key), { name: clean, url: href }]
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function withoutLink(links, name) {
  return links.filter((l) => l.name !== name);
}
