//! 経路グラフの組み立て・再計算・乗客の問い合わせ(12 §3、§5、§6)。
//! シミュレーションは `NodeId` と `RouteTable` に触れず、`Routing` の問い合わせだけを使う(12 RT-D-01)。

use std::sync::Arc;

use tm_pathfind::{
    AllPairsShortestPath, Cost, Graph, NodeId, RepeatedDijkstra, ReverseDijkstra, RouteTable,
};

use crate::balance::BalanceTable;
use crate::ids::{LineId, StationId};
use crate::state::GameState;
use crate::units::{Permille, RouteCost, SpeedMilli, Tiles, ride_cost};

/// 既定の方式(各始点 Dijkstra)で計算するノード数の上限。超えたら予備案 1(12 RT-D-16、§5.6)。
pub(crate) const FALLBACK_NODE_THRESHOLD: u32 = 192;

const NO_NODE: u32 = u32::MAX;

/// グラフを組み立てるのに要る値だけを `GameState` から抜き出した入力(12 §3.1)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteGraphInput {
    /// 出現済みの駅の数(`StationId` は `0..station_count`)。
    pub station_count: u32,
    pub transfer_cost: RouteCost,
    pub stop_allowance_s: u32,
    /// 添字 = `LineId`。
    pub lines: Vec<RouteLineInput>,
}

/// 1 路線の停車駅と区間(12 §3.1)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteLineInput {
    /// 停車駅を Head 側から。通過駅は含めない。
    pub stops: Vec<StationId>,
    /// `gaps[k]` = `stops[k]` と `stops[k+1]` の間のマス(通過駅の分を含む)。
    pub gaps: Vec<Tiles>,
    pub speed: SpeedMilli,
    pub attractiveness: Permille,
}

impl RouteGraphInput {
    /// 導出データ(`Line::stops`、`Line::sections`)が更新済みであることが前提。
    pub(crate) fn from_state(state: &GameState, balance: &BalanceTable) -> RouteGraphInput {
        let transit = &balance.spec().transit;
        RouteGraphInput {
            station_count: u32::try_from(state.stations.len()).expect("駅の数が u32 を超えた"),
            transfer_cost: Cost(transit.transfer_cost),
            stop_allowance_s: transit.stop_allowance_s,
            lines: state
                .lines
                .iter()
                .map(|line| RouteLineInput {
                    stops: line
                        .stations
                        .iter()
                        .zip(&line.stops)
                        .filter(|&(_, &stop)| stop)
                        .map(|(&s, _)| s)
                        .collect(),
                    gaps: line.sections.iter().map(|sec| sec.tiles).collect(),
                    speed: balance.speed(line.levels.speed),
                    attractiveness: balance.attractiveness(line.levels.attractiveness),
                })
                .collect(),
        }
    }
}

/// ノードの種類。`Board(S, l)` は駅 S で路線 l に乗っている状態(12 §3.2)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    In(StationId),
    Out(StationId),
    Board(StationId, LineId),
}

/// `NodeId` と駅・路線の対応。`In = 2S`、`Out = 2S + 1`、続けて (`LineId`, 停車順) の順(12 §3.2)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeMap {
    station_count: u32,
    line_count: u32,
    line_base: Vec<u32>,
    board_lookup: Vec<u32>,
    kinds: Vec<NodeKind>,
}

impl NodeMap {
    pub fn node_count(&self) -> u32 {
        self.kinds.len() as u32
    }

    pub fn station_count(&self) -> u32 {
        self.station_count
    }

    /// `s ≥ station_count` なら panic する。
    pub fn in_node(&self, s: StationId) -> NodeId {
        assert!(s.0 < self.station_count, "未出現の駅 {s}");
        NodeId(2 * s.0)
    }

    /// `s ≥ station_count` なら panic する。
    pub fn out_node(&self, s: StationId) -> NodeId {
        assert!(s.0 < self.station_count, "未出現の駅 {s}");
        NodeId(2 * s.0 + 1)
    }

    /// 路線 `l` が駅 `s` に停車しなければ `None`。
    pub fn board_node(&self, s: StationId, l: LineId) -> Option<NodeId> {
        if s.0 >= self.station_count || l.0 >= self.line_count {
            return None;
        }
        let id = self.board_lookup[l.index() * self.station_count as usize + s.index()];
        (id != NO_NODE).then_some(NodeId(id))
    }

    pub fn kind(&self, n: NodeId) -> NodeKind {
        self.kinds[n.0 as usize]
    }
}

/// 経路グラフを組み立てる。辺の追加順は 12 §3.6 で固定(結果には影響しない)。
pub(crate) fn build_graph(input: &RouteGraphInput) -> (Graph, NodeMap) {
    let ns = input.station_count;
    let line_count = u32::try_from(input.lines.len()).expect("路線の数が u32 を超えた");
    let mut kinds: Vec<NodeKind> = (0..ns)
        .flat_map(|s| [NodeKind::In(StationId(s)), NodeKind::Out(StationId(s))])
        .collect();
    let mut line_base = Vec::with_capacity(input.lines.len());
    let mut board_lookup = vec![NO_NODE; input.lines.len() * ns as usize];
    let mut lines_at: Vec<Vec<(LineId, NodeId)>> = vec![Vec::new(); ns as usize];
    for (l, line) in input.lines.iter().enumerate() {
        debug_check_line(input, line);
        let lid = LineId::from_index(l);
        line_base.push(kinds.len() as u32);
        for &s in &line.stops {
            let id = NodeId(kinds.len() as u32);
            board_lookup[l * ns as usize + s.index()] = id.0;
            kinds.push(NodeKind::Board(s, lid));
            lines_at[s.index()].push((lid, id));
        }
    }

    let mut g = Graph::new(kinds.len() as u32);
    for (s, at) in lines_at.iter().enumerate() {
        let s = StationId::from_index(s);
        for &(_, b) in at {
            g.add_edge(NodeId(2 * s.0), b, Cost::ZERO);
            g.add_edge(b, NodeId(2 * s.0 + 1), Cost::ZERO);
        }
    }
    debug_assert!(input.transfer_cost.0 >= 1, "乗り換えコストが 0(BV-12)");
    for at in &lines_at {
        for &(l1, b1) in at {
            for &(l2, b2) in at {
                if l1 != l2 {
                    g.add_edge(b1, b2, input.transfer_cost);
                }
            }
        }
    }
    for (line, &base) in input.lines.iter().zip(&line_base) {
        for (k, &gap) in line.gaps.iter().enumerate() {
            let a = NodeId(base + k as u32);
            let b = NodeId(a.0 + 1);
            let w = ride_cost(gap, line.speed, line.attractiveness, input.stop_allowance_s);
            debug_assert!(w.0 >= 1, "走行コストが 0");
            g.add_edge(a, b, w);
            g.add_edge(b, a, w);
        }
    }

    let nodes = NodeMap {
        station_count: ns,
        line_count,
        line_base,
        board_lookup,
        kinds,
    };
    (g, nodes)
}

fn debug_check_line(input: &RouteGraphInput, line: &RouteLineInput) {
    debug_assert!(
        line.stops.iter().all(|s| s.0 < input.station_count),
        "未出現の駅に停車する路線 {line:?}"
    );
    debug_assert!(
        line.stops
            .iter()
            .enumerate()
            .all(|(i, s)| !line.stops[..i].contains(s)),
        "停車駅が重複する路線 {line:?}"
    );
    debug_assert_eq!(line.gaps.len(), line.stops.len().saturating_sub(1));
    debug_assert!(line.speed.0 >= 1 && line.attractiveness.0 >= 1);
}

/// 経路表の計算の方式(12 RT-D-16)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    AllSources,
    DestinationsOnly,
}

fn method_for(node_count: u32) -> Method {
    if node_count > FALLBACK_NODE_THRESHOLD {
        Method::DestinationsOnly
    } else {
        Method::AllSources
    }
}

/// 予備案 1 は問い合わせ(12 §6)が引く `Out(T)` の列だけを計算する。結果は既定の方式と同じ。
fn compute_table(graph: &Graph, nodes: &NodeMap, method: Method) -> RouteTable {
    match method {
        Method::AllSources => RepeatedDijkstra.compute(graph),
        Method::DestinationsOnly => {
            let targets: Vec<NodeId> = (0..nodes.station_count())
                .map(|t| nodes.out_node(StationId(t)))
                .collect();
            ReverseDijkstra.compute_to(graph, &targets)
        }
    }
}

