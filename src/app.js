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

// DOM elements
const captureBtn = document.getElementById('capture-btn');
const symbolSelect = document.getElementById('symbol-select');
const modeSelect = document.getElementById('mode-select');
const profileSelect = document.getElementById('profile-select');
const statusEl = document.getElementById('status');
const resultsEl = document.getElementById('results');
const badgeEl = document.getElementById('result-badge');
const summaryEl = document.getElementById('result-summary');
const detailsEl = document.getElementById('result-details');
const metaEl = document.getElementById('result-meta');
const closeBtn = document.getElementById('close-results');

// Load available profiles
async function loadProfiles() {
  try {
    const profiles = await invoke('list_profiles');
    profileSelect.textContent = '';
    for (const name of profiles) {
      const opt = document.createElement('option');
      opt.value = name;
      opt.textContent = name;
      profileSelect.appendChild(opt);
    }
  } catch (e) {
    console.error('Failed to load profiles:', e);
  }
}

// Capture handler
async function doCapture() {
  const symbol = symbolSelect.value;
  const mode = modeSelect.value;
  const profileName = profileSelect.value;

  statusEl.textContent = 'Capturing...';
  statusEl.className = 'working';
  captureBtn.disabled = true;

  try {
    if (mode === 'free') {
      const text = await invoke('capture_free', { prompt: '' });
      renderFreeResult(text);
      statusEl.textContent = 'Done';
      statusEl.className = 'success';
    } else {
      const report = await invoke('capture_and_compare', {
        symbol,
        mode,
        profileName,
      });
      renderResults(report);
      statusEl.textContent = 'Done — ' + report.api_latency_ms + 'ms';
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
  if (e.target.tagName === 'BUTTON' || e.target.tagName === 'SELECT' || e.target.tagName === 'INPUT') return;
  if (e.button !== 0) return;
  // Safety net: don't start window drag when cursor is near an edge
  // (the capture-phase resize handler should have already caught this,
  // but bail out here in case it didn't)
  if (isNearEdge(e)) return;
  e.preventDefault();
  invoke('start_drag');
}

dragHandle.addEventListener('mousedown', startDrag);
toolbar.addEventListener('mousedown', startDrag);

// Initialize
loadProfiles();
