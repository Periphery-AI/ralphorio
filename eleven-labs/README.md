# ElevenLabs Asset Pipeline (for future AI sessions)

This folder is the single source of truth for ElevenLabs usage in this repo.

## Files

- `API_KEY.txt`: raw ElevenLabs API key.

## Quick setup

```bash
export ELEVENLABS_API_KEY="$(cat eleven-labs/API_KEY.txt)"
mkdir -p assets/audio/voice assets/audio/sfx assets/audio/music
```

## 1) Voice (Text to Speech)

Use this for character lines, UI narration, and placeholder VO.

### Pick a voice

```bash
curl -sS -X GET "https://api.elevenlabs.io/v1/voices" \
  -H "xi-api-key: $ELEVENLABS_API_KEY" \
  -H "Accept: application/json"
```

Copy a `voice_id` from the response.

### Generate speech

```bash
VOICE_ID="EXAVITQu4vr4xnSDxMaL" # replace
TEXT="Factory online. Conveyor belt initialized."

curl -sS -X POST "https://api.elevenlabs.io/v1/text-to-speech/$VOICE_ID" \
  -H "xi-api-key: $ELEVENLABS_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"text\":\"$TEXT\",\"model_id\":\"eleven_multilingual_v2\"}" \
  --output assets/audio/voice/factory_online.mp3
```

## 2) Sound Effects

Use this for short one-shots (shoot, place, pickup, UI click).

```bash
PROMPT="Short retro-futuristic laser shot, bright attack, tight tail, no reverb"

curl -sS -X POST "https://api.elevenlabs.io/v1/sound-generation" \
  -H "xi-api-key: $ELEVENLABS_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"text\":\"$PROMPT\",\"duration_seconds\":1.0}" \
  --output assets/audio/sfx/laser_shot_01.mp3
```

## 3) Music

Use this for ambient loops and biome themes.

```bash
PROMPT="Loopable top-down factory ambience, retro synth pulse, minimal drums, 110 BPM"

curl -sS -X POST "https://api.elevenlabs.io/v1/music/compose" \
  -H "xi-api-key: $ELEVENLABS_API_KEY" \
  -H "Content-Type: application/json" \
  -d "{\"prompt\":\"$PROMPT\",\"duration_seconds\":60}" \
  --output assets/audio/music/factory_loop_01.mp3
```

## Conventions

- Keep filenames semantic and versioned: `feature_action_01.mp3`.
- Store temporary generations in `assets/audio/_scratch/` before promoting.
- Convert to engine target format in build pipeline if needed.

## Practical prompts

- Projectile shot: `"Tight 8-bit laser pop, bright transient, 0.3s tail"`
- Build placement: `"Soft mechanical clunk with subtle metallic click"`
- Beacon activation: `"Electronic chirp rising in pitch, optimistic UI style"`
- Background loop: `"Calm industrial ambience, subtle machinery rhythm, loopable"`

## Notes

- Keep this README command-first for automation.
- If endpoints or payload fields change, confirm against official docs:
  - https://elevenlabs.io/docs
  - https://api.elevenlabs.io/docs
