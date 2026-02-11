#!/usr/bin/env node

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const RETRO_DIFFUSION_ENDPOINT = 'https://api.retrodiffusion.ai/v1/inferences';
const DEFAULT_WIDTH = 64;
const DEFAULT_HEIGHT = 64;

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORLD_VARIANTS = [
  {
    id: 'enemy-biter-small',
    category: 'enemy',
    kind: 'biter_small',
    label: 'Biter Small',
    description: 'Small melee biter silhouette with warm carapace tones.',
    prompt:
      'top-down retro pixel art alien biter creature, readable silhouette, transparent background, centered single sprite',
    seed: 3701402,
    output: 'public/sprites/world-enemy-biter-small.png',
  },
  {
    id: 'enemy-biter-medium',
    category: 'enemy',
    kind: 'biter_medium',
    label: 'Biter Medium',
    description: 'Heavier biter profile with broader body and darker shell accents.',
    prompt:
      'top-down retro pixel art medium alien biter, heavier silhouette, transparent background, centered single sprite',
    seed: 3701411,
    output: 'public/sprites/world-enemy-biter-medium.png',
  },
  {
    id: 'enemy-spitter-small',
    category: 'enemy',
    kind: 'spitter_small',
    label: 'Spitter Small',
    description: 'Small ranged spitter with a distinct acid-sack profile.',
    prompt:
      'top-down retro pixel art small alien spitter enemy, readable ranged silhouette, transparent background, centered single sprite',
    seed: 3701423,
    output: 'public/sprites/world-enemy-spitter-small.png',
  },
  {
    id: 'resource-iron-ore',
    category: 'resource',
    kind: 'iron_ore',
    label: 'Iron Ore Node',
    description: 'Chunky cool-gray ore cluster for early mining loops.',
    prompt:
      'top-down retro pixel art iron ore rock cluster, readable mining node silhouette, transparent background, centered single sprite',
    seed: 2702402,
    output: 'public/sprites/world-resource-iron-ore.png',
  },
  {
    id: 'resource-copper-ore',
    category: 'resource',
    kind: 'copper_ore',
    label: 'Copper Ore Node',
    description: 'Warm orange-brown ore cluster with strong contrast edges.',
    prompt:
      'top-down retro pixel art copper ore rock cluster, readable mining node silhouette, transparent background, centered single sprite',
    seed: 2702414,
    output: 'public/sprites/world-resource-copper-ore.png',
  },
  {
    id: 'resource-coal',
    category: 'resource',
    kind: 'coal',
    label: 'Coal Node',
    description: 'Dark carbon-rich cluster with matte highlights for readability.',
    prompt:
      'top-down retro pixel art coal rock cluster, readable mining node silhouette, transparent background, centered single sprite',
    seed: 2702421,
    output: 'public/sprites/world-resource-coal.png',
  },
  {
    id: 'structure-beacon',
    category: 'structure',
    kind: 'beacon',
    label: 'Beacon',
    description: 'Starter beacon with bright signal core and compact base.',
    prompt:
      'top-down retro pixel art industrial beacon building, clean silhouette, transparent background, centered single sprite',
    seed: 4403106,
    output: 'public/sprites/world-structure-beacon.png',
  },
  {
    id: 'structure-miner',
    category: 'structure',
    kind: 'miner',
    label: 'Miner',
    description: 'Compact mechanical drill body for resource extraction.',
    prompt:
      'top-down retro pixel art mining drill building, clean silhouette, transparent background, centered single sprite',
    seed: 4403113,
    output: 'public/sprites/world-structure-miner.png',
  },
  {
    id: 'structure-assembler',
    category: 'structure',
    kind: 'assembler',
    label: 'Assembler',
    description: 'Boxy crafting station with visible front-facing machine details.',
    prompt:
      'top-down retro pixel art assembler factory machine, clean silhouette, transparent background, centered single sprite',
    seed: 4403127,
    output: 'public/sprites/world-structure-assembler.png',
  },
];

