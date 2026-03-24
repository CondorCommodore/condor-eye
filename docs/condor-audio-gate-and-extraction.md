# Condor Audio Gate And Extraction

This document defines the post-transcription decision path for `condor_audio`.

The flow is intentionally two-stage:

1. Cheap local gate with Ollama decides whether a transcript chunk is worth deeper parsing.
2. Claude receives only the gated-positive chunks and returns structured extraction JSON.

This keeps the pipeline cheap, local-first, and predictable under heavy transcript volume.

## Trigger Point

Run the gate after each transcript chunk is written.

Inputs:

- app id: `zoom` or `discord`
- transcript id
- transcript text
- optional rolling context: previous 1-2 transcript chunks from the same tap

Outputs:

- gate decision
- optional extraction payload

## Stage 1: Ollama Gate

### Model Role

Ollama does triage only. It should not produce long summaries or final structured outputs.

The job is:

- decide whether this chunk contains action-worthy information
- classify the type of value in the chunk
- request escalation to Claude only when the chunk is worth the cost

### Decision Classes

- `ignore`
- `memory`
- `summary`
- `action`
- `urgent_action`

Interpretation:

- `ignore`: filler, chatter, acknowledgements, low-signal banter
- `memory`: useful context or facts, but no immediate action
- `summary`: discussion worth adding to recap, but not a task
- `action`: contains commitments, requests, decisions, or follow-ups
- `urgent_action`: time-sensitive action or operational risk

### Ollama Prompt

```text
You are a strict gate for a transcript pipeline.

You receive one audio transcript chunk from Zoom or Discord.
Your job is to decide if the chunk is worth escalation to a stronger model for structured extraction.

Return JSON only.

Decision rules:
- Use "ignore" for filler, greetings, jokes, backchannel, repeated context, or content with no durable value.
- Use "memory" for durable facts, observations, market color, or design information that may matter later.
- Use "summary" for discussion that should appear in a recap but does not create immediate action.
- Use "action" for explicit requests, assigned work, concrete decisions, follow-ups, or unresolved questions.
- Use "urgent_action" for items that are time-sensitive, operationally risky, or likely to matter within the same trading session.

Be conservative. If the chunk is ambiguous, choose the lower class.

Return exactly this schema:
{
  "decision": "ignore" | "memory" | "summary" | "action" | "urgent_action",
  "confidence": 0.0,
  "reason": "one short sentence",
  "topics": ["short topic", "..."],
  "should_escalate": true
}
```

### Escalation Rule

Escalate to Claude when:

- `decision` is `action` or `urgent_action`
- or `decision` is `memory` or `summary` with `confidence >= 0.80`

Do not escalate `ignore`.

## Stage 2: Claude Structured Extraction

### Extraction Scope

Claude should extract durable structured data, not paraphrase the entire chunk.

Allowed extraction classes:

- decisions
- action items
- risks
- open questions
- entities
- project mentions
- symbols / instruments / market references

### Claude Prompt

```text
You are extracting durable structured knowledge from a transcript chunk.

Rules:
- Use only information present in the transcript or clearly implied by direct phrasing.
- Do not invent speakers, dates, projects, or action owners.
- If a field is unknown, leave it null or omit it according to the schema.
- Keep evidence snippets short and literal.
- Normalize obvious project names and ticker symbols when they are explicit in the text.

Return JSON only matching the provided schema.

Extraction priorities:
1. explicit decisions
2. explicit action items
3. urgent risks or operational warnings
4. unresolved questions
5. durable factual notes
```

## JSON Schema

Machine-readable schema lives at:

- [condor-audio-extraction-schema.json](/mnt/data/repos/condor-eye/docs/condor-audio-extraction-schema.json)

Top-level shape:

- transcript metadata
- gate decision
- extracted decisions
- extracted action items
- extracted risks
- extracted questions
- extracted facts
- routing hints

## Routing Hints

The extraction payload should help downstream routing without performing the routing itself.

Suggested hints:

- `route_coord`
- `route_brainstorm`
- `route_memory`
- `needs_human_review`

These are hints, not side effects.

## Examples

### Ignore

Transcript:

```text
yeah that makes sense, give me two minutes and i'll hop back on
```

Gate:

```json
{
  "decision": "ignore",
  "confidence": 0.97,
  "reason": "Backchannel coordination without durable project or market value.",
  "topics": [],
  "should_escalate": false
}
```

### Action

Transcript:

```text
we need to add a one point five second overlap between chunks in condor audio so words don't get cut at boundaries
```

Gate:

```json
{
  "decision": "action",
  "confidence": 0.95,
  "reason": "Concrete implementation request for the condor_audio pipeline.",
  "topics": ["condor_audio", "chunk stitching"],
  "should_escalate": true
}
```

## Implementation Notes

- The gate should run locally first, ideally via Ollama HTTP.
- The Claude extraction path should only run on gated-positive chunks.
- Store both gate output and extraction output next to the transcript for replayability.
- Keep the raw transcript as the source of truth; structured extraction is derived data.

## Recommended File Layout

For transcript id `zoom_20260324T143000.txt`:

- `transcripts/zoom_20260324T143000.txt`
- `transcripts/zoom_20260324T143000.gate.json`
- `transcripts/zoom_20260324T143000.extract.json`
