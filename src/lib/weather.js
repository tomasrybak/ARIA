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

/** Chance from which an hour counts as wet for the rain line. */
export const WET = 50;

/**
 * The one sentence worth acting on in the next hours: when rain starts, when
 * it stops, or that it stays dry. `wet` says whether it is about rain at all.
 * Null when the forecast has no precipitation chances to judge by — "dry"
 * would be a guess.
 */
export function rainLine(hourly) {
  if (!hourly.length || !hourly.every((h) => Number.isFinite(h.precip_prob))) return null;
  const wet = (h) => h.precip_prob >= WET;
  const noun = (h) => (describe(h.code).icon === "snow" ? "Snow" : "Rain");
  if (wet(hourly[0])) {
    const stop = hourly.findIndex((h) => !wet(h));
    const text =
      stop < 0 ? `${noun(hourly[0])} for the next ${hourly.length} h` : `${noun(hourly[0])} until ${hourly[stop].time}`;
    return { text, wet: true };
  }
  const start = hourly.findIndex(wet);
  if (start < 0) return { text: `Dry for the next ${hourly.length} h`, wet: false };
  let end = start;
  while (end < hourly.length && wet(hourly[end])) end++;
  const peak = Math.max(...hourly.slice(start, end).map((h) => h.precip_prob));
  return { text: `${noun(hourly[start])} from ${hourly[start].time} · ${Math.round(peak)}%`, wet: true };
}

/**
 * Pixel geometry for the hourly chart: the temperature line in a top band,
 * rain chance as bars in a band of their own below it. Two bands on one time
 * axis rather than one plot with two y-scales, so neither measure reads
 * against the other's axis. Columns are equal width, so an equal-flex row of
 * HTML labels under the chart lines up with them.
 */
export function hourlyChart(hourly, width, height, { gap = 10, rainShare = 0.26, pad = 18 } = {}) {
  const n = hourly.length;
  const col = width / n;
  const rainH = Math.round(height * rainShare);
  const rainTop = height - rainH;
  const tempBottom = rainTop - gap;
  const temps = hourly.map((h) => h.temp);
  const lo = Math.min(...temps);
  const hi = Math.max(...temps);
  // `pad` keeps room above the highest point for its label.
  const y = (t) => pad + (tempBottom - pad - 6) * (hi === lo ? 0.5 : (hi - t) / (hi - lo));
  const points = hourly.map((h, i) => ({ x: col * (i + 0.5), y: y(h.temp), temp: h.temp }));
  const bars = hourly.map((h, i) => {
    const prob = Number.isFinite(h.precip_prob) ? Math.max(0, Math.min(100, h.precip_prob)) : 0;
    const bh = (rainH * prob) / 100;
    return { x: col * (i + 0.2), w: col * 0.6, y: height - bh, h: bh, prob };
  });
  return { col, rainTop, tempBottom, points, bars, line: monotonePath(points) };
}

const r1 = (n) => Math.round(n * 10) / 10;

/**
 * SVG path through the points as a monotone cubic (Fritsch–Carlson): smooth,
 * but never above the higher or below the lower of two neighbouring hours,
 * so the curve can't invent a peak the forecast doesn't have.
 */
export function monotonePath(pts) {
  const n = pts.length;
  if (!n) return "";
  if (n === 1) return `M${r1(pts[0].x)},${r1(pts[0].y)}`;
  const dx = [];
  const m = [];
  for (let i = 0; i < n - 1; i++) {
    dx[i] = pts[i + 1].x - pts[i].x;
    m[i] = (pts[i + 1].y - pts[i].y) / dx[i];
  }
  const t = [m[0]];
  for (let i = 1; i < n - 1; i++) t[i] = m[i - 1] * m[i] <= 0 ? 0 : (m[i - 1] + m[i]) / 2;
  t[n - 1] = m[n - 2];
  for (let i = 0; i < n - 1; i++) {
    if (m[i] === 0) {
      t[i] = 0;
      t[i + 1] = 0;
      continue;
    }
    const a = t[i] / m[i];
    const b = t[i + 1] / m[i];
    const s = a * a + b * b;
    if (s > 9) {
      const k = 3 / Math.sqrt(s);
      t[i] = k * a * m[i];
      t[i + 1] = k * b * m[i];
    }
  }
  let d = `M${r1(pts[0].x)},${r1(pts[0].y)}`;
  for (let i = 0; i < n - 1; i++) {
    const h = dx[i] / 3;
    d +=
      ` C${r1(pts[i].x + h)},${r1(pts[i].y + h * t[i])}` +
      ` ${r1(pts[i + 1].x - h)},${r1(pts[i + 1].y - h * t[i + 1])}` +
      ` ${r1(pts[i + 1].x)},${r1(pts[i + 1].y)}`;
  }
  return d;
}
