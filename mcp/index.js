#!/usr/bin/env node

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";

// Detect WSL and use the Windows host gateway IP instead of localhost.
// WSL2's localhost doesn't route to Windows — need the Hyper-V gateway.
import { execFileSync } from "child_process";
function getDefaultHost() {
  // Explicit override — set this on non-Aurora machines (e.g., Surface)
  // to reach the Tauri app via Tailscale: CONDOR_EYE_HOST=100.70.34.55:9050
  if (process.env.CONDOR_EYE_HOST) return process.env.CONDOR_EYE_HOST;
  try {
    const route = execFileSync("ip", ["route", "show", "default"], { encoding: "utf-8" });
    const match = route.match(/via\s+([\d.]+)/);
    if (match) return `${match[1]}:9050`;
  } catch {}
  return "localhost:9050";
}
const DEFAULT_HOST = getDefaultHost();
function getDefaultAudioHost() {
  if (process.env.CONDOR_AUDIO_HOST) return process.env.CONDOR_AUDIO_HOST;
  if (process.env.CONDOR_EYE_AUDIO_HOST) return process.env.CONDOR_EYE_AUDIO_HOST;
  return "localhost:9051";
}
const DEFAULT_AUDIO_HOST = getDefaultAudioHost();
const HTTP_TIMEOUT_MS = 60_000;

const TOOLS = [
  {
    name: "condor_eye_capture",
    description:
      "Capture a screen region and get an AI description of what's visible. " +
      "If no region is specified, captures the full screen. " +
      "Returns a text description of the screen content.",
    inputSchema: {
      type: "object",
      properties: {
        prompt: {
          type: "string",
          description: "What to look for or describe. Default: general description of visible content.",
        },
        region: {
          type: "object",
          description: "Screen region to capture in pixels. Omit for full screen.",
          properties: {
            x: { type: "integer" }, y: { type: "integer" },
            width: { type: "integer" }, height: { type: "integer" },
          },
          required: ["x", "y", "width", "height"],
        },
        hwnd: {
          type: "integer",
          description: "Window handle from condor_eye_windows. If set, brings window to foreground before capture (unless no_focus is true).",
        },
        no_focus: {
          type: "boolean",
          description: "If true, capture the window region without stealing focus. Use with hwnd + region for polling captures that don't interrupt the user.",
        },
        keys: {
          type: "array",
          items: { type: "string" },
          description: "Key combos to send after focus, before capture. E.g. [\"ctrl+3\"] to switch to browser tab 3. Works with Firefox, Chrome, Edge (Ctrl+1-9 for tabs).",
        },
        host: { type: "string", description: `Condor Eye app host. Default: ${DEFAULT_HOST}` },
        include_image: {
          type: "boolean",
          description: "Include base64 image in response. Default: false (saves context space).",
        },
      },
    },
  },
  {
    name: "condor_eye_windows",
    description:
      "List visible windows with their screen positions and PIDs. Use this to find " +
      "a window's exact bounds before capturing it — avoids expensive AI-based locate.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "Search string to filter by window title (case-insensitive). Omit to list all windows.",
        },
        host: { type: "string", description: `Condor Eye app host. Default: ${DEFAULT_HOST}` },
      },
    },
  },
  {
    name: "condor_eye_locate",
    description:
      "Find a UI element on screen using AI vision. Captures the full screen and uses AI " +
      "to identify the target and return its bounding box. Prefer condor_eye_windows for " +
      "finding windows by title (free and instant) — use this tool for finding specific UI " +
      "elements within a window.",
    inputSchema: {
      type: "object",
      properties: {
        target: {
          type: "string",
          description: 'Description of what to find. E.g., "Chrome browser", "the terminal window", "VS Code editor".',
        },
        host: { type: "string", description: `Condor Eye app host. Default: ${DEFAULT_HOST}` },
      },
      required: ["target"],
    },
  },
  {
    name: "condor_eye_status",
    description: "Check if the Condor Eye app is running and get its current configuration.",
    inputSchema: {
      type: "object",
      properties: {
        host: { type: "string", description: `Condor Eye app host. Default: ${DEFAULT_HOST}` },
      },
    },
  },
  {
    name: "condor_audio_status",
    description: "Check the Condor Audio API status, configured targets, and active taps.",
    inputSchema: {
      type: "object",
      properties: {
        host: { type: "string", description: `Condor Audio host. Default: ${DEFAULT_AUDIO_HOST}` },
      },
    },
  },
  {
    name: "condor_audio_start",
    description: "Start a Condor Audio tap for a target app.",
    inputSchema: {
      type: "object",
      properties: {
        app: {
          type: "string",
          enum: ["zoom", "discord"],
          description: "Target app to tap.",
        },
        pid: {
          type: "integer",
          description: "Optional explicit PID override.",
        },
        host: { type: "string", description: `Condor Audio host. Default: ${DEFAULT_AUDIO_HOST}` },
      },
      required: ["app"],
    },
  },
  {
    name: "condor_audio_stop",
    description: "Stop an active audio tap by tap id.",
    inputSchema: {
      type: "object",
      properties: {
        tap_id: {
          type: "string",
          description: "Tap id returned by condor_audio_start or condor_audio_status.",
        },
        host: { type: "string", description: `Condor Audio host. Default: ${DEFAULT_AUDIO_HOST}` },
      },
      required: ["tap_id"],
    },
  },
  {
    name: "condor_audio_latest",
    description: "Fetch the latest transcript text for an active tap.",
    inputSchema: {
      type: "object",
      properties: {
        tap_id: {
          type: "string",
          description: "Tap id returned by condor_audio_start or condor_audio_status.",
        },
        host: { type: "string", description: `Condor Audio host. Default: ${DEFAULT_AUDIO_HOST}` },
      },
      required: ["tap_id"],
    },
  },
];

