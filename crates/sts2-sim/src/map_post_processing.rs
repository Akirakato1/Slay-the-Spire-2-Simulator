//! Port of `MegaCrit.Sts2.Core.Map.MapPostProcessing`. Three functions, all
//! pure transforms on the grid (no Rng):
//!
//! - `center_grid`: shift the whole grid left or right by one column when
//!   one side is empty and the other isn't.
//! - `spread_adjacent_map_points`: for each row, repeatedly move points to
//!   maximize minimum inter-point gap, subject to staying within ±1 column
//!   of each parent and each child.
//! - `straighten_paths`: bias points with exactly one parent + one child
//!   towards being between the two columnwise.
//!
//! The grid is `Vec<Vec<Option<MapPoint>>>` indexed as `[col][row]`.
//!
//! **Note on `spread_adjacent_map_points`**: the C# version iterates
//! `HashSet<int> allowedPositions`, and ties in the gap metric are
//! resolved by iteration order. We iterate ascending-column. If a real
//! divergence appears in oracle diff tests we may need to reproduce
//! .NET's HashSet<int> iteration order exactly.

use std::collections::HashSet;

use crate::map::{MapCoord, MapPoint};

pub type Grid = Vec<Vec<Option<MapPoint>>>;

pub fn center_grid(grid: &mut Grid, cols: i32, rows: i32) {
    let left_empty = is_column_empty(grid, 0, rows) && is_column_empty(grid, 1, rows);
    let right_empty = is_column_empty(grid, cols - 1, rows)
        && is_column_empty(grid, cols - 2, rows);
    let shift: i32 = match (left_empty, right_empty) {
        (true, false) => -1,
        (false, true) => 1,
        _ => 0,
    };
    if shift == 0 {
        return;
    }
    if shift > 0 {
        // Shift right: iterate columns from rightmost to leftmost so we
        // don't overwrite unread cells.
        for i in 0..rows as usize {
            for j in (0..cols).rev() {
                let mut moved = grid[j as usize][i].take();
                let dest = j + shift;
                if dest < cols {
                    if let Some(p) = moved.as_mut() {
                        p.coord.col = dest;
                    }
                    grid[dest as usize][i] = moved;
                }
            }
        }
    } else {
        // Shift left.
        for i in 0..rows as usize {
            for j in 0..cols {
                let mut moved = grid[j as usize][i].take();
                let dest = j + shift;
                if dest >= 0 {
                    if let Some(p) = moved.as_mut() {
                        p.coord.col = dest;
                    }
                    grid[dest as usize][i] = moved;
                }
            }
        }
    }
}

fn is_column_empty(grid: &Grid, col: i32, rows: i32) -> bool {
    if col < 0 || col >= grid.len() as i32 {
        return true;
    }
    (0..rows as usize).all(|r| grid[col as usize][r].is_none())
}

pub fn straighten_paths(grid: &mut Grid, cols: i32, rows: i32) {
    for i in 0..rows as usize {
        for j in 0..cols {
            // Re-fetch each iteration because we may have moved a point.
            let Some(here) = grid[j as usize][i].as_ref() else { continue };
            if here.parents.len() != 1 || here.children.len() != 1 {
                continue;
            }
            // C# uses .First() on parents/children — guaranteed to have
            // exactly one element by the check above. We just take any.
            let parent_col = here.parents.iter().next().unwrap().col;
            let child_col = here.children.iter().next().unwrap().col;
            let my_col = here.coord.col;

            let to_the_left = my_col < child_col && my_col < parent_col;
            let to_the_right = my_col > child_col && my_col > parent_col;

            if to_the_left && j < cols - 1 {
                let dest = j + 1;
                if grid[dest as usize][i].is_none() {
                    if let Some(mut moved) = grid[j as usize][i].take() {
                        moved.coord.col = dest;
                        grid[dest as usize][i] = Some(moved);
                    }
                    continue;
                }
            }
            if to_the_right && j > 0 {
                let dest = j - 1;
                if grid[dest as usize][i].is_none() {
                    if let Some(mut moved) = grid[j as usize][i].take() {
                        moved.coord.col = dest;
                        grid[dest as usize][i] = Some(moved);
                    }
                }
            }
        }
    }
}

fn neighbor_allowed_positions(column: i32, total_columns: i32) -> Vec<i32> {
    let mut out = Vec::with_capacity(3);
    for d in -1..=1 {
        let n = column + d;
        if n >= 0 && n < total_columns {
            out.push(n);
        }
    }
    out
}

fn allowed_positions(
    grid: &Grid,
    coord: MapCoord,
    total_columns: i32,
) -> Vec<i32> {
    let node = match grid[coord.col as usize][coord.row as usize].as_ref() {
        Some(n) => n,
        None => return Vec::new(),
    };
    let mut allowed: HashSet<i32> = (0..total_columns).collect();
    for parent in &node.parents {
        let neigh: HashSet<i32> =
            neighbor_allowed_positions(parent.col, total_columns).into_iter().collect();
        allowed = &allowed & &neigh;
    }
    for child in &node.children {
        let neigh: HashSet<i32> =
            neighbor_allowed_positions(child.col, total_columns).into_iter().collect();
        allowed = &allowed & &neigh;
    }
    // Tie-breaking: iterate ascending column. May diverge from .NET
    // HashSet<int> iteration order on ties; if oracle tests reveal
    // divergence we'll revisit.
    let mut v: Vec<i32> = allowed.into_iter().collect();
    v.sort();
    v
}

pub fn spread_adjacent_map_points(grid: &mut Grid, cols: i32, rows: i32) {
    for i in 0..rows as usize {
        // Row snapshot of present points (by coord).
        let mut row_coords: Vec<MapCoord> = (0..cols)
            .filter_map(|c| grid[c as usize][i].as_ref().map(|p| p.coord))
            .collect();
        loop {
            let mut moved_any = false;
            for k in 0..row_coords.len() {
                let coord = row_coords[k];
                let allowed = allowed_positions(grid, coord, cols);
                let current_col = coord.col;
                let row_cols: Vec<i32> = row_coords
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| *idx != k)
                    .map(|(_, c)| c.col)
                    .collect();
                let mut best_col = current_col;
                let mut best_gap = compute_gap(current_col, &row_cols);
                for candidate in allowed {
                    if candidate == current_col {
                        continue;
                    }
                    let dest_empty = grid[candidate as usize][i].is_none();
                    if !dest_empty {
                        continue;
                    }
                    let gap = compute_gap(candidate, &row_cols);
                    if gap > best_gap {
                        best_col = candidate;
                        best_gap = gap;
                    }
                }
                if best_col != current_col {
                    if let Some(mut moved) = grid[current_col as usize][i].take() {
                        moved.coord.col = best_col;
                        grid[best_col as usize][i] = Some(moved);
                    }
                    row_coords[k] = MapCoord::new(best_col, coord.row);
                    moved_any = true;
                }
            }
            if !moved_any {
                break;
            }
        }
    }
}

fn compute_gap(candidate_col: i32, other_cols: &[i32]) -> i32 {
    let mut min_gap = i32::MAX;
    for &c in other_cols {
        let d = (candidate_col - c).abs();
        if d < min_gap {
            min_gap = d;
        }
    }
    min_gap
}
