# Simple Audio GUI Replacement

## Goal

Treat `condor_audio` as its own product surface instead of making operators open the full Condor Eye capture window just to start or inspect audio taps.

## Chosen Shape

Use a standalone static UI in [`audio-mini-ui/`](../audio-mini-ui) that talks directly to the localhost audio API.

The backend contract is:

- manual tap mode is the default
- watcher auto-discovery is optional via `CONDOR_AUDIO_AUTO_WATCH=true`
- browser-based clients are allowed through CORS on the audio listener

## Why this is simpler

- No dependency on the main frameless Condor Eye GUI
- No Tauri frontend changes required to test audio
- Easy to hand off to Aurora: open a browser, paste token, start taps
- Static files can be served locally or opened directly from disk

## Operator path

1. Start `condor-eye`
2. Confirm `http://127.0.0.1:9051/api/condor_audio/status`
3. Open `audio-mini-ui/index.html`
4. Paste `CAPTURE_TOKEN`
5. Load sessions
6. Start Zoom or Discord tap
7. Pull latest transcript from the tap card

## Non-goals

- Replacing the main Condor Eye vision surface
- Solving true Windows audio-session enumeration in this step
- Adding tray notifications or consent indicators in this step

## Follow-up

If the mini UI path proves stable on Aurora, it becomes the default operator surface for audio capture work and the Tauri window can remain focused on vision/screenshot flows.
