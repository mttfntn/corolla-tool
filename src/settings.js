// The Settings window. Everything saves as soon as it changes.
const { invoke } = window.__TAURI__.core;

const FAMILIES = [
  ["image", "Images"],
  ["pdf", "PDF"],
  ["video", "Video"],
  ["audio", "Audio"],
  ["model", "3D"],
  ["archive", "Archives"],
  ["subtitle", "Subtitles"],
  ["text", "Text"],
  ["font", "Fonts"],
];

const $ = (id) => document.getElementById(id);
const SVG = "http://www.w3.org/2000/svg";

if (/Mac/.test(navigator.userAgent)) $("gesture-key").textContent = "Cmd";

let settings = null;
let page = "image";

async function store(next) {
  settings = await invoke("save_settings", { settings: next });
}

// ---------- Navigation ----------

function drawNav() {
  const nav = $("nav");
  nav.replaceChildren();
  const add = (id, title) => {
    const button = document.createElement("button");
    button.textContent = title;
    if (id === page) button.setAttribute("aria-current", "page");
    button.addEventListener("click", () => show(id));
    nav.append(button);
  };
  FAMILIES.forEach(([id, title]) => add(id, title));
  nav.append(document.createElement("hr"));
  add("presets", "Presets");
  add("general", "General");
}

async function show(id) {
  page = id;
  drawNav();
  const family = FAMILIES.find(([key]) => key === id);
  $("layout-page").hidden = !family;
  $("presets-page").hidden = id !== "presets";
  $("general-page").hidden = id !== "general";
  if (family) {
    $("layout-title").textContent = family[1];
    await drawLayout(id);
  } else if (id === "presets") {
    closeEditor();
    drawPresets();
  } else {
    await drawGeneral();
  }
}

// ---------- Arranging circles ----------

function layoutOf(family) {
  return settings.layout[family] || { order: [], hidden: [] };
}

async function drawLayout(family) {
  const items = await invoke("catalog", { family });
  const layout = layoutOf(family);
  const visible = items.filter((item) => !layout.hidden.includes(item.id));
  await drawPreview(family, items, visible);

  const toggles = $("toggles");
  toggles.replaceChildren();
  for (const item of items) {
    const label = document.createElement("label");
    label.className = "check";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = !layout.hidden.includes(item.id);
    box.addEventListener("change", async () => {
      const current = layoutOf(family);
      const hidden = box.checked
        ? current.hidden.filter((id) => id !== item.id)
        : [...current.hidden, item.id];
      await saveLayout(family, items.map((i) => i.id), hidden);
      await drawLayout(family);
    });
    label.append(box, " ", itemName(item));
    toggles.append(label);
  }
}

function itemName(item) {
  if (item.id === "combine") return "PDF (all in one)";
  if (item.id === "pdf" && page === "image") return "PDF (one each)";
  return `${item.line1} ${item.line2}`.trim();
}

async function saveLayout(family, order, hidden) {
  await store({
    ...settings,
    layout: { ...settings.layout, [family]: { order, hidden } },
  });
}

async function drawPreview(family, items, visible) {
  const svg = $("preview");
  svg.replaceChildren();
  const ring = await invoke("ring_for", { count: visible.length });
  const place = (i) => {
    const angle = (i * 2 * Math.PI) / visible.length;
    return [ring.orbit * Math.sin(angle), -ring.orbit * Math.cos(angle)];
  };
  const hub = document.createElementNS(SVG, "circle");
  hub.setAttribute("r", "23");
  hub.setAttribute("class", "hub");
  visible.forEach((item, i) => {
    const [x, y] = place(i);
    svg.append(WheelDraw.line(x, y, ring.circle));
  });
  const mark = document.createElementNS(SVG, "text");
  mark.setAttribute("class", "mark");
  mark.setAttribute("y", "1");
  mark.textContent = "©";
  svg.append(hub, mark);
  const nodes = visible.map((item, i) => {
    const [x, y] = place(i);
    const node = WheelDraw.circle(item, x, y, ring.circle);
    svg.append(node);
    return node;
  });

  const toSvg = (event) => {
    const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(svg.getScreenCTM().inverse());
    return [point.x, point.y];
  };
  const nearest = (x, y, except) => {
    let best = -1;
    let distance = ring.circle * 1.2;
    visible.forEach((_, i) => {
      if (i === except) return;
      const [cx, cy] = place(i);
      const d = Math.hypot(x - cx, y - cy);
      if (d < distance) {
        distance = d;
        best = i;
      }
    });
    return best;
  };

  nodes.forEach((node, i) => {
    node.addEventListener("pointerdown", (event) => {
      event.preventDefault();
      node.setPointerCapture(event.pointerId);
      node.classList.add("dragging");
      svg.append(node); // on top of the others while it moves
      let target = -1;
      const move = (e) => {
        const [x, y] = toSvg(e);
        node.setAttribute("transform", `translate(${x} ${y})`);
        const over = nearest(x, y, i);
        if (over !== target) {
          nodes[target]?.classList.remove("target");
          nodes[over]?.classList.add("target");
          target = over;
        }
      };
      const up = async () => {
        node.removeEventListener("pointermove", move);
        node.removeEventListener("pointerup", up);
        node.removeEventListener("pointercancel", up);
        if (target >= 0) {
          [visible[i], visible[target]] = [visible[target], visible[i]];
          const hiddenIds = items.filter((item) => !visible.includes(item)).map((item) => item.id);
          await saveLayout(family, [...visible.map((item) => item.id), ...hiddenIds], layoutOf(family).hidden);
        }
        await drawLayout(family);
      };
      node.addEventListener("pointermove", move);
      node.addEventListener("pointerup", up);
      node.addEventListener("pointercancel", up);
    });
  });
}

