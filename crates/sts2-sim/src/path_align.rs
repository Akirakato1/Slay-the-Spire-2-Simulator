//! Align a `.run` file's visited-node sequence onto a generated
//! `StandardActMap`. The `.run` file gives only ordered
//! `map_point_type` strings — no coordinates — so we walk the graph
//! from the starting point matching each entry to a child of the
//! current node.
//!
//! Port of `dashboard/scripts/mapgen/path_align.js`. Both this and the
//! JS port are pure functions of the graph + visited types; either
//! version's output is canonical.
//!
//! Used by:
//!   - `.run` file replay harness (Phase 0.4 validation) — translates
//!     a real player's path into graph coordinates so per-room state
//!     reconstruction can replay each combat / event correctly.
//!   - User-facing analysis tool — same translation feeds per-decision
//!     inference.

use crate::map::{MapCoord, MapPointType};
use crate::standard_act_map::StandardActMap;

/// Successful alignment: ordered sequence of graph coords matching the
/// visited list. `ambiguous` is true if more than one valid path
/// existed (we return the first; downstream tooling can flag).
#[derive(Debug, Clone)]
pub struct AlignedPath {
    pub coords: Vec<MapCoord>,
    pub ambiguous: bool,
}

/// Failure reason. Distinct enum variants instead of strings so the
/// caller can branch on them (e.g. allow partial paths for runs that
/// ended early).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlignError {
    /// First entry was not an Ancient (start node).
    StartNotAncient(Option<String>),
    /// Last entry was not a Boss (only relevant if `allow_partial=false`).
    EndNotBoss(Option<String>),
    /// No DFS-reachable path through the graph matched all visited types.
    NoMatchingPath { entries: usize },
    /// `.run` file's `map_point_type` value isn't one we recognize.
    UnknownPointType(String),
}

/// A visited entry from the .run file. Free-form so the caller can
/// construct from whatever parser shape they have.
#[derive(Debug, Clone)]
pub struct VisitedEntry<'a> {
    pub map_point_type: &'a str,
    /// First room id, if any (used for Neow detection — Ancient is
    /// logged as "event" + model_id="EVENT.NEOW" in some schema
    /// versions). Pass None if you don't have it.
    pub first_room_model_id: Option<&'a str>,
}

/// Convert a `.run` file `map_point_type` string into our enum.
/// Returns None for unknown strings — callers decide whether that's
/// fatal (use `Err(AlignError::UnknownPointType)`).
pub fn point_type_from_run_log(entry: &VisitedEntry<'_>) -> Option<MapPointType> {
    // Map "event" + EVENT.NEOW → Ancient for the start-row match
    // (covers older .run schema versions).
    if entry.map_point_type == "event"
        && entry.first_room_model_id == Some("EVENT.NEOW")
    {
        return Some(MapPointType::Ancient);
    }
    match entry.map_point_type {
        "unknown" => Some(MapPointType::Unknown),
        "shop" => Some(MapPointType::Shop),
        "treasure" => Some(MapPointType::Treasure),
        "rest_site" => Some(MapPointType::RestSite),
        "monster" => Some(MapPointType::Monster),
        "elite" => Some(MapPointType::Elite),
        "boss" => Some(MapPointType::Boss),
        "ancient" => Some(MapPointType::Ancient),
        _ => None,
    }
}

