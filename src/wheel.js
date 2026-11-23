// Draws what the Rust core tells it to. Hit-testing and positions live in Rust
// (wheel.rs), because the webview gets no pointer events during an OS file drag.
const { listen } = window.__TAURI__.event;

const body = document.body;
const disc = document.getElementById("disc");
const lines = document.getElementById("lines");
const circles = document.getElementById("circles");
const top_ = document.getElementById("top");
const second = document.getElementById("second");

const BUSY_DISC = 132;

function setText(first, next) {
  top_.textContent = first;
  second.textContent = next;
}

// "350kb", as in the mockup.
function formatSize(bytes) {
  const units = ["b", "kb", "mb", "gb", "tb"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = value >= 100 || unit === 0 ? Math.round(value) : value.toFixed(1);
  return `${shown}${units[unit]}`;
}

listen("wheel:catch", () => {
  body.dataset.mode = "catch";
});

listen("wheel:open", ({ payload }) => {
  lines.replaceChildren();
  circles.replaceChildren();
  if (payload.items.length === 0) {
    disc.setAttribute("r", BUSY_DISC);
    setText("NO COMMON", "FORMAT");
    body.dataset.mode = "empty";
    return;
  }
  disc.setAttribute("r", payload.disc);
  for (const item of payload.items) {
    if (!item.disabled) lines.append(WheelDraw.line(item.x, item.y, payload.circle));
    circles.append(WheelDraw.circle(item, item.x, item.y, payload.circle));
  }
  const count = payload.count === 1 ? "1 FILE" : `${payload.count} FILES`;
  setText(formatSize(payload.total_bytes), count);
  body.dataset.mode = "open";
});

listen("wheel:hover", ({ payload }) => {
  [...circles.children].forEach((el, i) => el.classList.toggle("hover", i === payload));
});

listen("wheel:progress", ({ payload }) => {
  disc.setAttribute("r", BUSY_DISC);
  setText(`${payload.percent}%`, `${payload.done}/${payload.total}`);
  body.dataset.mode = "busy";
});

listen("wheel:hide", () => {
  body.dataset.mode = "hidden";
});