// ---------- Presets ----------

let editing = null; // the tool being edited, or a new one

function describe(tool) {
  const op = tool.op;
  if (op.kind === "compress") {
    const on = tool.families
      .map((f) => FAMILIES.find(([key]) => key === f)?.[1])
      .join(", ");
    const size = op.max_edge ? `, longest side ${op.max_edge} px` : "";
    return `Compress at quality ${op.quality}${size}. ${on}.`;
  }
  if (op.kind === "resize_divide") return `Images ${op.by} times smaller.`;
  return `Images fit within ${op.edge} px.`;
}

function miniCircle(tool) {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", "-60 -60 120 120");
  svg.setAttribute("aria-hidden", "true");
  svg.append(WheelDraw.circle(tool, 0, 0, 56));
  return svg;
}

function drawPresets() {
  const list = $("preset-list");
  list.replaceChildren();
  for (const tool of settings.tools) {
    const row = document.createElement("div");
    row.className = "preset";
    const about = document.createElement("div");
    about.className = "about";
    about.textContent = describe(tool);
    const edit = document.createElement("button");
    edit.textContent = "Edit";
    edit.addEventListener("click", () => openEditor(structuredClone(tool)));
    const remove = document.createElement("button");
    remove.textContent = "Delete";
    remove.addEventListener("click", async () => {
      await store({ ...settings, tools: settings.tools.filter((t) => t.id !== tool.id) });
      closeEditor();
      drawPresets();
    });
    row.append(miniCircle(tool), about, edit, remove);
    list.append(row);
  }
  if (settings.tools.length === 0) {
    const none = document.createElement("p");
    none.className = "hint";
    none.textContent = "No compress or resize circles. Add one below.";
    list.append(none);
  }
}

function newId() {
  return `custom-${Date.now().toString(36)}`;
}

function openEditor(tool) {
  editing = tool;
  const compress = tool.op.kind === "compress";
  $("editor").hidden = false;
  $("editor-error").textContent = "";
  $("editor-title").textContent = settings.tools.some((t) => t.id === tool.id) ? "Edit circle" : "New circle";
  document.querySelector('[data-for="compress"]').hidden = !compress;
  document.querySelector('[data-for="resize"]').hidden = compress;
  if (compress) {
    $("line1").value = tool.line1;
    $("line2").value = tool.line2;
    $("quality").value = tool.op.quality;
    $("quality-value").textContent = tool.op.quality;
    $("max-edge").value = tool.op.max_edge ?? "";
    document.querySelectorAll("[data-family]").forEach((box) => {
      box.checked = tool.families.includes(box.dataset.family);
    });
    $("line1").focus();
  } else {
    const divide = tool.op.kind === "resize_divide";
    document.querySelector(`input[name="resize-mode"][value="${divide ? "divide" : "edge"}"]`).checked = true;
    $("resize-value").value = divide ? tool.op.by : tool.op.edge;
    syncResize();
    $("resize-value").focus();
  }
  $("editor").scrollIntoView({ block: "nearest" });
}

function closeEditor() {
  editing = null;
  $("editor").hidden = true;
}

