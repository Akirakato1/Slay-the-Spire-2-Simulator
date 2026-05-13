//! Port of `MegaCrit.Sts2.Core.Map.MapPathPruning`.
//!
//! Algorithm (high level):
//! - `prune_and_repair` runs up to 3 iterations of:
//!   - `prune_duplicate_segments`: find paths that share start/end/types
//!     fingerprints, then prune (or break parent-child relationships in)
//!     all but one of each match group.
//!   - `repair_pruned_point_types`: top up Shop / Elite / RestSite / Unknown
//!     to their target counts by promoting Monsters in spots where the
//!     type's validity rules are satisfied.
//!   - early exit if no repair was made.
//!
//! HashSet iteration order caveat: C# `MapPoint.Children` is
//! `HashSet<MapPoint>` with reference equality, so iteration order in
//! `find_all_paths` is bucket-order (effectively random). The Rust port
//! uses `HashSet<MapCoord>` whose iteration order is also non-deterministic
//! but DIFFERENT from C#. If the oracle diff tests reveal a divergence on
//! pruning, the fix is to switch `MapPoint.{parents,children}` to an
//! insertion-ordered container.

use std::collections::BTreeMap;

use crate::map::{MapCoord, MapPointType};
use crate::shuffle::{stable_shuffle, unstable_shuffle};
use crate::standard_act_map::StandardActMap;

/// Top-level entry point. Mirrors `MapPathPruning.PruneAndRepair`.
pub fn prune_and_repair(sam: &mut StandardActMap) {
    for _ in 0..3 {
        prune_duplicate_segments(sam);
        if !repair_pruned_point_types(sam) {
            break;
        }
    }
}

fn repair_pruned_point_types(sam: &mut StandardActMap) -> bool {
    let counts = sam.point_type_counts_clone();
    let mut any = false;
    any |= repair_point_type(sam, MapPointType::Shop, counts.num_of_shops);
    any |= repair_point_type(sam, MapPointType::Elite, counts.num_of_elites);
    any |= repair_point_type(sam, MapPointType::RestSite, counts.num_of_rests);
    any |= repair_point_type(sam, MapPointType::Unknown, counts.num_of_unknowns);
    any
}

fn repair_point_type(
    sam: &mut StandardActMap,
    target_type: MapPointType,
    target_count: i32,
) -> bool {
    let current = sam
        .iter_grid_points()
        .filter(|p| p.point_type == target_type)
        .count() as i32;
    let mut needed = target_count - current;
    if needed <= 0 {
        return false;
    }

    let mut candidates: Vec<MapCoord> = sam
        .iter_grid_points()
        .filter(|p| p.point_type == MapPointType::Monster && p.can_be_modified)
        .map(|p| p.coord)
        .collect();
    sam.with_rng(|rng| stable_shuffle(&mut candidates, rng));

    let mut changed = false;
    for coord in candidates {
        if needed == 0 {
            break;
        }
        let valid = match sam.get_point(coord.col, coord.row) {
            Some(p) => sam.is_valid_point_type(target_type, p),
            None => false,
        };
        if valid {
            if let Some(p) = sam.get_point_mut_pub(coord.col, coord.row) {
                p.point_type = target_type;
                needed -= 1;
                changed = true;
            }
        }
    }
    changed
}

fn prune_duplicate_segments(sam: &mut StandardActMap) {
    let mut iterations = 0;
    let starting = sam.starting().coord;
    let mut matching = find_matching_segments(sam, starting);
    while prune_paths(sam, &matching) {
        iterations += 1;
        if iterations > 50 {
            panic!(
                "Unable to prune matching segments in {iterations} iterations"
            );
        }
        matching = find_matching_segments(sam, starting);
    }
}

fn find_matching_segments(
    sam: &StandardActMap,
    starting: MapCoord,
) -> Vec<Vec<Vec<MapCoord>>> {
    let all_paths = find_all_paths(sam, starting);
    // SortedDictionary<string, ...> -> BTreeMap to keep key order deterministic
    // independent of HashMap randomness.
    let mut segments: BTreeMap<String, Vec<Vec<MapCoord>>> = BTreeMap::new();
    for path in &all_paths {
        add_segments_to_dictionary(sam, path, &mut segments);
    }
    segments.into_values().filter(|v| v.len() > 1).collect()
}

