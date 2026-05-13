//! Port of `MegaCrit.Sts2.Core.Map.StandardActMap` — the per-act map
//! generator. Pipeline (run inside the constructor):
//!
//!   1. `generate_map`         — 7 path traversals seeded from row 1
//!   2. `assign_point_types`   — bottom row → RestSite, mid → Treasure/Elite,
//!                                row 1 → Monster, rest filled from
//!                                MapPointTypeCounts via repeated
//!                                StableShuffle.
//!   3. `MapPathPruning::prune_and_repair` (M-D, not yet ported; gated by
//!                                `enable_pruning`).
//!   4. `MapPostProcessing::{center_grid, spread_adjacent_map_points,
//!                                straighten_paths}` (M-E, ports here in
//!                                this commit chunk).
//!
//! Bit-exactness is mandatory: order of every RNG draw must match the C#
//! exactly, including the order each act's `get_map_point_types` consumes
//! from before path generation starts.

use std::collections::HashSet;

use crate::act::ActModel;
use crate::map::{MapCoord, MapPoint, MapPointType, MapPointTypeCounts};
use crate::rng::Rng;
use crate::shuffle::{stable_shuffle, unstable_shuffle};

/// Map grid width. Hard-coded in C# as `_mapWidth = 7`.
pub const COLS: i32 = 7;

/// Header row that path generation seeds from. The C# constant
/// `_iterations` matches `_mapWidth`; we keep the name for fidelity.
const ITERATIONS: i32 = 7;

#[derive(Debug, Clone)]
pub struct StandardActMap {
    cols: i32,
    rows: i32, // _mapLength = actModel.GetNumberOfRooms(isMultiplayer) + 1
    /// `[col][row]` storage. `None` where the cell is empty.
    grid: Vec<Vec<Option<MapPoint>>>,
    boss: MapPoint,
    starting: MapPoint,
    second_boss: Option<MapPoint>,
    start_map_points: HashSet<MapCoord>,
    should_replace_treasure_with_elites: bool,
    point_type_counts: MapPointTypeCounts,
    rng: Rng,
}

impl StandardActMap {
    /// Mirrors the C# ctor. `enable_pruning` is wired through but pruning
    /// itself is M-D (not yet ported); pass false until that lands.
    pub fn new(
        mut rng: Rng,
        act: &dyn ActModel,
        is_multiplayer: bool,
        should_replace_treasure_with_elites: bool,
        has_second_boss: bool,
        point_type_counts_override: Option<MapPointTypeCounts>,
        enable_pruning: bool,
    ) -> Self {
        let map_length = act.get_number_of_rooms(is_multiplayer) + 1;
        let cols = COLS;
        let rows = map_length;

        // _pointTypeCounts = mapPointTypeCountsOverride ?? actModel.GetMapPointTypes(mapRng);
        // C# uses null-coalescing: only call GetMapPointTypes if override is null.
        let point_type_counts = match point_type_counts_override {
            Some(c) => c,
            None => act.get_map_point_types(&mut rng),
        };

        // BossMapPoint = new MapPoint(cols/2, rows). StartingMapPoint = (cols/2, 0).
        let boss = MapPoint::new(cols / 2, rows);
        let starting = MapPoint::new(cols / 2, 0);
        let second_boss = if has_second_boss {
            Some(MapPoint::new(cols / 2, rows + 1))
        } else {
            None
        };

        let mut sam = Self {
            cols,
            rows,
            grid: vec![vec![None; rows as usize]; cols as usize],
            boss,
            starting,
            second_boss,
            start_map_points: HashSet::new(),
            should_replace_treasure_with_elites,
            point_type_counts,
            rng,
        };

        sam.generate_map();
        sam.assign_point_types();
        if enable_pruning {
            // M-D will land MapPathPruning::prune_and_repair here.
            // unimplemented!("MapPathPruning is part of map port chunk M-D");
        }
        sam.center_grid();
        sam.spread_adjacent_map_points();
        sam.straighten_paths();

        sam
    }

