export interface GenerationConfig {
  distance: number; rounds: number; shots: number; noise: number; seed: number;
  basis: 'x' | 'z'; commit: number; buffer: number; measure: boolean; warmup: number; repeats: number;
}

export function parseGenerationConfig(value: unknown): GenerationConfig {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid generation settings');
  const row = value as Record<string, unknown>;
  const keys = ['distance', 'rounds', 'shots', 'noise', 'seed', 'basis', 'commit', 'buffer', 'measure', 'warmup', 'repeats'];
  if (Object.keys(row).some(key => !keys.includes(key))) throw new Error('Unknown generation setting');
  function integer(key: string, min: number, max: number) {
    const n = row[key];
    if (typeof n !== 'number' || !Number.isSafeInteger(n) || n < min || n > max) throw new Error(`${key} must be an integer from ${min} to ${max}`);
    return n;
  }
  const distance = integer('distance', 3, 7);
  if (distance % 2 === 0) throw new Error('Distance must be odd');
  const rounds = integer('rounds', 2, 90);
  const shots = integer('shots', 1, 256);
  const commit = integer('commit', 1, 15), buffer = integer('buffer', 1, 15);
  if (typeof row.noise !== 'number' || !Number.isFinite(row.noise) || row.noise <= 0 || row.noise > 0.02) throw new Error('Noise probability must be greater than 0 and at most 0.02');
  if (row.basis !== 'x' && row.basis !== 'z') throw new Error('Basis must be x or z');
  if (typeof row.measure !== 'boolean') throw new Error('Timing selection must be boolean');
  if (rounds * (distance * distance - 1) > 1500) throw new Error('Use at most 1500 detectors for local interactive generation; reduce distance or rounds');
  if (Math.min(rounds + 1, commit + 2 * buffer) * (distance * distance - 1) > 400) throw new Error('Window graphs must have at most 400 detectors; reduce commit/buffer widths');
  if (Math.ceil((rounds + 1) / commit) < 2) throw new Error('Choose a smaller commit width to create both decoding layers');
  const repeats = integer('repeats', 1, 100);
  if (shots * repeats > 5000) throw new Error('Use at most 5000 timed samples per method');
  return { distance, rounds, shots, noise: row.noise, seed: integer('seed', 0, 4294967295), basis: row.basis,
    commit, buffer, measure: row.measure, warmup: integer('warmup', 0, 10), repeats };
}
