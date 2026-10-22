// WEATHER formatting: WMO weather codes to a label and an icon, and units.
// The collector passes Open-Meteo's numbers through already converted to the
// chosen units, so nothing here converts — it only labels.

// WMO 4677 "weather interpretation codes" as Open-Meteo uses them.
// `icon` names a <symbol id="wx-…"> in index.html.
const CODES = {
  0: ["Clear", "clear"],
  1: ["Mainly clear", "clear"],
  2: ["Partly cloudy", "partly"],
  3: ["Overcast", "cloud"],
  45: ["Fog", "fog"],
  48: ["Rime fog", "fog"],
  51: ["Light drizzle", "drizzle"],
  53: ["Drizzle", "drizzle"],
  55: ["Heavy drizzle", "drizzle"],
  56: ["Freezing drizzle", "drizzle"],
  57: ["Freezing drizzle", "drizzle"],
  61: ["Light rain", "rain"],
  63: ["Rain", "rain"],
  65: ["Heavy rain", "rain"],
  66: ["Freezing rain", "rain"],
  67: ["Freezing rain", "rain"],
  71: ["Light snow", "snow"],
  73: ["Snow", "snow"],
  75: ["Heavy snow", "snow"],
  77: ["Snow grains", "snow"],
  80: ["Rain showers", "rain"],
  81: ["Rain showers", "rain"],
  82: ["Violent showers", "rain"],
  85: ["Snow showers", "snow"],
  86: ["Snow showers", "snow"],
  95: ["Thunderstorm", "storm"],
  96: ["Thunderstorm, hail", "storm"],
  99: ["Thunderstorm, hail", "storm"],
};

/**
 * Label and icon for a WMO code. Clear and partly cloudy have night
 * variants; an unknown code reads as cloud rather than as a lie about sun.
 */
export function describe(code, isDay = true) {
  const [label, icon] = CODES[code] ?? ["Unknown", "cloud"];
  if (!isDay && icon === "clear") return { label, icon: "night" };
  if (!isDay && icon === "partly") return { label, icon: "night-partly" };
  return { label, icon };
}

export function fmtTemp(value) {
  if (!Number.isFinite(value)) return "—";
  // Round first so -0.4 shows as 0°, not -0°.
  const n = Math.round(value);
  return `${n === 0 ? 0 : n}°`;
}

export function windUnit(units) {
  return units === "imperial" ? "mph" : "km/h";
}

export function fmtWind(value, units) {
  if (!Number.isFinite(value)) return "—";
  return `${Math.round(value)} ${windUnit(units)}`;
}

/** Precipitation chance worth showing; below 20 % it is noise on a HUD. */
export function fmtPrecip(prob) {
  return Number.isFinite(prob) && prob >= 20 ? `${Math.round(prob)}%` : "";
}

/**
 * Short day name for a "YYYY-MM-DD" date at the place. Parsed as a calendar
 * date, not an instant, so the viewer's own time zone can't shift it a day.
 */
export function dayLabel(date, index) {
  if (index === 0) return "Today";
  const [y, m, d] = String(date).split("-").map(Number);
  if (!y || !m || !d) return "";
  return new Date(Date.UTC(y, m - 1, d)).toLocaleDateString("en", { weekday: "short", timeZone: "UTC" });
}

/** "Plzeň" plus "Plzeňský kraj, Czechia" → one line for search results. */
export function placeLine(place) {
  return place.detail ? `${place.name} · ${place.detail}` : place.name;
}

/**
 * Where one day's min–max sits on the week's scale, as CSS percentages, so
 * the daily bars line up and a warm day reads as warm at a glance.
 */
export function rangeBar(min, max, weekMin, weekMax) {
  const span = weekMax - weekMin;
  if (!(span > 0)) return { left: 0, width: 100 };
  const left = ((min - weekMin) / span) * 100;
  const width = ((max - min) / span) * 100;
  return { left: Math.max(0, left), width: Math.max(2, Math.min(100 - left, width)) };
}
