import { test } from "node:test";
import assert from "node:assert/strict";
import { PRIVACY_HINTS, currentGame, playtime, privacyHint, topPlaying } from "../src/lib/steam.js";

test("the game running here wins over the one Steam reports", () => {
  const state = { local: { id: 1, name: "Hades" }, status: "connected", me: { game: "Dota 2" } };
  assert.equal(currentGame(state), "Hades");
  assert.equal(currentGame({ ...state, local: null }), "Dota 2");
});

test("the local game shows without a key", () => {
  assert.equal(currentGame({ local: { id: 1, name: "Hades" }, status: "disconnected", reason: "not connected" }), "Hades");
  assert.equal(currentGame({ local: null, status: "disconnected" }), null);
  assert.equal(currentGame(null), null);
});

test("a private profile is the hint shown, whatever else is hidden", () => {
  assert.equal(privacyHint(["profile", "game_details"]), PRIVACY_HINTS.profile);
  assert.equal(privacyHint(["game_details"]), PRIVACY_HINTS.game_details);
  assert.equal(privacyHint([]), null);
  assert.equal(privacyHint(undefined), null);
});

test("playtime reads in minutes, then tenths of hours, then whole hours", () => {
  assert.equal(playtime(0), "0 min");
  assert.equal(playtime(59), "59 min");
  assert.equal(playtime(60), "1.0 h");
  assert.equal(playtime(754), "13 h");
  assert.equal(playtime(190), "3.2 h");
  assert.equal(playtime(undefined), "0 min");
});

test("the headline friend is the first one in a game", () => {
  const friends = [{ name: "a", game: "CS2" }, { name: "b", game: null }];
  assert.equal(topPlaying(friends).name, "a");
  assert.equal(topPlaying([{ name: "b", game: null }]), null);
  assert.equal(topPlaying(undefined), null);
});