function parseArgs(argv) {
  const options = {
    checkCost: false,
    only: null,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];

    if (arg === '--check-cost') {
      options.checkCost = true;
      continue;
    }

    if (arg === '--only') {
      options.only = argv[i + 1] ?? null;
      i += 1;
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

async function requestImage(apiKey, payload) {
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

function spriteManifestPath(variant) {
  return variant.output.replace(/^public/, '');
}

async function writeManifest(variants) {
  const manifest = {
    schemaVersion: 1,
    defaults: {
      enemy: 'enemy-biter-small',
      resource: 'resource-iron-ore',
      structure: 'structure-beacon',
    },
    sprites: variants.map((variant) => ({
      id: variant.id,
      category: variant.category,
      kind: variant.kind,
      label: variant.label,
      description: variant.description,
      texturePath: spriteManifestPath(variant),
      metadataPath: `${spriteManifestPath(variant)}.json`,
      width: DEFAULT_WIDTH,
      height: DEFAULT_HEIGHT,
      source: 'retro-diffusion',
    })),
  };

  const manifestPath = resolve(process.cwd(), 'public/sprites/world-sprites.json');
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  console.log(`Updated world sprite manifest: ${manifestPath}`);
}

async function generateVariant(apiKey, variant, checkCost) {
  const payload = {
    prompt: variant.prompt,
    width: DEFAULT_WIDTH,
    height: DEFAULT_HEIGHT,
    num_images: 1,
    seed: variant.seed,
    check_cost: checkCost,
  };
  const data = await requestImage(apiKey, payload);
  const creditCost = Number.isFinite(data.credit_cost) ? data.credit_cost : null;
  const remainingCredits =
    Number.isFinite(data.remaining_credits) ? data.remaining_credits : null;
  const model = typeof data.model === 'string' ? data.model : null;

  if (checkCost) {
    return { creditCost, remainingCredits, model };
  }

  const base64Images = Array.isArray(data.base64_images) ? data.base64_images : [];
  if (base64Images.length === 0) {
    throw new Error(`No base64 image returned for ${variant.id}: ${JSON.stringify(data)}`);
  }

  const outputPath = resolve(process.cwd(), variant.output);
  await mkdir(dirname(outputPath), { recursive: true });
  await writeFile(outputPath, Buffer.from(base64Images[0], 'base64'));

  const metadataPath = `${outputPath}.json`;
  await writeFile(
    metadataPath,
    JSON.stringify(
      {
        id: variant.id,
        category: variant.category,
        kind: variant.kind,
        prompt: variant.prompt,
        seed: variant.seed,
        width: DEFAULT_WIDTH,
        height: DEFAULT_HEIGHT,
        model,
        remainingCredits,
        creditCost,
      },
      null,
      2,
    ),
    'utf8',
  );

  console.log(`Generated ${variant.id}: ${outputPath}`);
  console.log(`Metadata: ${metadataPath}`);
  return { creditCost, remainingCredits, model };
}

async function run() {
  const options = parseArgs(process.argv.slice(2));
  const variants =
    options.only === null
      ? WORLD_VARIANTS
      : WORLD_VARIANTS.filter((variant) => variant.id === options.only);

  if (variants.length === 0) {
    const supported = WORLD_VARIANTS.map((variant) => variant.id).join(', ');
    throw new Error(`Unknown variant id. Use one of: ${supported}`);
  }

  const apiKey = await resolveApiKey();
  let estimatedTotal = 0;

  for (const variant of variants) {
    const result = await generateVariant(apiKey, variant, options.checkCost);
    const cost = Number.isFinite(result.creditCost) ? result.creditCost : 0;
    estimatedTotal += cost;
    const remaining =
      Number.isFinite(result.remainingCredits) || result.remainingCredits === 0
        ? result.remainingCredits
        : 'unknown';
    const mode = options.checkCost ? 'Estimated' : 'Applied';
    console.log(`${mode} credit cost for ${variant.id}: ${cost} (remaining: ${remaining})`);
  }

  console.log(
    `${options.checkCost ? 'Estimated' : 'Applied'} total credit cost: ${estimatedTotal}`,
  );

  if (!options.checkCost) {
    await writeManifest(WORLD_VARIANTS);
  }
}

run().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
