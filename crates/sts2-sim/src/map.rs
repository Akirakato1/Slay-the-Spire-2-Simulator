//! Map data types — coordinate, point types, point state, the per-node
//! MapPoint struct, and MapPointTypeCounts. Ports of the data classes in
//! `MegaCrit.Sts2.Core.Map.*` minus the engine glue (events, signals, UI
//! callbacks, AbstractModel quests — those will land when needed).
//!
//! Generation logic (StandardActMap, MapPathPruning, MapPostProcessing)
//! lives in later chunks of the map port.

use std::collections::HashSet;
use std::ops::Deref;

use crate::rng::Rng;

/// Insertion-ordered set of `MapCoord`s, used for `MapPoint`'s `parents`
/// and `children`. Mirrors how C#'s `HashSet<MapPoint>` iterates on small
/// sets that never rehash — the internal slots array is filled in
/// insertion order and iterated in that order, so Vec-insertion-order
/// here matches C# call-for-call. Deref makes the common immutable API
/// (iter, len, is_empty, contains, first, ...) work directly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoordSet(Vec<MapCoord>);

impl CoordSet {
    pub fn new() -> Self { Self(Vec::new()) }

    /// Insert if not already present. Returns true iff the coord was added.
    pub fn insert(&mut self, coord: MapCoord) -> bool {
        if self.0.contains(&coord) {
            false
        } else {
            self.0.push(coord);
            true
        }
    }

    /// Remove the coord if present (preserves the order of remaining
    /// elements). Returns true iff removed.
    pub fn remove(&mut self, coord: &MapCoord) -> bool {
        match self.0.iter().position(|c| c == coord) {
            Some(idx) => {
                self.0.remove(idx);
                true
            }
            None => false,
        }
    }
}

impl Deref for CoordSet {
    type Target = [MapCoord];
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<'a> IntoIterator for &'a CoordSet {
    type Item = &'a MapCoord;
    type IntoIter = std::slice::Iter<'a, MapCoord>;
    fn into_iter(self) -> Self::IntoIter { self.0.iter() }
}

impl FromIterator<MapCoord> for CoordSet {
    fn from_iter<T: IntoIterator<Item = MapCoord>>(iter: T) -> Self {
        let mut s = Self::new();
        for c in iter {
            s.insert(c);
        }
        s
    }
}

/// `MegaCrit.Sts2.Core.Map.MapCoord`. Plain `(col, row)` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapCoord {
    pub col: i32,
    pub row: i32,
}

impl MapCoord {
    pub fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }
}

/// `MegaCrit.Sts2.Core.Map.MapPointType`. Variant ordinals match the C# enum
/// — the game compares `PointType > Unassigned` ordinally in at least one
/// site (`MapPoint.IsDescendantPathSame`), so the order is load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(i32)]
pub enum MapPointType {
    Unassigned = 0,
    Unknown = 1,
    Shop = 2,
    Treasure = 3,
    RestSite = 4,
    Monster = 5,
    Elite = 6,
    Boss = 7,
    Ancient = 8,
}

impl MapPointType {
    /// Parse the lowercase snake_case form used in `.run` files
    /// (e.g. `"rest_site"` → `RestSite`). Returns `None` for unknown values.
    pub fn from_run_log_str(s: &str) -> Option<Self> {
        Some(match s {
            "unassigned" => MapPointType::Unassigned,
            "unknown" => MapPointType::Unknown,
            "shop" => MapPointType::Shop,
            "treasure" => MapPointType::Treasure,
            "rest_site" => MapPointType::RestSite,
            "monster" => MapPointType::Monster,
            "elite" => MapPointType::Elite,
            "boss" => MapPointType::Boss,
            "ancient" => MapPointType::Ancient,
            _ => return None,
        })
    }
}

/// `MegaCrit.Sts2.Core.Map.MapPointState`. Same ordinal mapping as the C# enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(i32)]
pub enum MapPointState {
    None = 0,
    Travelable = 1,
    Traveled = 2,
    Untravelable = 3,
}

/// `MegaCrit.Sts2.Core.Map.MapPoint`.
///
/// In C# the class uses `HashSet<MapPoint>` for parents/children with
/// default (reference) equality. Within a single `ActMap` every `(col, row)`
/// has exactly one `MapPoint`, so we represent parents/children by `MapCoord`
/// instead — that gives us value-based identity that survives clones.
///
/// We checked every call site that iterates `parents` / `children` in
/// `StandardActMap.cs`: each one uses `.Contains()`, `.Any()`, or early-
/// return — i.e. iteration order does not affect output. So using
/// `HashSet<MapCoord>` here is safe.
///
/// `_quests: List<AbstractModel>` and `NodeMarkedChanged` event from the C#
/// class are intentionally omitted; quests land with the Quests port and
/// the event is a Godot UI hook we don't need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapPoint {
    pub coord: MapCoord,
    pub point_type: MapPointType,
    pub can_be_modified: bool,
    pub parents: CoordSet,
    pub children: CoordSet,
}

impl MapPoint {
    pub fn new(col: i32, row: i32) -> Self {
        Self {
            coord: MapCoord::new(col, row),
            point_type: MapPointType::Unassigned,
            can_be_modified: true,
            parents: CoordSet::new(),
            children: CoordSet::new(),
        }
    }

