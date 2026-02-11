# Retro Diffusion Character Pipeline

This folder contains the workflow to generate a top-down animated character spritesheet for the Bevy client.

## API key

The Retro Diffusion API key is stored in:

- `retro-diffusion/API_KEY.txt`

You can also override it at runtime:

```bash
RD_API_KEY=your_key_here node retro-diffusion/generate-character.mjs
```

## Generate a character spritesheet

Default command:

```bash
node retro-diffusion/generate-character.mjs
```

This generates:

- `public/sprites/factorio-character-sheet.png`
- `public/sprites/factorio-character-sheet.png.json` (prompt/seed/credits metadata)

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

The game client expects the generated spritesheet at:

- `public/sprites/factorio-character-sheet.png`

If you generate to a different file, update the path in:

- `game-client/src/lib.rs`
