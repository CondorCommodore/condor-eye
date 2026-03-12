Take a screenshot using Condor Eye. Target: $ARGUMENTS

## Instructions

1. **Parse the target** — Extract:
   - **App name**: the application to capture (e.g., "firefox", "chrome", "thinkorswim", "terminal")
   - **Tab number** (optional): if the user says "tab 3" or "tab3", extract the number (1-9)
   - **Tab direction** (optional): "next tab" or "prev tab" for relative navigation

2. **Find the window** — Call `condor_eye_windows` with the app name as query.

3. **Select the best match** — Pick the most relevant window. If no match, show available windows and ask.

4. **Capture with focus + tab switch** — Use curl to call the HTTP API directly (supports hwnd and keys params):
   ```bash
   curl -s -X POST http://172.23.128.1:9050/api/capture \
     -H "Content-Type: application/json" \
     -d '{"hwnd": <HWND>, "keys": [<KEY_COMBOS>], "prompt": "<PROMPT>", "region": {"x": <X>, "y": <Y>, "width": <W>, "height": <H>}}'
   ```
   - Always pass `hwnd` from step 2 (brings window to foreground)
   - If tab number specified: add `"keys": ["ctrl+<N>"]`
   - If "next tab": add `"keys": ["ctrl+tab"]`
   - If "prev tab": add `"keys": ["ctrl+shift+tab"]`
   - Use the window bounds as the region
   - Set `prompt` to ask for detailed description of visible content

5. **Show the image** — Add `"include_image": true` to the request. Save the base64 image to a temp PNG file and use the Read tool to display it inline. Always show the actual screenshot.

6. **Present results** — Show window title, tab info, and key observations from the AI description.

## Tab Switching Reference

| User says | Keys to send |
|-----------|-------------|
| "tab 1" through "tab 8" | `ctrl+1` through `ctrl+8` |
| "tab 9" or "last tab" | `ctrl+9` (always goes to last tab) |
| "next tab" | `ctrl+tab` |
| "prev tab" / "previous tab" | `ctrl+shift+tab` |

## Notes

- Always use `condor_eye_windows` first (free, instant) — never `condor_eye_locate` (expensive AI call)
- The `hwnd` param triggers `SetForegroundWindow` + `AttachThreadInput` — reliably brings any window to front even if buried
- If the target mentions a specific UI element within a window, capture the full window and focus the prompt on that element
- If Condor Eye app is not running, tell the user to start it: `cd ~/code/dev-tools/condor-eye && cargo.exe tauri dev`