async function isReachable(host) {
  try {
    const resp = await fetch(`http://${host}/api/status`, {
      signal: AbortSignal.timeout(3_000),
    });
    return resp.ok;
  } catch { return false; }
}

async function isAudioReachable(host) {
  try {
    const token = await getCaptureToken();
    const resp = await fetch(`http://${host}/api/condor_audio/status`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(3_000),
    });
    return resp.ok;
  } catch { return false; }
}

async function tryLaunch() {
  try {
    const { execFile } = await import("child_process");
    const { promisify } = await import("util");
    const execFileAsync = promisify(execFile);
    await execFileAsync("powershell.exe", [
      "-Command",
      `Start-Process 'C:\\Users\\mikem\\AppData\\Local\\Condor Eye\\condor-eye.exe'`,
    ]);
    // Wait up to 8s for the app to start
    for (let i = 0; i < 16; i++) {
      await new Promise(r => setTimeout(r, 500));
      if (await isReachable(DEFAULT_HOST)) return true;
    }
  } catch {}
  return false;
}

async function ensureRunning(host) {
  if (await isReachable(host)) return;
  // Try auto-launch
  if (host === DEFAULT_HOST && await tryLaunch()) return;
  throw new Error(
    `Condor Eye is not running (${host} unreachable). ` +
    `Auto-launch failed. Start it manually from Windows Start menu.`
  );
}

// Resolve capture token from 1Password (cached per session)
let _captureToken = null;
async function getCaptureToken() {
  if (_captureToken) return _captureToken;
  try {
    const { execFileSync } = await import("child_process");
    _captureToken = execFileSync("op.exe", ["read", "op://Dev/condor-eye-capture/token"], { encoding: "utf-8" }).trim();
    return _captureToken;
  } catch {
    throw new Error("Failed to read capture token from 1Password. Run: op.exe signin");
  }
}

async function callApi(host, method, path, body, authRequired = false, skipEnsure = false) {
  if (!skipEnsure) {
    await ensureRunning(host);
  }
  const url = `http://${host}${path}`;
  const headers = { "Content-Type": "application/json" };
  if (authRequired) {
    const token = await getCaptureToken();
    headers["Authorization"] = `Bearer ${token}`;
  }
  const options = {
    method,
    headers,
    signal: AbortSignal.timeout(HTTP_TIMEOUT_MS),
  };
  if (body) options.body = JSON.stringify(body);
  const resp = await fetch(url, options);
  const data = await resp.json();
  if (!resp.ok) throw new Error(data.error || `HTTP ${resp.status}`);
  return data;
}