/// DFS from `current` down through children to Boss. Returns every path as
/// a Vec of MapCoords (in order).
fn find_all_paths(sam: &StandardActMap, current: MapCoord) -> Vec<Vec<MapCoord>> {
    let mut paths: Vec<Vec<MapCoord>> = Vec::new();
    let Some(point) = sam.get_point(current.col, current.row) else {
        return paths;
    };
    if point.point_type == MapPointType::Boss {
        paths.push(vec![current]);
        return paths;
    }
    // Iteration over Children is HashSet-ordered; document the caveat in
    // the module doc.
    let children: Vec<MapCoord> = point.children.iter().copied().collect();
    for child in children {
        let sub = find_all_paths(sam, child);
        for sub_path in sub {
            let mut p = Vec::with_capacity(sub_path.len() + 1);
            p.push(current);
            p.extend(sub_path);
            paths.push(p);
        }
    }
    paths
}

fn add_segments_to_dictionary(
    sam: &StandardActMap,
    path: &[MapCoord],
    segments: &mut BTreeMap<String, Vec<Vec<MapCoord>>>,
) {
    for i in 0..path.len().saturating_sub(1) {
        if !is_valid_segment_start_map_point(sam, path[i]) {
            continue;
        }
        for j in 2..(path.len() - i) {
            let end_coord = path[i + j];
            if !is_valid_segment_end_map_point(sam, end_coord) {
                continue;
            }
            let segment: Vec<MapCoord> = path[i..=i + j].to_vec();
            let key = generate_segment_key(sam, &segment);
            match segments.get_mut(&key) {
                None => {
                    segments.insert(key, vec![segment]);
                }
                Some(existing) => {
                    if !any_overlapping_segments(existing, &segment) {
                        existing.push(segment);
                    }
                }
            }
        }
    }
}

fn is_valid_segment_start_map_point(sam: &StandardActMap, coord: MapCoord) -> bool {
    let Some(p) = sam.get_point(coord.col, coord.row) else {
        return false;
    };
    p.children.len() > 1 || p.coord.row == 0
}

fn is_valid_segment_end_map_point(sam: &StandardActMap, coord: MapCoord) -> bool {
    let Some(p) = sam.get_point(coord.col, coord.row) else {
        return false;
    };
    p.parents.len() >= 2
}

fn generate_segment_key(sam: &StandardActMap, segment: &[MapCoord]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let start = segment[0];
    let end = segment[segment.len() - 1];
    if start.row == 0 {
        // C# format: "{start.row}-{end.col},{end.row}-"
        let _ = write!(s, "{}-{},{}-", start.row, end.col, end.row);
    } else {
        // "{start.col},{start.row}-{end.col},{end.row}-"
        let _ = write!(
            s,
            "{},{}-{},{}-",
            start.col, start.row, end.col, end.row
        );
    }
    // Append point-types comma-joined.
    for (idx, c) in segment.iter().enumerate() {
        if idx > 0 {
            s.push(',');
        }
        let pt = sam
            .get_point(c.col, c.row)
            .map(|p| p.point_type as i32)
            .unwrap_or(MapPointType::Unassigned as i32);
        let _ = write!(s, "{}", pt);
    }
    s
}

fn any_overlapping_segments(existing: &[Vec<MapCoord>], segment: &[MapCoord]) -> bool {
    existing.iter().any(|e| overlapping_segment(e, segment))
}

fn overlapping_segment(a: &[MapCoord], b: &[MapCoord]) -> bool {
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    // C#: for i in 1..=a.Count-2: if a[i] == b[i]
    let upper = a.len().saturating_sub(2);
    for i in 1..=upper {
        if i >= b.len() {
            break;
        }
        if a[i] == b[i] {
            return true;
        }
    }
    false
}