    pub fn cols(&self) -> i32 { self.cols }
    pub fn rows(&self) -> i32 { self.rows }
    pub fn boss(&self) -> &MapPoint { &self.boss }
    pub fn starting(&self) -> &MapPoint { &self.starting }
    pub fn second_boss(&self) -> Option<&MapPoint> { self.second_boss.as_ref() }
    pub fn rng_counter(&self) -> i32 { self.rng.counter() }
    pub fn start_map_points(&self) -> &HashSet<MapCoord> { &self.start_map_points }

    /// All grid `MapPoint`s in (col-then-row) order, excluding `None` cells
    /// and excluding the boss/starting/second-boss specials.
    pub fn iter_grid_points(&self) -> impl Iterator<Item = &MapPoint> {
        self.grid.iter().flatten().filter_map(|x| x.as_ref())
    }

    pub fn get_point(&self, col: i32, row: i32) -> Option<&MapPoint> {
        if col == self.boss.coord.col && row == self.boss.coord.row {
            return Some(&self.boss);
        }
        if let Some(sb) = &self.second_boss {
            if col == sb.coord.col && row == sb.coord.row {
                return Some(sb);
            }
        }
        if col == self.starting.coord.col && row == self.starting.coord.row {
            return Some(&self.starting);
        }
        if (0..self.cols).contains(&col) && (0..self.rows).contains(&row) {
            self.grid[col as usize][row as usize].as_ref()
        } else {
            None
        }
    }

    fn get_point_mut(&mut self, col: i32, row: i32) -> Option<&mut MapPoint> {
        if col == self.boss.coord.col && row == self.boss.coord.row {
            return Some(&mut self.boss);
        }
        if let Some(sb) = &mut self.second_boss {
            if col == sb.coord.col && row == sb.coord.row {
                return Some(sb);
            }
        }
        if col == self.starting.coord.col && row == self.starting.coord.row {
            return Some(&mut self.starting);
        }
        if (0..self.cols).contains(&col) && (0..self.rows).contains(&row) {
            self.grid[col as usize][row as usize].as_mut()
        } else {
            None
        }
    }

    fn get_or_create_grid_point(&mut self, col: i32, row: i32) -> MapCoord {
        if self.grid[col as usize][row as usize].is_none() {
            self.grid[col as usize][row as usize] = Some(MapPoint::new(col, row));
        }
        MapCoord::new(col, row)
    }

    /// `AddChildPoint`: bidirectional link.
    fn add_child(&mut self, parent: MapCoord, child: MapCoord) {
        if let Some(p) = self.get_point_mut(parent.col, parent.row) {
            p.children.insert(child);
        }
        if let Some(c) = self.get_point_mut(child.col, child.row) {
            c.parents.insert(parent);
        }
    }

    fn generate_map(&mut self) {
        // 7 path iterations, each starting at row 1.
        for i in 0..ITERATIONS {
            let mut col = self.rng.next_int_range(0, COLS);
            // On iteration index 1 (second iteration), retry until we get a
            // start column that wasn't used in iteration 0. (Mirrors C# `if (i == 1)`.)
            if i == 1 {
                while self.start_map_points.contains(&MapCoord::new(col, 1)) {
                    col = self.rng.next_int_range(0, COLS);
                }
            }
            let start = self.get_or_create_grid_point(col, 1);
            self.start_map_points.insert(start);
            self.path_generate(start);
        }

        // Link the bottom row to the boss.
        let last_row = self.rows - 1;
        let bottoms: Vec<MapCoord> = (0..self.cols)
            .filter_map(|c| self.grid[c as usize][last_row as usize]
                .as_ref()
                .map(|p| p.coord))
            .collect();
        let boss_coord = self.boss.coord;
        for c in bottoms {
            self.add_child(c, boss_coord);
        }

        if let Some(sb_coord) = self.second_boss.as_ref().map(|p| p.coord) {
            self.add_child(boss_coord, sb_coord);
        }

        // Link the starting point to all row-1 entries.
        let row_1_points: Vec<MapCoord> = (0..self.cols)
            .filter_map(|c| self.grid[c as usize][1usize]
                .as_ref()
                .map(|p| p.coord))
            .collect();
        let starting_coord = self.starting.coord;
        for c in row_1_points {
            self.add_child(starting_coord, c);
        }
    }