/// 経路表・`NodeMap`・その表を作った入力の組。再計算で `Arc` ごと差し替える(12 RT-D-09)。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RouteSnapshot {
    pub table: RouteTable,
    pub nodes: NodeMap,
    /// INV-13 の検査に使う(12 RT-D-14)。
    pub input: RouteGraphInput,
}

impl RouteSnapshot {
    fn build(input: RouteGraphInput) -> RouteSnapshot {
        let (graph, nodes) = build_graph(&input);
        let table = compute_table(&graph, &nodes, method_for(graph.node_count()));
        RouteSnapshot {
            table,
            nodes,
            input,
        }
    }
}

/// 経路表と要再計算の印(12 §5)。`Clone` は表を複製しない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Routing {
    snap: Arc<RouteSnapshot>,
    dirty: bool,
}

impl Default for Routing {
    /// 表のない仮の値(要再計算)。`GameState` を組み立てる途中で `Routing::new` の前に置く。
    fn default() -> Self {
        Routing {
            snap: Arc::new(RouteSnapshot::build(RouteGraphInput {
                station_count: 0,
                transfer_cost: Cost(1),
                stop_allowance_s: 0,
                lines: Vec::new(),
            })),
            dirty: true,
        }
    }
}

/// 待っている乗客が乗る列車の条件(12 §6.1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoardChoice {
    pub line: LineId,
    /// この駅を出たあとに向かう停車駅がこれである列車にだけ乗る。
    pub next_stop: StationId,
}

/// 乗っている乗客が停車駅で表から読む計画(列車の向きを見る前。12 §6.1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RidePlan {
    Arrive,
    Continue {
        next_stop: StationId,
    },
    Transfer {
        to_line: LineId,
    },
    /// 表に経路がない(起きないはず。12 §6.6)。
    Unreachable,
}

/// 列車の向きまで見た乗客の行動(12 §6.1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RiderAction {
    Arrive,
    Stay,
    /// 降りて駅の `waiting` に戻る(乗り換え・向きが逆・経路なし)。
    AlightToWait,
}

impl Routing {
    /// 初期の表を作る(12 RT-D-08)。イベントは出さない。
    pub(crate) fn new(state: &GameState, balance: &BalanceTable) -> Routing {
        Routing::from_input(RouteGraphInput::from_state(state, balance))
    }

    fn from_input(input: RouteGraphInput) -> Routing {
        Routing {
            snap: Arc::new(RouteSnapshot::build(input)),
            dirty: false,
        }
    }

    /// 10 §5.3 の表の処理から呼ぶ。
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 手順 2 [02 D27]: 印があれば入力が同じでも再計算して `Some(node_count)`(12 RT-D-10)。
    pub(crate) fn recompute_if_dirty(
        &mut self,
        state: &GameState,
        balance: &BalanceTable,
    ) -> Option<u32> {
        if !self.dirty {
            return None;
        }
        *self = Routing::new(state, balance);
        Some(self.snap.nodes.node_count())
    }

    /// INV-13 の追加分: 表を作った入力がいまの状態から作った入力と一致するか(12 RT-D-14)。
    /// `Err` は食い違いの説明。
    pub(crate) fn check_up_to_date(
        &self,
        state: &GameState,
        balance: &BalanceTable,
    ) -> Result<(), String> {
        let now = RouteGraphInput::from_state(state, balance);
        let built = &self.snap.input;
        if *built == now {
            return Ok(());
        }
        if built.station_count != now.station_count {
            return Err(format!(
                "経路表の駅の数 {} != 状態 {}",
                built.station_count, now.station_count
            ));
        }
        let differs = built.lines.iter().zip(&now.lines).position(|(a, b)| a != b);
        Err(match differs {
            Some(l) => format!(
                "経路表の路線 L{l} {:?} != 状態 {:?}",
                built.lines[l], now.lines[l]
            ),
            None if built.lines.len() != now.lines.len() => format!(
                "経路表の路線の数 {} != 状態 {}",
                built.lines.len(),
                now.lines.len()
            ),
            None => format!("経路表の入力 {built:?} != 状態 {now:?}"),
        })
    }

    fn has_station(&self, s: StationId) -> bool {
        s.0 < self.snap.nodes.station_count()
    }

    /// 駅 `at` で待つ、目的駅 `dest` の乗客が乗る列車の条件。`at == dest` か到達できなければ `None`。
    pub(crate) fn boarding_choice(&self, at: StationId, dest: StationId) -> Option<BoardChoice> {
        if at == dest {
            return None;
        }
        if !(self.has_station(at) && self.has_station(dest)) {
            debug_assert!(false, "未出現の駅への問い合わせ {at} → {dest}");
            return None;
        }
        let (table, m) = (&self.snap.table, &self.snap.nodes);
        let t = m.out_node(dest);
        let b = table.next_hop(m.in_node(at), t)?;
        let NodeKind::Board(_, line) = m.kind(b) else {
            debug_assert!(false, "入口の次ホップが乗車ノードでない");
            return None;
        };
        match table.next_hop(b, t).map(|v| m.kind(v)) {
            Some(NodeKind::Board(s2, l2)) if l2 == line && s2 != at => Some(BoardChoice {
                line,
                next_stop: s2,
            }),
            other => {
                debug_assert!(false, "Q-1 が破れた: {at} → {dest} で {other:?}");
                None
            }
        }
    }

    /// `boarding_choice(at, dest) == Some(BoardChoice { line, next_stop: train_next_stop })`。
    pub(crate) fn wants_to_board(
        &self,
        at: StationId,
        dest: StationId,
        line: LineId,
        train_next_stop: StationId,
    ) -> bool {
        self.boarding_choice(at, dest)
            == Some(BoardChoice {
                line,
                next_stop: train_next_stop,
            })
    }

    /// 路線 `line` で停車駅 `at` に着いた、目的駅 `dest` の乗客の計画(表だけで決まる部分)。
    pub(crate) fn ride_plan(&self, line: LineId, at: StationId, dest: StationId) -> RidePlan {
        if !(self.has_station(at) && self.has_station(dest)) {
            debug_assert!(false, "未出現の駅への問い合わせ {at} → {dest}");
            return RidePlan::Unreachable;
        }
        let m = &self.snap.nodes;
        let Some(b) = m.board_node(at, line) else {
            debug_assert!(false, "{line} は {at} に停車しない");
            return RidePlan::Unreachable;
        };
        let Some(v) = self.snap.table.next_hop(b, m.out_node(dest)) else {
            return RidePlan::Unreachable;
        };
        match m.kind(v) {
            NodeKind::Out(_) => RidePlan::Arrive,
            NodeKind::Board(s2, l2) if l2 == line => RidePlan::Continue { next_stop: s2 },
            NodeKind::Board(_, l2) => RidePlan::Transfer { to_line: l2 },
            NodeKind::In(_) => {
                debug_assert!(false, "乗車ノードから入口への辺はない");
                RidePlan::Unreachable
            }
        }
    }

    /// `ride_plan` を列車の向きと照合した行動。`train_next_stop` は `at` を出発したあとに向かう停車駅(12 RT-D-12)。
    pub(crate) fn rider_action(
        &self,
        line: LineId,
        at: StationId,
        dest: StationId,
        train_next_stop: StationId,
    ) -> RiderAction {
        match self.ride_plan(line, at, dest) {
            RidePlan::Arrive => RiderAction::Arrive,
            RidePlan::Continue { next_stop } if next_stop == train_next_stop => RiderAction::Stay,
            RidePlan::Continue { .. } | RidePlan::Transfer { .. } => RiderAction::AlightToWait,
            RidePlan::Unreachable => {
                debug_assert!(false, "経路のない乗客 {line} {at} → {dest}");
                RiderAction::AlightToWait
            }
        }
    }

    /// 駅 `from` から行ける目的駅(`from` 以外)を `StationId` の昇順に。
    pub(crate) fn reachable_dests(&self, from: StationId) -> impl Iterator<Item = StationId> + '_ {
        let valid = self.has_station(from);
        debug_assert!(valid, "未出現の駅 {from}");
        let n = if valid {
            self.snap.nodes.station_count()
        } else {
            0
        };
        (0..n)
            .map(StationId)
            .filter(move |&t| t != from && self.station_cost(from, t).is_some())
    }

    /// 駅の間の最短コスト `D(from, to)`。`from == to` なら `Some(0)`。
    pub(crate) fn station_cost(&self, from: StationId, to: StationId) -> Option<RouteCost> {
        if !(self.has_station(from) && self.has_station(to)) {
            debug_assert!(false, "未出現の駅への問い合わせ {from} → {to}");
            return None;
        }
        if from == to {
            return Some(Cost::ZERO);
        }
        let m = &self.snap.nodes;
        self.snap.table.cost(m.in_node(from), m.out_node(to))
    }
}

