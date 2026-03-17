import { getStroke } from './lib/perfect-freehand.js';

const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;
const { PhysicalSize, PhysicalPosition } = window.__TAURI__.dpi;

const appWindow = getCurrentWindow();

// Pen state — declared early because the capture-phase resize handler checks it.
let penActive = false;
// Grid editor state — declared early for redrawStrokes reference.
let gridActive = false;
let _lastMouseX = 0, _lastMouseY = 0;

// --- Edge/corner resize for frameless transparent window ---
// Tauri 2's startResizeDragging rejects compound corner directions on WebView2,
// so edges use native resize and corners use manual pointer-tracked resize.
const RESIZE_BORDER = 6;   // edge detection strip (px)
const CORNER_SIZE = 40;    // corner grab zone (px from each frame corner)
const MIN_SIZE = 200;      // minimum window dimension (px)

// Read frame geometry from DOM so JS stays in sync with CSS
const FRAME_TOP = document.getElementById('drag-handle')?.offsetHeight ?? 32;
const FRAME_BOTTOM = document.getElementById('toolbar')?.offsetHeight ?? 100;

// Sign vectors for each resize direction — avoids stringly-typed includes() checks
const DIR_SIGNS = {
  North:     { dx:  0, dy: -1 }, South:     { dx:  0, dy:  1 },
  West:      { dx: -1, dy:  0 }, East:      { dx:  1, dy:  0 },
  NorthWest: { dx: -1, dy: -1 }, NorthEast: { dx:  1, dy: -1 },
  SouthWest: { dx: -1, dy:  1 }, SouthEast: { dx:  1, dy:  1 },
};

const CURSOR_MAP = {
  NorthWest: 'nwse-resize', SouthEast: 'nwse-resize',
  NorthEast: 'nesw-resize', SouthWest: 'nesw-resize',
  North: 'ns-resize', South: 'ns-resize',
  East: 'ew-resize',  West: 'ew-resize',
};

function getResizeDirection(e) {
  const w = window.innerWidth;
  const h = window.innerHeight;
  const x = e.clientX;
  const y = e.clientY;
  const frameBottom = h - FRAME_BOTTOM;

  // Corners: square zones at each blue frame corner (checked first)
  const nearFrameTop    = Math.abs(y - FRAME_TOP) < CORNER_SIZE;
  const nearFrameBottom = Math.abs(y - frameBottom) < CORNER_SIZE;
  const nearLeft        = x < CORNER_SIZE;
  const nearRight       = x > w - CORNER_SIZE;

  if (nearFrameTop && nearLeft)     return 'NorthWest';
  if (nearFrameTop && nearRight)    return 'NorthEast';
  if (nearFrameBottom && nearLeft)  return 'SouthWest';
  if (nearFrameBottom && nearRight) return 'SouthEast';

  return null;
}

function isNearEdge(e) {
  return getResizeDirection(e) !== null;
}

// Cursor feedback — only write to dataset when direction changes
let lastCursor = null;
document.addEventListener('mousemove', (e) => {
  const direction = getResizeDirection(e);
  const cursor = direction ? CURSOR_MAP[direction] : null;
  if (cursor === lastCursor) return;
  lastCursor = cursor;
  if (cursor) {
    document.documentElement.dataset.resizeCursor = cursor;
  } else {
    delete document.documentElement.dataset.resizeCursor;
  }
});

// Corner directions need manual resize (native startResizeDragging fails on WebView2)
const CORNER_DIRECTIONS = new Set(['NorthWest', 'NorthEast', 'SouthWest', 'SouthEast']);

// Manual corner resize state
let resizing = null;
let resizeRaf = 0;

document.addEventListener('pointermove', (e) => {
  if (!resizing || !resizing.ready) return;
  resizing.lastScreenX = e.screenX;
  resizing.lastScreenY = e.screenY;
  // Throttle to display refresh rate
  if (!resizeRaf) {
    resizeRaf = requestAnimationFrame(() => {
      resizeRaf = 0;
      if (!resizing || !resizing.ready) return;
      const { dx: sx, dy: sy } = DIR_SIGNS[resizing.direction];
      const deltaX = (resizing.lastScreenX - resizing.startX) * resizing.scale;
      const deltaY = (resizing.lastScreenY - resizing.startY) * resizing.scale;

      let w = resizing.origW + sx * deltaX;
      let h = resizing.origH + sy * deltaY;
      w = Math.max(MIN_SIZE, w);
      h = Math.max(MIN_SIZE, h);

      let x = resizing.origX;
      let y = resizing.origY;
      if (sx < 0) x = resizing.origX + (resizing.origW - w);
      if (sy < 0) y = resizing.origY + (resizing.origH - h);

      appWindow.setSize(new PhysicalSize(Math.round(w), Math.round(h)));
      appWindow.setPosition(new PhysicalPosition(Math.round(x), Math.round(y)));
    });
  }
});

document.addEventListener('pointerup', (e) => {
  if (resizing) {
    e.target.releasePointerCapture(e.pointerId);
    resizing = null;
  }
});

