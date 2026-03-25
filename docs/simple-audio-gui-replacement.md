# Simple Audio GUI Replacement

## Goal

Replace the Condor Eye GUI for the audio project with a much smaller local tool.

This replacement should:

- not load the Condor Eye screen-capture UI
- not depend on the panel/grid editor
- let the operator choose a channel/app target
- show live or near-live speech-to-text output
- stay local-first on Aurora

## Minimal Product

A tiny desktop window with three areas:

1. Channel selector
   - `Zoom`
   - `Discord`
   - optional manual PID/session selector if multiple matches exist

2. Controls
   - `Refresh Sessions`
   - `Start Listening`
   - `Stop Listening`
   - status line: `idle`, `listening`, `transcribing`, `error`

3. Transcript view
   - scrolling plain-text area
   - newest chunk appended at the bottom
   - timestamp prefix per chunk
   - optional copy button

## Suggested Architecture

Keep the capture/transcription backend separate from the GUI:

- backend:
  - local audio API only
  - `GET /api/condor_audio/sessions`
  - `POST /api/condor_audio/taps`
  - `DELETE /api/condor_audio/taps/:id`
  - `GET /api/condor_audio/taps/:id/latest-transcript`
  - `GET /api/condor_audio/transcripts?app=...`

- frontend:
  - a tiny dedicated app or local webview
  - polls transcript endpoints every 1-2 seconds
  - does not import or render Condor Eye capture/vision components

## Recommended UI Stack

Best fit for speed:

- Option A: plain local web app
  - one static HTML file
  - small JS file
  - talks to `127.0.0.1:9051`
  - lowest implementation cost

- Option B: tiny Tauri app
  - separate crate/app from Condor Eye
  - same Rust backend concepts if needed later
  - more packaging overhead, but cleaner long-term

For the current task, Option A is the right first move.

## Proposed Screen

Top row:

- target dropdown
- refresh button
- start button
- stop button

Middle row:

- status badge
- active tap id
- last transcript timestamp

Main body:

- large read-only transcript pane

Footer:

- backend URL
- output directory
- whisper URL

## Data Flow

1. User clicks `Refresh Sessions`
2. UI calls `GET /api/condor_audio/sessions`
3. User selects `Zoom` or `Discord`
4. User clicks `Start Listening`
5. UI calls `POST /api/condor_audio/taps`
6. UI polls:
   - `GET /api/condor_audio/taps/:id`
   - `GET /api/condor_audio/taps/:id/latest-transcript`
7. Transcript text is appended into the transcript pane
8. User clicks `Stop Listening`
9. UI calls `DELETE /api/condor_audio/taps/:id`

## Why This Is Better

- much smaller surface area than Condor Eye
- no screen-capture code, no vision code, no panel UI
- easier to debug the audio path in isolation
- matches the actual operator need: choose channel, read text

## Implementation Recommendation

Build the replacement as a separate tiny frontend against the existing audio API.

Do not embed it into the Condor Eye GUI.
Do not make it depend on Condor Eye panel state.
Treat `condor_audio` as its own product surface.
