# Condor Eye

Screen capture + AI vision tool for Claude agents. Tauri 2 app (Rust backend, WebView2 frontend) that captures screen regions, analyzes them via Anthropic API, and optionally compares against Redis ground truth data.

## Quick Start

### Prerequisites

- Rust toolchain on Windows (not WSL)
- `ANTHROPIC_API_KEY` — stored in 1Password as `Anthropic_API_Key` (field: `credential`)

### Launch (from WSL)

```bash
# One-liner: fetch key from 1Password, write a launcher bat, run it
python3 -c "
import subprocess
key = subprocess.check_output(['op.exe', 'item', 'get', 'Anthropic_API_Key', '--field', 'credential', '--reveal'], text=True).strip()
exe = r'\\\\wsl.localhost\\Ubuntu\\home\\mikem\\code\\dev-tools\\condor-eye\\src-tauri\\target\\debug\\condor-eye.exe'
with open('/mnt/c/Users/mikem/launch_ce.bat', 'w', newline='\r\n') as f:
    f.write('@echo off\r\n')
    f.write(f'set \"ANTHROPIC_API_KEY={key}\"\r\n')
    f.write(f'start \"\" \"{exe}\"\r\n')
"
powershell.exe -Command "& 'C:\Users\mikem\launch_ce.bat'"
```

### Verify

```bash
# From WSL — check the HTTP API is responding
curl http://172.23.128.1:9050/status
# Should return: {"running":true,"version":"0.1.0","api_key_configured":true,...}
```

### Why the bat file?

Windows environment variables set from WSL (`$env:VAR` in PowerShell, `set VAR` in cmd) don't reliably propagate through `Start-Process`. The bat file approach bakes the key into a script that runs in the same cmd session as the exe.

## Development

```bash
cd ~/code/dev-tools/condor-eye

# Dev mode (requires Windows Rust toolchain)
cargo.exe tauri dev

# Build release
cargo.exe tauri build

# Tests (no display needed)
cd src-tauri && cargo.exe test
```

## HTTP API (port 9050)

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/status` | GET | Health check + config |
| `/api/capture` | POST | Screenshot + AI analysis |
| `/api/locate` | POST | Find UI element on screen |

## MCP Integration

Registered as a global Claude Code MCP server. Tools:
- `condor_eye_capture` — capture + describe screen content
- `condor_eye_locate` — find a window/element on screen
- `condor_eye_windows` — list visible windows (free, no API call)
- `condor_eye_status` — health check

## Environment Variables

| Variable | Required | Default |
|----------|----------|---------|
| `ANTHROPIC_API_KEY` | Yes | (none) |
| `REDIS_URL` | No | `redis://127.0.0.1:6379` |
| `CLAUDE_MODEL` | No | `claude-haiku-4-5-20251001` |
| `CONDOR_EYE_BIND` | No | `0.0.0.0` |
| `CONDOR_EYE_PORT` | No | `9050` |

## Paths

- Source: `~/code/dev-tools/condor-eye/`
- Debug exe: `src-tauri/target/debug/condor-eye.exe`
- Profiles: `profiles/*.json`
- Frontend: `src/` (vanilla HTML/CSS/JS)