document.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  const direction = getResizeDirection(e);
  // When pen is active, only intercept if we're on a resize corner/edge —
  // otherwise let the event pass through to the canvas for drawing.
  if (penActive && !direction) return;
  if (direction) {
    e.preventDefault();
    e.stopImmediatePropagation();
    if (CORNER_DIRECTIONS.has(direction)) {
      // Capture pointer SYNCHRONOUSLY — cannot defer to async
      e.target.setPointerCapture(e.pointerId);
      resizing = { direction, startX: e.screenX, startY: e.screenY, ready: false };
      Promise.all([appWindow.outerPosition(), appWindow.outerSize(), appWindow.scaleFactor()])
        .then(([pos, size, scale]) => {
          if (resizing) {
            resizing.origX = pos.x;  resizing.origY = pos.y;
            resizing.origW = size.width;  resizing.origH = size.height;
            resizing.scale = scale;
            resizing.ready = true;
          }
        });
    } else {
      appWindow.startResizeDragging(direction);
    }
  }
}, true);

// --- Focus box: draggable/resizable highlight region inside capture frame ---
const focusBox = document.getElementById('focus-box');
const focusToggle = document.getElementById('focus-toggle');
let focusActive = false;
let focusOp = null; // { type: 'move'|'resize', ... }

const FOCUS_EDGE = 8;      // edge/corner detection for focus box (px)
const FOCUS_MIN = 40;      // minimum focus box dimension (px)

focusToggle.addEventListener('click', () => {
  focusActive = !focusActive;
  focusBox.classList.toggle('hidden', !focusActive);
  focusToggle.classList.toggle('active', focusActive);
  focusToggle.textContent = focusActive ? '[x]' : '[ ]';
});

// Detect which edge/corner of focus box the cursor is near
function getFocusEdge(e) {
  const rect = focusBox.getBoundingClientRect();
  const x = e.clientX - rect.left;
  const y = e.clientY - rect.top;
  const w = rect.width;
  const h = rect.height;

  const top = y < FOCUS_EDGE;
  const bottom = y > h - FOCUS_EDGE;
  const left = x < FOCUS_EDGE;
  const right = x > w - FOCUS_EDGE;

  if (top && left)     return 'NorthWest';
  if (top && right)    return 'NorthEast';
  if (bottom && left)  return 'SouthWest';
  if (bottom && right) return 'SouthEast';
  if (top)    return 'North';
  if (bottom) return 'South';
  if (left)   return 'West';
  if (right)  return 'East';
  return null;
}

// Update cursor when hovering over focus box edges
focusBox.addEventListener('mousemove', (e) => {
  if (focusOp) return;
  const edge = getFocusEdge(e);
  focusBox.style.cursor = edge ? CURSOR_MAP[edge] : 'move';
});

focusBox.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  e.stopPropagation();
  focusBox.setPointerCapture(e.pointerId);

  const edge = getFocusEdge(e);
  if (edge) {
    // Resize
    focusOp = {
      type: 'resize', edge,
      startX: e.clientX, startY: e.clientY,
      origLeft: focusBox.offsetLeft, origTop: focusBox.offsetTop,
      origW: focusBox.offsetWidth, origH: focusBox.offsetHeight,
    };
  } else {
    // Move
    focusOp = {
      type: 'move',
      startX: e.clientX, startY: e.clientY,
      origLeft: focusBox.offsetLeft, origTop: focusBox.offsetTop,
    };
  }
});

document.addEventListener('pointermove', (e) => {
  if (!focusOp) return;
  const dx = e.clientX - focusOp.startX;
  const dy = e.clientY - focusOp.startY;

  const minY = FRAME_TOP + 2;
  const maxBottom = window.innerHeight - FRAME_BOTTOM - 2;
  const minX = 2;
  const maxRight = window.innerWidth - 2;

  if (focusOp.type === 'move') {
    const maxX = maxRight - focusBox.offsetWidth;
    const maxY = maxBottom - focusBox.offsetHeight;
    focusBox.style.left = Math.max(minX, Math.min(maxX, focusOp.origLeft + dx)) + 'px';
    focusBox.style.top = Math.max(minY, Math.min(maxY, focusOp.origTop + dy)) + 'px';
  } else {
    const { dx: sx, dy: sy } = DIR_SIGNS[focusOp.edge];
    let l = focusOp.origLeft;
    let t = focusOp.origTop;
    let w = focusOp.origW;
    let h = focusOp.origH;

    if (sx > 0) w = Math.max(FOCUS_MIN, w + dx);
    if (sx < 0) { w = Math.max(FOCUS_MIN, w - dx); l = focusOp.origLeft + (focusOp.origW - w); }
    if (sy > 0) h = Math.max(FOCUS_MIN, h + dy);
    if (sy < 0) { h = Math.max(FOCUS_MIN, h - dy); t = focusOp.origTop + (focusOp.origH - h); }

    // Clamp to frame
    l = Math.max(minX, Math.min(maxRight - w, l));
    t = Math.max(minY, Math.min(maxBottom - h, t));

    focusBox.style.left = l + 'px';
    focusBox.style.top = t + 'px';
    focusBox.style.width = w + 'px';
    focusBox.style.height = h + 'px';
  }
});

document.addEventListener('pointerup', (e) => {
  if (focusOp) {
    focusOp = null;
  }
});

// Get focus box bounds relative to the capture frame (for prompt injection)
function getFocusRegion() {
  if (!focusActive) return null;
  const frameTop = FRAME_TOP + 2; // account for border
  const frameLeft = 2;
  return {
    x: focusBox.offsetLeft - frameLeft,
    y: focusBox.offsetTop - frameTop,
    width: focusBox.offsetWidth,
    height: focusBox.offsetHeight,
  };
}

