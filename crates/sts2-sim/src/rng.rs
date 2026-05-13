//! Bit-exact port of `MegaCrit.Sts2.Core.Random.Rng`.
//!
//! The MegaCrit class is a thin wrapper over .NET's `System.Random` constructed
//! as `new Random((int)seed)`. With a seed argument, .NET (including .NET 9)
//! uses the legacy Knuth-subtractive PRNG for back-compat — NOT xoshiro256**.
//! This module re-implements that exact algorithm so the simulator advances
//! state in lock-step with the real game.
//!
//! Every method here is diff-tested against the live game DLL via the
//! `sts2-sim-oracle-tests` crate. Don't edit without re-running those tests.

use crate::hash::deterministic_hash_code;

const MBIG: i32 = i32::MAX; // 2_147_483_647
const MSEED: i32 = 161_803_398;

#[derive(Debug, Clone)]
pub struct Rng {
    seed: u32,
    counter: i32,
    seed_array: [i32; 56],
    inext: usize,
    inextp: usize,
}

impl Rng {
    /// Construct an Rng with the given seed and counter. The counter is
    /// fast-forwarded after seeding, exactly matching the C# constructor:
    /// `new Rng(uint seed = 0, int counter = 0)`.
    pub fn new(seed: u32, counter: i32) -> Self {
        // The C# constructor does `new Random((int)seed)` — a bitwise
        // reinterpretation, not a checked cast. `seed as i32` matches.
        let seed_i32 = seed as i32;
        let mut rng = Self {
            seed,
            counter: 0,
            seed_array: [0; 56],
            inext: 0,
            inextp: 21,
        };
        rng.initialize_from_seed(seed_i32);
        rng.fast_forward_counter(counter);
        rng
    }

    /// Convenience constructor mirroring the C# `new Rng(uint seed, string name)`:
    /// `new Rng(seed + (uint)GetDeterministicHashCode(name), 0)`. Every named
    /// stream in the game (RunRngSet, PlayerRngSet, ad-hoc per-event,
    /// per-relic, etc.) is constructed via this overload.
    pub fn new_named(base_seed: u32, name: &str) -> Self {
        let hashed = deterministic_hash_code(name) as u32;
        Self::new(base_seed.wrapping_add(hashed), 0)
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn counter(&self) -> i32 {
        self.counter
    }

    fn initialize_from_seed(&mut self, seed: i32) {
        // .NET's `Math.Abs(int.MinValue)` would throw; the BCL pre-handles it.
        let subtraction = if seed == i32::MIN {
            i32::MAX
        } else {
            seed.wrapping_abs()
        };
        let mut mj = MSEED.wrapping_sub(subtraction);
        let mut mk: i32 = 1;
        self.seed_array[55] = mj;
        for i in 1..55i32 {
            let idx = ((21 * i) % 55) as usize;
            self.seed_array[idx] = mk;
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk = mk.wrapping_add(MBIG);
            }
            mj = self.seed_array[idx];
        }
        for _ in 1..5 {
            for k in 1..56usize {
                let mut n = k as i32 + 30;
                if n >= 55 {
                    n -= 55;
                }
                let np = 1 + n as usize;
                self.seed_array[k] = self.seed_array[k].wrapping_sub(self.seed_array[np]);
                if self.seed_array[k] < 0 {
                    self.seed_array[k] = self.seed_array[k].wrapping_add(MBIG);
                }
            }
        }
        self.inext = 0;
        self.inextp = 21;
    }

    /// Returns a non-negative i32 in `[0, i32::MAX)`. Equivalent to .NET's
    /// `Random.Next()` (no-arg) and `Random.InternalSample()`. Advances the
    /// PRNG state by exactly one step.
    fn internal_sample(&mut self) -> i32 {
        let mut next_i = self.inext + 1;
        let mut next_ip = self.inextp + 1;
        if next_i >= 56 {
            next_i = 1;
        }
        if next_ip >= 56 {
            next_ip = 1;
        }
        let mut diff =
            self.seed_array[next_i].wrapping_sub(self.seed_array[next_ip]);
        if diff == i32::MAX {
            diff -= 1;
        }
        if diff < 0 {
            diff = diff.wrapping_add(MBIG);
        }
        self.seed_array[next_i] = diff;
        self.inext = next_i;
        self.inextp = next_ip;
        diff
    }

    /// Returns a `f64` in `[0.0, 1.0)`. Equivalent to .NET's `Random.Sample()`
    /// and `Random.NextDouble()`. Advances one step.
    fn sample(&mut self) -> f64 {
        self.internal_sample() as f64 * (1.0 / MBIG as f64)
    }

    /// Advances the internal PRNG by `target_count - counter` raw
    /// `InternalSample` steps. Mirrors `Rng.FastForwardCounter` in the
    /// decompile: panics if asked to rewind.
    pub fn fast_forward_counter(&mut self, target_count: i32) {
        if self.counter > target_count {
            panic!(
                "Cannot fast-forward an Rng counter to a lower number \
                 (current = {}, target = {})",
                self.counter, target_count
            );
        }
        while self.counter < target_count {
            self.counter += 1;
            self.internal_sample();
        }
    }

