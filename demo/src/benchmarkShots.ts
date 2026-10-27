// Convert the saved CLI corrections back to their *input* shots. Only measured
// actual_observables, never a decoder prediction, can be used as ground truth.
export interface BenchmarkShotInput { defects: number[]; observables: number[] }

export function shotsFromCorrectionCapture(value: unknown): { shots: BenchmarkShotInput[] } {
  if (!value || typeof value !== 'object') throw new Error('Expected a CLI corrections capture');
  const capture = value as Record<string, unknown>;
  if (capture.schema_version !== 1 || typeof capture.num_detectors !== 'number'
      || !Number.isInteger(capture.num_detectors) || capture.num_detectors <= 0
      || typeof capture.num_observables !== 'number' || !Number.isInteger(capture.num_observables)
      || capture.num_observables < 0 || !Array.isArray(capture.shots) || capture.shots.length === 0) {
    throw new Error('Expected a nonempty schema-v1 CLI corrections capture');
  }
  const validateIds = (ids: unknown, limit: number, label: string): number[] => {
    if (!Array.isArray(ids) || ids.some(id => !Number.isInteger(id) || id < 0 || id >= limit)
        || new Set(ids).size !== ids.length) {
      throw new Error(`Invalid ${label} in captured shot`);
    }
    return [...ids] as number[];
  };
  return {
    shots: capture.shots.map((value: unknown) => {
      if (!value || typeof value !== 'object') throw new Error('Invalid captured shot');
      const shot = value as Record<string, unknown>;
      return {
        defects: validateIds(shot.defects, capture.num_detectors as number, 'defects'),
        observables: validateIds(shot.actual_observables, capture.num_observables as number, 'actual observables'),
      };
    }),
  };
}