// --- Drawing pen (perfect-freehand) ---
const drawCanvas = document.getElementById('draw-canvas');
const drawCtx = drawCanvas.getContext('2d');
const penToggle = document.getElementById('pen-toggle');
const penColorBtn = document.getElementById('pen-color');
const penClear = document.getElementById('pen-clear');

const PEN_COLORS = [
  { hex: '#ff4444', name: 'red' },
  { hex: '#00e5ff', name: 'cyan' },
  { hex: '#ffd740', name: 'yellow' },
  { hex: '#69ff69', name: 'green' },
  { hex: '#ffffff', name: 'white' },
];
let penColorIndex = 0;
let penColor = PEN_COLORS[0].hex;
let penStrokes = [];   // completed: { outline, color }
let currentPoints = null;  // in-progress [x, y, pressure][]
let drawRaf = 0;

const PEN_OPTIONS = {
  size: 6,
  thinning: 0.5,
  smoothing: 0.5,
  streamline: 0.5,
  simulatePressure: true,
};

// Size canvas bitmap to match CSS layout (1:1 — no DPR scaling).
// Canvas display size is set by CSS position constraints; bitmap just
// needs to match so drawing coordinates map directly to pixels.
function resizeDrawCanvas() {
  const w = drawCanvas.clientWidth;
  const h = drawCanvas.clientHeight;
  if (drawCanvas.width === w && drawCanvas.height === h) return;
  drawCanvas.width = w;
  drawCanvas.height = h;
  redrawStrokes();
}

function redrawStrokes() {
  drawCtx.clearRect(0, 0, drawCanvas.width, drawCanvas.height);
  for (const stroke of penStrokes) {
    renderStroke(stroke.outline, stroke.color);
  }
  if (currentPoints && currentPoints.length > 1) {
    const outline = getStroke(currentPoints, PEN_OPTIONS);
    renderStroke(outline, penColor);
  }
  // Vision overlays drawn on top of pen strokes
  if (typeof renderVisionOverlays === 'function') renderVisionOverlays();
  // Grid buffer lines on top of everything when grid editor is active
  if (gridActive && typeof renderGridDividers === 'function') {
    const hover = gridDrag ? gridDrag : hitTestBuffer(_lastMouseX, _lastMouseY);
    renderGridDividers(hover);
  }
}

function renderStroke(outline, color) {
  if (outline.length < 2) return;
  drawCtx.fillStyle = color;
  drawCtx.beginPath();
  drawCtx.moveTo(outline[0][0], outline[0][1]);
  for (let i = 1; i < outline.length - 1; i++) {
    const xc = (outline[i][0] + outline[i + 1][0]) / 2;
    const yc = (outline[i][1] + outline[i + 1][1]) / 2;
    drawCtx.quadraticCurveTo(outline[i][0], outline[i][1], xc, yc);
  }
  drawCtx.closePath();
  drawCtx.fill();
}

// Toggle pen on/off
penToggle.addEventListener('click', () => {
  penActive = !penActive;
  drawCanvas.classList.toggle('active', penActive);
  penToggle.classList.toggle('active', penActive);
  // Raise canvas above everything when drawing, and disable pointer events
  // on overlapping elements so they don't intercept pen strokes.
  drawCanvas.style.zIndex = penActive ? '12' : '';
  const blockEvents = penActive ? 'none' : '';
  document.getElementById('results').style.pointerEvents = blockEvents;
  focusBox.style.pointerEvents = blockEvents;
});

// Cycle pen color
penColorBtn.addEventListener('click', () => {
  penColorIndex = (penColorIndex + 1) % PEN_COLORS.length;
  penColor = PEN_COLORS[penColorIndex].hex;
  penColorBtn.style.background = penColor;
  penToggle.style.setProperty('--pen-color', penColor);
});

// Clear all strokes
penClear.addEventListener('click', () => {
  penStrokes = [];
  currentPoints = null;
  redrawStrokes();
});

// Prevent toolbar drag from eating pen button clicks
for (const el of [penToggle, penColorBtn, penClear]) {
  el.addEventListener('mousedown', (e) => { e.stopPropagation(); e.stopImmediatePropagation(); });
  el.addEventListener('pointerdown', (e) => { e.stopPropagation(); e.stopImmediatePropagation(); });
}

