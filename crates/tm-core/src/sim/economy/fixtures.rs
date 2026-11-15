//! 経済・会社・おすすめのテストで使う `GameState` の組み立て。

use std::sync::OnceLock;

use crate::balance::{BalanceSpec, BalanceTable};
use crate::geom::GridPos;
use crate::ids::{CompanyLevel, Level, LineId, StationId};
use crate::param::{ParamKey, ParamSet};
use crate::rng::RngStreams;
use crate::routing::Routing;
use crate::sim::company::unlocked_at;
use crate::sim::movement::place_initial_train;
use crate::sim::topology::fixtures::{line, stations};
use crate::sim::topology::{add_line_tracks, insert_serving_line, rebuild_line_derived};
use crate::state::{
    CompanyState, GameState, INCOME_WINDOW_SECS, PaxMatrix, Progress, RebirthState, Stats,
    TrackGrid,
};
use crate::time::Tick;
use crate::units::Money;

pub(crate) fn bal() -> &'static BalanceTable {
    static B: OnceLock<BalanceTable> = OnceLock::new();
    B.get_or_init(BalanceTable::builtin)
}

/// 組み込みの表を `edit` で変えた表。
pub(crate) fn custom(edit: impl FnOnce(&mut BalanceSpec)) -> BalanceTable {
    let mut spec = bal().spec().clone();
    edit(&mut spec);
    BalanceTable::from_spec(spec).expect("検証を通る表")
}

/// 駅 `cells` = (x, y, ホーム長のレベル)、路線 `lines` = (駅の並び, 両数のレベル)、会社レベル `c`。
/// 解放は `c` 以下のもの、列車は各路線 1 + L(R1) 本、所持金は 0。
pub(crate) fn world(
    b: &BalanceTable,
    cells: &[(i32, i32, u8)],
    lines: &[(&[u32], u8)],
    c: u8,
) -> GameState {
    let mut sts = stations(cells);
    let mut ls = Vec::new();
    for (i, &(order, cars)) in lines.iter().enumerate() {
        let l = line(i as u32, order, &sts, cars);
        for &s in &l.stations {
            insert_serving_line(&mut sts[s.index()].serving_lines, l.id);
        }
        ls.push(l);
    }
    let max = b.max_stations();
    let mut plan: Vec<GridPos> = sts.iter().map(|s| s.pos).collect();
    let mut x = 1;
    while plan.len() < max as usize {
        let p = GridPos { x, y: 16 };
        if !plan.contains(&p) {
            plan.push(p);
        }
        x += 1;
    }
    let mut waiting = PaxMatrix::new(max);
    for _ in &sts {
        waiting.push_row();
    }
    let mut tracks = TrackGrid::new(b.spec().map.width, b.spec().map.height);
    for seg in ls.iter().flat_map(|l| &l.segments) {
        add_line_tracks(&mut tracks, &seg.tiles);
    }
    let mut st = GameState {
        tick: Tick::ZERO,
        money: Money::ZERO,
        company: CompanyState {
            level: CompanyLevel(c),
            line_slots: Level::ZERO,
            track_tiles: tracks.occupied(),
        },
        station_plan: plan,
        stations: sts,
        lines: ls,
        trains: Vec::new(),
        waiting,
        onboard: PaxMatrix::new(max),
        tracks,
        progress: Progress {
            unlocked: unlocked_at(b, CompanyLevel(c)),
            upgraded_once: ParamSet::default(),
            total_earned: Money::ZERO,
            max_money_unit: 0,
            goal_reached: false,
        },
        rebirth: RebirthState::default(),
        stats: Stats {
            income_this_sec: Money::ZERO,
            income_window: [Money::ZERO; INCOME_WINDOW_SECS],
            window_head: 0,
            arrivals: 0,
            delivered: 0,
            purchases: 0,
        },
        rng: RngStreams::new(1),
        routing: Routing::default(),
        map_revision: 0,
        #[cfg(feature = "test-support")]
        ledger: Default::default(),
    };
    place_trains(&mut st, b);
    st.routing = Routing::new(&st, b);
    st
}

/// 各路線に 1 + L(R1) 本になるまで列車を置く。
pub(crate) fn place_trains(st: &mut GameState, b: &BalanceTable) {
    for i in 0..st.lines.len() {
        let l = LineId::from_index(i);
        let n = b.max_trains(st.lines[i].levels.max_trains);
        for k in st.lines[i].trains.len() as u32..n {
            place_initial_train(st, l, k, n);
        }
    }
}

pub(crate) fn set_station(st: &mut GameState, s: u32, p: ParamKey, l: u8) {
    *st.stations[s as usize]
        .levels
        .get_mut(p)
        .expect("駅のパラメータ") = Level(l);
}

/// 路線のレベルを変え、導出データを作り直す(R1 なら列車も足す)。
pub(crate) fn set_line(st: &mut GameState, b: &BalanceTable, l: u32, p: ParamKey, lv: u8) {
    let line = &mut st.lines[l as usize];
    *line.levels.get_mut(p).expect("路線のパラメータ") = Level(lv);
    rebuild_line_derived(line, &st.stations, b);
    place_trains(st, b);
}

pub(crate) fn all_upgraded() -> ParamSet {
    ParamSet((1 << 12) - 1)
}

pub(crate) fn yen(v: u64) -> Money {
    Money::from_u64(v)
}

pub(crate) fn sid(i: u32) -> StationId {
    StationId(i)
}
