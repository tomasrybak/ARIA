// STEAM: the wording the widget needs and no DOM. The collector emits
// `{ local, status, ... }`: `local` is the game running on this machine
// (from the appmanifests, no key needed), the rest is the Web API side.

const PRIVACY = "Steam → Edit Profile → Privacy Settings";

/** What each entry of `hidden` keeps from the widget, and the setting that fixes it. */
export const PRIVACY_HINTS = {
  profile: `Your profile is private. ${PRIVACY} → My profile → Public`,
  friend_list: `Your friends list is private. ${PRIVACY} → Friends List → Public`,
  game_details: `Your game details are private. ${PRIVACY} → Game details → Public`,
};

/** The most important privacy hint, or null. A private profile hides everything, so it comes first. */
export function privacyHint(hidden) {
  const first = (hidden ?? []).find((h) => PRIVACY_HINTS[h]);
  return first ? PRIVACY_HINTS[first] : null;
}

/** The game you are in: the one running here first, then the one Steam reports (another PC, a Deck). */
export function currentGame(state) {
  if (state?.local) return state.local.name;
  return state?.status === "connected" ? (state.me?.game ?? null) : null;
}

/** "45 min", "3.2 h" under ten hours, "214 h" above. */
export function playtime(minutes) {
  const m = Math.max(0, Math.round(minutes ?? 0));
  if (m < 60) return `${m} min`;
  const h = m / 60;
  return h < 10 ? `${h.toFixed(1)} h` : `${Math.round(h)} h`;
}

/** The friend most worth a headline: the first one in a game (the collector sorts them first). */
export function topPlaying(friends) {
  return (friends ?? []).find((f) => f.game) ?? null;
}