// Drawing event handlers on the canvas
drawCanvas.addEventListener('pointerdown', (e) => {
  if (!penActive || e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  drawCanvas.setPointerCapture(e.pointerId);
  currentPoints = [[e.offsetX, e.offsetY, e.pressure]];
});

drawCanvas.addEventListener('pointermove', (e) => {
  if (!currentPoints) return;
  currentPoints.push([e.offsetX, e.offsetY, e.pressure]);
  if (!drawRaf) {
    drawRaf = requestAnimationFrame(() => {
      drawRaf = 0;
      redrawStrokes();
    });
  }
});

drawCanvas.addEventListener('pointerup', (e) => {
  if (!currentPoints) return;
  drawCanvas.releasePointerCapture(e.pointerId);
  const outline = getStroke(currentPoints, PEN_OPTIONS);
  penStrokes.push({ outline, color: penColor });
  currentPoints = null;
  redrawStrokes();
});

// ResizeObserver fires after the initial layout is computed, avoiding the
// race where getBoundingClientRect returns stale/default dimensions.
new ResizeObserver(() => resizeDrawCanvas()).observe(drawCanvas);

// DOM elements
const captureBtn = document.getElementById('capture-btn');
const promptInput = document.getElementById('prompt-input');
const destEl = document.getElementById('capture-dest');
const statusEl = document.getElementById('status');
const resultsEl = document.getElementById('results');
const badgeEl = document.getElementById('result-badge');
const summaryEl = document.getElementById('result-summary');
const detailsEl = document.getElementById('result-details');
const metaEl = document.getElementById('result-meta');
const closeBtn = document.getElementById('close-results');

// Capture destinations — click to cycle
const DESTINATIONS = [
  { id: 'api',        label: 'API',        css: 'api' },
  { id: 'aurora-01',  label: 'aurora-01',  css: 'agent' },
  { id: 'surface-01', label: 'surface-01', css: 'agent' },
  { id: 'discord',    label: '#fleet',     css: 'discord' },
];
let destIndex = 0;

// Stop mousedown from bubbling to toolbar's startDrag handler
destEl.addEventListener('mousedown', (e) => {
  e.stopPropagation();
  e.stopImmediatePropagation();
});
destEl.addEventListener('pointerdown', (e) => {
  e.stopPropagation();
  e.stopImmediatePropagation();
});
destEl.addEventListener('click', () => {
  destIndex = (destIndex + 1) % DESTINATIONS.length;
  const dest = DESTINATIONS[destIndex];
  destEl.textContent = dest.label;
  destEl.className = dest.css;
});

// Build the prompt (user text + focus region context)
function buildPrompt() {
  const userPrompt = promptInput.value.trim();
  const focus = getFocusRegion();
  let prompt = userPrompt || 'Describe what you see on screen.';
  if (focus) {
    prompt += `\n\nIMPORTANT: A yellow "FOCUS" box highlights a region at approximately (${focus.x}, ${focus.y}) with size ${focus.width}x${focus.height} pixels. Pay special attention to the content inside this highlighted area and describe it in detail first, then describe the surrounding context.`;
  }
  return prompt;
}

// Capture handler — behavior depends on selected destination
async function doCapture() {
  const dest = DESTINATIONS[destIndex];
  statusEl.textContent = dest.id === 'api' ? 'Capturing...' : `Capturing → ${dest.label}...`;
  statusEl.className = 'working';
  captureBtn.disabled = true;

  try {
    // All destinations start with an API capture (Anthropic analyzes the image)
    const prompt = buildPrompt();
    const text = await invoke('capture_free', { prompt });

    if (dest.id === 'api') {
      // Show result locally
      renderFreeResult(text);
      statusEl.textContent = 'Done';
      statusEl.className = 'success';
    } else if (dest.id === 'discord') {
      // Post to Discord #fleet
      const msg = await invoke('share_discord', { channel: 'fleet' });
      renderFreeResult(text);
      statusEl.textContent = msg;
      statusEl.className = 'success';
    } else {
      // Send to coord agent
      const msg = await invoke('share_coord', { agentId: dest.id });
      renderFreeResult(text);
      statusEl.textContent = msg;
      statusEl.className = 'success';
    }
  } catch (err) {
    statusEl.textContent = 'Error: ' + String(err);
    statusEl.className = 'error';
    resultsEl.classList.add('hidden');
  } finally {
    captureBtn.disabled = false;
  }
}

// Render free-mode AI response
function renderFreeResult(text) {
  resultsEl.classList.remove('hidden');
  badgeEl.textContent = 'AI';
  badgeEl.className = 'badge-extract';
  summaryEl.textContent = 'Free observation';
  detailsEl.textContent = '';
  const pre = document.createElement('pre');
  pre.style.whiteSpace = 'pre-wrap';
  pre.style.wordBreak = 'break-word';
  pre.textContent = text;
  detailsEl.appendChild(pre);
  metaEl.textContent = '';
}

// Helper: create a text line element
function createLine(text, className) {
  const div = document.createElement('div');
  if (className) div.className = className;
  div.textContent = text;
  return div;
}

// Render comparison results using safe DOM methods (no innerHTML)
function renderResults(report) {
  resultsEl.classList.remove('hidden');

  // Badge
  const statusMap = {
    PASS: ['PASS', 'badge-pass'],
    WARN: ['WARN', 'badge-warn'],
    FAIL: ['FAIL', 'badge-fail'],
    EXTRACT_ONLY: ['EXTRACT', 'badge-extract'],
  };
  const [label, cls] = statusMap[report.overall] || ['?', ''];
  badgeEl.textContent = label;
  badgeEl.className = cls;

  // Summary
  if (report.overall === 'EXTRACT_ONLY') {
    summaryEl.textContent = 'Extracted ' + report.extracted_bids + ' bids, ' + report.extracted_asks + ' asks';
  } else {
    summaryEl.textContent =
      report.extracted_bids + '/' + report.truth_bids + ' bids, ' +
      report.extracted_asks + '/' + report.truth_asks + ' asks';
  }

  // Details — clear and rebuild with safe DOM methods
  detailsEl.textContent = '';

  if (report.mismatches.length > 0) {
    detailsEl.appendChild(createLine('Mismatches:', 'mismatch-line'));
    for (const m of report.mismatches) {
      detailsEl.appendChild(createLine(
        '  ' + m.side + ' $' + m.price.toFixed(2) + ': extracted=' + m.extracted_volume + ', truth=' + m.truth_volume,
        'mismatch-line'
      ));
    }
  }
  if (report.missing.length > 0) {
    detailsEl.appendChild(createLine('Missing (in truth, not on screen):', 'missing-line'));
    for (const m of report.missing) {
      detailsEl.appendChild(createLine(
        '  ' + m.side + ' $' + m.price.toFixed(2) + ' x ' + m.volume,
        'missing-line'
      ));
    }
  }
  if (report.extra.length > 0) {
    detailsEl.appendChild(createLine('Extra (on screen, not in truth):', 'extra-line'));
    for (const e of report.extra) {
      detailsEl.appendChild(createLine(
        '  ' + e.side + ' $' + e.price.toFixed(2) + ' x ' + (e.volume != null ? e.volume : '?'),
        'extra-line'
      ));
    }
  }
  if (report.mismatches.length === 0 && report.missing.length === 0 && report.extra.length === 0) {
    const msg = report.overall === 'EXTRACT_ONLY'
      ? 'Extract-only mode — no comparison performed'
      : 'All levels match';
    const em = document.createElement('em');
    em.textContent = msg;
    detailsEl.appendChild(em);
  }

  // Metadata
  metaEl.textContent =
    'Latency: ' + report.api_latency_ms + 'ms | ' +
    'Cost: $' + report.estimated_cost_usd.toFixed(4) + ' | ' +
    'Time: ' + new Date(report.timestamp).toLocaleTimeString();
}

// Event listeners
captureBtn.addEventListener('click', doCapture);
promptInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') doCapture();
});
closeBtn.addEventListener('click', () => resultsEl.classList.add('hidden'));