    pub fn next_bool(&mut self) -> bool {
        self.counter += 1;
        self.next_int_raw(2) == 0
    }

    /// `Rng.NextInt(int maxExclusive)` — wraps `_random.Next(maxExclusive)`.
    /// .NET's `Next(int)` is `(int)(Sample() * maxExclusive)`.
    pub fn next_int(&mut self, max_exclusive: i32) -> i32 {
        if max_exclusive < 0 {
            panic!("max_exclusive must be >= 0, got {max_exclusive}");
        }
        self.counter += 1;
        self.next_int_raw(max_exclusive)
    }

    fn next_int_raw(&mut self, max_exclusive: i32) -> i32 {
        (self.sample() * max_exclusive as f64) as i32
    }

    /// `Rng.NextInt(int min, int max)` — wraps `_random.Next(min, max)`.
    /// For ranges that fit in i32 this is `(int)(Sample() * range) + min`;
    /// the large-range branch (range > i32::MAX) is unused in STS2 so we
    /// match only the small-range path and panic otherwise.
    pub fn next_int_range(&mut self, min_inclusive: i32, max_exclusive: i32) -> i32 {
        if min_inclusive >= max_exclusive {
            panic!(
                "Minimum must be lower than maximum (got {min_inclusive}, {max_exclusive})"
            );
        }
        self.counter += 1;
        let range = (max_exclusive as i64) - (min_inclusive as i64);
        if range > i32::MAX as i64 {
            panic!("ranges wider than i32::MAX not supported (got {range})");
        }
        (self.sample() * range as f64) as i32 + min_inclusive
    }

    /// `Rng.NextDouble()` — returns a `f64` in `[0.0, 1.0)`.
    pub fn next_double(&mut self) -> f64 {
        self.counter += 1;
        self.sample()
    }

    /// `Rng.NextDouble(double min, double max)`.
    pub fn next_double_range(&mut self, min: f64, max: f64) -> f64 {
        if min > max {
            panic!("Minimum must not be higher than maximum (got {min}, {max})");
        }
        self.counter += 1;
        self.sample() * (max - min) + min
    }

    /// `Rng.NextFloat(float max)` — delegates to `NextFloat(0, max)`.
    pub fn next_float(&mut self, max: f32) -> f32 {
        self.next_float_range(0.0, max)
    }

    /// `Rng.NextFloat(float min, float max)`. C# computes the result as
    /// `(float)(NextDouble() * (max - min) + min)`. The (max-min) subtraction
    /// happens in single precision before being promoted to double — preserve
    /// that ordering exactly.
    pub fn next_float_range(&mut self, min: f32, max: f32) -> f32 {
        if min > max {
            panic!("Minimum must not be higher than maximum (got {min}, {max})");
        }
        self.counter += 1;
        let s = self.sample();
        let span = (max - min) as f64;
        (s * span + min as f64) as f32
    }

    /// `Rng.NextUnsignedInt(uint max)` — delegates to (0, max).
    pub fn next_unsigned_int(&mut self, max_exclusive: u32) -> u32 {
        self.next_unsigned_int_range(0, max_exclusive)
    }

    /// `Rng.NextUnsignedInt(uint min, uint max)`. C# does
    /// `min + (uint)(NextDouble() * (double)(max - min))`. The `max - min` is
    /// uint arithmetic (wrapping), promoted to double for the multiply, then
    /// truncated back to uint.
    pub fn next_unsigned_int_range(
        &mut self,
        min_inclusive: u32,
        max_exclusive: u32,
    ) -> u32 {
        if min_inclusive >= max_exclusive {
            panic!(
                "Minimum must be lower than maximum (got {min_inclusive}, {max_exclusive})"
            );
        }
        self.counter += 1;
        let s = self.sample();
        let span = max_exclusive.wrapping_sub(min_inclusive) as f64;
        let offset = (s * span) as u32;
        min_inclusive.wrapping_add(offset)
    }

    /// `Rng.NextGaussianDouble(mean, stdDev, min, max)`. Box-Muller transform
    /// with a rejection loop until the standardized result lands in `[0, 1]`,
    /// then linearly scales to `[min, max]`. The MegaCrit counter is
    /// incremented exactly once per call regardless of how many rejection
    /// iterations occur.
    pub fn next_gaussian_double(
        &mut self,
        mean: f64,
        std_dev: f64,
        min: f64,
        max: f64,
    ) -> f64 {
        if min > max {
            panic!("Minimum must not be higher than maximum (got {min}, {max})");
        }
        self.counter += 1;
        // The C# source uses this literal; the constant is 2*pi rounded to f64.
        const TWO_PI: f64 = 6.283_185_307_179_586_2;
        let mut result;
        loop {
            let u1 = self.sample();
            let u2 = self.sample();
            let magnitude = (-2.0 * u1.ln()).sqrt();
            let angle = TWO_PI * u2;
            let z = magnitude * angle.cos();
            result = mean + z * std_dev;
            if result >= 0.0 && result <= 1.0 {
                break;
            }
        }
        result * (max - min) + min
    }