async function handleCapture(args) {
  const host = args.host || DEFAULT_HOST;
  const includeImage = args.include_image || false;
  const body = {};
  if (args.prompt) body.prompt = args.prompt;
  if (args.region) body.region = args.region;
  if (args.hwnd) body.hwnd = args.hwnd;
  if (args.no_focus) body.no_focus = true;
  if (args.keys) body.keys = args.keys;
  const result = await callApi(host, "POST", "/api/capture", body, true);
  const response = {
    description: result.description,
    latency_ms: result.latency_ms,
    region: result.region,
    cost_estimate_usd: result.cost_estimate_usd,
  };
  if (includeImage) response.image = result.image;
  return response;
}

async function handleLocate(args) {
  const host = args.host || DEFAULT_HOST;
  return callApi(host, "POST", "/api/locate", { target: args.target });
}

async function handleWindows(args) {
  const host = args.host || DEFAULT_HOST;
  await ensureRunning(host);
  const base = `http://${host}`;
  const path = args.query
    ? `/api/windows?query=${encodeURIComponent(args.query)}`
    : "/api/windows";
  const resp = await fetch(`${base}${path}`, {
    signal: AbortSignal.timeout(10_000),
  });
  const data = await resp.json();
  if (!resp.ok) throw new Error(data.error || `HTTP ${resp.status}`);
  return data;
}

async function handleStatus(args) {
  const host = args.host || DEFAULT_HOST;
  return callApi(host, "GET", "/api/status");
}

async function handleAudioStatus(args) {
  const host = args.host || DEFAULT_AUDIO_HOST;
  await ensureAudioRunning(host);
  return callApi(host, "GET", "/api/condor_audio/status", null, true, true);
}

async function handleAudioStart(args) {
  const host = args.host || DEFAULT_AUDIO_HOST;
  await ensureAudioRunning(host);
  return callApi(host, "POST", "/api/condor_audio/taps", {
    app: args.app,
    ...(args.pid ? { pid: args.pid } : {}),
  }, true, true);
}

async function handleAudioStop(args) {
  const host = args.host || DEFAULT_AUDIO_HOST;
  await ensureAudioRunning(host);
  return callApi(host, "DELETE", `/api/condor_audio/taps/${encodeURIComponent(args.tap_id)}`, null, true, true);
}

async function handleAudioLatest(args) {
  const host = args.host || DEFAULT_AUDIO_HOST;
  await ensureAudioRunning(host);
  return callApi(
    host,
    "GET",
    `/api/condor_audio/taps/${encodeURIComponent(args.tap_id)}/latest-transcript`,
    null,
    true,
    true
  );
}

async function ensureAudioRunning(host) {
  if (await isAudioReachable(host)) return;
  if (host === DEFAULT_AUDIO_HOST) {
    await ensureRunning(DEFAULT_HOST);
    for (let i = 0; i < 10; i++) {
      await new Promise(r => setTimeout(r, 400));
      if (await isAudioReachable(host)) return;
    }
  }
  throw new Error(
    `Condor Audio API is not running (${host} unreachable). ` +
    `Start the desktop app and ensure the localhost-only audio listener is enabled.`
  );
}

const server = new Server(
  { name: "condor-eye", version: "0.1.0" },
  { capabilities: { tools: {} } }
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools: TOOLS }));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;
  try {
    let result;
    switch (name) {
      case "condor_eye_capture": result = await handleCapture(args || {}); break;
      case "condor_eye_windows": result = await handleWindows(args || {}); break;
      case "condor_eye_locate": result = await handleLocate(args || {}); break;
      case "condor_eye_status": result = await handleStatus(args || {}); break;
      case "condor_audio_status": result = await handleAudioStatus(args || {}); break;
      case "condor_audio_start": result = await handleAudioStart(args || {}); break;
      case "condor_audio_stop": result = await handleAudioStop(args || {}); break;
      case "condor_audio_latest": result = await handleAudioLatest(args || {}); break;
      default: return { content: [{ type: "text", text: `Unknown tool: ${name}` }], isError: true };
    }
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  } catch (error) {
    return { content: [{ type: "text", text: `Condor Eye error: ${error.message}` }], isError: true };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