// Listen for global shortcut trigger
const { listen } = window.__TAURI__.event;
listen('trigger-capture', () => {
  doCapture();
});

// Window controls
document.getElementById('minimize-btn').addEventListener('click', () => invoke('plugin:window|minimize'));
document.getElementById('close-btn').addEventListener('click', () => invoke('plugin:window|close'));

// Window dragging via Rust command
const dragHandle = document.getElementById('drag-handle');
const toolbar = document.getElementById('toolbar');

function startDrag(e) {
  if (e.target.tagName === 'BUTTON' || e.target.tagName === 'INPUT' || e.target.tagName === 'SPAN') return;
  if (e.button !== 0) return;
  if (isNearEdge(e)) return;
  e.preventDefault();
  invoke('start_drag');
}

dragHandle.addEventListener('mousedown', startDrag);
toolbar.addEventListener('mousedown', startDrag);

// --- Vision overlay: polls bookmap_vision server and draws wall indicators ---
let visionOverlays = [];  // [{x, y, w, h, color, label, side, intensity}]
let visionActive = false;
let visionTimer = null;
// Frame persistence: track walls across frames, only render stable ones
let wallHistory = new Map(); // key -> { count, lastSeen, data }
const WALL_PERSIST_FRAMES = 3; // must appear in 3+ frames to render
const WALL_EXPIRE_FRAMES = 5; // disappear after 5 frames without seeing
let frameCounter = 0;

function renderVisionOverlays() {
  // Called after redrawStrokes — draws vision boxes on top of pen strokes
  const cw = drawCanvas.width;
  const ch = drawCanvas.height;
  if (!cw || !ch || !visionOverlays.length) return;

  drawCtx.save();
  for (const ov of visionOverlays) {
    const x = ov.x * cw;
    const y = ov.y * ch;
    const w = ov.w * cw;
    const h = ov.h * ch;
    const centerY = y + h / 2;
    const alpha = Math.min(1, (ov.age || 1) / 3); // fade in over 3 frames

    // Thin horizontal line at wall center (not a box)
    const intensity = ov.intensity || 0.5;
    const lineWidth = 1 + intensity * 3; // 1-4px based on wall strength
    drawCtx.strokeStyle = ov.color || '#00e5ff';
    drawCtx.globalAlpha = 0.4 + intensity * 0.5; // stronger walls more opaque
    drawCtx.lineWidth = lineWidth;
    drawCtx.beginPath();
    drawCtx.moveTo(x, centerY);
    drawCtx.lineTo(x + w, centerY);
    drawCtx.stroke();

    // Small intensity tick on the edge (bid=left, ask=right)
    const tickX = ov.side === 'ask' ? x + w : x;
    const tickW = 4 + intensity * 12; // 4-16px tick proportional to strength
    const tickDir = ov.side === 'ask' ? -1 : 1;
    drawCtx.fillStyle = ov.color || '#00e5ff';
    drawCtx.globalAlpha = 0.6 + intensity * 0.4;
    drawCtx.fillRect(tickX, centerY - lineWidth, tickW * tickDir, lineWidth * 2);

    // Compact label
    if (ov.label) {
      drawCtx.globalAlpha = 0.8;
      drawCtx.font = 'bold 10px monospace';
      const lx = ov.side === 'ask' ? x + w - 40 : x + 4;
      drawCtx.fillStyle = 'rgba(0,0,0,0.6)';
      const tm = drawCtx.measureText(ov.label);
      drawCtx.fillRect(lx - 1, centerY - 10, tm.width + 4, 12);
      drawCtx.fillStyle = ov.color || '#00e5ff';
      drawCtx.fillText(ov.label, lx + 1, centerY - 1);
    }

    drawCtx.globalAlpha = 1;
  }

  // Bias bar at top of frame
  const bias = visionOverlays._bias;
  if (bias) {
    const barW = 200;
    const barH = 8;
    const barX = (cw - barW) / 2;
    const barY = 4;
    const mid = barX + barW / 2;
    const bidW = bias.bid * barW / 2;
    const askW = bias.ask * barW / 2;

    drawCtx.fillStyle = '#00aaff';
    drawCtx.fillRect(mid - bidW, barY, bidW, barH);
    drawCtx.fillStyle = '#ff4400';
    drawCtx.fillRect(mid, barY, askW, barH);
    drawCtx.strokeStyle = '#444';
    drawCtx.lineWidth = 1;
    drawCtx.strokeRect(barX, barY, barW, barH);

    // Center tick
    drawCtx.fillStyle = '#fff';
    drawCtx.fillRect(mid - 1, barY - 2, 2, barH + 4);
  }
  drawCtx.restore();
}

