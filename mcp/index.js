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
import { readFileSync } from "fs";
function getDefaultHost() {
  try {
    const route = execFileSync("ip", ["route", "show", "default"], { encoding: "utf-8" });
    const match = route.match(/via\s+([\d.]+)/);
    if (match) return `${match[1]}:9050`;
  } catch {}
  return "localhost:9050";
}
const DEFAULT_HOST = getDefaultHost();
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
];

async function callApi(host, method, path, body) {
  const url = `http://${host}${path}`;
  const options = {
    method,
    headers: { "Content-Type": "application/json" },
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
  const result = await callApi(host, "POST", "/api/capture", body);
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
      default: return { content: [{ type: "text", text: `Unknown tool: ${name}` }], isError: true };
    }
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  } catch (error) {
    return { content: [{ type: "text", text: `Condor Eye error: ${error.message}` }], isError: true };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
