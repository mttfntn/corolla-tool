// Draws circles. Shared by the wheel and the Settings preview, so they look alike.
const WheelDraw = (() => {
  const SVG = "http://www.w3.org/2000/svg";
  const HUB_EDGE = 26; // just outside the hub, where connecting lines start

  function element(name, attributes = {}) {
    const node = document.createElementNS(SVG, name);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
    return node;
  }

  // Sizes text to fit across a circle. Capitals are about 0.68 em wide in Arial.
  function fit(text, largest, radius) {
    const scale = radius / 56;
    const width = radius * 1.56;
    return Math.min(largest * scale, width / (Math.max(text.length, 1) * 0.68));
  }

  function label(item, radius) {
    const group = element("g", { class: "label" });
    const add = (text, y, size) => {
      const line = element("text", { y, "font-size": size.toFixed(1) });
      line.textContent = text;
      group.append(line);
    };
    if (!item.line2) {
      add(item.line1, 0, fit(item.line1, 34, radius));
      return group;
    }
    // A short second line like "/2" is the part that matters, so it's the larger one.
    const first = fit(item.line1, 28, radius);
    const second = fit(item.line2, item.line2.length <= 3 ? 38 : 22, radius);
    const offset = (first + second) / 4 + radius * 0.02;
    add(item.line1, -offset, first);
    add(item.line2, offset, second);
    return group;
  }

  function circle(item, x, y, radius) {
    const group = element("g", {
      class: "tool" + (item.disabled ? " disabled" : ""),
      transform: `translate(${x} ${y})`,
    });
    group.append(element("circle", { r: radius }), label(item, radius));
    return group;
  }

  // From the hub to the circle's edge. Greyed-out circles get no line.
  function line(x, y, radius) {
    const distance = Math.hypot(x, y) || 1;
    const [ux, uy] = [x / distance, y / distance];
    return element("line", {
      x1: ux * HUB_EDGE,
      y1: uy * HUB_EDGE,
      x2: ux * (distance - radius),
      y2: uy * (distance - radius),
    });
  }

  return { circle, line };
})();