// Vision overlays are now rendered as part of redrawStrokes() directly.
// No patching needed — fetchVisionData() calls redrawStrokes() after updating overlay data.

// Check if a fractional coordinate falls inside any buffer zone
function isInBuffer(xFrac, yFrac) {
  for (const buf of gridBuffers.v) {
    if (xFrac >= buf[0] && xFrac <= buf[1]) return true;
  }
  for (const buf of gridBuffers.h) {
    if (yFrac >= buf[0] && yFrac <= buf[1]) return true;
  }
  return false;
}

// Check if an overlay box overlaps any buffer zone
function overlapsBuffer(x, y, w, h) {
  // Check center point and all four corners
  const cx = x + w / 2, cy = y + h / 2;
  return isInBuffer(cx, cy) ||
         isInBuffer(x, y) || isInBuffer(x + w, y) ||
         isInBuffer(x, y + h) || isInBuffer(x + w, y + h);
}

async function fetchVisionData() {
  try {
    const data = await invoke('fetch_vision');
    if (!data || !data.ts) return;

    const overlays = [];
    const frameW = data.frame_size?.[0] || 1;
    const frameH = data.frame_size?.[1] || 1;

    // Per-panel wall rendering — positions walls within each detected panel
    const panels = data.panels || [];

    // Fallback: if only 1 panel (full frame), use global walls directly
    if (panels.length <= 1 && (data.bid_walls?.length || data.ask_walls?.length)) {
      for (const wall of (data.bid_walls || []).slice(0, 3)) {
        const rowH = (wall.rows || 10) / frameH;
        overlays.push({
          x: 0, y: wall.y_pct - rowH / 2, w: 0.45, h: rowH,
          color: '#00ccff', side: 'bid', intensity: wall.intensity,
          label: `${(wall.intensity * 100).toFixed(0)}`,
        });
      }
      for (const wall of (data.ask_walls || []).slice(0, 3)) {
        const rowH = (wall.rows || 10) / frameH;
        overlays.push({
          x: 0.55, y: wall.y_pct - rowH / 2, w: 0.45, h: rowH,
          color: '#ff4400', side: 'ask', intensity: wall.intensity,
          label: `${(wall.intensity * 100).toFixed(0)}`,
        });
      }
    }

    for (const panel of panels) {
      const p = panel.panel;
      if (!p || p.h < 200) continue; // skip header/status panels

      // Panel bounds as fractions of the full frame
      const px = p.x_pct, py = p.y_pct, pw = p.w_pct, ph = p.h_pct;

      // Bid walls — top 3 strongest, thin cyan lines
      const bidWalls = (panel.bid_walls || []).slice(0, 3);
      for (const wall of bidWalls) {
        const rowH = (wall.rows || 10) / p.h;
        const oy = py + (wall.y_pct - rowH / 2) * ph;
        const oh = rowH * ph;
        if (overlapsBuffer(px, oy, pw, oh)) continue;
        overlays.push({
          x: px, y: oy, w: pw * 0.45, h: oh,
          color: '#00ccff', side: 'bid',
          intensity: wall.intensity,
          label: `${(wall.intensity * 100).toFixed(0)}`,
        });
      }

      // Ask walls — top 3 strongest, thin red lines
      const askWalls = (panel.ask_walls || []).slice(0, 3);
      for (const wall of askWalls) {
        const rowH = (wall.rows || 10) / p.h;
        const oy = py + (wall.y_pct - rowH / 2) * ph;
        const oh = rowH * ph;
        if (overlapsBuffer(px + pw * 0.55, oy, pw * 0.45, oh)) continue;
        overlays.push({
          x: px + pw * 0.55, y: oy, w: pw * 0.45, h: oh,
          color: '#ff4400', side: 'ask',
          intensity: wall.intensity,
          label: `${(wall.intensity * 100).toFixed(0)}`,
        });
      }
    }

    // TODO: frame persistence disabled for debugging — render all walls directly
    overlays._bias = {
      bid: data.bid_intensity || 0,
      ask: data.ask_intensity || 0,
    };

    visionOverlays = overlays;
    console.log(`[vision] ${overlays.length} overlays from ${(data.panels || []).length} panels`);
    redrawStrokes();
  } catch {
    // Vision server not available — clear overlays silently
    if (visionOverlays.length) {
      visionOverlays = [];
      redrawStrokes();
    }
  }
}

function startVision() {
  if (visionActive) return;
  visionActive = true;
  visionTimer = setInterval(fetchVisionData, 1000);
  fetchVisionData();
  console.log('[vision] overlay polling started');
}

