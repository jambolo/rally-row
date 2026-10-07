/** Default seed, so repeated runs over the same inputs give identical odds. */
export const SIMULATION_SEED = 0x2545f491;

/** sfc32: 32-bit integer operations only, so every JavaScript engine produces the same sequence. */
export function sfc32(seed: number): () => number {
  let a = 0x9e3779b9,
    b = 0x243f6a88,
    c = 0xb7e15162,
    d = seed >>> 0;
  const next = () => {
    const t0 = (a + b) | 0;
    a = b ^ (b >>> 9);
    b = (c + (c << 3)) | 0;
    c = (c << 21) | (c >>> 11);
    d = (d + 1) | 0;
    const t = (t0 + d) | 0;
    c = (c + t) | 0;
    return (t >>> 0) / 4294967296;
  };
  // Early outputs still reflect the fixed initial state.
  for (let i = 0; i < 15; i++) next();
  return next;
}

/** Box–Muller turns two uniforms into two normals, so the second is kept for the next call. */
export function normalSource(uniform: () => number): () => number {
  let spare = 0,
    hasSpare = false;
  return () => {
    if (hasSpare) {
      hasSpare = false;
      return spare;
    }
    const u1 = uniform(),
      u2 = uniform();
    // 1 - u1 lies in (0, 1], so the logarithm is finite.
    const r = Math.sqrt(-2 * Math.log(1 - u1)),
      phi = 2 * Math.PI * u2;
    spare = r * Math.sin(phi);
    hasSpare = true;
    return r * Math.cos(phi);
  };
}
