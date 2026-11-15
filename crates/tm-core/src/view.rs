//! 読み取り専用のビュー(10 §7.2)。内部の構造を表示に要る値に変えて返す。

use crate::balance::BalanceTable;
use crate::command::{Command, CommandError, CommandPreview, LineEnd};
use crate::game::Game;
use crate::geom::{Bend, Dir8, GridPos, MapPoint, TrackMask, TrackSegment};
use crate::ids::{CompanyLevel, Level, LineId, StationId, TrainId};
use crate::param::{Feature, FeatureSet, LineLevels, ParamKey, RebirthUpgrade, StationLevels};
use crate::recommend::Recommendation;
use crate::sim;
use crate::state::{Direction, GameState, Line, Station, Train, TrainPhase, lay_track};
use crate::time::{POS_UNITS_PER_TILE, Tick, Ticks};
use crate::units::{
    Cars, MilliPeople, Money, People, Permille, SpeedMilli, spawn_per_sec, train_capacity,
};

/// `Game` の読み取り専用のビュー。UI はこれだけを受け取る。
#[derive(Clone, Copy)]
pub struct GameView<'a> {
    game: &'a Game,
}

impl<'a> GameView<'a> {
    pub(crate) fn new(game: &'a Game) -> Self {
        GameView { game }
    }

