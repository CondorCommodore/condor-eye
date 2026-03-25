# Condor Audio Mini UI

This is a standalone replacement UI for the audio project.

It does not load the Condor Eye GUI.
It only talks to the local audio API.

## Purpose

- choose `Zoom` or `Discord`
- start a tap
- read transcript text
- stop the tap

## Run

1. Start the backend on Aurora.
2. Keep the audio API on `http://127.0.0.1:9051`.
3. Open `index.html` in a browser, or serve this folder with any static file server.
4. Paste the `CAPTURE_TOKEN` into the UI.

## Recommended Backend Mode

```env
CONDOR_AUDIO_AUTO_WATCH=0
CONDOR_AUDIO_BIND=127.0.0.1
CONDOR_AUDIO_PORT=9051
```

That keeps the backend in simple manual-tap mode.
