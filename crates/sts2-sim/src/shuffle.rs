//! Port of `MegaCrit.Sts2.Core.Extensions.ListExtensions.StableShuffle` and
//! `UnstableShuffle`. `UnstableShuffle` is byte-for-byte the same Fisher-Yates
//! we already have in `Rng::shuffle`; we expose both names so call sites that
//! mirror the C# source read naturally.
//!
//! `StableShuffle` is the load-bearing one: it sorts the list by natural `Ord`
//! before shuffling. The game uses it whenever the input came from an
//! unordered container (typically a `HashSet`) — sorting first makes the
//! shuffle output independent of iteration order.

use crate::rng::Rng;

/// `list.UnstableShuffle(rng)` — Fisher-Yates from the back. Alias for
/// `Rng::shuffle` (already oracle-validated by the `shuffle_matches` test).
pub fn unstable_shuffle<T>(list: &mut [T], rng: &mut Rng) {
    rng.shuffle(list);
}

/// `list.StableShuffle(rng)` — sort by natural ordering, then Fisher-Yates.
pub fn stable_shuffle<T: Ord>(list: &mut [T], rng: &mut Rng) {
    list.sort();
    rng.shuffle(list);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_one_element_lists_are_noops() {
        let mut rng = Rng::new(42, 0);
        let mut empty: Vec<i32> = vec![];
        stable_shuffle(&mut empty, &mut rng);
        unstable_shuffle(&mut empty, &mut rng);
        assert!(empty.is_empty());

        let mut one = vec![7];
        stable_shuffle(&mut one, &mut rng);
        unstable_shuffle(&mut one, &mut rng);
        assert_eq!(one, vec![7]);
    }

    #[test]
    fn stable_shuffle_is_independent_of_initial_order() {
        let mut a = vec![3, 1, 2, 5, 4];
        let mut b = vec![5, 4, 3, 2, 1];
        let mut rng_a = Rng::new(99, 0);
        let mut rng_b = Rng::new(99, 0);
        stable_shuffle(&mut a, &mut rng_a);
        stable_shuffle(&mut b, &mut rng_b);
        assert_eq!(a, b, "stable_shuffle must yield identical output for permutations of the same set");
    }
}