    fn st(&self) -> &'a GameState {
        &self.game.state
    }

    fn bal(&self) -> &'a BalanceTable {
        &self.game.config.balance
    }

    /// 最後に処理した tick。
    pub fn tick(&self) -> Tick {
        self.st().tick
    }

    /// バランス表。
    pub fn balance(&self) -> &'a BalanceTable {
        self.bal()
    }

    /// 所持金。
    pub fn money(&self) -> Money {
        self.st().money
    }

    /// 累計収入。
    pub fn total_earned(&self) -> Money {
        self.st().progress.total_earned
    }

    /// 直近 `INCOME_WINDOW_SECS` 秒の平均の収入(10 TD-15)。
    pub fn income_per_sec(&self) -> Money {
        sim::income_per_sec(self.st())
    }

    /// 会社レベルとゲージ。
    pub fn company(&self) -> CompanyView {
        let (st, bal) = (self.st(), self.bal());
        CompanyView {
            level: st.company.level,
            max_level: CompanyLevel(bal.spec().company.max_level),
            cap: bal.cap(st.company.level),
            gauge: sim::gauge_view(st, bal),
            station_count: count_u32(st.stations.len()),
        }
    }

    /// (使っている路線の数, 枠の数。転生の強化の分を含む)。
    pub fn line_slots(&self) -> (u32, u32) {
        let st = self.st();
        (
            count_u32(st.lines.len()),
            self.bal().line_slots(st.company.line_slots) + sim::rebirth::line_bonus(st, self.bal()),
        )
    }

    /// R2 の現在のレベル(10 TD-39)。
    pub fn line_slot_level(&self) -> Level {
        self.st().company.line_slots
    }

    /// 解放済みか。
    pub fn is_unlocked(&self, f: Feature) -> bool {
        self.st().progress.unlocked.contains(f)
    }

    /// 解放済みの機能。
    pub fn unlocked(&self) -> FeatureSet {
        self.st().progress.unlocked
    }

    /// NEW の印(解放済みで一度も強化していない。10 TD-33)。
    pub fn is_new(&self, p: ParamKey) -> bool {
        self.is_unlocked(Feature::Param(p)) && !self.st().progress.upgraded_once.contains(p)
    }

    /// いまの会社レベルでの上限。
    pub fn level_limit(&self, p: ParamKey) -> Level {
        self.bal().limit(p, self.st().company.level)
    }

    /// 所持金が届いたことのある最大の単位。
    pub fn max_money_unit(&self) -> u8 {
        self.st().progress.max_money_unit
    }

    /// ゴールに届いたか。
    pub fn goal_reached(&self) -> bool {
        self.st().progress.goal_reached
    }

    /// 転生の状態 (T1)。
    pub fn rebirth(&self) -> RebirthView {
        let (st, bal) = (self.st(), self.bal());
        let upgrades = RebirthUpgrade::ALL.map(|k| {
            let def = bal.rebirth_upgrade(k);
            let level = st.rebirth.level(k);
            let cost = (level < def.max).then(|| bal.rebirth_cost(k, level));
            RebirthUpgradeView {
                upgrade: k,
                level,
                max: def.max,
                cost,
                affordable: cost.is_some_and(|c| st.rebirth.points >= c),
            }
        });
        RebirthView {
            count: st.rebirth.count,
            points: st.rebirth.points,
            can_rebirth: st.progress.goal_reached,
            next_points: bal.rebirth_points(st.company.level),
            upgrades,
        }
    }

    /// マップのマス数 (幅, 高さ)。
    pub fn map_size(&self) -> (u8, u8) {
        let t = &self.st().tracks;
        (t.width, t.height)
    }

    /// 静的なマップ層の版(10 TD-28)。
    pub fn map_revision(&self) -> u64 {
        self.st().map_revision
    }

    /// 駅(`StationId` 昇順)。
    pub fn stations(&self) -> impl ExactSizeIterator<Item = StationView<'a>> + 'a {
        let v = *self;
        self.st().stations.iter().map(move |s| v.station_view(s))
    }

    /// 駅 1 つ。
    pub fn station(&self, id: StationId) -> Option<StationView<'a>> {
        self.st()
            .stations
            .get(id.index())
            .map(|s| self.station_view(s))
    }

    /// そのマスにある駅。
    pub fn station_at(&self, pos: GridPos) -> Option<StationId> {
        self.st()
            .stations
            .iter()
            .find(|s| s.pos == pos)
            .map(|s| s.id)
    }

    /// 路線(`LineId` 昇順)。
    pub fn lines(&self) -> impl ExactSizeIterator<Item = LineView<'a>> + 'a {
        let v = *self;
        self.st().lines.iter().map(move |l| v.line_view(l))
    }

    /// 路線 1 つ。
    pub fn line(&self, id: LineId) -> Option<LineView<'a>> {
        self.st().lines.get(id.index()).map(|l| self.line_view(l))
    }

    /// 列車(`TrainId` 昇順)。
    pub fn trains(&self) -> impl ExactSizeIterator<Item = TrainView> + 'a {
        let v = *self;
        self.st().trains.iter().map(move |t| v.train_view(t))
    }

    /// 列車 1 つ。
    pub fn train(&self, id: TrainId) -> Option<TrainView> {
        self.st().trains.get(id.index()).map(|t| self.train_view(t))
    }

    /// 線路のマス。
    pub fn track_grid(&self) -> TrackGridView<'a> {
        let t = &self.st().tracks;
        TrackGridView {
            width: t.width,
            height: t.height,
            masks: &t.masks,
        }
    }

    /// 状態を変えずに検査する(`Game::validate` と同じ)。
    pub fn validate(&self, cmd: &Command) -> Result<CommandPreview, CommandError> {
        self.game.validate(cmd)
    }

    /// 路線を作る途中で、`selected` の次に `next` を足せるか(存在・重複・路線の空き枠。お金は見ない)。
    pub fn validate_line_draft(
        &self,
        selected: &[StationId],
        next: StationId,
    ) -> Result<(), CommandError> {
        sim::check_line_draft(self.st(), self.bal(), selected, next)
    }

    /// 線路の完成図 [01 §6.2]。`out` は clear してから書く。未出現の駅なら空。
    pub fn preview_track(
        &self,
        from: StationId,
        to: StationId,
        bend: Bend,
        out: &mut Vec<GridPos>,
    ) {
        out.clear();
        let st = self.st();
        if let (Some(a), Some(b)) = (st.stations.get(from.index()), st.stations.get(to.index())) {
            lay_track(a.pos, b.pos, bend, out);
        }
    }

    /// 既定の曲がり方(10 TD-22): 両端を除いた線路のあるマスが多い方。同じなら `StraightFirst`。
    pub fn default_bend(&self, from: StationId, to: StationId) -> Bend {
        let mut tiles = Vec::new();
        let mut used = |bend| {
            self.preview_track(from, to, bend, &mut tiles);
            let inner = tiles.get(1..tiles.len().saturating_sub(1)).unwrap_or(&[]);
            let tracks = &self.st().tracks;
            inner.iter().filter(|&&p| !tracks.get(p).is_empty()).count()
        };
        let straight = used(Bend::StraightFirst);
        if used(Bend::DiagonalFirst) > straight {
            Bend::DiagonalFirst
        } else {
            Bend::StraightFirst
        }
    }

    /// 延伸できる端 `[(Head, 先頭の駅), (Tail, 末尾の駅)]`。存在しない路線なら panic する。
    pub fn line_ends(&self, line: LineId) -> [(LineEnd, StationId); 2] {
        let l = &self.st().lines[line.index()];
        [
            (LineEnd::Head, l.stations[0]),
            (LineEnd::Tail, *l.stations.last().expect("路線に駅がある")),
        ]
    }

    /// おすすめ [01 D-25]。対象が 1 つもなければ `None`。
    pub fn recommendation(&self) -> Option<Recommendation> {
        sim::recommendation(self.st(), self.bal())
    }

    fn station_view(&self, s: &'a Station) -> StationView<'a> {
        let (st, bal) = (self.st(), self.bal());
        let spawns = s.is_connected() && st.routing.reachable_dests(s.id).next().is_some();
        StationView {
            id: s.id,
            pos: s.pos,
            levels: s.levels,
            connected: s.is_connected(),
            waiting: s.waiting_total,
            capacity: bal.station_capacity(s.levels.capacity),
            spawn_per_sec: if spawns {
                spawn_per_sec(
                    bal.spawn_rate(s.levels.spawn),
                    sim::max_stopping_attractiveness(st, bal, s.id),
                )
            } else {
                MilliPeople::ZERO
            },
            fare: bal
                .fare(s.levels.fare)
                .mul_small(sim::rebirth::fare_mul(st, bal)),
            platform: bal.platform(s.levels.platform),
            lines: &s.serving_lines,
            born_level: bal.station_born_level(s.id),
        }
    }

    fn line_view(&self, l: &'a Line) -> LineView<'a> {
        let (st, bal) = (self.st(), self.bal());
        let lv = l.levels;
        LineView {
            id: l.id,
            stations: &l.stations,
            stops: &l.stops,
            segments: &l.segments,
            levels: lv,
            trains: &l.trains,
            speed: bal.speed(lv.speed),
            cars: bal.cars(lv.cars),
            train_capacity: train_capacity(
                bal.car_capacity(lv.car_capacity),
                bal.cars(lv.cars),
                bal.crowding(lv.crowding),
            ),
            boarding_rate: bal.boarding_rate(lv.boarding),
            attractiveness: bal.attractiveness(lv.attractiveness),
            min_platform: l
                .stations
                .iter()
                .map(|s| bal.platform(st.stations[s.index()].levels.platform))
                .min()
                .expect("路線に駅がある"),
            born_level: l.born_level,
        }
    }

    fn train_view(&self, t: &Train) -> TrainView {
        let line = &self.st().lines[t.line.index()];
        let lv = line.levels;
        let bal = self.bal();
        let (point, heading) = train_point(line, t);
        TrainView {
            id: t.id,
            line: t.line,
            point,
            heading,
            riders: t.riders,
            capacity: train_capacity(
                bal.car_capacity(lv.car_capacity),
                bal.cars(lv.cars),
                bal.crowding(lv.crowding),
            ),
            phase: match t.phase {
                TrainPhase::Running => TrainPhaseView::Running {
                    next_stop: sim::next_stop(line, t),
                },
                TrainPhase::Dwelling { at, remaining } => TrainPhaseView::Dwelling {
                    at: line.stations[usize::from(at)],
                    remaining,
                },
            },
        }
    }
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).expect("個数は u32")
}

