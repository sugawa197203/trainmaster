//! boarding・spawn のテストで使う状態の組み立てと、コマンドなしの 1 tick。

use crate::balance::BalanceTable;
use crate::event::GameEvent;
use crate::ids::{CompanyLevel, Level, LineId, StationId, TrainId};
use crate::param::{FeatureSet, ParamSet};
use crate::rng::RngStreams;
use crate::routing::Routing;
use crate::sim::boarding::serve_arrival;
use crate::sim::movement::advance_all;
use crate::sim::spawn::spawn_all;
use crate::sim::topology::fixtures::{line, stations};
use crate::state::{
    CompanyState, Direction, GameState, INCOME_WINDOW_SECS, PaxMatrix, Progress, RebirthState,
    Stats, TrackGrid, Train, TrainPhase,
};
use crate::time::Tick;
use crate::units::{Money, People, PosUnits};

pub(crate) const SEED: u64 = 0x5EED_0106;

/// 駅 `(x, y, ホーム長のレベル)` と路線 `(駅の並び, 両数のレベル)` から状態を作る。列車はない。
pub(crate) fn state(
    cells: &[(i32, i32, u8)],
    lines: &[(&[u32], u8)],
    bal: &BalanceTable,
) -> GameState {
    let mut stations = stations(cells);
    let lines: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(i, &(order, cars))| line(u32::try_from(i).unwrap(), order, &stations, cars))
        .collect();
    for l in &lines {
        for s in &l.stations {
            stations[s.index()].serving_lines.push(l.id);
        }
    }
    let mut waiting = PaxMatrix::new(bal.max_stations());
    for _ in &stations {
        waiting.push_row();
    }
    let mut st = GameState {
        tick: Tick::ZERO,
        money: Money::ZERO,
        company: CompanyState {
            level: CompanyLevel::ZERO,
            line_slots: Level::ZERO,
            track_tiles: 0,
        },
        station_plan: stations.iter().map(|s| s.pos).collect(),
        stations,
        lines,
        trains: Vec::new(),
        waiting,
        onboard: PaxMatrix::new(bal.max_stations()),
        tracks: TrackGrid::new(bal.spec().map.width, bal.spec().map.height),
        progress: Progress {
            unlocked: FeatureSet::default(),
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
        rng: RngStreams::new(SEED),
        routing: Routing::default(),
        map_revision: 0,
        #[cfg(feature = "test-support")]
        ledger: Default::default(),
    };
    st.routing = Routing::new(&st, bal);
    st
}

/// S0 —(3)— S1 —(4)— S2 の路線 L0。
pub(crate) fn one_line(bal: &BalanceTable) -> GameState {
    state(&[(2, 5, 0), (5, 5, 0), (9, 5, 0)], &[(&[0, 1, 2], 0)], bal)
}

/// L0: S0 —(3)— S1 —(4)— S2、L1: S1 —(4)— S3。S1 で乗り換える。
pub(crate) fn transfer_net(bal: &BalanceTable) -> GameState {
    state(
        &[(2, 5, 0), (5, 5, 0), (9, 5, 0), (5, 9, 0)],
        &[(&[0, 1, 2], 0), (&[1, 3], 0)],
        bal,
    )
}

/// 路線 `l` に列車を足す(乗客 0 人)。
pub(crate) fn add_train(
    st: &mut GameState,
    l: u32,
    section: u16,
    dir: Direction,
    pos: u64,
    phase: TrainPhase,
) -> TrainId {
    let id = TrainId::from_index(st.trains.len());
    st.trains.push(Train {
        id,
        line: LineId(l),
        section,
        dir,
        pos: PosUnits(pos),
        phase,
        riders: People::ZERO,
    });
    st.onboard.push_row();
    st.lines[l as usize].trains.push(id);
    id
}

/// 区間 `section` を向き `dir` で走り、いま着いた列車。
pub(crate) fn arriving_train(st: &mut GameState, l: u32, section: u16, dir: Direction) -> TrainId {
    let len = st.lines[l as usize].sections[usize::from(section)].length.0;
    add_train(st, l, section, dir, len, TrainPhase::Running)
}

pub(crate) fn put_waiting(st: &mut GameState, s: u32, t: u32, n: u64) {
    st.waiting.add(s, StationId(t), People(n));
    let w = &mut st.stations[s as usize].waiting_total;
    *w = w.add_sat(People(n));
}

pub(crate) fn put_onboard(st: &mut GameState, tr: TrainId, t: u32, n: u64) {
    st.onboard.add(tr.0, StationId(t), People(n));
    let r = &mut st.trains[tr.index()].riders;
    *r = r.add_sat(People(n));
}

pub(crate) fn waiting(st: &GameState, s: u32, t: u32) -> u64 {
    st.waiting.get(s, StationId(t)).0
}

pub(crate) fn onboard(st: &GameState, tr: TrainId, t: u32) -> u64 {
    st.onboard.get(tr.0, StationId(t)).0
}

/// 駅と列車にいる人数の合計。
pub(crate) fn in_system(st: &GameState) -> u128 {
    st.waiting
        .as_slice()
        .iter()
        .chain(st.onboard.as_slice())
        .map(|&v| u128::from(v))
        .sum()
}

/// コマンドと経路の再計算のない 1 tick(11 §2.1 の手順 0・3・4・5 のうち乗客に関わる部分)。
pub(crate) fn tick(
    st: &mut GameState,
    bal: &BalanceTable,
    arrived: &mut Vec<TrainId>,
    events: &mut Vec<GameEvent>,
) {
    st.tick = st.tick.next();
    advance_all(st, bal, arrived);
    for &t in arrived.iter() {
        serve_arrival(st, bal, t, events);
    }
    if st.tick.is_second_boundary() {
        spawn_all(st, bal);
    }
}

/// INV-08・INV-09 と、待ち人数の合計の上限(11 §6.4)。
pub(crate) fn check_pax_invariants(st: &GameState, bal: &BalanceTable) {
    for (i, stn) in st.stations.iter().enumerate() {
        let row = u32::try_from(i).unwrap();
        assert_eq!(st.waiting.row_total(row), stn.waiting_total, "INV-08: S{i}");
        assert_eq!(
            st.waiting.get(row, stn.id),
            People::ZERO,
            "INV-08: waiting[s][s]"
        );
        assert!(
            stn.waiting_total.0 <= super::waiting_limit(bal, stn),
            "S{i} の待ち人数が上限を超えた"
        );
        let n = st.stations.len();
        assert!(
            st.waiting.row(row)[n..].iter().all(|&v| v == 0),
            "INV-08: 未出現の目的駅"
        );
    }
    for tr in &st.trains {
        assert_eq!(
            st.onboard.row_total(tr.id.0),
            tr.riders,
            "INV-08: {}",
            tr.id
        );
        let n = st.stations.len();
        assert!(
            st.onboard.row(tr.id.0)[n..].iter().all(|&v| v == 0),
            "INV-08: 未出現の目的駅"
        );
        let lv = st.lines[tr.line.index()].levels;
        let cap = crate::units::train_capacity(
            bal.car_capacity(lv.car_capacity),
            bal.cars(lv.cars),
            bal.crowding(lv.crowding),
        );
        assert!(tr.riders <= cap, "INV-09: {}", tr.id);
    }
}
