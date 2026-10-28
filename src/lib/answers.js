// Spoken-length answers over the collectors' latest data: "how hot is my GPU",
// "what's this song". Pure, one function per question; each takes the
// collector state as the widget gets it and returns one short sentence.

import { partition, STORES } from "./freegames.js";
import { describe as describeWeather, fmtTemp } from "./weather.js";

const MAX_LISTED = 3;
const DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/** Full names people say for the symbols the crypto widget tracks. */
export const COIN_NAMES = {
  BTC: "bitcoin",
  ETH: "ethereum",
  SOL: "solana",
  ADA: "cardano",
  XRP: "ripple",
  DOGE: "dogecoin",
  LTC: "litecoin",
  DOT: "polkadot",
};

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
const pct = (v) => `${Math.round(v)}%`;
const deg = (v) => `${Math.round(v)} °C`;

function listed(items) {
  const shown = items.slice(0, MAX_LISTED).join(", ");
  return items.length > MAX_LISTED ? `${shown} and ${items.length - MAX_LISTED} more` : shown;
}

/** "3 h 32 min", "26 min". */
export function hoursMinutes(secs) {
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return h ? `${h} h ${m} min` : `${m} min`;
}

export function gpuAnswer(gpu) {
  if (!gpu) return "No GPU readings yet";
  const temp = Number.isFinite(gpu.temp_c) ? `, ${deg(gpu.temp_c)}` : "";
  return `GPU at ${pct(gpu.gpu)}${temp}`;
}

export function cpuAnswer(hw) {
  if (!hw) return "No CPU readings yet";
  const temp = Number.isFinite(hw.cpu_temp) ? `, ${deg(hw.cpu_temp)}` : "";
  return `CPU at ${pct(hw.cpu)}${temp}`;
}

export function systemAnswer(hw, gpu) {
  if (!hw) return "No readings yet";
  const parts = [`CPU ${pct(hw.cpu)}`, `RAM ${pct(hw.ram)}`];
  if (gpu) parts.push(`GPU ${pct(gpu.gpu)}`);
  return parts.join(", ");
}

export function songAnswer(music) {
  if (music?.status !== "playing" && music?.status !== "paused") return "Nothing is playing";
  const who = music.artist ? ` by ${music.artist}` : "";
  return `${music.title}${who}${music.status === "paused" ? " (paused)" : ""}`;
}

/**
 * The class going on now or the next one this week. `classes` are this week's
 * timetable entries (`day` 0 = Monday, `start`/`end` "HH:MM").
 */
export function nextClassAnswer(classes, now) {
  const date = new Date(now);
  const today = (date.getDay() + 6) % 7;
  const minutes = date.getHours() * 60 + date.getMinutes();
  const toMin = (hhmm) => {
    const [h, m] = hhmm.split(":").map(Number);
    return h * 60 + m;
  };
  const sorted = [...(classes ?? [])].sort((a, b) => a.day - b.day || toMin(a.start) - toMin(b.start));
  const where = (c) => (c.room ? ` in ${c.room}` : "");
  const current = sorted.find((c) => c.day === today && toMin(c.start) <= minutes && minutes < toMin(c.end));
  if (current) return `${current.subject} until ${current.end}${where(current)}`;
  const next = sorted.find((c) => c.day > today || (c.day === today && toMin(c.start) > minutes));
  if (!next) return "No more classes this week";
  if (next.day === today) return `${next.subject} at ${next.start}${where(next)}`;
  return `No more classes today. Next: ${next.subject} on ${DAYS[next.day]} at ${next.start}`;
}

export function mailAnswer(email) {
  if (email?.status !== "connected") return "Mail isn't connected";
  const counted = email.accounts.filter((a) => a.unread != null);
  const total = counted.reduce((sum, a) => sum + a.unread, 0);
  if (!counted.length) return "Can't reach your mail right now";
  if (!total) return "No unread mail";
  const per = counted.filter((a) => a.unread > 0);
  const detail = per.length > 1 ? ` (${per.map((a) => `${a.unread} ${a.label}`).join(", ")})` : ` in ${per[0].label}`;
  return `${total} unread${detail}`;
}

export function freeGamesAnswer(state, now) {
  if (state?.status !== "connected") return "Free games aren't loaded";
  const { live } = partition(state.games, now);
  if (!live.length) return "Nothing is free right now";
  return `Free now: ${listed(live.map((g) => `${g.title} on ${STORES[g.store] ?? g.store}`))}`;
}

export function weatherAnswer(state) {
  if (state?.status !== "connected") return "No weather set up";
  const c = state.forecast.current;
  const label = describeWeather(c.code, c.is_day).label.toLowerCase();
  const feels = Math.round(c.feels_like) !== Math.round(c.temp) ? `, feels like ${fmtTemp(c.feels_like)}` : "";
  return `${fmtTemp(c.temp)} and ${label} in ${state.place.name}${feels}`;
}

export function cryptoAnswer(state, symbol) {
  if (state?.status !== "connected") return "Prices aren't loaded";
  const coin = state.coins.find((c) => c.symbol === symbol);
  if (!coin) return `${symbol} isn't on your crypto widget`;
  const price = coin.price.toLocaleString("en-US", { maximumFractionDigits: coin.price < 10 ? 4 : 0 });
  const change = Math.abs(coin.change_1d) < 0.05
    ? "flat today"
    : `${coin.change_1d > 0 ? "up" : "down"} ${Math.abs(coin.change_1d).toFixed(1)}% today`;
  const name = COIN_NAMES[symbol] ?? symbol;
  return `${name[0].toUpperCase()}${name.slice(1)} is $${price}, ${change}`;
}

/** Name-list entries for the coins on the widget: each by symbol and by full name. */
export function coinNames(state) {
  if (state?.status !== "connected") return [];
  return state.coins.flatMap((c) => {
    const out = [{ name: c.symbol, symbol: c.symbol }];
    if (COIN_NAMES[c.symbol]) out.push({ name: COIN_NAMES[c.symbol], symbol: c.symbol });
    return out;
  });
}

export function githubAnswer(state) {
  if (state?.status !== "connected") return "GitHub isn't connected";
  if (!state.notifications && !state.open_prs) return "Nothing new on GitHub";
  return `${plural(state.notifications, "notification")}, ${plural(state.open_prs, "open pull request")}`;
}

export function screenTimeAnswer(state) {
  if (!state) return "No screen time yet";
  const top = [...(state.apps ?? [])].sort((a, b) => b.secs - a.secs)[0];
  const most = top ? `, most in ${top.name} (${hoursMinutes(top.secs)})` : "";
  return `${hoursMinutes(state.total)} today${most}`;
}

export function friendsAnswer(state) {
  if (state?.status !== "connected") return "Steam isn't connected";
  const online = state.friends.filter((f) => f.presence !== "offline");
  if (!online.length) return "No friends online";
  const playing = online.filter((f) => f.game).map((f) => `${f.name} in ${f.game}`);
  const head = `${plural(online.length, "friend")} online`;
  return playing.length ? `${head}: ${listed(playing)}` : head;
}
