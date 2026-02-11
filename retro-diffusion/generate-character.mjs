#!/usr/bin/env node

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const RETRO_DIFFUSION_ENDPOINT = 'https://api.retrodiffusion.ai/v1/inferences';
const DEFAULT_STYLE = 'rd_animation__four_angle_walking';
const LEGACY_STYLE = 'animation__four_angle_walking';
const DEFAULT_OUTPUT = 'public/sprites/factorio-character-sheet.png';
const DEFAULT_PROMPT =
  'top-down factory engineer with orange work jacket and teal pants, readable silhouette, clean retro pixel art, transparent background';

const __dirname = dirname(fileURLToPath(import.meta.url));

function parseArgs(argv) {
  const options = {
    prompt: DEFAULT_PROMPT,
    seed: 14610359,
    output: DEFAULT_OUTPUT,
    checkCost: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];

    if (arg === '--prompt') {
      options.prompt = argv[i + 1] ?? options.prompt;
      i += 1;
      continue;
    }

    if (arg === '--seed') {
      const parsed = Number(argv[i + 1]);
      if (Number.isFinite(parsed)) {
        options.seed = Math.trunc(parsed);
      }
      i += 1;
      continue;
    }

    if (arg === '--output') {
      options.output = argv[i + 1] ?? options.output;
      i += 1;
      continue;
    }

    if (arg === '--check-cost') {
      options.checkCost = true;
    }
  }

  return options;
}

async function resolveApiKey() {
  if (process.env.RD_API_KEY && process.env.RD_API_KEY.trim().length > 0) {
    return process.env.RD_API_KEY.trim();
  }

  const keyPath = resolve(__dirname, 'API_KEY.txt');
  const key = await readFile(keyPath, 'utf8');
  const trimmed = key.trim();
  if (!trimmed) {
    throw new Error(`No Retro Diffusion API key found in ${keyPath}`);
  }
  return trimmed;
}

async function requestSpritesheet(apiKey, payload) {
  const response = await fetch(RETRO_DIFFUSION_ENDPOINT, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'X-RD-Token': apiKey,
    },
    body: JSON.stringify(payload),
  });

  const text = await response.text();
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    throw new Error(`Retro Diffusion returned non-JSON response: ${text}`);
  }

  if (!response.ok) {
    const detail = typeof data === 'object' ? JSON.stringify(data) : text;
    throw new Error(`Retro Diffusion request failed (${response.status}): ${detail}`);
  }

  return data;
}

async function run() {
  const options = parseArgs(process.argv.slice(2));
  const apiKey = await resolveApiKey();

  const basePayload = {
    prompt: options.prompt,
    width: 48,
    height: 48,
    num_images: 1,
    seed: options.seed,
    return_spritesheet: true,
    check_cost: options.checkCost,
  };

  let data;
  try {
    data = await requestSpritesheet(apiKey, {
      ...basePayload,
      prompt_style: DEFAULT_STYLE,
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    if (!message.includes('prompt_style')) {
      throw error;
    }

    data = await requestSpritesheet(apiKey, {
      ...basePayload,
      prompt_style: LEGACY_STYLE,
    });
  }

  if (options.checkCost) {
    console.log(`Estimated credit cost: ${data.credit_cost ?? 'unknown'}`);
    return;
  }

  const base64Images = Array.isArray(data.base64_images) ? data.base64_images : [];
  if (base64Images.length === 0) {
    throw new Error(`No base64 image returned by Retro Diffusion: ${JSON.stringify(data)}`);
  }

  const outputPath = resolve(process.cwd(), options.output);
  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, Buffer.from(base64Images[0], 'base64'));

  const metadataPath = `${outputPath}.json`;
  await writeFile(
    metadataPath,
    JSON.stringify(
      {
        prompt: options.prompt,
        seed: options.seed,
        styleTried: [DEFAULT_STYLE, LEGACY_STYLE],
        remainingCredits: data.remaining_credits,
        creditCost: data.credit_cost,
      },
      null,
      2,
    ),
    'utf8',
  );

  console.log(`Generated spritesheet: ${outputPath}`);
  console.log(`Metadata: ${metadataPath}`);
}

run().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
