//! 公開 API `Game`・`GameConfig`(10 §7.1)と `state_hash`(10 §9.2)。

use std::sync::Arc;

use tm_numeric::ja_unit_index;

use crate::RULES_VERSION;
use crate::balance::BalanceTable;
use crate::command::{Command, CommandError, CommandPreview};
use crate::event::GameEvent;
use crate::geom::{Bend, GridPos};
use crate::hash::StateHasher;
use crate::ids::{CompanyLevel, Level, StationId};
use crate::invariants::{self, InvariantViolation, MonotonicSnapshot};
use crate::mapgen::plan_stations;
use crate::param::{ParamSet, StationLevels};
use crate::rng::RngStreams;
use crate::routing::Routing;
use crate::sim::{self, SimScratch};
use crate::state::{
    CompanyState, Direction, GameState, INCOME_WINDOW_SECS, PaxMatrix, Progress, RebirthState,
    Station, Stats, TrackGrid, TrainPhase,
};
use crate::time::Tick;
use crate::units::{MilliPeople, Money, People};
use crate::view::GameView;

/// ゲームの設定。
#[derive(Clone, Debug)]
pub struct GameConfig {
    /// 混ぜた後の最終シード [02 D24]。
    pub seed: u64,
    /// 検証済みの表。セッション間で共有する。
    pub balance: Arc<BalanceTable>,
}

/// ゲーム状態の唯一の所有者。一時停止は持たない(`step` を呼ばなければ止まる。10 §0.2)。
#[derive(Clone, Debug)]
pub struct Game {
    pub(crate) config: GameConfig,
    pub(crate) state: GameState,
    queue: Vec<Command>,
    scratch: SimScratch,
}

impl Game {
    /// 新しいゲーム。失敗しない(10 TD-06)。経路表は初期状態から作り、イベントは出さない。
    pub fn new(config: GameConfig) -> Self {
        let bal = &*config.balance;
        let spec = bal.spec();
        let plan = plan_stations(config.seed, &spec.map, &spec.company, bal.max_stations());
        Game::from_plan(config, plan)
    }

    /// テスト用: 駅の位置と出現順を指定して作る(`plan.len() == max_stations`)。
    #[cfg(feature = "test-support")]
    pub fn with_station_plan(config: GameConfig, plan: Vec<GridPos>) -> Self {
        assert_eq!(
            plan.len(),
            config.balance.max_stations() as usize,
            "station_plan の長さは max_stations"
        );
        Game::from_plan(config, plan)
    }

    /// キューと作業領域が空の `Game`(`restore` が使う)。
    pub(crate) fn from_state(config: GameConfig, state: GameState) -> Self {
        Game {
            config,
            state,
            queue: Vec::new(),
            scratch: SimScratch::default(),
        }
    }

    fn from_plan(config: GameConfig, plan: Vec<GridPos>) -> Self {
        let state = initial_state(config.seed, &config.balance, plan);
        let game = Game::from_state(config, state);
        #[cfg(debug_assertions)]
        if let Err(v) = game.check_invariants() {
            panic!("初期状態で {} が破れた: {}", v.id, v.detail);
        }
        game
    }

    /// 設定。
    pub fn config(&self) -> &GameConfig {
        &self.config
    }

    /// 状態を変えずに検査する(10 §5.4)。
    pub fn validate(&self, cmd: &Command) -> Result<CommandPreview, CommandError> {
        sim::check_command(&self.state, &self.config.balance, cmd).map(|c| c.preview())
    }

    /// 次の `step` の最初に適用するコマンドを積む。ここでは検査しない。
    pub fn enqueue(&mut self, cmd: Command) {
        self.queue.push(cmd);
    }

    /// まだ適用していないコマンド。
    pub fn pending(&self) -> &[Command] {
        &self.queue
    }

    /// ちょうど 1 tick 進め、この tick のイベントを `events` の末尾に足す(clear はしない)。
    pub fn step(&mut self, events: &mut Vec<GameEvent>) {
        sim::run_tick(
            &mut self.state,
            &self.config.balance,
            &mut self.queue,
            &mut self.scratch,
            events,
        );
    }