/// 路線のマスの列(全区間をつないだもの)の `i` 番目(`0..=offsets の最後`)。
fn tile_at(line: &Line, i: u32) -> GridPos {
    let j = (0..line.segments.len())
        .rev()
        .find(|&j| line.offsets[j] <= i)
        .expect("offsets[0] = 0");
    line.segments[j].tiles[(i - line.offsets[j]) as usize]
}

fn lerp(a: GridPos, b: GridPos, r: u64) -> MapPoint {
    let u = POS_UNITS_PER_TILE as i64;
    let r = r as i64;
    MapPoint {
        x: i64::from(a.x) * u + i64::from(b.x - a.x) * r,
        y: i64::from(a.y) * u + i64::from(b.y - a.y) * r,
    }
}

/// 出発した停車駅から `pos` だけマスの列をたどった位置と、進む向きの線路の向き(10 §7.2)。
fn train_point(line: &Line, t: &Train) -> (MapPoint, Dir8) {
    let u = POS_UNITS_PER_TILE;
    let dep = sim::departure_index(line, t.section, t.dir);
    let base = u64::from(line.offsets[dep]) * u;
    let total = *line.offsets.last().expect("路線に駅がある");
    let step = |a: u32, b: u32| {
        let (p, q) = (tile_at(line, a), tile_at(line, b));
        Dir8::from_delta(q.x - p.x, q.y - p.y)
    };
    match t.dir {
        Direction::Forward => {
            let x = base + t.pos.0;
            let i = u32::try_from(x / u).expect("マスの番号は u32");
            let r = x % u;
            if i < total {
                (
                    lerp(tile_at(line, i), tile_at(line, i + 1), r),
                    step(i, i + 1),
                )
            } else {
                (lerp(tile_at(line, i), tile_at(line, i), 0), step(i - 1, i))
            }
        }
        Direction::Backward => {
            let x = base - t.pos.0;
            let i = u32::try_from(x.div_ceil(u)).expect("マスの番号は u32");
            let r = u64::from(i) * u - x;
            if i > 0 {
                (
                    lerp(tile_at(line, i), tile_at(line, i - 1), r),
                    step(i, i - 1),
                )
            } else {
                (lerp(tile_at(line, 0), tile_at(line, 0), 0), step(1, 0))
            }
        }
    }
}