#[cfg(test)]
mod tests {
    use std::ops::RangeInclusive;
    use std::sync::OnceLock;

    use proptest::collection::vec;
    use proptest::prelude::*;
    use proptest::test_runner::RngSeed;
    use tm_pathfind::FloydWarshall;

    use super::*;
    use crate::geom::GridPos;
    use crate::hash::StateHasher;
    use crate::ids::{CompanyLevel, Level};
    use crate::param::{FeatureSet, LineLevels, ParamSet, StationLevels};
    use crate::rng::{RngStreams, StreamRng, below};
    use crate::state::{
        CompanyState, INCOME_WINDOW_SECS, Line, PaxMatrix, Progress, RebirthState, Section,
        Station, Stats, TrackGrid,
    };
    use crate::time::Tick;
    use crate::units::{MilliPeople, Money, People, section_length};

    const SEED_A: u64 = 1;
    /// G-04 のゴールデン(12 §8.5)。
    const G04_TABLE_HASH: u64 = 0xE645_B77C_F2FA_38DD;
    const MAX_TEST_STATIONS: u32 = 32;

    fn s(i: u32) -> StationId {
        StationId(i)
    }

    fn l(i: u32) -> LineId {
        LineId(i)
    }

    fn n(i: u32) -> NodeId {
        NodeId(i)
    }