    fn path_generate(&mut self, start: MapCoord) {
        let mut current = start;
        while current.row < self.rows - 1 {
            let next = self.generate_next_coord(current);
            let next_coord = self.get_or_create_grid_point(next.col, next.row);
            self.add_child(current, next_coord);
            current = next_coord;
        }
    }

    fn generate_next_coord(&mut self, current: MapCoord) -> MapCoord {
        let col = current.col;
        let lo = (col - 1).max(0);
        let hi = (col + 1).min(COLS - 1);
        let mut deltas: Vec<i32> = vec![-1, 0, 1];
        stable_shuffle(&mut deltas, &mut self.rng);

        for delta in deltas {
            let target_row = current.row + 1;
            let target_col = match delta {
                -1 => lo,
                0 => col,
                1 => hi,
                _ => unreachable!(),
            };
            if !self.has_invalid_crossover(current, target_col) {
                return MapCoord::new(target_col, target_row);
            }
        }
        panic!(
            "Cannot find next node: seed={}, current=({}, {})",
            self.rng.seed(),
            current.col,
            current.row
        );
    }

    /// Mirrors `HasInvalidCrossover` in C#. Detects the X-crossing pattern
    /// where moving diagonally would cross an existing edge between the
    /// neighbor cell and its child on the opposite diagonal.
    fn has_invalid_crossover(&self, current: MapCoord, target_x: i32) -> bool {
        let diff = target_x - current.col;
        // C# checks `diff == 0 || diff == 7` — the latter is a side-effect of
        // unsigned-style wrap that can't happen with -1/0/+1 deltas. We mirror
        // the early-out for diff == 0 (no crossing possible when moving
        // straight down).
        if diff == 0 || diff == 7 {
            return false;
        }
        let Some(neighbor) = self.get_point(target_x, current.row) else {
            return false;
        };
        for child_coord in &neighbor.children {
            let opposing = child_coord.col - neighbor.coord.col;
            if opposing == -diff {
                return true;
            }
        }
        false
    }

    fn for_each_in_row_collect_coords(&self, row: i32) -> Vec<MapCoord> {
        (0..self.cols)
            .filter_map(|c| self.grid[c as usize][row as usize]
                .as_ref()
                .map(|p| p.coord))
            .collect()
    }

    fn assign_point_types(&mut self) {
        let last_row = self.rows - 1;
        // Bottom row → RestSite, locked.
        for c in self.for_each_in_row_collect_coords(last_row) {
            if let Some(p) = self.get_point_mut(c.col, c.row) {
                p.point_type = MapPointType::RestSite;
                p.can_be_modified = false;
            }
        }
        // Row N-7 → Treasure or Elite, locked.
        let treasure_row = self.rows - 7;
        if treasure_row >= 0 {
            let pt = if self.should_replace_treasure_with_elites {
                MapPointType::Elite
            } else {
                MapPointType::Treasure
            };
            for c in self.for_each_in_row_collect_coords(treasure_row) {
                if let Some(p) = self.get_point_mut(c.col, c.row) {
                    p.point_type = pt;
                    p.can_be_modified = false;
                }
            }
        }
        // Row 1 → Monster, locked.
        for c in self.for_each_in_row_collect_coords(1) {
            if let Some(p) = self.get_point_mut(c.col, c.row) {
                p.point_type = MapPointType::Monster;
                p.can_be_modified = false;
            }
        }

        // Queue the remaining counts and assign to random Unassigned points.
        let mut queue: Vec<MapPointType> = Vec::new();
        for _ in 0..self.point_type_counts.num_of_rests {
            queue.push(MapPointType::RestSite);
        }
        for _ in 0..self.point_type_counts.num_of_shops {
            queue.push(MapPointType::Shop);
        }
        for _ in 0..self.point_type_counts.num_of_elites {
            queue.push(MapPointType::Elite);
        }
        for _ in 0..self.point_type_counts.num_of_unknowns {
            queue.push(MapPointType::Unknown);
        }
        // C# uses a Queue<MapPointType>; we use VecDeque-equivalent via Vec
        // with front-removal in `get_next_valid_point_type` below. The
        // call site does up-to-3 passes.
        let mut queue: std::collections::VecDeque<MapPointType> = queue.into();
        self.assign_remaining_types_to_random_points(&mut queue);

        // Any leftover Unassigned → Monster.
        let unassigned: Vec<MapCoord> = self
            .iter_grid_points()
            .filter(|p| p.point_type == MapPointType::Unassigned)
            .map(|p| p.coord)
            .collect();
        for c in unassigned {
            if let Some(p) = self.get_point_mut(c.col, c.row) {
                p.point_type = MapPointType::Monster;
            }
        }

        self.boss.point_type = MapPointType::Boss;
        self.starting.point_type = MapPointType::Ancient;
        if let Some(sb) = self.second_boss.as_mut() {
            sb.point_type = MapPointType::Boss;
        }
    }

