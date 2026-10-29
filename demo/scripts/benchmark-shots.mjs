#!/usr/bin/env node
// Derive original inputs from a checked-in CLI correction capture. Never use
// the decoder's predicted observable flips as measured ground truth.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { shotsFromCorrectionCapture } from '../src/benchmarkShots.ts';

export { shotsFromCorrectionCapture };

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) {
    console.error('Usage: node demo/scripts/benchmark-shots.mjs CAPTURE.corrections.json SHOTS.json');
    process.exitCode = 2;
  } else {
    try {
      const capture = JSON.parse(readFileSync(input, 'utf8'));
      const shots = shotsFromCorrectionCapture(capture);
      writeFileSync(output, JSON.stringify(shots, null, 2) + '\n');
      console.log(`Wrote ${shots.shots.length} captured shot inputs to ${output}`);
    } catch (error) {
      console.error(`Could not prepare benchmark shots: ${error.message}`);
      process.exitCode = 1;
    }
  }
}
