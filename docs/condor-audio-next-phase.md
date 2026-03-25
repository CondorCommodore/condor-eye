# Condor Audio Next Phase

## Goal

Move `condor_audio` from the current process-discovery prototype to a reliable
Windows audio-session-driven service that is ready for unattended daily use.

## Next Phase Work

### 1. Replace process discovery with true audio-session discovery

- Enumerate active Windows render sessions instead of shelling out to `Get-Process`.
- Match sessions to Zoom and Discord by owning PID and resolved executable path.
- Auto-start taps only for active sessions.
- Auto-stop taps when sessions go inactive or expire, not just when the process exits.

### 2. Separate worker health from transcript health

- Keep a tap `Running` while capture is live, even if a transcript POST fails.
- Track worker lifecycle separately from per-chunk transcription errors.
- Add an explicit health field or error counters so status is observable without spawning duplicate taps.

### 3. Remove shell-based liveness checks from the hot path

- Stop spawning `powershell.exe` inside the capture loop.
- Move PID/session liveness checks to the watcher cadence or native Windows APIs.
- Keep the capture worker focused on audio, chunking, and transcript delivery.

### 4. Add operator-visible consent and status

- Show toast/tray state when any tap is active.
- Surface the last transcription error and last successful transcript timestamp in status.
- Make it obvious when capture is running but whisper is degraded.

### 5. Finish the unattended nightly path

- Confirm Aurora startup path for `condor-eye` and `whisper-server`.
- Add the nightly bring-up mechanism in `home-lab` using the existing systemd/timer pattern.
- Document the exact operator test steps for Zoom and Discord on Aurora.

## Acceptance

- Starting Zoom or Discord with active audio causes a tap to appear without manual API calls.
- Idle/background Zoom or Discord processes do not start capture.
- A failed whisper request does not create duplicate taps.
- A spoken phrase becomes visible in transcript text within the current chunk SLA.
- Aurora can survive the nightly restart path without manual intervention.
