#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));

const CHARACTER_VARIANTS = [
  {
    id: 'engineer-default',
    label: 'Engineer',
    description: 'Balanced starter silhouette with high-contrast orange jacket.',
    prompt:
      'top-down factory engineer with orange work jacket and teal pants, readable silhouette, clean retro pixel art, transparent background',
    seed: 14610359,
    output: 'public/sprites/character-engineer-default.png',
    previewFrame: { x: 0, y: 96 },
  },
  {
    id: 'surveyor-cyan',
    label: 'Surveyor',
    description: 'Cool-toned scout kit with cyan highlights for quick readability.',
    prompt:
      'top-down retro surveyor in slate jacket with cyan scarf, utility harness, readable silhouette, transparent background',
    seed: 9024175,
    output: 'public/sprites/character-surveyor-cyan.png',
    previewFrame: { x: 0, y: 96 },
  },
  {
    id: 'machinist-rose',
    label: 'Machinist',
    description: 'Darker workwear palette with brass accents and heavier profile.',
    prompt:
      'top-down pixel machinist in maroon coveralls with brass accents and dark gloves, readable silhouette, transparent background',
    seed: 7811542,
    output: 'public/sprites/character-machinist-rose.png',
    previewFrame: { x: 0, y: 96 },
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

function runCommand(command, args) {
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(command, args, {
      stdio: 'inherit',
      env: process.env,
    });

    child.on('error', rejectPromise);
    child.on('exit', (code) => {
      if (code === 0) {
        resolvePromise();
        return;
      }
      rejectPromise(new Error(`command failed (${code}): ${command} ${args.join(' ')}`));
    });
  });
}

async function writeManifest(variants) {
  const manifest = {
    schemaVersion: 1,
    defaultSpriteId: 'engineer-default',
    sprites: variants.map((variant) => ({
      id: variant.id,
      label: variant.label,
      description: variant.description,
      sheetPath: variant.output.replace(/^public/, ''),
      metadataPath: `${variant.output.replace(/^public/, '')}.json`,
      previewFrame: variant.previewFrame,
    })),
  };

  const manifestPath = resolve(process.cwd(), 'public/sprites/character-sprites.json');
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  console.log(`Updated sprite manifest: ${manifestPath}`);
}

async function run() {
  const options = parseArgs(process.argv.slice(2));
  const scriptPath = resolve(__dirname, 'generate-character.mjs');

  const variants =
    options.only === null
      ? CHARACTER_VARIANTS
      : CHARACTER_VARIANTS.filter((variant) => variant.id === options.only);

  if (variants.length === 0) {
    const supported = CHARACTER_VARIANTS.map((variant) => variant.id).join(', ');
    throw new Error(`Unknown variant id. Use one of: ${supported}`);
  }

  for (const variant of variants) {
    const args = [
      scriptPath,
      '--prompt',
      variant.prompt,
      '--seed',
      String(variant.seed),
      '--output',
      variant.output,
    ];

    if (options.checkCost) {
      args.push('--check-cost');
    }

    await runCommand(process.execPath, args);
  }

  if (!options.checkCost) {
    await writeManifest(CHARACTER_VARIANTS);
  }
}

run().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