function stopVision() {
  visionActive = false;
  if (visionTimer) { clearInterval(visionTimer); visionTimer = null; }
  visionOverlays = [];
  redrawStrokes();
  console.log('[vision] overlay polling stopped');
}

// Auto-discover vision server via Tauri IPC (retry every 10s for 5 attempts)
(async function() {
  for (let attempt = 0; attempt < 5; attempt++) {
    if (attempt > 0) await new Promise(r => setTimeout(r, 10000));
    try {
      console.log(`[vision] attempt ${attempt + 1}: trying IPC fetch_vision`);
      const data = await invoke('fetch_vision');
      if (data) {
        console.log('[vision] IPC connected, starting overlay polling');
        startVision();
        return;
      }
    } catch (err) {
      console.log(`[vision] IPC failed: ${err}`);
    }
  }
  console.log('[vision] vision server not available after 5 attempts, overlay disabled');
})();

// --- Panel grid editor: draggable buffer zone lines ---
// Each buffer is a PAIR of lines defining a dead zone where no overlays draw.
// Yellow lines for visibility against dark heatmap backgrounds.
const gridToggle = document.getElementById('grid-toggle');
const GRID_HIT_ZONE = 10; // px from line to register a grab
const GRID_LINE_COLOR = 'rgba(255, 200, 0, 0.7)';
const GRID_LINE_HOVER = 'rgba(255, 255, 0, 1.0)';
const GRID_FILL_COLOR = 'rgba(255, 200, 0, 0.08)'; // subtle fill between buffer lines

// Buffer state: each buffer is [lineA, lineB] as fractions (0-1)
// v: 1 vertical buffer (left line, right line)
// h: 3 horizontal buffers (top line, bottom line) × 3
let gridBuffers = loadGridBuffers();
let gridDrag = null; // { bufferIdx, lineIdx (0 or 1), orientation }

function defaultBuffers() {
  return {
    v: [[0.43, 0.50]],                                          // 1 vertical center buffer
    h: [[0.00, 0.03], [0.18, 0.20], [0.43, 0.45], [0.65, 0.67], [0.95, 1.00]], // top + 3 row dividers + bottom
  };
}

async function loadGridConfigFromFile() {
  try {
    const resp = await fetch('http://localhost:9050/api/grid');
    if (resp.ok) {
      const config = await resp.json();
      // Restore window position/size
      if (config.window) {
        const w = config.window;
        console.log(`[grid] restoring window: (${w.x},${w.y}) ${w.w}x${w.h}`);
        await appWindow.setPosition(new PhysicalPosition(w.x, w.y));
        await appWindow.setSize(new PhysicalSize(w.w, w.h));
      }
      // Restore buffers
      if (config.buffers && config.buffers.v && config.buffers.h) {
        console.log('[grid] restored buffer config from file');
        return config.buffers;
      }
    }
  } catch {}
  return null;
}

function loadGridBuffers() {
  try {
    const saved = localStorage.getItem('condor-eye-grid-v2');
    if (saved) return JSON.parse(saved);
  } catch {}
  // Try loading from file asynchronously after init
  loadGridConfigFromFile().then(data => {
    if (data) {
      gridBuffers = data;
      localStorage.setItem('condor-eye-grid-v2', JSON.stringify(data));
      redrawStrokes();
    }
  });
  return defaultBuffers();
}

async function saveGridBuffers() {
  localStorage.setItem('condor-eye-grid-v2', JSON.stringify(gridBuffers));
  // Persist buffers + window geometry to file so it survives cache clears
  try {
    const pos = await appWindow.outerPosition();
    const size = await appWindow.outerSize();
    const config = {
      buffers: gridBuffers,
      window: { x: pos.x, y: pos.y, w: size.width, h: size.height },
    };
    fetch('http://localhost:9050/api/grid', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(config),
    }).then(r => {
      if (r.ok) console.log('[grid] config saved to file');
      else console.error('[grid] save failed:', r.status);
    }).catch(err => console.error('[grid] save error:', err));
  } catch {}
}

function bufferToPixel(frac, orientation) {
  const cw = drawCanvas.width, ch = drawCanvas.height;
  return orientation === 'v' ? frac * cw : frac * ch;
}

function pixelToBuffer(px, orientation) {
  const cw = drawCanvas.width, ch = drawCanvas.height;
  return orientation === 'v' ? px / cw : px / ch;
}

// Hit-test: which buffer line is under the pointer?
function hitTestBuffer(x, y) {
  for (let bi = 0; bi < gridBuffers.v.length; bi++) {
    for (let li = 0; li < 2; li++) {
      const px = bufferToPixel(gridBuffers.v[bi][li], 'v');
      if (Math.abs(x - px) < GRID_HIT_ZONE) return { bufferIdx: bi, lineIdx: li, orientation: 'v' };
    }
  }
  for (let bi = 0; bi < gridBuffers.h.length; bi++) {
    for (let li = 0; li < 2; li++) {
      const px = bufferToPixel(gridBuffers.h[bi][li], 'h');
      if (Math.abs(y - px) < GRID_HIT_ZONE) return { bufferIdx: bi, lineIdx: li, orientation: 'h' };
    }
  }
  return null;
}

