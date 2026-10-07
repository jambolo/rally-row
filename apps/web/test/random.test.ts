import { describe, expect, it } from 'vitest';
import { SIMULATION_SEED, normalSource, sfc32 } from '../src/random.ts';

const draw = (g: () => number, n: number) => Array.from({ length: n }, g);

describe('seeded random', () => {
  it('sfc32 reproduces the reference outputs', () => {
    expect(SIMULATION_SEED).toBe(0x2545f491);
    expect(draw(sfc32(SIMULATION_SEED), 3).map((x) => x * 4294967296)).toEqual([1736660801, 1795052019, 785429819]);
    expect(draw(sfc32(0), 3).map((x) => x * 4294967296)).toEqual([473542192, 3964317691, 2295682031]);
  });

  it('sfc32 repeats a sequence for the same seed', () => {
    expect(draw(sfc32(42), 1000)).toEqual(draw(sfc32(42), 1000));
  });

  it('sfc32 sequences differ between seeds', () => {
    expect(draw(sfc32(1), 10)).not.toEqual(draw(sfc32(2), 10));
  });

  it('sfc32 outputs lie in [0, 1) with mean near one half', () => {
    const g = sfc32(SIMULATION_SEED);
    const n = 100_000;
    let min = Infinity,
      max = -Infinity,
      sum = 0;
    for (let i = 0; i < n; i++) {
      const x = g();
      if (x < min) min = x;
      if (x > max) max = x;
      sum += x;
    }
    expect(min).toBeGreaterThanOrEqual(0);
    expect(max).toBeLessThan(1);
    expect(Math.abs(sum / n - 0.5)).toBeLessThan(0.005);
  });

  it('normalSource returns the cached spare on alternate calls', () => {
    const values = [0.25, 0.125, 0.5, 0.75];
    let calls = 0;
    const normal = normalSource(() => values[calls++]!);
    const first = Math.sqrt(-2 * Math.log(0.75)) * Math.SQRT1_2;
    expect(normal()).toBeCloseTo(first, 12);
    expect(calls).toBe(2);
    expect(normal()).toBeCloseTo(first, 12);
    expect(calls).toBe(2);
    expect(normal()).toBeCloseTo(0, 12);
    expect(calls).toBe(4);
    expect(normal()).toBeCloseTo(-Math.sqrt(-2 * Math.log(0.5)), 12);
    expect(calls).toBe(4);
  });

  it('normalSource has mean near 0 and variance near 1', () => {
    const normal = normalSource(sfc32(SIMULATION_SEED));
    const n = 100_000;
    let sum = 0,
      sumSq = 0;
    for (let i = 0; i < n; i++) {
      const x = normal();
      sum += x;
      sumSq += x * x;
    }
    const mean = sum / n;
    const variance = sumSq / n - mean * mean;
    expect(Math.abs(mean)).toBeLessThan(0.02);
    expect(Math.abs(variance - 1)).toBeLessThan(0.02);
  });
});