/// 会社の表示用の値。
#[derive(Clone, Copy, Debug)]
pub struct CompanyView {
    /// 会社レベル。
    pub level: CompanyLevel,
    /// 会社レベルの最大。
    pub max_level: CompanyLevel,
    /// cap(C)。
    pub cap: u8,
    /// 会社ゲージ。
    pub gauge: GaugeView,
    /// 出現済みの駅の数。
    pub station_count: u32,
}

/// 会社ゲージ [01 §5.1]。
#[derive(Clone, Copy, Debug)]
pub struct GaugeView {
    /// レベルの合計。
    pub sum_level: u64,
    /// 上限の合計。
    pub sum_limit: u64,
    /// 0..=100(70% を満タンとした表示)。
    pub display_percent: u8,
}

/// 駅の表示用の値。
#[derive(Clone, Copy)]
pub struct StationView<'a> {
    /// 駅。
    pub id: StationId,
    /// 位置。
    pub pos: GridPos,
    /// レベル。
    pub levels: StationLevels,
    /// 路線に入っているか。
    pub connected: bool,
    /// 待っている人数の合計。
    pub waiting: People,
    /// S3 の値。
    pub capacity: People,
    /// 魅力度を掛けた実際の発生量。
    pub spawn_per_sec: MilliPeople,
    /// S2 の値。
    pub fare: Money,
    /// S4 の値。
    pub platform: Cars,
    /// この駅を含む路線。
    pub lines: &'a [LineId],
    /// 出た会社レベル(強化の値段の下限。`BalanceTable::floor_level`)。
    pub born_level: CompanyLevel,
}

/// 路線の表示用の値。
#[derive(Clone, Copy)]
pub struct LineView<'a> {
    /// 路線。
    pub id: LineId,
    /// Head から Tail の順の駅。
    pub stations: &'a [StationId],
    /// 停車するか(`stations` と同じ長さ)。
    pub stops: &'a [bool],
    /// 駅の間の線路。
    pub segments: &'a [TrackSegment],
    /// レベル。
    pub levels: LineLevels,
    /// 列車(昇順)。
    pub trains: &'a [TrainId],
    /// T1 の値。
    pub speed: SpeedMilli,
    /// T5 の値。
    pub cars: Cars,
    /// 列車の定員。
    pub train_capacity: People,
    /// T3 の値。
    pub boarding_rate: People,
    /// R3 の値。
    pub attractiveness: Permille,
    /// 全駅のホーム長の最小。
    pub min_platform: Cars,
    /// 作った会社レベル(強化の値段の下限。`BalanceTable::floor_level`)。
    pub born_level: CompanyLevel,
}

/// 列車の表示用の値。
#[derive(Clone, Copy, Debug)]
pub struct TrainView {
    /// 列車。
    pub id: TrainId,
    /// 路線。
    pub line: LineId,
    /// いまの位置。
    pub point: MapPoint,
    /// いま走っている線路の向き。
    pub heading: Dir8,
    /// 乗っている人数。
    pub riders: People,
    /// 定員。
    pub capacity: People,
    /// 走行中か停車中か。
    pub phase: TrainPhaseView,
}

/// 列車の状態。
#[derive(Clone, Copy, Debug)]
pub enum TrainPhaseView {
    /// 走っている。
    Running {
        /// 次の停車駅。
        next_stop: StationId,
    },
    /// 停車している。
    Dwelling {
        /// 停車している駅。
        at: StationId,
        /// 残りの tick。
        remaining: Ticks,
    },
}

/// 線路のマス。添字 = y × width + x。
#[derive(Clone, Copy)]
pub struct TrackGridView<'a> {
    /// 横のマス数。
    pub width: u8,
    /// 縦のマス数。
    pub height: u8,
    /// 各マスの線路のつながり。
    pub masks: &'a [TrackMask],
}

/// 転生の状態の表示用の値 (T1)。
#[derive(Clone, Copy, Debug)]
pub struct RebirthView {
    /// 転生した回数。
    pub count: u32,
    /// 持っている転生ポイント。
    pub points: u64,
    /// いま転生できるか(累計収入がゴールに届いたか)。
    pub can_rebirth: bool,
    /// いま転生したらもらえるポイント。
    pub next_points: u64,
    /// 恒久の強化(`RebirthUpgrade` の順)。
    pub upgrades: [RebirthUpgradeView; 3],
}

/// 恒久の強化 1 つの表示用の値。
#[derive(Clone, Copy, Debug)]
pub struct RebirthUpgradeView {
    /// どの強化か。
    pub upgrade: RebirthUpgrade,
    /// いまのレベル。
    pub level: u8,
    /// レベルの最大。
    pub max: u8,
    /// 次のレベルのコスト(ポイント。最大レベルなら `None`)。
    pub cost: Option<u64>,
    /// ポイントが足りて買えるか。
    pub affordable: bool,
}
