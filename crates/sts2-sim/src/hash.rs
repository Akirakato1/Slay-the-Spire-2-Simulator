//! Bit-exact port of `MegaCrit.Sts2.Core.Helpers.StringHelper.GetDeterministicHashCode`.
//!
//! This is the classic .NET Framework pre-randomization `string.GetHashCode`
//! algorithm — the game relies on it to derive sub-seeds from human-readable
//! stream names (e.g. "shuffle", "combat_card_generation"). Two seed walks
//! (offset by one UTF-16 code unit) are combined at the end via a magic
//! prime multiply.
//!
//! C# strings are UTF-16. To get bit-identical hashes for non-ASCII inputs,
//! we iterate UTF-16 code units, not UTF-8 bytes. For ASCII inputs the two
//! are equivalent.

/// Returns the same `int` (i32) value as
/// `MegaCrit.Sts2.Core.Helpers.StringHelper.GetDeterministicHashCode(str)`.
pub fn deterministic_hash_code(s: &str) -> i32 {
    let mut h1: i32 = 352_654_597;
    let mut h2: i32 = h1;
    let mut prev: Option<u16> = None;
    // encode_utf16 yields one or two u16 code units per char. We want to
    // process them in pairs (i, i+1), advancing by 2 like the C# loop's
    // `i += 2`. Buffer one unit at a time and flush in pairs.
    for unit in s.encode_utf16() {
        match prev {
            None => prev = Some(unit),
            Some(first) => {
                h1 = h1.wrapping_mul(33) ^ (first as i32);
                h2 = h2.wrapping_mul(33) ^ (unit as i32);
                prev = None;
            }
        }
    }
    if let Some(last) = prev {
        h1 = h1.wrapping_mul(33) ^ (last as i32);
    }
    h1.wrapping_add(h2.wrapping_mul(1_566_083_941))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_pure_constant() {
        // No loop iterations → h1 == h2 == initial, result = h1 + h2*1566083941
        let expected = 352_654_597i32
            .wrapping_add(352_654_597i32.wrapping_mul(1_566_083_941));
        assert_eq!(deterministic_hash_code(""), expected);
    }

    #[test]
    fn single_char_matches_one_walk() {
        // Only h1 advances; h2 stays at initial.
        let h1 = 352_654_597i32.wrapping_mul(33) ^ ('A' as i32);
        let expected = h1.wrapping_add(352_654_597i32.wrapping_mul(1_566_083_941));
        assert_eq!(deterministic_hash_code("A"), expected);
    }
}
