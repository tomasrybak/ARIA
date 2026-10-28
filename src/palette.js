// Command bar view. Matching and running happen in the main window (see
// initCommands in main.js); this window only sends text and shows replies.
//
// Events: "palette-query" {id, text} → "palette-result" {id, run, items}
//         "palette-run" {id, text, index} → "palette-answer" {id, text, error}
// Replies carry the id of the request; anything older than the latest is dropped.

const { emitTo, listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;
const { LogicalSize } = window.__TAURI__.dpi;

const ANSWER_MS = 4000;
const WIDTH = 640;

const win = getCurrentWindow();
const panel = document.getElementById("panel");
const query = document.getElementById("query");
const list = document.getElementById("items");
const answer = document.getElementById("answer");

let seq = 0;
let items = [];
let selected = 0;
let hideTimer = null;

// The body always fills the window, so measure the content instead.
function fit() {
  const height = Math.ceil(panel.getBoundingClientRect().height);
  win.setSize(new LogicalSize(WIDTH, height)).catch(() => {});
}

function hide() {
  clearTimeout(hideTimer);
  win.hide().catch(() => {});
}

function reset() {
  clearTimeout(hideTimer);
  seq++;
  query.value = "";
  items = [];
  selected = 0;
  answer.hidden = true;
  render();
  query.focus();
}

function render() {
  list.replaceChildren(
    ...items.map((item, i) => {
      const li = document.createElement("li");
      li.textContent = item.label;
      li.classList.toggle("selected", i === selected);
      li.addEventListener("click", () => {
        selected = i;
        run();
      });
      return li;
    }),
  );
  fit();
}

function run() {
  const text = query.value.trim();
  if (!text) return;
  emitTo("main", "palette-run", { id: ++seq, text, index: selected });
}

query.addEventListener("input", () => {
  answer.hidden = true;
  emitTo("main", "palette-query", { id: ++seq, text: query.value });
});

query.addEventListener("keydown", (e) => {
  if (e.key === "Escape") hide();
  else if (e.key === "Enter") run();
  else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && items.length) {
    e.preventDefault();
    selected = (selected + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length;
    render();
  }
});

listen("palette-result", ({ payload }) => {
  if (payload.id !== seq) return;
  items = payload.items;
  selected = 0;
  render();
});

listen("palette-answer", ({ payload }) => {
  if (payload.id !== seq) return;
  if (!payload.text && !payload.error) return hide();
  items = [];
  answer.textContent = payload.text || "Something went wrong.";
  answer.classList.toggle("error", payload.error);
  answer.hidden = false;
  render();
  clearTimeout(hideTimer);
  hideTimer = setTimeout(hide, ANSWER_MS);
});

listen("palette-open", reset);
reset();
