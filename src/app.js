const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;
const { PhysicalSize, PhysicalPosition } = window.__TAURI__.dpi;

const appWindow = getCurrentWindow();

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