    /// 読み取り専用のビュー。
    pub fn view(&self) -> GameView<'_> {
        GameView::new(self)
    }

    /// 最後に処理した tick。
    pub fn now(&self) -> Tick {
        self.state.tick
    }

    /// 次に処理する tick(`now() + 1`)。
    pub fn next_tick(&self) -> Tick {
        self.state.tick.next()
    }

    /// 固定順序の FNV-1a 64(10 §9.2)。導出データ・キュー・作業領域・台帳は含めない。
    pub fn state_hash(&self) -> u64 {
        state_hash(&self.state)
    }

    /// 10 §3.9 の INV-01〜INV-21 を検査する。
    pub fn check_invariants(&self) -> Result<(), InvariantViolation> {
        invariants::check_all(&self.state, &self.config.balance)
    }

    /// 「減らせない」の検査に使う写し。
    pub fn monotonic_snapshot(&self) -> MonotonicSnapshot {
        invariants::snapshot(&self.state)
    }

    /// テスト用: 人数の台帳と、駅と列車にいまいる人数(11 §6.5)。
    #[cfg(feature = "test-support")]
    pub fn pax_ledger(&self) -> PaxLedgerView {
        let st = &self.state;
        let sum = |m: &PaxMatrix| m.as_slice().iter().map(|&v| u128::from(v)).sum();
        PaxLedgerView {
            spawned: st.ledger.spawned,
            capacity_loss: st.ledger.capacity_loss,
            delivered: st.ledger.delivered,
            waiting_cap_loss: st.ledger.waiting_cap_loss,
            waiting: sum(&st.waiting),
            onboard: sum(&st.onboard),
        }
    }

    /// テスト用(ベンチマークの最悪の場面。11 §10.1): 全列車を走行中にし、次の tick に到着する位置に置く。
    #[cfg(feature = "test-support")]
    pub fn put_all_trains_before_arrival(&mut self) {
        let st = &mut self.state;
        for tr in &mut st.trains {
            let len = st.lines[tr.line.index()].sections[usize::from(tr.section)].length;
            tr.phase = TrainPhase::Running;
            tr.pos = crate::units::PosUnits(len.0 - 1);
        }
    }

    /// テスト用: 次の tick に経路を再計算させる(ベンチマーク用)。
    #[cfg(feature = "test-support")]
    pub fn mark_routes_dirty(&mut self) {
        self.state.routing.mark_dirty();
    }
}

/// テスト用: 人数の台帳の読み出し。保存の等式は `spawned == waiting + onboard + delivered + waiting_cap_loss`。
#[cfg(feature = "test-support")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaxLedgerView {
    /// 発生させた人数。
    pub spawned: u128,
    /// 定員のため生まれなかった人数(等式に入れない)。
    pub capacity_loss: u128,
    /// 目的駅で降りた人数。
    pub delivered: u128,
    /// 降りて戻る乗客のうち、駅の待ち人数の上限で消えた人数。
    pub waiting_cap_loss: u128,
    /// いま駅で待っている人数。
    pub waiting: u128,
    /// いま列車に乗っている人数。
    pub onboard: u128,
}

#[cfg(feature = "test-support")]
impl PaxLedgerView {
    /// 保存の等式(11 §6.5)が成り立つか。
    pub fn is_conserved(&self) -> bool {
        self.spawned == self.waiting + self.onboard + self.delivered + self.waiting_cap_loss
    }
}