    fn assign_remaining_types_to_random_points(
        &mut self,
        queue: &mut std::collections::VecDeque<MapPointType>,
    ) {
        let mut iterations = 0;
        while iterations < 3 && !queue.is_empty() {
            // Collect Unassigned points, StableShuffle them.
            let mut unassigned: Vec<MapCoord> = self
                .iter_grid_points()
                .filter(|p| p.point_type == MapPointType::Unassigned)
                .map(|p| p.coord)
                .collect();
            stable_shuffle(&mut unassigned, &mut self.rng);
            for coord in unassigned {
                if queue.is_empty() {
                    break;
                }
                let chosen = self.get_next_valid_point_type(queue, coord);
                if chosen != MapPointType::Unassigned {
                    if let Some(p) = self.get_point_mut(coord.col, coord.row) {
                        p.point_type = chosen;
                    }
                }
            }
            iterations += 1;
        }
    }

    fn get_next_valid_point_type(
        &self,
        queue: &mut std::collections::VecDeque<MapPointType>,
        coord: MapCoord,
    ) -> MapPointType {
        let n = queue.len();
        let Some(point) = self.get_point(coord.col, coord.row) else {
            return MapPointType::Unassigned;
        };
        for _ in 0..n {
            let pt = queue.pop_front().expect("non-empty");
            if self
                .point_type_counts
                .should_ignore_map_point_rules_for_map_point_type(pt)
            {
                return pt;
            }
            if self.is_valid_point_type(pt, point) {
                return pt;
            }
            queue.push_back(pt);
        }
        MapPointType::Unassigned
    }

    /// Composite validity check exposed for `MapPathPruning` (M-D).
    pub fn is_valid_point_type(&self, pt: MapPointType, point: &MapPoint) -> bool {
        self.is_valid_for_upper(pt, point)
            && Self::is_valid_for_lower(pt, point)
            && Self::is_valid_with_parents(pt, point, self)
            && Self::is_valid_with_children(pt, point, self)
            && Self::is_valid_with_siblings(pt, point, self)
    }

    fn is_valid_for_lower(pt: MapPointType, point: &MapPoint) -> bool {
        point.coord.row >= 6 || !LOWER_RESTRICTIONS.contains(&pt)
    }

    fn is_valid_for_upper(&self, pt: MapPointType, point: &MapPoint) -> bool {
        point.coord.row < self.rows - 3 || !UPPER_RESTRICTIONS.contains(&pt)
    }

    fn is_valid_with_parents(
        pt: MapPointType,
        point: &MapPoint,
        sam: &StandardActMap,
    ) -> bool {
        if !PARENT_RESTRICTIONS.contains(&pt) {
            return true;
        }
        for c in point.parents.iter().chain(point.children.iter()) {
            if let Some(p) = sam.get_point(c.col, c.row) {
                if p.point_type == pt {
                    return false;
                }
            }
        }
        true
    }

    fn is_valid_with_children(
        pt: MapPointType,
        point: &MapPoint,
        sam: &StandardActMap,
    ) -> bool {
        if !CHILD_RESTRICTIONS.contains(&pt) {
            return true;
        }
        for c in &point.children {
            if let Some(p) = sam.get_point(c.col, c.row) {
                if p.point_type == pt {
                    return false;
                }
            }
        }
        true
    }