    /// Records `child_coord` as a child of this point. The caller is
    /// responsible for adding `self.coord` to the child's `parents` (mirrors
    /// the bidirectional bookkeeping of C#'s `AddChildPoint`).
    pub fn add_child(&mut self, child_coord: MapCoord) {
        self.children.insert(child_coord);
    }

    /// True iff `sibling.col == self.col - 1` (mirrors `IsAdjacentLeft`).
    pub fn is_adjacent_left(&self, sibling: &MapPoint) -> bool {
        self.coord.col - 1 == sibling.coord.col
    }

    /// True iff `sibling.col == self.col + 1` (mirrors `IsAdjacentRight`).
    pub fn is_adjacent_right(&self, sibling: &MapPoint) -> bool {
        self.coord.col + 1 == sibling.coord.col
    }
}

/// `MegaCrit.Sts2.Core.Map.MapPointTypeCounts`. Per-act target counts of
/// "soft" point types to scatter through the map. `NumOfElites` defaults
/// to 5 here; the C# class multiplies by 1.6 when the SwarmingElites
/// ascension is active — that ascension scaling will be wired in when the
/// ascension subsystem is ported.
#[derive(Debug, Clone)]
pub struct MapPointTypeCounts {
    pub point_types_that_ignore_rules: HashSet<MapPointType>,
    pub num_of_elites: i32,
    pub num_of_shops: i32,
    pub num_of_unknowns: i32,
    pub num_of_rests: i32,
}

impl MapPointTypeCounts {
    /// Mirrors the C# `new MapPointTypeCounts(int unknownCount, int restCount)`
    /// with the post-construction defaults: 5 elites, 3 shops, empty
    /// ignored-rules set.
    pub fn new(unknown_count: i32, rest_count: i32) -> Self {
        Self {
            point_types_that_ignore_rules: HashSet::new(),
            num_of_elites: 5,
            num_of_shops: 3,
            num_of_unknowns: unknown_count,
            num_of_rests: rest_count,
        }
    }

    /// `MapPointTypeCounts.StandardRandomUnknownCount(Rng)`. Returns
    /// `rng.NextGaussianInt(12, 1, 10, 14)` — Gaussian-distributed integer
    /// in [10, 14] around 12 with stdev 1. Used to vary the unknown-count
    /// across runs.
    pub fn standard_random_unknown_count(rng: &mut Rng) -> i32 {
        rng.next_gaussian_int(12, 1, 10, 14)
    }

    pub fn should_ignore_map_point_rules_for_map_point_type(
        &self,
        point_type: MapPointType,
    ) -> bool {
        self.point_types_that_ignore_rules.contains(&point_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coord_equality_and_hash_treat_field_values_as_identity() {
        let a = MapCoord::new(3, 4);
        let b = MapCoord::new(3, 4);
        let c = MapCoord::new(4, 3);
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set: HashSet<MapCoord> = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b), "MapCoord hash/eq treat field values as identity");
        assert!(!set.contains(&c));
    }

    #[test]
    fn map_point_type_ordinals_match_csharp_enum() {
        // Load-bearing because StandardActMap compares `PointType > Unassigned`.
        assert_eq!(MapPointType::Unassigned as i32, 0);
        assert_eq!(MapPointType::Unknown as i32, 1);
        assert_eq!(MapPointType::Shop as i32, 2);
        assert_eq!(MapPointType::Treasure as i32, 3);
        assert_eq!(MapPointType::RestSite as i32, 4);
        assert_eq!(MapPointType::Monster as i32, 5);
        assert_eq!(MapPointType::Elite as i32, 6);
        assert_eq!(MapPointType::Boss as i32, 7);
        assert_eq!(MapPointType::Ancient as i32, 8);
    }

    #[test]
    fn map_point_new_starts_unassigned_and_modifiable() {
        let p = MapPoint::new(2, 7);
        assert_eq!(p.coord, MapCoord::new(2, 7));
        assert_eq!(p.point_type, MapPointType::Unassigned);
        assert!(p.can_be_modified);
        assert!(p.parents.is_empty());
        assert!(p.children.is_empty());
    }

    #[test]
    fn map_point_adjacency_helpers() {
        let center = MapPoint::new(3, 5);
        let left = MapPoint::new(2, 5);
        let right = MapPoint::new(4, 5);
        let far = MapPoint::new(5, 5);
        assert!(center.is_adjacent_left(&left));
        assert!(!center.is_adjacent_left(&right));
        assert!(center.is_adjacent_right(&right));
        assert!(!center.is_adjacent_right(&far));
    }

    #[test]
    fn standard_random_unknown_count_in_range() {
        // Gaussian rejection sampler returns ints in [10, 14] inclusive.
        let mut rng = Rng::new(42, 0);
        for _ in 0..200 {
            let v = MapPointTypeCounts::standard_random_unknown_count(&mut rng);
            assert!((10..=14).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn point_type_counts_defaults() {
        let c = MapPointTypeCounts::new(12, 4);
        assert_eq!(c.num_of_unknowns, 12);
        assert_eq!(c.num_of_rests, 4);
        assert_eq!(c.num_of_shops, 3);
        assert_eq!(c.num_of_elites, 5);
        assert!(c.point_types_that_ignore_rules.is_empty());
        assert!(!c.should_ignore_map_point_rules_for_map_point_type(MapPointType::Monster));
    }
}