    fn balance() -> &'static BalanceTable {
        static B: OnceLock<BalanceTable> = OnceLock::new();
        B.get_or_init(BalanceTable::builtin)
    }

    fn config() -> ProptestConfig {
        let mut c = ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        };
        if matches!(c.rng_seed, RngSeed::Random) {
            c.rng_seed = RngSeed::Fixed(SEED_A);
        }
        c
    }

    fn line(stops: &[u32], gaps: &[u32]) -> RouteLineInput {
        RouteLineInput {
            stops: stops.iter().map(|&i| s(i)).collect(),
            gaps: gaps.iter().map(|&g| Tiles(g)).collect(),
            speed: SpeedMilli(1000),
            attractiveness: Permille(1000),
        }
    }

    fn input(station_count: u32, lines: Vec<RouteLineInput>) -> RouteGraphInput {
        RouteGraphInput {
            station_count,
            transfer_cost: Cost(1000),
            stop_allowance_s: 2,
            lines,
        }
    }

    fn g02() -> RouteGraphInput {
        input(4, vec![line(&[0, 1, 2], &[6, 4]), line(&[1, 3], &[5])])
    }

    fn g03() -> RouteGraphInput {
        input(2, vec![line(&[0, 1], &[6]), line(&[0, 1], &[6])])
    }

    fn edge_cost(g: &Graph, a: u32, b: u32) -> Option<u64> {
        g.edges_from(n(a))
            .iter()
            .filter(|e| e.0 == n(b))
            .map(|e| e.1.0)
            .min()
    }

    fn dist(r: &Routing, a: u32, b: u32) -> Option<u64> {
        r.snap.table.cost(n(a), n(b)).map(|c| c.0)
    }

    fn hop(r: &Routing, a: u32, b: u32) -> Option<u32> {
        r.snap.table.next_hop(n(a), n(b)).map(|v| v.0)
    }

    fn choice(line: u32, next_stop: u32) -> Option<BoardChoice> {
        Some(BoardChoice {
            line: l(line),
            next_stop: s(next_stop),
        })
    }

    /// 駅の列・各区間のマス・停車の有無で表した路線(テスト用)。
    #[derive(Clone, Debug)]
    struct LineSpec {
        stations: Vec<u32>,
        stops: Vec<bool>,
        tiles: Vec<u32>,
        speed: Level,
        attr: Level,
    }

    impl LineSpec {
        fn new(stations: &[u32], tiles: &[u32], stops: &[bool]) -> LineSpec {
            LineSpec {
                stations: stations.to_vec(),
                stops: stops.to_vec(),
                tiles: tiles.to_vec(),
                speed: Level::ZERO,
                attr: Level::ZERO,
            }
        }

        fn offsets(&self) -> Vec<u32> {
            let mut acc = 0;
            std::iter::once(0)
                .chain(self.tiles.iter().map(|&t| {
                    acc += t;
                    acc
                }))
                .collect()
        }

        fn stop_indices(&self) -> Vec<usize> {
            (0..self.stations.len())
                .filter(|&i| self.stops[i])
                .collect()
        }

        fn input(&self) -> RouteLineInput {
            let off = self.offsets();
            let idx = self.stop_indices();
            RouteLineInput {
                stops: idx.iter().map(|&i| s(self.stations[i])).collect(),
                gaps: idx
                    .windows(2)
                    .map(|w| Tiles(off[w[1]] - off[w[0]]))
                    .collect(),
                speed: balance().speed(self.speed),
                attractiveness: balance().attractiveness(self.attr),
            }
        }

        fn line(&self, id: usize) -> Line {
            let off = self.offsets();
            let sections = self
                .stop_indices()
                .windows(2)
                .map(|w| {
                    let tiles = Tiles(off[w[1]] - off[w[0]]);
                    Section {
                        from: w[0] as u16,
                        to: w[1] as u16,
                        tiles,
                        length: section_length(tiles),
                    }
                })
                .collect();
            Line {
                id: LineId::from_index(id),
                stations: self.stations.iter().map(|&i| s(i)).collect(),
                segments: Vec::new(),
                levels: LineLevels {
                    speed: self.speed,
                    attractiveness: self.attr,
                    ..LineLevels::default()
                },
                trains: Vec::new(),
                stops: self.stops.clone(),
                sections,
                offsets: off,
                created_at: Tick(0),
                born_level: crate::ids::CompanyLevel::ZERO,
            }
        }
    }

    #[derive(Clone, Debug)]
    struct NetSpec {
        station_count: u32,
        lines: Vec<LineSpec>,
        transfer_cost: u64,
    }

    impl NetSpec {
        fn input(&self) -> RouteGraphInput {
            RouteGraphInput {
                station_count: self.station_count,
                transfer_cost: Cost(self.transfer_cost),
                stop_allowance_s: balance().spec().transit.stop_allowance_s,
                lines: self.lines.iter().map(LineSpec::input).collect(),
            }
        }

        fn routing(&self) -> Routing {
            Routing::from_input(self.input())
        }
    }

    fn line_spec(ns: u32) -> impl Strategy<Value = LineSpec> {
        let n = ns as usize;
        (
            Just((0..ns).collect::<Vec<u32>>()).prop_shuffle(),
            2..=n,
            vec(proptest::bool::weighted(0.8), n),
            vec(1u32..=39, n),
            0u8..=10,
            0u8..=10,
        )
            .prop_map(|(order, len, stops, tiles, sp, at)| LineSpec {
                stations: order[..len].to_vec(),
                stops: stops[..len].to_vec(),
                tiles: tiles[..len - 1].to_vec(),
                speed: Level(sp),
                attr: Level(at),
            })
    }

    /// 12 §8.6 の「ゲーム形」: 駅 2..=32、路線 0..=6、区間 1..=39 マス、速度・魅力度 Lv 0..=10。
    fn net_spec(transfer: RangeInclusive<u64>) -> impl Strategy<Value = NetSpec> {
        (2u32..=MAX_TEST_STATIONS, transfer)
            .prop_flat_map(|(ns, tc)| (Just(ns), vec(line_spec(ns), 0..=6), Just(tc)))
            .prop_map(|(station_count, lines, transfer_cost)| NetSpec {
                station_count,
                lines,
                transfer_cost,
            })
    }

    fn test_state(station_count: u32, lines: &[LineSpec]) -> GameState {
        let lines: Vec<Line> = lines.iter().enumerate().map(|(i, x)| x.line(i)).collect();
        let stations = (0..station_count)
            .map(|i| Station {
                id: s(i),
                pos: GridPos {
                    x: 2 * i as i32 + 1,
                    y: 1,
                },
                spawned_at: Tick(0),
                levels: StationLevels::default(),
                spawn_acc: MilliPeople::ZERO,
                waiting_total: People::ZERO,
                serving_lines: lines
                    .iter()
                    .filter(|x| x.stations.contains(&s(i)))
                    .map(|x| x.id)
                    .collect(),
            })
            .collect();
        let mut state = GameState {
            tick: Tick(0),
            money: Money::ZERO,
            company: CompanyState {
                level: CompanyLevel::ZERO,
                line_slots: Level::ZERO,
                track_tiles: 0,
            },
            station_plan: Vec::new(),
            stations,
            lines,
            trains: Vec::new(),
            waiting: PaxMatrix::new(balance().max_stations()),
            onboard: PaxMatrix::new(balance().max_stations()),
            tracks: TrackGrid::new(balance().spec().map.width, balance().spec().map.height),
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
            rng: RngStreams::new(SEED_A),
            routing: Routing::default(),
            map_revision: 0,
            #[cfg(feature = "test-support")]
            ledger: Default::default(),
        };
        state.routing = Routing::new(&state, balance());
        state
    }

    /// `state.routing` の再計算。`Routing` の複製は表を複製しないので、借用を分けるのに使う。
    fn recompute(state: &mut GameState) -> Option<u32> {
        let mut r = state.routing.clone();
        let nc = r.recompute_if_dirty(state, balance());
        state.routing = r;
        nc
    }

    fn push_station(state: &mut GameState) {
        let id = StationId::from_index(state.stations.len());
        state.stations.push(Station {
            id,
            pos: GridPos { x: 30, y: 16 },
            spawned_at: Tick(0),
            levels: StationLevels::default(),
            spawn_acc: MilliPeople::ZERO,
            waiting_total: People::ZERO,
            serving_lines: Vec::new(),
        });
    }

    /// 乗客の位置。`forward` は路線の停車駅の添字が増える向き。
    #[derive(Clone, Copy, Debug)]
    enum Pos {
        Waiting(StationId),
        Riding {
            line: LineId,
            at: StationId,
            forward: bool,
        },
    }

    /// 12 §8.4 R-09 の抽象化した乗客。どの停車駅にもどちら向きにもすぐ列車が来る。
    #[derive(Debug)]
    struct Walker {
        pos: Pos,
        dest: StationId,
        cost: u64,
        transfers: u64,
        turned_back: u64,
        moves: u32,
    }

    fn index_of(stops: &[StationId], at: StationId) -> usize {
        stops.iter().position(|&x| x == at).expect("停車駅にいる")
    }

    impl Walker {
        fn new(from: StationId, dest: StationId) -> Walker {
            Walker {
                pos: Pos::Waiting(from),
                dest,
                cost: 0,
                transfers: 0,
                turned_back: 0,
                moves: 0,
            }
        }

        fn ride(&mut self, input: &RouteGraphInput, line: LineId, i: usize, forward: bool) {
            let ln = &input.lines[line.index()];
            let (j, gap) = if forward {
                (i + 1, ln.gaps[i])
            } else {
                (i - 1, ln.gaps[i - 1])
            };
            self.cost += ride_cost(gap, ln.speed, ln.attractiveness, input.stop_allowance_s).0;
            self.pos = Pos::Riding {
                line,
                at: ln.stops[j],
                forward,
            };
        }

        /// 問い合わせを 1 回して進める。着いたら `true`。
        fn step(&mut self, r: &Routing, input: &RouteGraphInput) -> Result<bool, TestCaseError> {
            self.moves += 1;
            match self.pos {
                Pos::Waiting(at) => {
                    let Some(c) = r.boarding_choice(at, self.dest) else {
                        return Err(TestCaseError::fail(format!(
                            "{at} から {} への経路がない",
                            self.dest
                        )));
                    };
                    let stops = &input.lines[c.line.index()].stops;
                    let i = index_of(stops, at);
                    let forward = stops.get(i + 1) == Some(&c.next_stop);
                    self.ride(input, c.line, i, forward);
                }
                Pos::Riding {
                    line,
                    at,
                    mut forward,
                } => {
                    let stops = &input.lines[line.index()].stops;
                    let i = index_of(stops, at);
                    if forward && i + 1 == stops.len() {
                        forward = false;
                    } else if !forward && i == 0 {
                        forward = true;
                    }
                    let next = if forward { stops[i + 1] } else { stops[i - 1] };
                    match r.rider_action(line, at, self.dest, next) {
                        RiderAction::Arrive => return Ok(true),
                        RiderAction::Stay => self.ride(input, line, i, forward),
                        RiderAction::AlightToWait => {
                            match r.ride_plan(line, at, self.dest) {
                                RidePlan::Transfer { .. } => self.transfers += 1,
                                _ => self.turned_back += 1,
                            }
                            self.pos = Pos::Waiting(at);
                        }
                    }
                }
            }
            Ok(false)
        }
    }

    /// 入力を「増やす」変更(駅・路線・停車駅の追加、速度・魅力度の上昇)を 1 つ当てる(G-8)。
    fn grow(input: &mut RouteGraphInput, (kind, a, b, c): (u8, u32, u32, u32)) {
        let ns = input.station_count;
        match kind {
            0 => input.station_count = (ns + 1).min(MAX_TEST_STATIONS),
            1 if input.lines.len() < 6 => {
                let x = a % ns;
                let y = (x + 1 + b % (ns - 1)) % ns;
                input.lines.push(line(&[x, y], &[1 + c % 39]));
            }
            2 | 3 if !input.lines.is_empty() => {
                let li = a as usize % input.lines.len();
                let ln = &mut input.lines[li];
                let free: Vec<StationId> =
                    (0..ns).map(s).filter(|x| !ln.stops.contains(x)).collect();
                if free.is_empty() {
                    return;
                }
                let x = free[b as usize % free.len()];
                if kind == 2 {
                    let gap = Tiles(1 + (c / 2) % 39);
                    if ln.stops.is_empty() {
                        ln.stops.push(x);
                    } else if c % 2 == 0 {
                        ln.stops.insert(0, x);
                        ln.gaps.insert(0, gap);
                    } else {
                        ln.stops.push(x);
                        ln.gaps.push(gap);
                    }
                } else {
                    let split: Vec<usize> =
                        (0..ln.gaps.len()).filter(|&k| ln.gaps[k].0 >= 2).collect();
                    if split.is_empty() {
                        return;
                    }
                    let k = split[c as usize % split.len()];
                    let first = 1 + (c / 7) % (ln.gaps[k].0 - 1);
                    let rest = ln.gaps[k].0 - first;
                    ln.gaps[k] = Tiles(first);
                    ln.gaps.insert(k + 1, Tiles(rest));
                    ln.stops.insert(k + 1, x);
                }
            }
            4 if !input.lines.is_empty() => {
                let li = a as usize % input.lines.len();
                let v = &mut input.lines[li].speed;
                *v = SpeedMilli((v.0 + 500).min(6000));
            }
            5 if !input.lines.is_empty() => {
                let li = a as usize % input.lines.len();
                let v = &mut input.lines[li].attractiveness;
                *v = Permille((v.0 + 200).min(3000));
            }
            _ => {}
        }
    }

    /// 全駅の `Out(T)` の列の、距離(列の昇順)と次ホップ(同じ順)のハッシュ(12 §8.5 G-04)。
    fn out_columns_hash(t: &RouteTable, m: &NodeMap) -> u64 {
        let nc = t.node_count();
        let outs: Vec<NodeId> = (0..m.station_count()).map(|i| m.out_node(s(i))).collect();
        let mut h = StateHasher::new();
        h.write_u32(nc);
        h.write_u32(outs.len() as u32);
        for &to in &outs {
            for u in 0..nc {
                h.write_u64(t.cost(n(u), to).map_or(u64::MAX, |c| c.0));
            }
        }
        for &to in &outs {
            for u in 0..nc {
                h.write_u32(t.next_hop(n(u), to).map_or(u32::MAX, |x| x.0));
            }
        }
        h.finish()
    }

    /// 01 の最大(27 駅、6 路線がすべて 27 駅に停車)。駅の順・区間・レベルは SEED_A の乱数で決める。
    fn g04_input() -> RouteGraphInput {
        const NS: u32 = 27;
        let mut rng = StreamRng::new(SEED_A, 0);
        let lines = (0..6)
            .map(|_| {
                let mut order: Vec<u32> = (0..NS).collect();
                for i in (1..order.len()).rev() {
                    let j = below(&mut rng, i as u32 + 1) as usize;
                    order.swap(i, j);
                }
                let tiles: Vec<u32> = (1..NS).map(|_| 1 + below(&mut rng, 39)).collect();
                let mut spec = LineSpec::new(&order, &tiles, &[true; NS as usize]);
                spec.speed = Level(below(&mut rng, 11) as u8);
                spec.attr = Level(below(&mut rng, 11) as u8);
                spec.input()
            })
            .collect();
        RouteGraphInput {
            station_count: NS,
            transfer_cost: Cost(balance().spec().transit.transfer_cost),
            stop_allowance_s: balance().spec().transit.stop_allowance_s,
            lines,
        }
    }

    /// 方式を指定して作った `Routing`(R-13 の比較用)。
    fn routing_with(input: RouteGraphInput, method: Method) -> Routing {
        let (graph, nodes) = build_graph(&input);
        let table = compute_table(&graph, &nodes, method);
        Routing {
            snap: Arc::new(RouteSnapshot {
                table,
                nodes,
                input,
            }),
            dirty: false,
        }
    }

    fn uses_destinations_only(r: &Routing) -> bool {
        let (t, m) = (&r.snap.table, &r.snap.nodes);
        (0..m.station_count())
            .all(|i| t.has_target(m.out_node(s(i))) && !t.has_target(m.in_node(s(i))))
            && m.station_count() > 0
    }

    /// 12 §8.4 R-13: §6 の問い合わせがすべての駅・路線・目的駅の組で一致する。
    fn assert_same_queries(a: &Routing, b: &Routing) -> Result<(), TestCaseError> {
        let input = &a.snap.input;
        prop_assert_eq!(input, &b.snap.input);
        let ns = input.station_count;
        for from in (0..ns).map(s) {
            prop_assert!(
                a.reachable_dests(from).eq(b.reachable_dests(from)),
                "{}",
                from
            );
            for dest in (0..ns).map(s) {
                prop_assert_eq!(a.station_cost(from, dest), b.station_cost(from, dest));
                prop_assert_eq!(a.boarding_choice(from, dest), b.boarding_choice(from, dest));
            }
        }
        for (li, ln) in input.lines.iter().enumerate() {
            let li = LineId::from_index(li);
            for (i, &at) in ln.stops.iter().enumerate() {
                let nexts: Vec<StationId> = [i.checked_sub(1), Some(i + 1)]
                    .into_iter()
                    .flatten()
                    .filter_map(|j| ln.stops.get(j).copied())
                    .collect();
                for dest in (0..ns).map(s).filter(|&d| a.station_cost(at, d).is_some()) {
                    prop_assert_eq!(a.ride_plan(li, at, dest), b.ride_plan(li, at, dest));
                    for &x in &nexts {
                        prop_assert_eq!(
                            a.rider_action(li, at, dest, x),
                            b.rider_action(li, at, dest, x)
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// ノード数が 193〜256 のゲーム形(駅 27..=32、6 路線、ほぼ全駅に停車)。
    fn big_net_spec() -> impl Strategy<Value = NetSpec> {
        (27u32..=MAX_TEST_STATIONS)
            .prop_flat_map(|ns| {
                let n = ns as usize;
                let line = (
                    Just((0..ns).collect::<Vec<u32>>()).prop_shuffle(),
                    (n - 3)..=n,
                    vec(proptest::bool::weighted(0.95), n),
                    vec(1u32..=39, n),
                    0u8..=10,
                    0u8..=10,
                )
                    .prop_map(|(order, len, stops, tiles, sp, at)| LineSpec {
                        stations: order[..len].to_vec(),
                        stops: stops[..len].to_vec(),
                        tiles: tiles[..len - 1].to_vec(),
                        speed: Level(sp),
                        attr: Level(at),
                    });
                (Just(ns), vec(line, 6))
            })
            .prop_map(|(station_count, lines)| NetSpec {
                station_count,
                lines,
                transfer_cost: 1000,
            })
            .prop_filter("ノード数 > 192", |net| {
                build_graph(&net.input()).1.node_count() > FALLBACK_NODE_THRESHOLD
            })
    }

    fn check_q1_q2(net: &NetSpec, r: &Routing) -> Result<(), TestCaseError> {
        let inp = net.input();
        let ns = net.station_count;
        for at in (0..ns).map(s) {
            for dest in (0..ns).map(s) {
                let Some(c) = r.boarding_choice(at, dest) else {
                    prop_assert!(at == dest || r.station_cost(at, dest).is_none());
                    continue;
                };
                let stops = &inp.lines[c.line.index()].stops;
                let i = index_of(stops, at);
                let adjacent = (i > 0 && stops[i - 1] == c.next_stop)
                    || stops.get(i + 1) == Some(&c.next_stop);
                prop_assert!(adjacent && c.next_stop != at, "Q-1: {at} -> {dest}: {c:?}");
            }
        }
        for (li, ln) in inp.lines.iter().enumerate() {
            let li = LineId::from_index(li);
            for (i, &at) in ln.stops.iter().enumerate() {
                let nexts = [i.checked_sub(1), Some(i + 1)]
                    .into_iter()
                    .flatten()
                    .filter_map(|j| ln.stops.get(j).copied());
                for x in nexts {
                    for dest in (0..ns).map(s).filter(|&d| r.station_cost(at, d).is_some()) {
                        if r.rider_action(li, at, dest, x) == RiderAction::AlightToWait {
                            prop_assert!(
                                !r.wants_to_board(at, dest, li, x),
                                "Q-2: {li} {at} -> {dest} via {x}"
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn check_walks(net: &NetSpec, r: &Routing) -> Result<(), TestCaseError> {
        let inp = net.input();
        let nc = r.snap.nodes.node_count();
        for from in (0..net.station_count).map(s) {
            for dest in r.reachable_dests(from) {
                let mut w = Walker::new(from, dest);
                while !w.step(r, &inp)? {
                    prop_assert!(w.moves <= nc, "Q-3: {from} -> {dest} が {nc} 手で着かない");
                }
                prop_assert!(w.moves <= nc);
                prop_assert_eq!(w.turned_back, 0);
                let total = w.cost + w.transfers * net.transfer_cost;
                prop_assert_eq!(
                    Some(total),
                    r.station_cost(from, dest).map(|c| c.0),
                    "{} -> {}",
                    from,
                    dest
                );
            }
        }
        Ok(())
    }

    fn first_difference(a: &RouteTable, b: &RouteTable) -> Option<String> {
        let nc = a.node_count();
        if nc != b.node_count() {
            return Some(format!("node_count {nc} != {}", b.node_count()));
        }
        (0..nc)
            .flat_map(|u| (0..nc).map(move |t| (n(u), n(t))))
            .find_map(|(u, t)| {
                let x = (a.cost(u, t), a.next_hop(u, t));
                let y = (b.cost(u, t), b.next_hop(u, t));
                (x != y).then(|| format!("{u:?} -> {t:?}: {x:?} != {y:?}"))
            })
    }

    #[test]
    fn r01_g02_node_ids_edges_and_golden() {
        let (g, m) = build_graph(&g02());
        assert_eq!(m.node_count(), 13);
        assert_eq!(g.edge_count(), 18);
        for i in 0..4 {
            assert_eq!(m.in_node(s(i)), n(2 * i));
            assert_eq!(m.out_node(s(i)), n(2 * i + 1));
            assert_eq!(m.kind(n(2 * i)), NodeKind::In(s(i)));
            assert_eq!(m.kind(n(2 * i + 1)), NodeKind::Out(s(i)));
        }
        let boards = [(0, 0, 8), (1, 0, 9), (2, 0, 10), (1, 1, 11), (3, 1, 12)];
        for (st, ln, id) in boards {
            assert_eq!(m.board_node(s(st), l(ln)), Some(n(id)));
            assert_eq!(m.kind(n(id)), NodeKind::Board(s(st), l(ln)));
        }
        assert_eq!(m.board_node(s(3), l(0)), None);
        assert_eq!(m.board_node(s(0), l(1)), None);
        for (a, b, w) in [(8, 9, 800), (9, 10, 600), (11, 12, 700), (9, 11, 1000)] {
            assert_eq!(edge_cost(&g, a, b), Some(w), "{a}->{b}");
            assert_eq!(edge_cost(&g, b, a), Some(w), "{b}->{a}");
        }

        let r = Routing::from_input(g02());
        let golden = [
            (0, 7, 2500, 8),
            (9, 7, 1700, 11),
            (2, 7, 700, 11),
            (4, 1, 1400, 10),
            (6, 1, 2500, 12),
            (2, 5, 600, 9),
            (6, 5, 2300, 12),
        ];
        for (a, b, d, h) in golden {
            assert_eq!(dist(&r, a, b), Some(d), "dist {a}->{b}");
            assert_eq!(hop(&r, a, b), Some(h), "next_hop {a}->{b}");
        }
        let path: Vec<u32> = r.snap.table.path(n(0), n(7)).map(|v| v.0).collect();
        assert_eq!(path, [0, 8, 9, 11, 12, 7]);

        assert_eq!(r.boarding_choice(s(0), s(3)), choice(0, 1));
        assert_eq!(r.boarding_choice(s(1), s(3)), choice(1, 3));
        assert_eq!(r.boarding_choice(s(2), s(0)), choice(0, 1));
        assert_eq!(r.boarding_choice(s(1), s(2)), choice(0, 2));
        assert_eq!(r.boarding_choice(s(3), s(3)), None);
        assert_eq!(
            r.ride_plan(l(0), s(1), s(3)),
            RidePlan::Transfer { to_line: l(1) }
        );
        assert_eq!(
            r.ride_plan(l(0), s(0), s(3)),
            RidePlan::Continue { next_stop: s(1) }
        );
        assert_eq!(
            r.ride_plan(l(1), s(1), s(3)),
            RidePlan::Continue { next_stop: s(3) }
        );
        assert_eq!(r.ride_plan(l(1), s(3), s(3)), RidePlan::Arrive);
        assert_eq!(r.station_cost(s(0), s(3)), Some(Cost(2500)));
        assert_eq!(r.station_cost(s(3), s(0)), Some(Cost(2500)));
        assert_eq!(r.station_cost(s(2), s(2)), Some(Cost::ZERO));
    }

    #[test]
    fn r02_g03_tie_picks_smaller_line_both_ways() {
        let r = Routing::from_input(g03());
        let m = &r.snap.nodes;
        let ids = [(0, 0, 4), (1, 0, 5), (0, 1, 6), (1, 1, 7)];
        for (st, ln, id) in ids {
            assert_eq!(m.board_node(s(st), l(ln)), Some(n(id)));
        }
        assert_eq!(dist(&r, 0, 3), Some(800));
        assert_eq!(hop(&r, 0, 3), Some(4));
        assert_eq!(r.boarding_choice(s(0), s(1)), choice(0, 1));
        assert_eq!(hop(&r, 2, 1), Some(5));
        assert_eq!(r.boarding_choice(s(1), s(0)), choice(0, 0));
        assert!(r.wants_to_board(s(0), s(1), l(0), s(1)));
        assert!(!r.wants_to_board(s(0), s(1), l(1), s(1)));

        let mut faster = g03();
        faster.lines[1].speed = SpeedMilli(1500);
        let r = Routing::from_input(faster);
        assert_eq!(r.boarding_choice(s(0), s(1)), choice(1, 1));
        assert_eq!(r.boarding_choice(s(1), s(0)), choice(1, 0));
    }

    #[test]
    fn r03_pass_station_has_no_board_node() {
        let specs = [
            LineSpec::new(&[0, 1, 2], &[3, 4], &[true, false, true]),
            LineSpec::new(&[1, 0], &[5], &[true, true]),
        ];
        let state = test_state(3, &specs);
        let input = &state.routing.snap.input;
        assert_eq!(input.lines[0].stops, [s(0), s(2)]);
        assert_eq!(input.lines[0].gaps, [Tiles(7)]);

        let (g, m) = build_graph(input);
        assert_eq!(m.board_node(s(1), l(0)), None);
        let (a, b) = (
            m.board_node(s(0), l(0)).unwrap(),
            m.board_node(s(2), l(0)).unwrap(),
        );
        let w = ride_cost(Tiles(7), SpeedMilli(1000), Permille(1000), 2).0;
        assert_eq!(w, 900);
        assert_eq!(edge_cost(&g, a.0, b.0), Some(w));
        assert_eq!(edge_cost(&g, b.0, a.0), Some(w));

        let r = &state.routing;
        assert_eq!(r.boarding_choice(s(1), s(2)), choice(1, 0));
        assert_eq!(
            r.ride_plan(l(1), s(0), s(2)),
            RidePlan::Transfer { to_line: l(0) }
        );
    }

    #[test]
    fn r04_pass_end_station_has_no_ride_edge_beyond() {
        let specs = [LineSpec::new(&[0, 1, 2], &[3, 4], &[true, true, false])];
        let state = test_state(3, &specs);
        let input = &state.routing.snap.input;
        assert_eq!(input.lines[0].stops, [s(0), s(1)]);
        assert_eq!(input.lines[0].gaps, [Tiles(3)]);
        let (g, m) = build_graph(input);
        assert_eq!(m.board_node(s(2), l(0)), None);
        let b1 = m.board_node(s(1), l(0)).unwrap();
        let mut to: Vec<NodeId> = g.edges_from(b1).iter().map(|e| e.0).collect();
        to.sort();
        assert_eq!(to, [m.out_node(s(1)), m.board_node(s(0), l(0)).unwrap()]);
        let r = &state.routing;
        assert_eq!(r.station_cost(s(0), s(2)), None);
        assert_eq!(r.boarding_choice(s(0), s(2)), None);
        assert_eq!(r.reachable_dests(s(0)).collect::<Vec<_>>(), [s(1)]);
    }

    #[test]
    fn r05_one_and_zero_stop_lines() {
        let inp = input(
            3,
            vec![
                line(&[0, 1], &[6]),
                line(&[0], &[]),
                line(&[], &[]),
                line(&[1, 2], &[4]),
            ],
        );
        let (g, m) = build_graph(&inp);
        assert_eq!(m.node_count(), 11);
        assert_eq!(m.board_node(s(0), l(1)), Some(n(8)));
        assert!((0..3).all(|i| m.board_node(s(i), l(2)).is_none()));
        assert_eq!(m.board_node(s(1), l(3)), Some(n(9)));
        let mut from8: Vec<u32> = g.edges_from(n(8)).iter().map(|e| e.0.0).collect();
        from8.sort();
        assert_eq!(from8, [1, 6]);

        let r = Routing::from_input(inp);
        for a in 0..3 {
            for b in (0..3).filter(|&b| b != a) {
                let path: Vec<NodeId> = r.snap.table.path(n(2 * a), n(2 * b + 1)).collect();
                assert!(!path.is_empty(), "S{a} -> S{b}");
                assert!(!path.contains(&n(8)), "S{a} -> S{b}: {path:?}");
            }
        }
        assert_eq!(
            r.ride_plan(l(1), s(0), s(2)),
            RidePlan::Transfer { to_line: l(0) }
        );
    }

    #[test]
    fn r06_g7_fails_without_transfer_cost() {
        let r = Routing::from_input(g02());
        let d = |a, b| r.station_cost(s(a), s(b)).unwrap().0;
        assert_eq!(d(0, 3), 2500);
        assert_eq!(d(0, 1) + d(1, 3), 1500);
        assert!(d(0, 3) > d(0, 1) + d(1, 3));
        assert!(d(0, 3) <= d(0, 1) + 1000 + d(1, 3));
    }

    #[test]
    fn ride_edge_uses_stop_allowance_from_input() {
        let mut fast = line(&[0, 1, 2], &[6, 3]);
        fast.speed = SpeedMilli(1500);
        fast.attractiveness = Permille(1200);
        for (stop_s, want) in [(0, [600, 167]), (5, [1100, 583])] {
            let mut inp = input(3, vec![line(&[0, 1], &[6]), fast.clone()]);
            inp.stop_allowance_s = stop_s;
            let (g, _) = build_graph(&inp);
            assert_eq!(edge_cost(&g, 6, 7), Some(want[0]), "stop_s={stop_s}");
            assert_eq!(edge_cost(&g, 9, 10), Some(want[1]), "stop_s={stop_s}");
            assert_eq!(edge_cost(&g, 10, 9), Some(want[1]), "stop_s={stop_s}");
        }
    }

    #[test]
    fn r07_new_unconnected_station_keeps_behaviour() {
        let specs = [
            LineSpec::new(&[0, 1, 2], &[6, 4], &[true; 3]),
            LineSpec::new(&[1, 3], &[5], &[true; 2]),
            LineSpec::new(&[0, 1], &[6], &[true; 2]),
        ];
        let mut state = test_state(4, &specs);
        let before = state.routing.clone();
        push_station(&mut state);
        state.routing.mark_dirty();
        assert_eq!(recompute(&mut state), Some(17));
        let after = &state.routing;
        assert_eq!(before.snap.nodes.board_node(s(0), l(0)), Some(n(8)));
        assert_eq!(after.snap.nodes.board_node(s(0), l(0)), Some(n(10)));
        assert_same_behaviour(&before, after, 4).unwrap();
    }

    fn assert_same_behaviour(
        before: &Routing,
        after: &Routing,
        ns: u32,
    ) -> Result<(), TestCaseError> {
        let input = &before.snap.input;
        for a in (0..ns).map(s) {
            for b in (0..ns).map(s) {
                prop_assert_eq!(
                    before.boarding_choice(a, b),
                    after.boarding_choice(a, b),
                    "{} -> {}",
                    a,
                    b
                );
            }
        }
        for (li, ln) in input.lines.iter().enumerate() {
            let li = LineId::from_index(li);
            for &at in &ln.stops {
                for dest in (0..ns).map(s) {
                    prop_assert_eq!(
                        before.ride_plan(li, at, dest),
                        after.ride_plan(li, at, dest),
                        "{} {} -> {}",
                        li,
                        at,
                        dest
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn r10_turnaround_at_end_stays_on_board() {
        let r = Routing::from_input(input(3, vec![line(&[0, 1, 2], &[3, 3])]));
        assert_eq!(
            r.ride_plan(l(0), s(2), s(0)),
            RidePlan::Continue { next_stop: s(1) }
        );
        assert_eq!(r.rider_action(l(0), s(2), s(0), s(1)), RiderAction::Stay);
        assert_eq!(r.rider_action(l(0), s(0), s(2), s(1)), RiderAction::Stay);
        assert_eq!(r.rider_action(l(0), s(2), s(2), s(1)), RiderAction::Arrive);
        assert_eq!(
            r.rider_action(l(0), s(1), s(0), s(2)),
            RiderAction::AlightToWait
        );
        assert!(!r.wants_to_board(s(1), s(0), l(0), s(2)));
        assert!(r.wants_to_board(s(1), s(0), l(0), s(0)));
    }

    #[test]
    fn r10_transfer_alights_even_if_direction_matches() {
        let r = Routing::from_input(g02());
        assert_eq!(
            r.rider_action(l(0), s(1), s(3), s(2)),
            RiderAction::AlightToWait
        );
        assert_eq!(r.rider_action(l(0), s(0), s(3), s(1)), RiderAction::Stay);
        assert_eq!(r.rider_action(l(1), s(3), s(3), s(1)), RiderAction::Arrive);
    }

    #[test]
    fn r11_check_up_to_date_detects_missing_mark() {
        let specs = [LineSpec::new(&[0, 1, 2], &[6, 4], &[true; 3])];
        let mut state = test_state(3, &specs);
        assert!(!state.routing.is_dirty());
        assert_eq!(state.routing.check_up_to_date(&state, balance()), Ok(()));

        state.lines[0].levels.speed = Level(1);
        assert!(state.routing.check_up_to_date(&state, balance()).is_err());
        assert_eq!(recompute(&mut state), None);
        assert!(state.routing.check_up_to_date(&state, balance()).is_err());

        state.routing.mark_dirty();
        assert!(state.routing.is_dirty());
        assert_eq!(recompute(&mut state), Some(9));
        assert!(!state.routing.is_dirty());
        assert_eq!(state.routing.check_up_to_date(&state, balance()), Ok(()));
        assert_eq!(recompute(&mut state), None);

        push_station(&mut state);
        assert!(state.routing.check_up_to_date(&state, balance()).is_err());
        state.routing.mark_dirty();
        recompute(&mut state);
        assert_eq!(state.routing.check_up_to_date(&state, balance()), Ok(()));

        state.lines[0].stops[1] = false;
        state.lines[0].sections = specs[0].line(0).sections[..0].to_vec();
        assert!(state.routing.check_up_to_date(&state, balance()).is_err());
    }

    #[test]
    fn recompute_runs_when_dirty_and_swaps_snapshot() {
        let state = test_state(3, &[LineSpec::new(&[0, 1, 2], &[6, 4], &[true; 3])]);
        let mut r = state.routing.clone();
        assert!(Arc::ptr_eq(&r.snap, &state.routing.snap));
        r.mark_dirty();
        assert_eq!(r.recompute_if_dirty(&state, balance()), Some(9));
        assert!(!Arc::ptr_eq(&r.snap, &state.routing.snap));
        assert_eq!(r, state.routing);
    }

    #[test]
    fn default_is_dirty_and_out_of_date() {
        let mut state = test_state(3, &[LineSpec::new(&[0, 1], &[6], &[true; 2])]);
        let built = state.routing.clone();
        state.routing = Routing::default();
        assert!(state.routing.is_dirty());
        assert!(state.routing.check_up_to_date(&state, balance()).is_err());
        assert_eq!(recompute(&mut state), Some(8));
        assert_eq!(state.routing, built);
    }

    #[test]
    fn from_state_reads_levels_stops_and_sections() {
        let mut specs = [
            LineSpec::new(&[2, 0, 1, 3], &[2, 3, 4], &[true, false, true, true]),
            LineSpec::new(&[3, 1], &[5], &[true; 2]),
        ];
        specs[0].speed = Level(2);
        specs[1].attr = Level(3);
        let state = test_state(4, &specs);
        let got = RouteGraphInput::from_state(&state, balance());
        let want = RouteGraphInput {
            station_count: 4,
            transfer_cost: Cost(1000),
            stop_allowance_s: 2,
            lines: vec![
                RouteLineInput {
                    stops: vec![s(2), s(1), s(3)],
                    gaps: vec![Tiles(5), Tiles(4)],
                    speed: SpeedMilli(2000),
                    attractiveness: Permille(1000),
                },
                RouteLineInput {
                    stops: vec![s(3), s(1)],
                    gaps: vec![Tiles(5)],
                    speed: SpeedMilli(1000),
                    attractiveness: Permille(1600),
                },
            ],
        };
        assert_eq!(got, want);
    }

    #[test]
    fn r12_reachable_dests_fixed() {
        let mut inp = g02();
        inp.station_count = 5;
        let r = Routing::from_input(inp);
        assert_eq!(
            r.reachable_dests(s(0)).collect::<Vec<_>>(),
            [s(1), s(2), s(3)]
        );
        assert_eq!(
            r.reachable_dests(s(3)).collect::<Vec<_>>(),
            [s(0), s(1), s(2)]
        );
        assert_eq!(r.reachable_dests(s(4)).count(), 0);
        assert_eq!(r.station_cost(s(4), s(4)), Some(Cost::ZERO));
        assert_eq!(r.station_cost(s(4), s(0)), None);
        assert_eq!(r.boarding_choice(s(0), s(4)), None);
    }

    #[test]
    fn method_switches_above_192_nodes() {
        assert_eq!(method_for(FALLBACK_NODE_THRESHOLD), Method::AllSources);
        assert_eq!(
            method_for(FALLBACK_NODE_THRESHOLD + 1),
            Method::DestinationsOnly
        );
        assert_eq!(FALLBACK_NODE_THRESHOLD, 192);
    }

    #[test]
    fn g04_full_network_out_columns_hash() {
        let inp = g04_input();
        let (g, m) = build_graph(&inp);
        assert_eq!(m.node_count(), 216);
        assert_eq!(g.edge_count(), 1446);
        assert_eq!(method_for(m.node_count()), Method::DestinationsOnly);
        let full = compute_table(&g, &m, Method::AllSources);
        assert_eq!(full, FloydWarshall.compute(&g));
        let part = compute_table(&g, &m, Method::DestinationsOnly);
        assert_eq!(out_columns_hash(&full, &m), G04_TABLE_HASH);
        assert_eq!(out_columns_hash(&part, &m), G04_TABLE_HASH);
    }

    #[test]
    fn r13_full_network_recompute_uses_destinations_only() {
        let inp = g04_input();
        let r = Routing::from_input(inp.clone());
        assert!(uses_destinations_only(&r));
        let full = routing_with(inp, Method::AllSources);
        assert!(!uses_destinations_only(&full));
        assert_same_queries(&full, &r).unwrap();

        let specs: Vec<LineSpec> = (0..6)
            .map(|i| {
                let order: Vec<u32> = (0..27).map(|k| (k * 5 + i) % 27).collect();
                LineSpec::new(&order, &[3; 26], &[true; 27])
            })
            .collect();
        let mut state = test_state(27, &specs);
        assert_eq!(state.routing.snap.nodes.node_count(), 216);
        assert!(uses_destinations_only(&state.routing));
        state.routing.mark_dirty();
        assert_eq!(recompute(&mut state), Some(216));
        assert!(uses_destinations_only(&state.routing));
        assert_eq!(state.routing.check_up_to_date(&state, balance()), Ok(()));
    }

    #[test]
    fn r13_threshold_boundary() {
        let lines = |ns: u32, len: usize, last_stops: usize| -> Vec<LineSpec> {
            (0..6)
                .map(|i| {
                    let order: Vec<u32> = (0..len as u32).map(|k| (k + i) % ns).collect();
                    let stops: Vec<bool> = (0..len).map(|k| i < 5 || k < last_stops).collect();
                    LineSpec::new(&order, &vec![2; len - 1], &stops)
                })
                .collect()
        };
        for (ns, len, last_stops, nodes, dest_only) in [
            (24, 24, 24, 192, false),
            (25, 24, 22, 192, false),
            (25, 24, 23, 193, true),
            (25, 24, 24, 194, true),
            (32, 32, 32, 256, true),
        ] {
            let net = NetSpec {
                station_count: ns,
                lines: lines(ns, len, last_stops),
                transfer_cost: 1000,
            };
            let r = net.routing();
            assert_eq!(r.snap.nodes.node_count(), nodes);
            assert_eq!(uses_destinations_only(&r), dest_only, "n = {nodes}");
            let other = if dest_only {
                Method::AllSources
            } else {
                Method::DestinationsOnly
            };
            assert_same_queries(&r, &routing_with(net.input(), other)).unwrap();
        }
    }

    #[test]
    fn pathfind_bench_uses_the_same_threshold() {
        let bench = include_str!("../../tm-pathfind/benches/apsp.rs");
        let line = format!("const THRESHOLD: u32 = {FALLBACK_NODE_THRESHOLD};");
        assert!(bench.contains(&line), "benches/apsp.rs に {line} がない");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "停車しない")]
    fn ride_plan_at_non_stop_panics_in_debug() {
        let r = Routing::from_input(g02());
        let _ = r.ride_plan(l(1), s(0), s(3));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "未出現の駅")]
    fn query_for_unknown_station_panics_in_debug() {
        let r = Routing::from_input(g02());
        let _ = r.boarding_choice(s(0), s(4));
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn bad_queries_are_harmless_in_release() {
        let r = Routing::from_input(g02());
        assert_eq!(r.boarding_choice(s(0), s(4)), None);
        assert_eq!(r.ride_plan(l(1), s(0), s(3)), RidePlan::Unreachable);
        assert_eq!(r.station_cost(s(9), s(0)), None);
        assert_eq!(r.reachable_dests(s(9)).count(), 0);
    }

    proptest! {
        #![proptest_config(config())]

        #[test]
        fn d03_game_graph_fw_eq_dijkstra(net in net_spec(1000..=1000)) {
            let (g, _) = build_graph(&net.input());
            let fw = FloydWarshall.compute(&g);
            let dj = RepeatedDijkstra.compute(&g);
            if let Some(diff) = first_difference(&fw, &dj) {
                return Err(TestCaseError::fail(diff));
            }
            prop_assert_eq!(fw, dj);
        }

        #[test]
        fn from_state_matches_spec(net in net_spec(1000..=1000)) {
            let state = test_state(net.station_count, &net.lines);
            prop_assert_eq!(RouteGraphInput::from_state(&state, balance()), net.input());
            prop_assert_eq!(state.routing.check_up_to_date(&state, balance()), Ok(()));
        }

        #[test]
        fn r06_graph_properties(net in net_spec(1..=2000)) {
            let inp = net.input();
            let (g, m) = build_graph(&inp);
            for u in 0..m.node_count() {
                for &(v, w) in g.edges_from(n(u)) {
                    let (ku, kv) = (m.kind(n(u)), m.kind(v));
                    prop_assert!(!matches!(ku, NodeKind::Out(_)), "G-1: {ku:?} -> {kv:?}");
                    prop_assert!(!matches!(kv, NodeKind::In(_)), "G-1: {ku:?} -> {kv:?}");
                    let zero_ok = matches!(
                        (ku, kv),
                        (NodeKind::In(_), NodeKind::Board(..)) | (NodeKind::Board(..), NodeKind::Out(_))
                    );
                    prop_assert_eq!(w.0 == 0, zero_ok, "G-2/G-3: {:?} -> {:?} = {}", ku, kv, w.0);
                }
            }
            let r = Routing::from_input(inp);
            let ns = net.station_count;
            let d = |a: u32, b: u32| r.station_cost(s(a), s(b)).map(|c| c.0);
            for a in 0..ns {
                for b in 0..ns {
                    prop_assert_eq!(d(a, b), d(b, a), "G-4: S{} S{}", a, b);
                    for c in 0..ns {
                        if let (Some(ab), Some(bc), Some(ac)) = (d(a, b), d(b, c), d(a, c)) {
                            prop_assert!(ac <= ab + net.transfer_cost + bc, "G-7: S{a} S{b} S{c}");
                        } else if d(a, b).is_some() && d(b, c).is_some() {
                            prop_assert!(false, "G-7: S{a} -> S{c} に行けない");
                        }
                    }
                }
            }
        }

        #[test]
        fn r07_renumbering_keeps_behaviour(net in net_spec(1000..=1000)) {
            prop_assume!(net.station_count < MAX_TEST_STATIONS);
            let before = net.routing();
            let mut inp = net.input();
            inp.station_count += 1;
            let after = Routing::from_input(inp);
            assert_same_behaviour(&before, &after, net.station_count)?;
        }

        #[test]
        fn r08_q1_q2(net in net_spec(1000..=1000)) {
            check_q1_q2(&net, &net.routing())?;
        }

        #[test]
        fn r09_walk_follows_shortest_cost(net in net_spec(1..=2000)) {
            check_walks(&net, &net.routing())?;
        }

        #[test]
        fn r13_both_methods_answer_the_same(net in net_spec(1..=2000)) {
            let inp = net.input();
            assert_same_queries(
                &routing_with(inp.clone(), Method::AllSources),
                &routing_with(inp, Method::DestinationsOnly),
            )?;
        }

        #[test]
        fn r09_walk_arrives_while_network_grows(
            net in net_spec(1000..=1000),
            pick in (any::<u32>(), any::<u32>()),
            changes in vec((0u32..40, (0u8..6, any::<u32>(), any::<u32>(), any::<u32>())), 0..=8),
        ) {
            let mut inp = net.input();
            let mut r = Routing::from_input(inp.clone());
            let from = s(pick.0 % net.station_count);
            let dests: Vec<StationId> = r.reachable_dests(from).collect();
            prop_assume!(!dests.is_empty());
            let dest = dests[pick.1 as usize % dests.len()];
            let mut changes = changes;
            changes.sort_by_key(|c| c.0);
            let mut w = Walker::new(from, dest);
            let mut next_change = 0;
            let mut since = 0;
            loop {
                while next_change < changes.len() && changes[next_change].0 <= w.moves {
                    grow(&mut inp, changes[next_change].1);
                    let grown = Routing::from_input(inp.clone());
                    for t in r.reachable_dests(from) {
                        prop_assert!(grown.station_cost(from, t).is_some(), "G-8: {from} -> {t}");
                    }
                    r = grown;
                    next_change += 1;
                    since = 0;
                }
                if w.step(&r, &inp)? {
                    break;
                }
                since += 1;
                prop_assert!(since <= r.snap.nodes.node_count(), "Q-4: {from} -> {dest} が着かない {w:?}");
            }
        }

        #[test]
        fn r12_reachable_dests_match_station_cost(net in net_spec(1000..=1000)) {
            let r = net.routing();
            for from in (0..net.station_count).map(s) {
                let got: Vec<StationId> = r.reachable_dests(from).collect();
                let want: Vec<StationId> = (0..net.station_count)
                    .map(s)
                    .filter(|&t| t != from && r.station_cost(from, t).is_some())
                    .collect();
                prop_assert!(got.windows(2).all(|p| p[0] < p[1]));
                prop_assert_eq!(&got, &want);
                let served = net.input().lines.iter().any(|ln| ln.stops.contains(&from));
                if !served {
                    prop_assert!(got.is_empty());
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 16, ..config() })]

        #[test]
        fn over_192_nodes_d03_r08_r09_r13(net in big_net_spec()) {
            let inp = net.input();
            let (g, m) = build_graph(&inp);
            prop_assert!((193..=256).contains(&m.node_count()));
            let fw = FloydWarshall.compute(&g);
            let dj = RepeatedDijkstra.compute(&g);
            if let Some(diff) = first_difference(&fw, &dj) {
                return Err(TestCaseError::fail(diff));
            }
            let r = net.routing();
            prop_assert!(uses_destinations_only(&r));
            assert_same_queries(&routing_with(inp, Method::AllSources), &r)?;
            check_q1_q2(&net, &r)?;
            check_walks(&net, &r)?;
        }
    }
}