fn initial_state(seed: u64, bal: &BalanceTable, plan: Vec<GridPos>) -> GameState {
    let spec = bal.spec();
    let base = usize::from(spec.company.stations_base);
    let stations: Vec<Station> = plan[..base]
        .iter()
        .enumerate()
        .map(|(i, &pos)| Station {
            id: StationId::from_index(i),
            pos,
            spawned_at: Tick::ZERO,
            levels: StationLevels::default(),
            spawn_acc: MilliPeople::ZERO,
            waiting_total: People::ZERO,
            serving_lines: Vec::new(),
        })
        .collect();
    let mut waiting = PaxMatrix::new(bal.max_stations());
    for _ in &stations {
        waiting.push_row();
    }
    let money = bal.initial_money();
    let mut st = GameState {
        tick: Tick::ZERO,
        money,
        company: CompanyState {
            level: CompanyLevel::ZERO,
            line_slots: Level::ZERO,
            track_tiles: 0,
        },
        station_plan: plan,
        stations,
        lines: Vec::new(),
        trains: Vec::new(),
        waiting,
        onboard: PaxMatrix::new(bal.max_stations()),
        tracks: TrackGrid::new(spec.map.width, spec.map.height),
        progress: Progress {
            unlocked: sim::unlocked_at(bal, CompanyLevel::ZERO),
            upgraded_once: ParamSet::default(),
            total_earned: Money::ZERO,
            max_money_unit: ja_unit_index(money),
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
        rng: RngStreams::new(seed),
        routing: Routing::default(),
        map_revision: 0,
        #[cfg(feature = "test-support")]
        ledger: Default::default(),
    };
    st.routing = Routing::new(&st, bal);
    st
}

fn bend_u8(b: Bend) -> u8 {
    match b {
        Bend::StraightFirst => 0,
        Bend::DiagonalFirst => 1,
    }
}

fn dir_u8(d: Direction) -> u8 {
    match d {
        Direction::Forward => 0,
        Direction::Backward => 1,
    }
}

fn write_matrix(h: &mut StateHasher, m: &PaxMatrix) {
    h.write_u32(m.rows());
    h.write_u32(m.cols());
    for &v in m.as_slice() {
        h.write_u64(v);
    }
}

/// 10 §9.2 の表の順に書く。この順は仕様(変えたら `RULES_VERSION` とゴールデン値を更新する)。
pub(crate) fn state_hash(st: &GameState) -> u64 {
    let mut h = StateHasher::new();
    h.write_bytes(b"TMSH");
    h.write_u32(RULES_VERSION);

    h.write_u64(st.tick.0);

    h.write_money(st.money);
    h.write_money(st.progress.total_earned);

    h.write_u8(st.company.level.0);
    h.write_u8(st.company.line_slots.0);
    h.write_u32(st.company.track_tiles);

    h.write_len(st.station_plan.len());
    for p in &st.station_plan {
        h.write_i32(p.x);
        h.write_i32(p.y);
    }

    h.write_len(st.stations.len());
    for s in &st.stations {
        h.write_u32(s.id.0);
        h.write_u64(s.spawned_at.0);
        let lv = s.levels;
        for l in [lv.spawn, lv.fare, lv.capacity, lv.platform] {
            h.write_u8(l.0);
        }
        h.write_u64(s.spawn_acc.0);
    }

    h.write_len(st.lines.len());
    for l in &st.lines {
        h.write_u32(l.id.0);
        h.write_u64(l.created_at.0);
        h.write_u8(l.born_level.0);
        h.write_len(l.stations.len());
        for s in &l.stations {
            h.write_u32(s.0);
        }
        h.write_len(l.segments.len());
        for seg in &l.segments {
            h.write_u8(bend_u8(seg.bend));
        }
        let lv = l.levels;
        for v in [
            lv.speed,
            lv.crowding,
            lv.boarding,
            lv.car_capacity,
            lv.cars,
            lv.max_trains,
            lv.attractiveness,
        ] {
            h.write_u8(v.0);
        }
        h.write_len(l.trains.len());
        for t in &l.trains {
            h.write_u32(t.0);
        }
    }

    h.write_len(st.trains.len());
    for t in &st.trains {
        h.write_u32(t.id.0);
        h.write_u32(t.line.0);
        h.write_u16(t.section);
        h.write_u8(dir_u8(t.dir));
        h.write_u64(t.pos.0);
        match t.phase {
            TrainPhase::Running => h.write_u8(0),
            TrainPhase::Dwelling { at, remaining } => {
                h.write_u8(1);
                h.write_u16(at);
                h.write_u32(remaining.0);
            }
        }
    }

    write_matrix(&mut h, &st.waiting);
    write_matrix(&mut h, &st.onboard);

    let p = &st.progress;
    h.write_u32(p.unlocked.0);
    h.write_u16(p.upgraded_once.0);
    h.write_u8(p.max_money_unit);
    h.write_bool(p.goal_reached);

    let r = &st.rebirth;
    h.write_u32(r.count);
    h.write_u64(r.points);
    h.write_u8(r.fare_boost);
    h.write_u8(r.train_boost);
    h.write_u8(r.line_boost);

    let s = &st.stats;
    h.write_money(s.income_this_sec);
    for &m in &s.income_window {
        h.write_money(m);
    }
    h.write_u8(s.window_head);
    h.write_u64(s.arrivals);
    h.write_u128(s.delivered);
    h.write_u32(s.purchases);

    h.write_u64(st.rng.passenger_dest.stream());
    h.write_u64(st.rng.passenger_dest.words());

    h.write_u64(st.map_revision);
    h.finish()
}

#[cfg(test)]
mod tests;
