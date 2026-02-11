# Retro Diffusion Character Pipeline

This folder contains the workflow to generate a top-down animated character spritesheet for the Bevy client.

## API key

The Retro Diffusion API key is stored in:

- `retro-diffusion/API_KEY.txt`

You can also override it at runtime:

```bash
RD_API_KEY=your_key_here node retro-diffusion/generate-character.mjs
```

## Generate a single character spritesheet

Default command:

```bash
node retro-diffusion/generate-character.mjs
```

This generates:

- `public/sprites/factorio-character-sheet.png`
- `public/sprites/factorio-character-sheet.png.json` (prompt/seed/credits metadata)

## Generate the selectable 3-variant sprite set

Use the curated preset generator:

```bash
node retro-diffusion/generate-character-set.mjs
```

This regenerates:

- `public/sprites/character-engineer-default.png`
- `public/sprites/character-surveyor-cyan.png`
- `public/sprites/character-machinist-rose.png`
- each `*.png.json` metadata file
- `public/sprites/character-sprites.json` manifest

Generate one preset only:

```bash
node retro-diffusion/generate-character-set.mjs --only surveyor-cyan
```

Check cost for the preset workflow without writing assets:

```bash
node retro-diffusion/generate-character-set.mjs --check-cost
```

## Useful options

Custom prompt:

```bash
node retro-diffusion/generate-character.mjs --prompt "top-down retro engineer in orange suit with helmet"
```

Custom seed:

```bash
node retro-diffusion/generate-character.mjs --seed 123456
```

Custom output path:

```bash
node retro-diffusion/generate-character.mjs --output public/sprites/my-character-sheet.png
```

Check credit cost only:

```bash
node retro-diffusion/generate-character.mjs --check-cost
```

## Notes on format

- Uses Retro Diffusion animation style `rd_animation__four_angle_walking` (with fallback to legacy `animation__four_angle_walking`).
- Expected frame resolution is `48x48`.
- Returned as transparent PNG spritesheet (`return_spritesheet: true`).
- Bevy treats this as a 4x4 grid (`4 directions x 4 frames`).

## Integration target

The curated multiplayer character set is consumed by:

- `src/game/character-sprites.ts` (web selector metadata)
- `worker/src/lib.rs` (allowed sprite id validation)
- `game-client/src/lib.rs` (WASM sprite atlas mapping)

When adding/replacing variants, keep sprite ids and paths consistent across those files and
regenerate `public/sprites/character-sprites.json`.
