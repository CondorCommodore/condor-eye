# Condor Audio Mini UI

Standalone browser UI for the local `condor_audio` API.

## Purpose

This is the operator surface for manual tap testing without opening or depending on the main Condor Eye GUI.

It talks directly to:

- `GET /api/condor_audio/status`
- `GET /api/condor_audio/sessions`
- `POST /api/condor_audio/taps`
- `DELETE /api/condor_audio/taps/{tap_id}`
- `GET /api/condor_audio/taps/{tap_id}/latest-transcript`

## Run

1. Start `condor-eye` so the audio API is listening on `127.0.0.1:9051`.
2. Open `index.html` in a browser.
3. Enter the `CAPTURE_TOKEN` value.
4. Check status, load sessions, then start a Zoom or Discord tap.

If the API is bound to a different host or port, change the base URL in the UI.

## Notes

- Browser access requires CORS on the audio API listener.
- Manual mode is the default path. Auto-watch is optional via `CONDOR_AUDIO_AUTO_WATCH=true`.
- This UI is intentionally static and dependency-free so it can be copied or hosted anywhere local.