function resizeLabel() {
  const mode = document.querySelector('input[name="resize-mode"]:checked')?.value;
  const value = $("resize-value").value || "?";
  return ["RSZ", mode === "divide" ? `/${value}` : value];
}

function syncResize() {
  const divide = document.querySelector('input[name="resize-mode"]:checked')?.value === "divide";
  $("resize-label").textContent = divide ? "Divide size by" : "Longest side, pixels";
  $("resize-value").min = divide ? 2 : 16;
  $("resize-preview").textContent = resizeLabel().join(" ");
}

function readEditor() {
  const whole = (text) => Number.parseInt(text, 10);
  if (editing.op.kind === "compress") {
    const line1 = $("line1").value.trim();
    if (!line1) return { error: "Give the circle a label." };
    const families = [...document.querySelectorAll("[data-family]:checked")].map((box) => box.dataset.family);
    if (families.length === 0) return { error: "Choose at least one file type." };
    const edge = whole($("max-edge").value);
    if ($("max-edge").value && !(edge >= 16)) return { error: "The longest side must be at least 16 pixels." };
    return {
      tool: {
        ...editing,
        line1,
        line2: $("line2").value.trim(),
        families,
        op: { kind: "compress", quality: whole($("quality").value), max_edge: edge || null },
      },
    };
  }
  const divide = document.querySelector('input[name="resize-mode"]:checked').value === "divide";
  const value = whole($("resize-value").value);
  if (divide && !(value >= 2 && value <= 64)) return { error: "Divide by a whole number from 2 to 64." };
  if (!divide && !(value >= 16)) return { error: "The longest side must be at least 16 pixels." };
  const [line1, line2] = resizeLabel();
  return {
    tool: {
      ...editing,
      line1,
      line2,
      families: ["image"],
      op: divide ? { kind: "resize_divide", by: value } : { kind: "resize_edge", edge: value },
    },
  };
}

$("add-compress").addEventListener("click", () => {
  openEditor({
    id: newId(),
    line1: "CMPRSS",
    line2: "",
    families: ["image", "pdf", "video"],
    op: { kind: "compress", quality: 60, max_edge: null },
  });
});

$("add-resize").addEventListener("click", () => {
  openEditor({
    id: newId(),
    line1: "RSZ",
    line2: "/3",
    families: ["image"],
    op: { kind: "resize_divide", by: 3 },
  });
});

$("cancel-edit").addEventListener("click", closeEditor);
$("quality").addEventListener("input", () => {
  $("quality-value").textContent = $("quality").value;
});
$("resize-value").addEventListener("input", syncResize);
document.querySelectorAll('input[name="resize-mode"]').forEach((radio) => {
  radio.addEventListener("change", syncResize);
});

$("editor").addEventListener("submit", async (event) => {
  event.preventDefault();
  const { tool, error } = readEditor();
  $("editor-error").textContent = error || "";
  if (!tool) return;
  const exists = settings.tools.some((t) => t.id === tool.id);
  const tools = exists ? settings.tools.map((t) => (t.id === tool.id ? tool : t)) : [...settings.tools, tool];
  await store({ ...settings, tools });
  closeEditor();
  drawPresets();
});

// ---------- General ----------

async function drawGeneral() {
  $("enabled").checked = settings.enabled;
  $("login").checked = await invoke("get_login");
  $("login-error").textContent = "";
  document.querySelectorAll('input[name="quality"]').forEach((radio) => {
    radio.checked = Number(radio.value) === settings.quality;
  });
  $("reset").textContent = "Reset everything";
  resetArmed = false;
}

$("enabled").addEventListener("change", () => store({ ...settings, enabled: $("enabled").checked }));

$("login").addEventListener("change", async () => {
  const want = $("login").checked;
  try {
    await invoke("set_login", { on: want });
    $("login-error").textContent = "";
  } catch (error) {
    $("login").checked = !want;
    $("login-error").textContent = `Couldn't change this: ${error}`;
  }
});

document.querySelectorAll('input[name="quality"]').forEach((radio) => {
  radio.addEventListener("change", () => store({ ...settings, quality: Number(radio.value) }));
});

let resetArmed = false;
$("reset").addEventListener("click", async () => {
  if (!resetArmed) {
    resetArmed = true;
    $("reset").textContent = "Click again to confirm";
    return;
  }
  await store(await invoke("default_settings"));
  await drawGeneral();
});

(async () => {
  settings = await invoke("get_settings");
  await show("image");
})();