// Draw buffer zone lines and fills
function renderGridDividers(hoverHit) {
  const cw = drawCanvas.width, ch = drawCanvas.height;
  if (!cw || !ch) return;

  drawCtx.save();

  // Vertical buffers
  for (let bi = 0; bi < gridBuffers.v.length; bi++) {
    const px0 = bufferToPixel(gridBuffers.v[bi][0], 'v');
    const px1 = bufferToPixel(gridBuffers.v[bi][1], 'v');
    // Fill between lines
    drawCtx.fillStyle = GRID_FILL_COLOR;
    drawCtx.fillRect(px0, 0, px1 - px0, ch);
    // Draw both lines
    for (let li = 0; li < 2; li++) {
      const px = li === 0 ? px0 : px1;
      const isHover = hoverHit && hoverHit.orientation === 'v' && hoverHit.bufferIdx === bi && hoverHit.lineIdx === li;
      drawCtx.strokeStyle = isHover ? GRID_LINE_HOVER : GRID_LINE_COLOR;
      drawCtx.lineWidth = isHover ? 3 : 1.5;
      drawCtx.setLineDash([8, 4]);
      drawCtx.beginPath();
      drawCtx.moveTo(px, 0);
      drawCtx.lineTo(px, ch);
      drawCtx.stroke();
    }
  }

  // Horizontal buffers
  for (let bi = 0; bi < gridBuffers.h.length; bi++) {
    const px0 = bufferToPixel(gridBuffers.h[bi][0], 'h');
    const px1 = bufferToPixel(gridBuffers.h[bi][1], 'h');
    // Fill between lines
    drawCtx.fillStyle = GRID_FILL_COLOR;
    drawCtx.fillRect(0, px0, cw, px1 - px0);
    // Draw both lines
    for (let li = 0; li < 2; li++) {
      const px = li === 0 ? px0 : px1;
      const isHover = hoverHit && hoverHit.orientation === 'h' && hoverHit.bufferIdx === bi && hoverHit.lineIdx === li;
      drawCtx.strokeStyle = isHover ? GRID_LINE_HOVER : GRID_LINE_COLOR;
      drawCtx.lineWidth = isHover ? 3 : 1.5;
      drawCtx.setLineDash([8, 4]);
      drawCtx.beginPath();
      drawCtx.moveTo(0, px);
      drawCtx.lineTo(cw, px);
      drawCtx.stroke();
    }
  }

  drawCtx.setLineDash([]);
  drawCtx.restore();
}

// Toggle grid editor
gridToggle.addEventListener('click', () => {
  gridActive = !gridActive;
  gridToggle.classList.toggle('active', gridActive);
  drawCanvas.classList.toggle('active', gridActive);
  drawCanvas.style.zIndex = gridActive ? '12' : '';
  // Disable pointer events on overlapping elements when grid is active
  const blockEvents = gridActive ? 'none' : '';
  document.getElementById('results').style.pointerEvents = blockEvents;
  focusBox.style.pointerEvents = blockEvents;
  // Disable pen if grid is active
  if (gridActive && penActive) {
    penActive = false;
    penToggle.classList.remove('active');
  }
  redrawStrokes();
});

// Stop propagation on grid button clicks
for (const el of [gridToggle]) {
  el.addEventListener('mousedown', (e) => { e.stopPropagation(); e.stopImmediatePropagation(); });
  el.addEventListener('pointerdown', (e) => { e.stopPropagation(); e.stopImmediatePropagation(); });
}

// Grid pointer events — intercept canvas events when grid is active
drawCanvas.addEventListener('pointerdown', (e) => {
  if (!gridActive || e.button !== 0) return;
  const hit = hitTestBuffer(e.offsetX, e.offsetY);
  if (!hit) return;
  e.preventDefault();
  e.stopPropagation();
  drawCanvas.setPointerCapture(e.pointerId);
  gridDrag = hit;
}, true); // capture phase to beat pen handler

drawCanvas.addEventListener('pointermove', (e) => {
  if (!gridActive) return;

  if (gridDrag) {
    const frac = pixelToBuffer(
      gridDrag.orientation === 'v' ? e.offsetX : e.offsetY,
      gridDrag.orientation
    );
    const clamped = Math.max(0.02, Math.min(0.98, frac));
    gridBuffers[gridDrag.orientation][gridDrag.bufferIdx][gridDrag.lineIdx] = clamped;
    redrawStrokes();
    return;
  }

  // Hover: update cursor
  const hit = hitTestBuffer(e.offsetX, e.offsetY);
  if (hit) {
    drawCanvas.style.cursor = hit.orientation === 'v' ? 'col-resize' : 'row-resize';
  } else {
    drawCanvas.style.cursor = 'default';
  }
  redrawStrokes();
});

drawCanvas.addEventListener('pointerup', (e) => {
  if (!gridDrag) return;
  drawCanvas.releasePointerCapture(e.pointerId);
  // Ensure line A < line B within each buffer
  for (const buf of gridBuffers.h) { if (buf[0] > buf[1]) buf.reverse(); }
  for (const buf of gridBuffers.v) { if (buf[0] > buf[1]) buf.reverse(); }
  // Sort buffers by position
  gridBuffers.h.sort((a, b) => a[0] - b[0]);
  gridBuffers.v.sort((a, b) => a[0] - b[0]);
  saveGridBuffers();
  gridDrag = null;
  redrawStrokes();
  console.log('[grid] buffers saved:', JSON.stringify(gridBuffers));
});

// Track mouse for grid hover highlights
drawCanvas.addEventListener('mousemove', (e) => {
  _lastMouseX = e.offsetX;
  _lastMouseY = e.offsetY;
});