fn prune_paths(
    sam: &mut StandardActMap,
    matching: &[Vec<Vec<MapCoord>>],
) -> bool {
    for group in matching {
        let mut group = group.clone();
        sam.with_rng(|rng| unstable_shuffle(&mut group, rng));
        let pruned = prune_all_but_last(sam, &group);
        if pruned != 0 {
            return true;
        }
        if break_a_parent_child_relationship_in_any_segment(sam, &group) {
            return true;
        }
    }
    false
}

fn prune_all_but_last(
    sam: &mut StandardActMap,
    matches: &[Vec<MapCoord>],
) -> i32 {
    let mut count = 0i32;
    for segment in matches {
        if count == matches.len() as i32 - 1 {
            return count;
        }
        if prune_segment(sam, segment) {
            count += 1;
        }
    }
    count
}

fn prune_segment(sam: &mut StandardActMap, segment: &[MapCoord]) -> bool {
    let mut flag = false;
    for i in 0..segment.len().saturating_sub(1) {
        let coord = segment[i];
        if !sam.is_in_map(coord) {
            return true;
        }
        let Some(point) = sam.get_point(coord.col, coord.row) else {
            continue;
        };
        if point.children.len() > 1 || point.parents.len() > 1 {
            continue;
        }
        // Snapshot info we need before mutating.
        let parent_coords: Vec<MapCoord> = point.parents.iter().copied().collect();
        let child_coords: Vec<MapCoord> = point.children.iter().copied().collect();

        // C#: parents.Any(p => p.Children.Count == 1 && !IsRemoved(grid, p))
        let any_solo_parent_in_map = parent_coords.iter().any(|pc| {
            sam.get_point(pc.col, pc.row)
                .map(|p| p.children.len() == 1 && !sam.is_removed(*pc))
                .unwrap_or(false)
        });
        if any_solo_parent_in_map {
            continue;
        }

        // C#: segment.Skip(i).ToArray() — points from i onwards.
        // !array.Any(n => n.Children.Count > 1 && n.parents.Count == 1)
        let any_branch_with_single_parent = segment[i..].iter().any(|c| {
            sam.get_point(c.col, c.row)
                .map(|p| p.children.len() > 1 && p.parents.len() == 1)
                .unwrap_or(false)
        });
        if any_branch_with_single_parent {
            continue;
        }

        // C#: segment[segment.Length-1].parents.Count == 1  → return false
        let last_coord = segment[segment.len() - 1];
        let last_parent_count = sam
            .get_point(last_coord.col, last_coord.row)
            .map(|p| p.parents.len())
            .unwrap_or(0);
        if last_parent_count == 1 {
            return false;
        }

        // C#: children.Where(c => !segment.Contains(c)).Any(c => c.parents.Count == 1)
        let any_external_child_with_single_parent = child_coords.iter().any(|cc| {
            if segment.contains(cc) {
                return false;
            }
            sam.get_point(cc.col, cc.row)
                .map(|p| p.parents.len() == 1)
                .unwrap_or(false)
        });
        if any_external_child_with_single_parent {
            continue;
        }

        sam.remove_point(coord);
        flag = true;
    }
    flag
}

fn break_a_parent_child_relationship_in_any_segment(
    sam: &mut StandardActMap,
    matches: &[Vec<MapCoord>],
) -> bool {
    for segment in matches {
        if break_a_parent_child_relationship_in_segment(sam, segment) {
            return true;
        }
    }
    false
}

fn break_a_parent_child_relationship_in_segment(
    sam: &mut StandardActMap,
    segment: &[MapCoord],
) -> bool {
    let mut flag = false;
    for i in 0..segment.len().saturating_sub(1) {
        let parent_coord = segment[i];
        let child_coord = segment[i + 1];
        let (parent_has_multi_children, child_has_non_single_parents) = {
            let parent = sam.get_point(parent_coord.col, parent_coord.row);
            let child = sam.get_point(child_coord.col, child_coord.row);
            (
                parent.map(|p| p.children.len() >= 2).unwrap_or(false),
                child.map(|c| c.parents.len() != 1).unwrap_or(false),
            )
        };
        if parent_has_multi_children && child_has_non_single_parents {
            sam.remove_child_link(parent_coord, child_coord);
            flag = true;
        }
    }
    flag
}
