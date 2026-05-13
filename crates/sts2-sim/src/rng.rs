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
