Take a screenshot using Condor Eye. Target: $ARGUMENTS

## Modes

| Invocation | Behavior |
|------------|----------|
| `/screen` (no args) | Capture the Condor Eye frame region (what's visible through the transparent center) |
| `/screen full` | Full screen capture (no region constraint) |
| `/screen <app name>` | Find that window by title, capture its bounds |
| `/screen <app> tab 3` | Find window, switch to tab 3, then capture |

## Instructions

### Default mode (no arguments or empty target)

Capture just the Condor Eye frame region. Call the HTTP API with no `region` param and no `hwnd`. The app hides itself, captures its own footprint, and returns what was underneath the frame.

```bash
curl -s -X POST http://localhost:9050/api/capture \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Describe what you see in detail.", "include_image": true}'
```

Save the base64 image to a temp PNG, display it with Read, and present the AI description.

### "full" mode

Same as default but pass the full screen dimensions:

```bash
curl -s -X POST http://localhost:9050/api/capture \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Describe what you see on screen.", "include_image": true, "region": {"x": 0, "y": 0, "width": 3840, "height": 2160}}'
```

Adjust the width/height to match your display resolution.

### Window-targeted mode (app name given)

1. **Find the window** — Call `condor_eye_windows` with the app name as query.

2. **Select the best match** — Pick the most relevant window. If no match, show available windows and ask.

3. **Capture with focus + tab switch**:
   ```bash
   curl -s -X POST http://localhost:9050/api/capture \
     -H "Content-Type: application/json" \
     -d '{"hwnd": <HWND>, "keys": [<KEY_COMBOS>], "prompt": "<PROMPT>", "region": {"x": <X>, "y": <Y>, "width": <W>, "height": <H>}, "include_image": true}'
   ```
   - Always pass `hwnd` (brings window to foreground)
   - If tab number specified: add `"keys": ["ctrl+<N>"]`
   - If "next tab": add `"keys": ["ctrl+tab"]`
   - If "prev tab": add `"keys": ["ctrl+shift+tab"]`
   - Use the window bounds as the region

4. **Show the image** — Save the base64 image to a temp PNG file and use the Read tool to display it inline.

5. **Present results** — Show window title, tab info, and key observations from the AI description.

## Tab Switching Reference

| User says | Keys to send |
|-----------|-------------|
| "tab 1" through "tab 8" | `ctrl+1` through `ctrl+8` |
| "tab 9" or "last tab" | `ctrl+9` (always goes to last tab) |
| "next tab" | `ctrl+tab` |
| "prev tab" / "previous tab" | `ctrl+shift+tab` |

## Notes

- Always use `condor_eye_windows` first (free, instant) — never `condor_eye_locate` (expensive AI call)
- The `hwnd` param brings any window to foreground reliably
- If Condor Eye is not running, tell the user to start it: `cargo tauri dev`
- If connecting from WSL, the HTTP API may be on the Windows gateway IP instead of localhost