    fn is_valid_with_siblings(
        pt: MapPointType,
        point: &MapPoint,
        sam: &StandardActMap,
    ) -> bool {
        if !SIBLING_RESTRICTIONS.contains(&pt) {
            return true;
        }
        for parent_coord in &point.parents {
            let Some(parent) = sam.get_point(parent_coord.col, parent_coord.row)
            else { continue };
            for sibling_coord in &parent.children {
                if *sibling_coord == point.coord {
                    continue;
                }
                if let Some(s) = sam.get_point(sibling_coord.col, sibling_coord.row) {
                    if s.point_type == pt {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Move the in-grid point at `(col, row)` to `(new_col, row)`. Rewires
    /// every parent's children set and every child's parents set so they
    /// reference the new coord. Boss / starting / second_boss are handled
    /// because `get_point_mut` resolves them. This is the operation that
    /// PostProcessing's three passes need to use any time they shift a
    /// MapPoint's column — otherwise neighbor edges go stale (C# gets this
    /// for free via reference equality; we don't).
    fn move_in_grid_point(&mut self, col: i32, row: i32, new_col: i32) {
        if col == new_col {
            return;
        }
        let old_coord = MapCoord::new(col, row);
        let new_coord = MapCoord::new(new_col, row);

        // Snapshot neighbor coords before mutating; they themselves don't move.
        let (parent_coords, child_coords) = {
            let Some(p) = self.grid[col as usize][row as usize].as_ref() else {
                return;
            };
            (
                p.parents.iter().copied().collect::<Vec<_>>(),
                p.children.iter().copied().collect::<Vec<_>>(),
            )
        };

        // Rewire each parent's children set.
        for pc in &parent_coords {
            if let Some(parent) = self.get_point_mut(pc.col, pc.row) {
                parent.children.remove(&old_coord);
                parent.children.insert(new_coord);
            }
        }
        // Rewire each child's parents set.
        for cc in &child_coords {
            if let Some(child) = self.get_point_mut(cc.col, cc.row) {
                child.parents.remove(&old_coord);
                child.parents.insert(new_coord);
            }
        }

        // Move the cell itself and update its own coord.
        if let Some(mut moved) = self.grid[col as usize][row as usize].take() {
            moved.coord = new_coord;
            self.grid[new_col as usize][row as usize] = Some(moved);
        }
    }

    fn is_column_empty(&self, col: i32) -> bool {
        if col < 0 || col >= self.cols {
            return true;
        }
        (0..self.rows as usize).all(|r| self.grid[col as usize][r].is_none())
    }

    /// `MapPostProcessing.CenterGrid` — shifts the entire grid left or right
    /// by 1 column when one side is empty and the other isn't. Because every
    /// in-grid point shifts by the same delta, we use a bulk remap rather
    /// than calling `move_in_grid_point` N times.
    pub(crate) fn center_grid(&mut self) {
        let left_empty = self.is_column_empty(0) && self.is_column_empty(1);
        let right_empty = self.is_column_empty(self.cols - 1)
            && self.is_column_empty(self.cols - 2);
        let shift: i32 = match (left_empty, right_empty) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        if shift == 0 {
            return;
        }
        let remap = |c: MapCoord| MapCoord::new(c.col + shift, c.row);
        let remap_set = |s: &HashSet<MapCoord>| -> HashSet<MapCoord> {
            s.iter().map(|&c| remap(c)).collect()
        };

        // Rebuild the grid in shifted positions; cells that fall off the
        // edge are dropped (matches C#, which leaves grid[old, i] = null
        // and only writes if num2 is in range).
        let mut new_grid: Vec<Vec<Option<MapPoint>>> =
            vec![vec![None; self.rows as usize]; self.cols as usize];
        for col in 0..self.cols {
            for row in 0..self.rows {
                if let Some(mut p) = self.grid[col as usize][row as usize].take() {
                    p.coord = remap(p.coord);
                    p.parents = remap_set(&p.parents);
                    p.children = remap_set(&p.children);
                    let new_col = p.coord.col;
                    if new_col >= 0 && new_col < self.cols {
                        new_grid[new_col as usize][row as usize] = Some(p);
                    }
                }
            }
        }
        self.grid = new_grid;

        // Specials (boss / starting / second_boss) sit at col = cols/2 which
        // is unaffected by ±1 shift, so their own coord is unchanged. But
        // their parents/children point at row-1 / row-(rows-1) in-grid
        // points that DID shift, so the sets must be remapped.
        self.boss.parents = remap_set(&self.boss.parents);
        self.boss.children = remap_set(&self.boss.children);
        self.starting.parents = remap_set(&self.starting.parents);
        self.starting.children = remap_set(&self.starting.children);
        if let Some(sb) = self.second_boss.as_mut() {
            sb.parents = remap_set(&sb.parents);
            sb.children = remap_set(&sb.children);
        }
    }

    /// `MapPostProcessing.StraightenPaths` — bias points with exactly one
    /// parent and one child toward sitting between the two columnwise.
    pub(crate) fn straighten_paths(&mut self) {
        for i in 0..self.rows {
            for j in 0..self.cols {
                let Some(here) = self.grid[j as usize][i as usize].as_ref()
                else { continue };
                if here.parents.len() != 1 || here.children.len() != 1 {
                    continue;
                }
                let parent_col = here.parents.iter().next().unwrap().col;
                let child_col = here.children.iter().next().unwrap().col;
                let my_col = here.coord.col;

                let to_the_left = my_col < child_col && my_col < parent_col;
                let to_the_right = my_col > child_col && my_col > parent_col;

                if to_the_left && j < self.cols - 1 {
                    let dest = j + 1;
                    if self.grid[dest as usize][i as usize].is_none() {
                        self.move_in_grid_point(j, i, dest);
                        continue;
                    }
                }
                if to_the_right && j > 0 {
                    let dest = j - 1;
                    if self.grid[dest as usize][i as usize].is_none() {
                        self.move_in_grid_point(j, i, dest);
                    }
                }
            }
        }
    }

    /// `MapPostProcessing.SpreadAdjacentMapPoints` — for each row, repeatedly
    /// move points to maximize the minimum inter-point gap, subject to each
    /// point staying within ±1 column of every parent and every child.
    ///
    /// **Iteration-order note**: the C# version iterates `HashSet<int>
    /// allowedPositions` in bucket order, which affects which candidate
    /// wins on gap ties. Here we iterate ascending column. If oracle diff
    /// tests reveal divergence on tie cases, revisit.
    pub(crate) fn spread_adjacent_map_points(&mut self) {
        for i in 0..self.rows {
            // `row_start_cols` snapshots which columns are occupied at the
            // *start* of this row's do-while loop. We iterate these to decide
            // which points to consider in order, mirroring C#'s `list` which
            // is built once before the do-while.
            let row_start_cols: Vec<i32> = (0..self.cols)
                .filter(|c| self.grid[*c as usize][i as usize].is_some())
                .collect();
            loop {
                let mut moved_any = false;
                for &start_col in &row_start_cols {
                    // The point that started at `start_col` may have moved
                    // earlier in this pass; in C# the foreach holds a stable
                    // reference. Here we find the point at its current
                    // position by scanning the row for an entry whose coord
                    // matches a tracking key. Cheaper proxy: if the start
                    // cell is still occupied use it, otherwise the point
                    // moved — find it by parents/children matching.
                    //
                    // For our cases the simpler invariant holds: each point
                    // only moves once per pass, and never INTO a cell that
                    // was occupied at row-start (because that cell holds a
                    // not-yet-processed point until we get to it). So we can
                    // process points in the order they appear at start, and
                    // for each one look up its current cell by scanning the
                    // row for a MapPoint whose parents/children are unique
                    // to it — but that's overkill. The simplest correct
                    // approach: track per-iteration moves in a map.
                    let Some(point) = self.grid[start_col as usize][i as usize].as_ref()
                    else { continue };
                    let coord = point.coord;
                    let allowed = self.allowed_positions(coord);
                    // Read OTHER points' current cols from the live grid,
                    // not from the stale row_start_cols snapshot. This is
                    // the parity fix that brings gap computation in line
                    // with C#'s `ComputeGap(col, list, mapPoint2)` which
                    // reads mapPoint.coord.col from references.
                    let other_cols: Vec<i32> = (0..self.cols)
                        .filter(|c| *c != coord.col
                            && self.grid[*c as usize][i as usize].is_some())
                        .collect();
                    let mut best_col = coord.col;
                    let mut best_gap = compute_gap(coord.col, &other_cols);
                    for candidate in allowed {
                        if candidate == coord.col {
                            continue;
                        }
                        let dest_empty =
                            self.grid[candidate as usize][i as usize].is_none();
                        if !dest_empty {
                            continue;
                        }
                        let gap = compute_gap(candidate, &other_cols);
                        if gap > best_gap {
                            best_col = candidate;
                            best_gap = gap;
                        }
                    }
                    if best_col != coord.col {
                        self.move_in_grid_point(coord.col, i, best_col);
                        moved_any = true;
                    }
                }
                if !moved_any {
                    break;
                }
            }
        }
    }

    fn allowed_positions(&self, coord: MapCoord) -> Vec<i32> {
        let Some(node) = self.get_point(coord.col, coord.row) else {
            return Vec::new();
        };
        let mut allowed: HashSet<i32> = (0..self.cols).collect();
        for parent in &node.parents {
            let n = neighbor_allowed_positions(parent.col, self.cols);
            allowed.retain(|c| n.contains(c));
        }
        for child in &node.children {
            let n = neighbor_allowed_positions(child.col, self.cols);
            allowed.retain(|c| n.contains(c));
        }
        let mut v: Vec<i32> = allowed.into_iter().collect();
        v.sort();
        v
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

// Static restriction sets from C# (HashSet<MapPointType>). Order doesn't
// matter — we only use `.contains()`.
const LOWER_RESTRICTIONS: &[MapPointType] =
    &[MapPointType::RestSite, MapPointType::Elite];
const UPPER_RESTRICTIONS: &[MapPointType] = &[MapPointType::RestSite];
const PARENT_RESTRICTIONS: &[MapPointType] = &[
    MapPointType::Elite,
    MapPointType::RestSite,
    MapPointType::Treasure,
    MapPointType::Shop,
];
const CHILD_RESTRICTIONS: &[MapPointType] = &[
    MapPointType::Elite,
    MapPointType::RestSite,
    MapPointType::Treasure,
    MapPointType::Shop,
];
const SIBLING_RESTRICTIONS: &[MapPointType] = &[
    MapPointType::RestSite,
    MapPointType::Monster,
    MapPointType::Unknown,
    MapPointType::Elite,
    MapPointType::Shop,
];

// Silence "unused" warning for unstable_shuffle on Linux; map gen uses it via
// stable_shuffle internally.
#[allow(dead_code)]
fn _hold_unstable_shuffle_alive<T>(list: &mut [T], rng: &mut Rng) {
    unstable_shuffle(list, rng);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::Overgrowth;

    #[test]
    fn overgrowth_map_has_correct_bounds() {
        let rng = Rng::new(12345, 0);
        let map = StandardActMap::new(rng, &Overgrowth, false, false, false, None, false);
        assert_eq!(map.cols(), 7);
        // Overgrowth: BaseNumberOfRooms = 15; rows = 16.
        assert_eq!(map.rows(), 16);
        assert_eq!(map.boss().coord, MapCoord::new(3, 16));
        assert_eq!(map.starting().coord, MapCoord::new(3, 0));
        assert_eq!(map.boss().point_type, MapPointType::Boss);
        assert_eq!(map.starting().point_type, MapPointType::Ancient);
    }

    #[test]
    fn every_grid_point_has_a_type_assigned() {
        let rng = Rng::new(98765, 0);
        let map = StandardActMap::new(rng, &Overgrowth, false, false, false, None, false);
        for p in map.iter_grid_points() {
            assert_ne!(
                p.point_type,
                MapPointType::Unassigned,
                "point at {:?} left unassigned",
                p.coord
            );
        }
    }
}
