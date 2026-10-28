import { test } from "node:test";
import assert from "node:assert/strict";
import {
  coinNames,
  cpuAnswer,
  cryptoAnswer,
  friendsAnswer,
  freeGamesAnswer,
  githubAnswer,
  gpuAnswer,
  hoursMinutes,
  mailAnswer,
  nextClassAnswer,
  screenTimeAnswer,
  songAnswer,
  systemAnswer,
  weatherAnswer,
} from "../src/lib/answers.js";
import { createRegistry } from "../src/lib/commands.js";
import { match } from "../src/lib/matcher.js";

// Wednesday 2026-10-28, 14:00 local.
const NOW = new Date(2026, 9, 28, 14, 0).getTime();

test("hardware", () => {
  assert.equal(gpuAnswer({ gpu: 13.4, temp_c: 48.2 }), "GPU at 13%, 48 °C");
  assert.equal(gpuAnswer({ gpu: 13.4, temp_c: null }), "GPU at 13%");
  assert.equal(gpuAnswer(null), "No GPU readings yet");
  assert.equal(cpuAnswer({ cpu: 9, cpu_temp: 54 }), "CPU at 9%, 54 °C");
  assert.equal(systemAnswer({ cpu: 9, ram: 28 }, { gpu: 13 }), "CPU 9%, RAM 28%, GPU 13%");
  assert.equal(systemAnswer({ cpu: 9, ram: 28 }, null), "CPU 9%, RAM 28%");
});

test("song", () => {
  assert.equal(songAnswer({ status: "playing", title: "hey there", artist: "Poppy" }), "hey there by Poppy");
  assert.equal(songAnswer({ status: "paused", title: "Intro", artist: "" }), "Intro (paused)");
  assert.equal(songAnswer({ status: "stopped" }), "Nothing is playing");
  assert.equal(songAnswer(null), "Nothing is playing");
});

test("next class: going on now, later today, later this week, none", () => {
  const classes = [
    { day: 2, start: "11:10", end: "12:50", subject: "KIV/UPS", room: "UC-326" },
    { day: 2, start: "13:00", end: "15:35", subject: "KIV/UPS", room: "UP-101" },
    { day: 3, start: "09:20", end: "11:00", subject: "KIV/ZOS", room: "UC-326" },
  ];
  assert.equal(nextClassAnswer(classes, NOW), "KIV/UPS until 15:35 in UP-101");
  assert.equal(nextClassAnswer(classes, new Date(2026, 9, 28, 12, 55).getTime()), "KIV/UPS at 13:00 in UP-101");
  assert.equal(nextClassAnswer(classes, new Date(2026, 9, 28, 16, 0).getTime()),
    "No more classes today. Next: KIV/ZOS on Thursday at 09:20");
  assert.equal(nextClassAnswer(classes, new Date(2026, 9, 30, 8, 0).getTime()), "No more classes this week");
});

test("mail", () => {
  const acc = (label, unread) => ({ label, unread });
  assert.equal(mailAnswer({ status: "connected", accounts: [acc("SEZNAM", 3), acc("ZČU", 0)] }), "3 unread in SEZNAM");
  assert.equal(mailAnswer({ status: "connected", accounts: [acc("SEZNAM", 3), acc("ZČU", 1)] }), "4 unread (3 SEZNAM, 1 ZČU)");
  assert.equal(mailAnswer({ status: "connected", accounts: [acc("SEZNAM", 0)] }), "No unread mail");
  assert.equal(mailAnswer({ status: "connected", accounts: [acc("SEZNAM", null)] }), "Can't reach your mail right now");
  assert.equal(mailAnswer({ status: "disconnected" }), "Mail isn't connected");
});

test("free games lists live ones only", () => {
  const state = {
    status: "connected",
    games: [
      { title: "Hades", store: "epic", starts_at: null, ends_at: NOW + 86_400_000 },
      { title: "Later", store: "steam", starts_at: NOW + 86_400_000, ends_at: null },
      { title: "Gone", store: "gog", starts_at: null, ends_at: NOW - 1 },
    ],
  };
  assert.equal(freeGamesAnswer(state, NOW), "Free now: Hades on Epic");
  assert.equal(freeGamesAnswer({ status: "connected", games: [] }, NOW), "Nothing is free right now");
});

test("weather", () => {
  const state = {
    status: "connected",
    place: { name: "Plzeň" },
    forecast: { current: { temp: 11.6, feels_like: 9.2, code: 3, is_day: true } },
  };
  assert.match(weatherAnswer(state), /^12° and .+ in Plzeň, feels like 9°$/);
  state.forecast.current.feels_like = 11.8;
  assert.doesNotMatch(weatherAnswer(state), /feels like/);
});

test("crypto, by symbol and by name", () => {
  const state = {
    status: "connected",
    coins: [{ symbol: "BTC", price: 80639.2, change_1d: -2.92 }, { symbol: "ETH", price: 2336, change_1d: 0.01 }],
  };
  assert.equal(cryptoAnswer(state, "BTC"), "Bitcoin is $80,639, down 2.9% today");
  assert.equal(cryptoAnswer(state, "ETH"), "Ethereum is $2,336, flat today");
  assert.equal(cryptoAnswer(state, "SOL"), "SOL isn't on your crypto widget");
  assert.deepEqual(coinNames(state).map((c) => c.name), ["BTC", "bitcoin", "ETH", "ethereum"]);
});

test("github, screen time, friends", () => {
  assert.equal(githubAnswer({ status: "connected", notifications: 1, open_prs: 2 }), "1 notification, 2 open pull requests");
  assert.equal(githubAnswer({ status: "connected", notifications: 0, open_prs: 0 }), "Nothing new on GitHub");

  assert.equal(hoursMinutes(12720), "3 h 32 min");
  assert.equal(screenTimeAnswer({ total: 12720, apps: [{ name: "Cs2", secs: 1080 }, { name: "Steam", secs: 1560 }] }),
    "3 h 32 min today, most in Steam (26 min)");

  const friends = [
    { name: "Pavel", presence: "online", game: "Overwatch 2" },
    { name: "Jan", presence: "away", game: null },
    { name: "Eva", presence: "offline", game: null },
  ];
  assert.equal(friendsAnswer({ status: "connected", friends }), "2 friends online: Pavel in Overwatch 2");
  assert.equal(friendsAnswer({ status: "connected", friends: [friends[2]] }), "No friends online");
});

test("questions reach the right action", () => {
  const registry = createRegistry();
  const lists = { coin: [{ name: "BTC", symbol: "BTC" }, { name: "bitcoin", symbol: "BTC" }], game: [], board: [], link: [] };
  const cases = {
    "how hot is my GPU?": "ask.gpu",
    "cpu temp": "ask.cpu",
    "what's this song": "ask.song",
    "what's next": "ask.class",
    "when is my next lecture": "ask.class",
    "any new mail": "ask.mail",
    "what's the weather": "ask.weather",
    "bitcoin price": "ask.crypto",
    "bitcoin": "ask.crypto",
    "who's online": "ask.friends",
    "how long have I played today": "ask.screentime",
    "any new PRs": "ask.github",
  };
  for (const [text, id] of Object.entries(cases)) {
    const m = match(text, registry, lists);
    assert.equal(m.kind, "run", `"${text}" → ${JSON.stringify(m)}`);
    assert.equal(m.steps[0].action, id, text);
  }
});
