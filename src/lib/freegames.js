// FREE GAMES: which giveaways are live now, which are coming, and how long
// each has left. The collector polls every 6 h, so expiry is decided here
// against the clock: an ended game drops out and an upcoming one goes live
// on time, between polls.
//
// A game is `{ title, store, url, starts_at, ends_at, worth, thumb }`, times
// in epoch ms or null.

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

export const STORES = { epic: "Epic", steam: "Steam", gog: "GOG" };

const byEnd = (a, b) => (a.ends_at ?? Infinity) - (b.ends_at ?? Infinity);

/** `{ live, upcoming }`: live ending soonest first, upcoming starting soonest first. */
export function partition(games, now) {
  const live = [];
  const upcoming = [];
  for (const g of games ?? []) {
    if (g.ends_at != null && g.ends_at <= now) continue;
    if (g.starts_at != null && g.starts_at > now) upcoming.push(g);
    else live.push(g);
  }
  live.sort(byEnd);
  upcoming.sort((a, b) => a.starts_at - b.starts_at);
  return { live, upcoming };
}

function span(ms) {
  if (ms < HOUR) return `${Math.max(1, Math.round(ms / 60_000))} min`;
  if (ms < DAY) return `${Math.round(ms / HOUR)} h`;
  return `${Math.round(ms / DAY)} d`;
}

/** "ends in 3 d", "ends in 5 h"; "" when the source names no end. */
export function endsLabel(endsAt, now) {
  if (endsAt == null) return "";
  return `ends in ${span(endsAt - now)}`;
}

/** "from Thu", or "in 5 h" when it is less than a day away. */
export function startsLabel(startsAt, now, locale) {
  if (startsAt - now < DAY) return `in ${span(startsAt - now)}`;
  return `from ${new Date(startsAt).toLocaleDateString(locale, { weekday: "short" })}`;
}

/** Under a day left: worth flagging. */
export const endingSoon = (g, now) => g.ends_at != null && g.ends_at - now < DAY;

/** How long S shows each game before the next. */
export const CYCLE_MS = 8000;

/**
 * Which game S shows at `now`, taken from the clock rather than kept as a
 * counter: any re-render (a poll, the minute timer) lands on the same game,
 * and every game gets the same time on screen.
 */
export function cycleIndex(now, count, period = CYCLE_MS) {
  return count > 0 ? Math.floor(now / period) % count : 0;
}
