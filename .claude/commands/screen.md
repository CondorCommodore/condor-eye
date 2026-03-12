Take a screenshot using Condor Eye. Target: $ARGUMENTS

## Instructions

1. **Find the window** — Call `condor_eye_windows` with a query derived from the target description. Extract the key application name (e.g., "thinkorswim", "Chrome", "terminal", "VS Code").

2. **Select the best match** — If multiple windows match, pick the most relevant one based on the target description. If no match, tell the user what windows are available and ask them to clarify.

3. **Capture the region** — Call `condor_eye_capture` with the window's bounds as the `region` parameter. Use a prompt that asks for detailed description of the target content.

4. **Present results** — Show a concise summary of what Condor Eye sees. Include the window title, dimensions, and key observations.

## Notes

- Always use `condor_eye_windows` first (free, instant) — never `condor_eye_locate` (expensive AI call)
- If the target mentions a specific UI element within a window (e.g., "the DOM in thinkorswim"), capture the full window and ask the AI to focus on that element in the prompt
- If Condor Eye app is not running, tell the user to start it: `cd ~/code/dev-tools/condor-eye && cargo.exe tauri dev`