/// Align a visited node sequence to graph coordinates. `allow_partial`
/// permits the last entry to be non-Boss (used when the run ended
/// before reaching the boss row).
pub fn align_path(
    map: &StandardActMap,
    visited: &[VisitedEntry<'_>],
    allow_partial: bool,
) -> Result<AlignedPath, AlignError> {
    // Translate visited entries to types up front; surface unknown
    // strings immediately.
    let mut types: Vec<MapPointType> = Vec::with_capacity(visited.len());
    for entry in visited {
        let t = point_type_from_run_log(entry)
            .ok_or_else(|| AlignError::UnknownPointType(entry.map_point_type.to_string()))?;
        types.push(t);
    }
    if types.is_empty() {
        return Err(AlignError::NoMatchingPath { entries: 0 });
    }

    // Sanity-check bookends.
    if types[0] != MapPointType::Ancient {
        return Err(AlignError::StartNotAncient(
            visited.first().map(|e| e.map_point_type.to_string()),
        ));
    }
    if !allow_partial && *types.last().unwrap() != MapPointType::Boss {
        return Err(AlignError::EndNotBoss(
            visited.last().map(|e| e.map_point_type.to_string()),
        ));
    }

    // DFS from the starting point. Caps solution count at 2 so we can
    // flag ambiguity without exploring the whole space.
    let start = map.starting().coord;
    let mut path = vec![start];
    let mut first_solution: Option<Vec<MapCoord>> = None;
    let mut solutions = 0usize;
    recurse(map, &types, start, 1, &mut path, &mut first_solution, &mut solutions);

    match first_solution {
        None => Err(AlignError::NoMatchingPath {
            entries: types.len(),
        }),
        Some(coords) => Ok(AlignedPath {
            coords,
            ambiguous: solutions > 1,
        }),
    }
}

fn recurse(
    map: &StandardActMap,
    types: &[MapPointType],
    current: MapCoord,
    idx: usize,
    path: &mut Vec<MapCoord>,
    first_solution: &mut Option<Vec<MapCoord>>,
    solutions: &mut usize,
) {
    if *solutions >= 2 {
        return;
    }
    if idx == types.len() {
        *solutions += 1;
        if first_solution.is_none() {
            *first_solution = Some(path.clone());
        }
        return;
    }
    let wanted = types[idx];
    let Some(node) = map.get_point(current.col, current.row) else {
        return;
    };
    // Snapshot children to avoid borrow issues during recursion.
    let children: Vec<MapCoord> = node.children.iter().copied().collect();
    for child_coord in children {
        let Some(child) = map.get_point(child_coord.col, child_coord.row) else {
            continue;
        };
        if child.point_type != wanted {
            continue;
        }
        path.push(child_coord);
        recurse(map, types, child_coord, idx + 1, path, first_solution, solutions);
        path.pop();
        if *solutions >= 2 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::Overgrowth;
    use crate::rng::Rng;

    fn fixed_map() -> StandardActMap {
        // Same seed as the bounds test in standard_act_map — gives a
        // deterministic Overgrowth layout.
        StandardActMap::new(
            Rng::new(12345, 0),
            &Overgrowth,
            false, false, false,
            None,
            true,  // enable pruning to match a realistic .run topology
            0,
        )
    }

    #[test]
    fn unknown_run_type_errors() {
        let map = fixed_map();
        let visited = vec![VisitedEntry {
            map_point_type: "lava",
            first_room_model_id: None,
        }];
        let err = align_path(&map, &visited, false).unwrap_err();
        assert!(matches!(err, AlignError::UnknownPointType(ref s) if s == "lava"));
    }

    #[test]
    fn missing_ancient_first_errors() {
        let map = fixed_map();
        let visited = vec![VisitedEntry {
            map_point_type: "monster",
            first_room_model_id: None,
        }];
        let err = align_path(&map, &visited, false).unwrap_err();
        assert!(matches!(err, AlignError::StartNotAncient(_)));
    }

    #[test]
    fn neow_event_at_start_is_ancient() {
        let map = fixed_map();
        // Single-entry visited starting with Neow's room — should pass
        // the bookend check but fail later for not reaching boss
        // (which is fine for testing the conversion).
        let visited = vec![VisitedEntry {
            map_point_type: "event",
            first_room_model_id: Some("EVENT.NEOW"),
        }];
        let err = align_path(&map, &visited, false).unwrap_err();
        // Must NOT be StartNotAncient — should be EndNotBoss instead.
        assert!(matches!(err, AlignError::EndNotBoss(_)),
            "expected EndNotBoss, got {:?}", err);
    }

    #[test]
    fn synthetic_full_path_aligns() {
        let map = fixed_map();
        // Walk a path through the graph by always taking the first
        // child, collect the types, then align — must round-trip.
        let mut cur = map.starting().coord;
        let mut visited_types: Vec<MapPointType> =
            vec![MapPointType::Ancient];
        loop {
            let Some(node) = map.get_point(cur.col, cur.row) else { break };
            if node.children.is_empty() { break; }
            // First child in insertion order.
            let next = *node.children.iter().next().unwrap();
            let next_node = map.get_point(next.col, next.row).unwrap();
            visited_types.push(next_node.point_type);
            cur = next;
            if next_node.point_type == MapPointType::Boss { break; }
        }
        // Build VisitedEntry list from types.
        let type_to_str = |t: MapPointType| -> &'static str {
            match t {
                MapPointType::Ancient => "ancient",
                MapPointType::Monster => "monster",
                MapPointType::Unknown => "unknown",
                MapPointType::Shop => "shop",
                MapPointType::Treasure => "treasure",
                MapPointType::RestSite => "rest_site",
                MapPointType::Elite => "elite",
                MapPointType::Boss => "boss",
                MapPointType::Unassigned => "unassigned",
            }
        };
        let visited: Vec<VisitedEntry> = visited_types
            .iter()
            .map(|&t| VisitedEntry {
                map_point_type: type_to_str(t),
                first_room_model_id: None,
            })
            .collect();
        let aligned = align_path(&map, &visited, false)
            .expect("synthetic path must align");
        assert_eq!(aligned.coords.len(), visited.len());
        assert_eq!(aligned.coords[0], map.starting().coord);
    }
}
