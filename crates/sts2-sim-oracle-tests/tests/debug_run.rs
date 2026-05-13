//! Debug helper for diagnosing replay divergences. Prints the generated
//! map's per-row types for a specific run+act so we can eyeball-compare
//! against the recorded sequence.

use sts2_sim::act::ActId;
use sts2_sim::map::MapPointType;
use sts2_sim::run_log;
use sts2_sim::run_state::RunState;

#[test]
#[ignore = "diagnostic only — uncomment to inspect a failing run"]
fn dump_underdocks_act0_for_7n4() {
    let log = run_log::from_path(
        r"C:\Users\zhuyl\OneDrive\Desktop\sts2_stats\sample runs\1775855780.run"
    ).unwrap();
    let mut rs = RunState::from_run_log(&log).unwrap();
    println!("acts: {:?}", rs.acts());
    println!("seed_string={} seed_uint={}",
        rs.seed_string(), rs.rng_set().seed_uint());
    let map = rs.enter_act(0).clone();
    println!("cols={} rows={} second_boss={:?}",
        map.cols(), map.rows(), map.second_boss().map(|p| p.coord));
    for row in 0..map.rows() {
        let mut line = format!("row {row:>2}: ");
        for col in 0..map.cols() {
            let cell = map.get_point(col, row).map(|p| {
                match p.point_type {
                    MapPointType::Unassigned => "U",
                    MapPointType::Unknown => "?",
                    MapPointType::Shop => "S",
                    MapPointType::Treasure => "T",
                    MapPointType::RestSite => "R",
                    MapPointType::Monster => "M",
                    MapPointType::Elite => "E",
                    MapPointType::Boss => "B",
                    MapPointType::Ancient => "A",
                }
            }).unwrap_or(".");
            line.push_str(cell);
        }
        println!("{line}");
    }
    println!("recorded: {:?}",
        log.map_point_history[0].iter()
            .map(|n| n.map_point_type.as_str())
            .collect::<Vec<_>>());
    let _ = ActId::Underdocks;
}