    /// `Rng.NextGaussianFloat(...)` — delegates to `NextGaussianDouble` then
    /// downcasts.
    pub fn next_gaussian_float(
        &mut self,
        mean: f32,
        std_dev: f32,
        min: f32,
        max: f32,
    ) -> f32 {
        self.next_gaussian_double(
            mean as f64,
            std_dev as f64,
            min as f64,
            max as f64,
        ) as f32
    }

    /// `Rng.NextGaussianInt(mean, stdDev, min, max)`. Quirks (preserved
    /// bug-for-bug): uses `Math.Sin` (not `Cos`), draws `1.0 - sample()`
    /// instead of `sample()`, banker's rounding (`Math.Round` default),
    /// and — unlike `NextGaussianDouble` — does NOT advance the MegaCrit
    /// counter even though it consumes PRNG state.
    pub fn next_gaussian_int(
        &mut self,
        mean: i32,
        std_dev: i32,
        min: i32,
        max: i32,
    ) -> i32 {
        const TWO_PI: f64 = 6.283_185_307_179_586_2;
        let mut result;
        loop {
            let u1 = 1.0 - self.sample();
            let u2 = 1.0 - self.sample();
            let magnitude = (-2.0 * u1.ln()).sqrt();
            let angle = TWO_PI * u2;
            let z = magnitude * angle.sin();
            let v = mean as f64 + std_dev as f64 * z;
            // C# `(int)Math.Round(double)` — banker's rounding, then
            // saturating cast (.NET 5+ semantics); Rust 1.45+ `as i32`
            // saturates to match.
            result = v.round_ties_even() as i32;
            if result >= min && result <= max {
                break;
            }
        }
        result
    }

    /// `Rng.NextItem<T>(IEnumerable<T>)`. Returns `None` on empty input
    /// (matching C#'s `return default(T)` when count == 0). Otherwise picks
    /// uniformly via `NextInt(0, items.len())`, which advances the counter
    /// by one.
    pub fn next_item<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let idx = self.next_int_range(0, items.len() as i32) as usize;
        Some(&items[idx])
    }

    /// `Rng.WeightedNextItem<T>(IEnumerable<T>, Func<T, float>)`. Calls
    /// `NextFloat(1f)` (which advances the counter once), then walks items in
    /// order subtracting weights in single-precision until the cumulative
    /// threshold is reached. Returns `None` if the threshold is never reached
    /// (matches C#'s `return default(T)` fallback).
    ///
    /// Weights are summed in f32 (matching LINQ's `Sum(Func<T, float>)` and
    /// the C# Rng's float arithmetic) — order-dependent round-off must match
    /// bit-exactly between implementations, so iteration order is the same on
    /// both sides.
    pub fn weighted_next_item<'a, T, F>(
        &mut self,
        items: &'a [T],
        weight_fn: F,
    ) -> Option<&'a T>
    where
        F: Fn(&T) -> f32,
    {
        let r = self.next_float(1.0);
        let total: f32 = items.iter().map(&weight_fn).sum();
        let mut threshold = r * total;
        for t in items {
            threshold -= weight_fn(t);
            if threshold <= 0.0 {
                return Some(t);
            }
        }
        None
    }

    /// In-place Fisher-Yates shuffle, matching `Rng.Shuffle<T>(IList<T>)` in
    /// the decompile. Note: each `next_int` advance increments the counter,
    /// so a shuffle of N items advances the counter by N-1.
    pub fn shuffle<T>(&mut self, list: &mut [T]) {
        let n = list.len();
        if n < 2 {
            return;
        }
        for i in (1..n).rev() {
            let j = self.next_int((i + 1) as i32) as usize;
            list.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handful of internal sanity checks that don't need the oracle. The
    /// real validation lives in the oracle-tests crate.
    #[test]
    fn fast_forward_advances_counter() {
        let mut r = Rng::new(42, 0);
        r.fast_forward_counter(100);
        assert_eq!(r.counter(), 100);
    }

    #[test]
    #[should_panic]
    fn fast_forward_cannot_rewind() {
        let mut r = Rng::new(42, 50);
        r.fast_forward_counter(10);
    }

    #[test]
    fn next_int_zero_returns_zero() {
        let mut r = Rng::new(42, 0);
        assert_eq!(r.next_int(0), 0);
        assert_eq!(r.counter(), 1);
    }

    #[test]
    fn shuffle_of_one_is_noop() {
        let mut r = Rng::new(42, 0);
        let mut v = vec![7];
        r.shuffle(&mut v);
        assert_eq!(v, vec![7]);
        assert_eq!(r.counter(), 0);
    }
}
