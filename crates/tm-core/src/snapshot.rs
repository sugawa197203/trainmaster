//! セーブ用のゲーム状態の写し `GameSnapshot` と `Game::snapshot`・`Game::restore`(10 §9.4、11 §9.3)。I/O はしない。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tm_numeric::BigNum;

use crate::RULES_VERSION;
use crate::balance::BalanceTable;
use crate::game::{Game, GameConfig};
use crate::geom::{Bend, GridPos};
use crate::ids::{CompanyLevel, Level, LineId, StationId, TrainId};
use crate::invariants::InvariantViolation;
use crate::param::{FeatureSet, LineLevels, ParamKey, ParamSet, StationLevels};
use crate::rng::{RngStreams, STREAM_PASSENGER_DEST, StreamRng};
use crate::routing::Routing;
use crate::sim;
use crate::state::{
    CompanyState, Direction, GameState, INCOME_WINDOW_SECS, Line, PaxMatrix, Progress,
    RebirthState, Station, Stats, Train, TrainPhase,
};
use crate::time::{Tick, Ticks};
use crate::units::{MilliPeople, Money, PosUnits};

/// `GameSnapshot` の形の版。フィールドや意味を変えたら上げる。2 = `SnapLine::born_level`(M8)。
/// 3 = `rebirth`(転生。T1)。
pub const SNAPSHOT_FORMAT_VERSION: u32 = 3;

// restore の後の step で足し込みがあふれないための上限。どれも 1 週間の遊びより何桁も大きい。
const MAX_TICK: u64 = 1 << 40;
const MAX_COUNT: u64 = 1 << 50;
const MAX_DELIVERED_HI: u64 = 1 << 36;
const MAX_PURCHASES: u32 = 1 << 24;

/// セーブ用の写し(10 §9.4)。`state_hash` の対象とシード・乱数の位置だけを持ち、導出データは `restore` で作り直す。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameSnapshot {
    /// `SNAPSHOT_FORMAT_VERSION`。
    pub format: u32,
    /// `RULES_VERSION`。
    pub rules_version: u32,
    /// `BalanceTable::config_hash()`。
    pub config_hash: u64,
    /// `GameConfig::seed`。
    pub seed: u64,
    /// 取ったときの `Game::state_hash()`。
    pub state_hash: u64,
    /// 最後に処理した tick。
    pub tick: u64,
    /// 所持金の `to_bits()`。
    pub money_bits: u64,
    /// 累計収入の `to_bits()`。
    pub total_earned_bits: u64,
    /// 会社。
    pub company: SnapCompany,
    /// 全駅の位置(出現順)。
    pub station_plan: Vec<(i32, i32)>,
    /// 出現済みの駅(添字 = `StationId`)。
    pub stations: Vec<SnapStation>,
    /// 路線(添字 = `LineId`)。
    pub lines: Vec<SnapLine>,
    /// 列車(添字 = `TrainId`)。
    pub trains: Vec<SnapTrain>,
    /// 駅で待つ人数(行 = 駅、列 = 目的駅)。
    pub waiting: SnapMatrix,
    /// 列車に乗っている人数(行 = 列車、列 = 目的駅)。
    pub onboard: SnapMatrix,
    /// 解放・単位・ゴール。
    pub progress: SnapProgress,
    /// 転生の状態 [T1]。形式 2 以前の写しにはないので既定値で読む(`RulesVersion` で断る)。
    #[serde(default)]
    pub rebirth: SnapRebirth,
    /// 統計。
    pub stats: SnapStats,
    /// 乗客の目的駅の乱数ストリームから取り出した語の数(10 TD-45)。
    pub passenger_dest_words: u64,
    /// 静的なマップ層の版。
    pub map_revision: u64,
}

/// 会社の写し。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapCompany {
    /// 会社レベル。
    pub level: u8,
    /// R2 のレベル。
    pub line_slots: u8,
    /// これまでに線路を敷いたマスの数。
    pub track_tiles: u32,
}

/// 駅の写し。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapStation {
    /// 出現した tick。
    pub spawned_at: u64,
    /// S1〜S4 のレベル。
    pub levels: [u8; 4],
    /// 発生の累積器(1/1000 人)。
    pub spawn_acc: u64,
}

/// 路線の写し。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapLine {
    /// Head から Tail の順の駅。
    pub stations: Vec<u32>,
    /// `stations[i]` → `stations[i+1]` の向きの曲がり方(10 TD-23)。
    pub bends: Vec<Bend>,
    /// T1〜T5・R1・R3 のレベル。
    pub levels: [u8; 7],
    /// 列車(昇順)。
    pub trains: Vec<u32>,
    /// 作った tick。
    pub created_at: u64,
    /// 作ったときの会社レベル。
    pub born_level: u8,
}

/// 列車の写し。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapTrain {
    /// 路線。
    pub line: u32,
    /// 路線の区間の添字。
    pub section: u16,
    /// 0 = Head → Tail、1 = 逆。
    pub dir: u8,
    /// 区間の始点からの距離。
    pub pos: u64,
    /// 走行か停車か。
    pub phase: SnapPhase,
}

/// 列車の状態の写し。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapPhase {
    /// 走行中。
    Running,
    /// 停車中。
    Dwelling {
        /// 停車している駅の路線の中の添字。
        at: u16,
        /// この tick を含めない残りの tick。
        remaining: u32,
    },
}

/// 人数の表の写し。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapMatrix {
    /// 行数。
    pub rows: u32,
    /// 列数(`max_stations`)。
    pub cols: u32,
    /// 行優先の全要素。
    pub data: Vec<u64>,
}

/// 進行の写し。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapProgress {
    /// 解放済みの機能(`FeatureSet` のビット)。
    pub unlocked: u32,
    /// 一度でも強化したパラメータ(`ParamSet` のビット)。
    pub upgraded_once: u16,
    /// 所持金が届いた最大の単位。
    pub max_money_unit: u8,
    /// ゴールに届いたか。
    pub goal_reached: bool,
}

/// 転生の状態の写し [T1]。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapRebirth {
    /// 転生した回数。
    pub count: u32,
    /// 持っている転生ポイント。
    pub points: u64,
    /// 運賃の倍率の強化のレベル。
    pub fare_boost: u8,
    /// 電車の数の強化のレベル。
    pub train_boost: u8,
    /// 路線の数の強化のレベル。
    pub line_boost: u8,
}

/// 統計の写し。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapStats {
    /// いまの 1 秒の収入の `to_bits()`。
    pub income_this_sec_bits: u64,
    /// 直近の各 1 秒の収入の `to_bits()`(`INCOME_WINDOW_SECS` 個)。
    pub income_window_bits: Vec<u64>,
    /// 次に書く位置。
    pub window_head: u8,
    /// 到着の回数。
    pub arrivals: u64,
    /// 届けた人数の下位 64 ビット(10 TD-46)。
    pub delivered_lo: u64,
    /// 届けた人数の上位 64 ビット。
    pub delivered_hi: u64,
    /// 成功した強化の数。
    pub purchases: u32,
}

/// `Game::restore` の失敗。
#[derive(Clone, Debug, PartialEq)]
pub enum SnapshotError {
    /// `SNAPSHOT_FORMAT_VERSION` が違う。
    Format {
        /// 写しの値。
        found: u32,
    },
    /// `RULES_VERSION` が違う。
    RulesVersion {
        /// 写しの値。
        found: u32,
    },
    /// config hash が違う。
    ConfigMismatch {
        /// 写しの値。
        expected: u64,
        /// いまの表の値。
        actual: u64,
    },
    /// 長さ・ID・範囲・お金のビット列が合わない。
    Malformed {
        /// 何が合わないか。
        detail: String,
    },
    /// 作り直した状態が不変条件を破る。
    Invariant(InvariantViolation),
    /// 作り直した状態の `state_hash` が写しの値と違う。
    HashMismatch {
        /// 写しの値。
        expected: u64,
        /// 作り直した状態の値。
        actual: u64,
    },
}

impl Game {
    /// セーブ用の写し(10 §9.4)。`pending()` が空でなければ panic する(`step` の後に呼ぶ)。
    pub fn snapshot(&self) -> GameSnapshot {
        assert!(
            self.pending().is_empty(),
            "snapshot はコマンドのキューが空のときだけ取れる"
        );
        let st = &self.state;
        GameSnapshot {
            format: SNAPSHOT_FORMAT_VERSION,
            rules_version: RULES_VERSION,
            config_hash: self.config.balance.config_hash(),
            seed: self.config.seed,
            state_hash: self.state_hash(),
            tick: st.tick.0,
            money_bits: st.money.to_bits(),
            total_earned_bits: st.progress.total_earned.to_bits(),
            company: SnapCompany {
                level: st.company.level.0,
                line_slots: st.company.line_slots.0,
                track_tiles: st.company.track_tiles,
            },
            station_plan: st.station_plan.iter().map(|p| (p.x, p.y)).collect(),
            stations: st.stations.iter().map(snap_station).collect(),
            lines: st.lines.iter().map(snap_line).collect(),
            trains: st.trains.iter().map(snap_train).collect(),
            waiting: snap_matrix(&st.waiting),
            onboard: snap_matrix(&st.onboard),
            progress: SnapProgress {
                unlocked: st.progress.unlocked.0,
                upgraded_once: st.progress.upgraded_once.0,
                max_money_unit: st.progress.max_money_unit,
                goal_reached: st.progress.goal_reached,
            },
            rebirth: SnapRebirth {
                count: st.rebirth.count,
                points: st.rebirth.points,
                fare_boost: st.rebirth.fare_boost,
                train_boost: st.rebirth.train_boost,
                line_boost: st.rebirth.line_boost,
            },
            stats: snap_stats(&st.stats),
            passenger_dest_words: st.rng.passenger_dest.words(),
            map_revision: st.map_revision,
        }
    }

    /// 写しから作り直す(10 §9.4、11 §9.3)。シードは写しのもの。導出データを作り直し、不変条件と
    /// `state_hash` を照合する。どんな写しでも panic せず、合わなければ `Err`。
    pub fn restore(balance: Arc<BalanceTable>, snap: &GameSnapshot) -> Result<Game, SnapshotError> {
        if snap.format != SNAPSHOT_FORMAT_VERSION {
            return Err(SnapshotError::Format { found: snap.format });
        }
        if snap.rules_version != RULES_VERSION {
            return Err(SnapshotError::RulesVersion {
                found: snap.rules_version,
            });
        }
        let actual = balance.config_hash();
        if snap.config_hash != actual {
            return Err(SnapshotError::ConfigMismatch {
                expected: snap.config_hash,
                actual,
            });
        }
        let malformed = |detail| SnapshotError::Malformed { detail };
        check_shape(snap, &balance).map_err(malformed)?;
        let state = build_state(snap, &balance).map_err(malformed)?;
        let game = Game::from_state(
            GameConfig {
                seed: snap.seed,
                balance,
            },
            state,
        );
        game.check_invariants().map_err(SnapshotError::Invariant)?;
        check_onboard_routes(&game.state).map_err(malformed)?;
        let actual = game.state_hash();
        if actual != snap.state_hash {
            return Err(SnapshotError::HashMismatch {
                expected: snap.state_hash,
                actual,
            });
        }
        Ok(game)
    }
}

fn snap_station(s: &Station) -> SnapStation {
    let l = s.levels;
    SnapStation {
        spawned_at: s.spawned_at.0,
        levels: [l.spawn.0, l.fare.0, l.capacity.0, l.platform.0],
        spawn_acc: s.spawn_acc.0,
    }
}

fn snap_line(l: &Line) -> SnapLine {
    let v = l.levels;
    SnapLine {
        stations: l.stations.iter().map(|s| s.0).collect(),
        bends: l.segments.iter().map(|seg| seg.bend).collect(),
        levels: [
            v.speed.0,
            v.crowding.0,
            v.boarding.0,
            v.car_capacity.0,
            v.cars.0,
            v.max_trains.0,
            v.attractiveness.0,
        ],
        trains: l.trains.iter().map(|t| t.0).collect(),
        created_at: l.created_at.0,
        born_level: l.born_level.0,
    }
}

fn snap_train(t: &Train) -> SnapTrain {
    SnapTrain {
        line: t.line.0,
        section: t.section,
        dir: match t.dir {
            Direction::Forward => 0,
            Direction::Backward => 1,
        },
        pos: t.pos.0,
        phase: match t.phase {
            TrainPhase::Running => SnapPhase::Running,
            TrainPhase::Dwelling { at, remaining } => SnapPhase::Dwelling {
                at,
                remaining: remaining.0,
            },
        },
    }
}

fn snap_matrix(m: &PaxMatrix) -> SnapMatrix {
    SnapMatrix {
        rows: m.rows(),
        cols: m.cols(),
        data: m.as_slice().to_vec(),
    }
}

fn snap_stats(s: &Stats) -> SnapStats {
    SnapStats {
        income_this_sec_bits: s.income_this_sec.to_bits(),
        income_window_bits: s.income_window.iter().map(|m| m.to_bits()).collect(),
        window_head: s.window_head,
        arrivals: s.arrivals,
        delivered_lo: s.delivered as u64,
        delivered_hi: (s.delivered >> 64) as u64,
        purchases: s.purchases,
    }
}

fn ensure(ok: bool, detail: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(detail()) }
}

fn station_levels(a: [u8; 4]) -> StationLevels {
    StationLevels {
        spawn: Level(a[0]),
        fare: Level(a[1]),
        capacity: Level(a[2]),
        platform: Level(a[3]),
    }
}

fn line_levels(a: [u8; 7]) -> LineLevels {
    LineLevels {
        speed: Level(a[0]),
        crowding: Level(a[1]),
        boarding: Level(a[2]),
        car_capacity: Level(a[3]),
        cars: Level(a[4]),
        max_trains: Level(a[5]),
        attractiveness: Level(a[6]),
    }
}

/// 値の表は `limit(p, max_level)` までしかないので、引く前に上限以下か見る。
fn check_levels(
    bal: &BalanceTable,
    c: CompanyLevel,
    owner: &dyn Fn() -> String,
    get: impl Fn(ParamKey) -> Option<Level>,
) -> Result<(), String> {
    for p in ParamKey::ALL {
        if let Some(l) = get(p) {
            let limit = bal.limit(p, c);
            ensure(l <= limit, || {
                format!("{} の {p:?} = {l} > 上限 {limit}", owner())
            })?;
        }
    }
    Ok(())
}

fn check_matrix(name: &str, m: &SnapMatrix, rows: usize, cols: u32) -> Result<(), String> {
    let len = u64::from(m.rows) * u64::from(m.cols);
    ensure(
        m.rows as usize == rows && m.cols == cols && m.data.len() as u64 == len,
        || {
            format!(
                "{name}: {} × {}、要素 {}(正しくは {rows} × {cols})",
                m.rows,
                m.cols,
                m.data.len()
            )
        },
    )?;
    for (r, row) in m.data.chunks(cols as usize).enumerate() {
        let sum = row.iter().try_fold(0u64, |acc, &v| acc.checked_add(v));
        ensure(sum.is_some(), || {
            format!("{name}[{r}] の合計が u64 を超える")
        })?;
    }
    Ok(())
}

fn check_counters(s: &GameSnapshot) -> Result<(), String> {
    let st = &s.stats;
    for (name, v, max) in [
        ("tick", s.tick, MAX_TICK),
        ("passenger_dest_words", s.passenger_dest_words, MAX_COUNT),
        ("arrivals", st.arrivals, MAX_COUNT),
        ("map_revision", s.map_revision, MAX_COUNT),
        ("delivered_hi", st.delivered_hi, MAX_DELIVERED_HI),
        (
            "purchases",
            u64::from(st.purchases),
            u64::from(MAX_PURCHASES),
        ),
    ] {
        ensure(v <= max, || format!("{name} = {v} > 上限 {max}"))?;
    }
    Ok(())
}

/// 列車の乗客の目的駅に、その路線の駅から行けること(INV-18 の列車の版。行けないと到着で止まる)。
fn check_onboard_routes(st: &GameState) -> Result<(), String> {
    let n = st.stations.len();
    for t in &st.trains {
        let line = &st.lines[t.line.index()];
        let at = line.stations[usize::from(line.sections[usize::from(t.section)].from)];
        for (d, &k) in st.onboard.row(t.id.0)[..n].iter().enumerate() {
            let dest = StationId::from_index(d);
            ensure(
                k == 0 || st.routing.station_cost(at, dest).is_some(),
                || format!("{} の {k} 人の目的駅 {dest} に {at} から行けない", t.id),
            )?;
        }
    }
    Ok(())
}

/// 11 §9.3 の 1: 作り直しで panic しないための形の検査。
fn check_shape(s: &GameSnapshot, bal: &BalanceTable) -> Result<(), String> {
    let spec = bal.spec();
    let rules = &spec.company;
    let c = CompanyLevel(s.company.level);
    ensure(c.0 <= rules.max_level, || format!("会社 {c}"))?;
    check_counters(s)?;
    let max_stations = bal.max_stations() as usize;
    let want =
        usize::from(rules.stations_base) + usize::from(c.0) * usize::from(rules.stations_per_level);
    ensure(s.stations.len() == want && want <= max_stations, || {
        format!("駅の数 {}、会社 {c} で {want}", s.stations.len())
    })?;
    ensure(s.station_plan.len() == max_stations, || {
        format!("station_plan の長さ {}", s.station_plan.len())
    })?;
    let m = &spec.map;
    let (w, h, g) = (i32::from(m.width), i32::from(m.height), i32::from(m.margin));
    for (i, &(x, y)) in s.station_plan.iter().enumerate() {
        ensure((g..w - g).contains(&x) && (g..h - g).contains(&y), || {
            format!("station_plan[{i}] = ({x}, {y}) が外周の内側にない")
        })?;
        ensure(!s.station_plan[..i].contains(&(x, y)), || {
            format!("station_plan[{i}] = ({x}, {y}) が重なる")
        })?;
    }
    let slots = Level(s.company.line_slots);
    check_levels(bal, c, &|| "会社".to_string(), |p| {
        (p == ParamKey::R2LineSlots).then_some(slots)
    })?;
    for k in crate::param::RebirthUpgrade::ALL {
        let max = bal.rebirth_upgrade(k).max;
        let level = match k {
            crate::param::RebirthUpgrade::FareBoost => s.rebirth.fare_boost,
            crate::param::RebirthUpgrade::TrainBoost => s.rebirth.train_boost,
            crate::param::RebirthUpgrade::LineBoost => s.rebirth.line_boost,
        };
        ensure(level <= max, || {
            format!("転生の {k:?} = {level} > 最大 {max}")
        })?;
    }
    ensure(s.rebirth.count <= MAX_COUNT as u32, || {
        format!("転生の回数 {}", s.rebirth.count)
    })?;
    for (i, st) in s.stations.iter().enumerate() {
        let lv = station_levels(st.levels);
        check_levels(bal, c, &|| format!("S{i}"), |p| lv.get(p))?;
    }
    ensure(s.lines.len() <= bal.max_lines() as usize, || {
        format!("路線の数 {}", s.lines.len())
    })?;
    let mut listed = vec![false; s.trains.len()];
    for (l, line) in s.lines.iter().enumerate() {
        let ss = &line.stations;
        ensure(ss.len() >= 2, || format!("L{l}: 駅 {}", ss.len()))?;
        for (j, &st) in ss.iter().enumerate() {
            ensure((st as usize) < s.stations.len(), || {
                format!("L{l}: 未出現の駅 {st}")
            })?;
            ensure(!ss[..j].contains(&st), || format!("L{l}: 駅 {st} が重複"))?;
        }
        ensure(line.bends.len() == ss.len() - 1, || {
            format!("L{l}: 曲がり方 {}、駅 {}", line.bends.len(), ss.len())
        })?;
        ensure(line.born_level <= c.0, || {
            format!("L{l}: 作ったときの会社 {}、いま {c}", line.born_level)
        })?;
        let lv = line_levels(line.levels);
        check_levels(bal, c, &|| format!("L{l}"), |p| lv.get(p))?;
        for &t in &line.trains {
            let ok = s
                .trains
                .get(t as usize)
                .is_some_and(|tr| tr.line as usize == l)
                && !std::mem::replace(&mut listed[t as usize], true);
            ensure(ok, || format!("L{l}: 列車 {t} が対応しない"))?;
        }
    }
    for (i, t) in s.trains.iter().enumerate() {
        ensure(listed[i], || format!("T{i}: 路線 {} が対応しない", t.line))?;
        ensure(t.dir <= 1, || format!("T{i}: 向き {}", t.dir))?;
    }
    check_matrix("waiting", &s.waiting, s.stations.len(), bal.max_stations())?;
    check_matrix("onboard", &s.onboard, s.trains.len(), bal.max_stations())?;
    let st = &s.stats;
    ensure(st.income_window_bits.len() == INCOME_WINDOW_SECS, || {
        format!("income_window_bits の長さ {}", st.income_window_bits.len())
    })?;
    ensure(usize::from(st.window_head) < INCOME_WINDOW_SECS, || {
        format!("window_head {}", st.window_head)
    })
}

fn money(bits: u64, name: &str) -> Result<Money, String> {
    BigNum::from_bits_checked(bits).ok_or_else(|| format!("{name} のビット列 {bits:#018x}"))
}

/// 11 §9.3 の 2: 状態を置き、導出データを作り直す。`check_shape` を通った写しだけを渡す。
fn build_state(s: &GameSnapshot, bal: &BalanceTable) -> Result<GameState, String> {
    let spec = bal.spec();
    let plan: Vec<GridPos> = s
        .station_plan
        .iter()
        .map(|&(x, y)| GridPos { x, y })
        .collect();
    let waiting = PaxMatrix::from_data(s.waiting.cols, s.waiting.data.clone());
    let onboard = PaxMatrix::from_data(s.onboard.cols, s.onboard.data.clone());
    let mut stations: Vec<Station> = s
        .stations
        .iter()
        .enumerate()
        .map(|(i, st)| {
            let id = StationId::from_index(i);
            Station {
                id,
                pos: plan[i],
                spawned_at: Tick(st.spawned_at),
                levels: station_levels(st.levels),
                spawn_acc: MilliPeople(st.spawn_acc),
                waiting_total: waiting.row_total(id.0),
                serving_lines: Vec::new(),
            }
        })
        .collect();
    let mut lines = Vec::with_capacity(s.lines.len());
    for (l, sl) in s.lines.iter().enumerate() {
        let id = LineId::from_index(l);
        let ids: Vec<StationId> = sl.stations.iter().map(|&x| StationId(x)).collect();
        let segments = ids
            .windows(2)
            .zip(&sl.bends)
            .map(|(w, &b)| sim::make_segment(&stations, w[0], w[1], b))
            .collect();
        let mut line = Line {
            id,
            stations: ids,
            segments,
            levels: line_levels(sl.levels),
            trains: sl.trains.iter().map(|&t| TrainId(t)).collect(),
            stops: Vec::new(),
            sections: Vec::new(),
            offsets: Vec::new(),
            created_at: Tick(sl.created_at),
            born_level: CompanyLevel(sl.born_level),
        };
        sim::rebuild_line_derived(&mut line, &stations, bal);
        for &st in &line.stations {
            stations[st.index()].serving_lines.push(id);
        }
        lines.push(line);
    }
    for stn in &stations {
        let limit = sim::waiting_limit(bal, stn);
        ensure(stn.waiting_total.0 <= limit, || {
            format!(
                "{} の待ち人数 {:?} > 上限 {limit}",
                stn.id, stn.waiting_total
            )
        })?;
    }
    let mut trains = Vec::with_capacity(s.trains.len());
    for (i, t) in s.trains.iter().enumerate() {
        let sections = lines[t.line as usize].sections.len();
        ensure(usize::from(t.section) < sections, || {
            format!("T{i}: 区間 {}、路線の区間 {sections}", t.section)
        })?;
        let id = TrainId::from_index(i);
        trains.push(Train {
            id,
            line: LineId(t.line),
            section: t.section,
            dir: if t.dir == 0 {
                Direction::Forward
            } else {
                Direction::Backward
            },
            pos: PosUnits(t.pos),
            phase: match t.phase {
                SnapPhase::Running => TrainPhase::Running,
                SnapPhase::Dwelling { at, remaining } => TrainPhase::Dwelling {
                    at,
                    remaining: Ticks(remaining),
                },
            },
            riders: onboard.row_total(id.0),
        });
    }
    let sv = &s.stats;
    let mut income_window = [Money::ZERO; INCOME_WINDOW_SECS];
    for (k, (w, &b)) in income_window
        .iter_mut()
        .zip(&sv.income_window_bits)
        .enumerate()
    {
        *w = money(b, &format!("income_window[{k}]"))?;
    }
    let delivered = (u128::from(sv.delivered_hi) << 64) | u128::from(sv.delivered_lo);
    #[cfg(feature = "test-support")]
    let ledger = {
        let sum = |m: &PaxMatrix| {
            m.as_slice()
                .iter()
                .fold(0u128, |acc, &v| acc.saturating_add(u128::from(v)))
        };
        crate::state::PaxLedger {
            spawned: delivered
                .saturating_add(sum(&waiting))
                .saturating_add(sum(&onboard)),
            capacity_loss: 0,
            delivered,
            waiting_cap_loss: 0,
        }
    };
    let mut st = GameState {
        tick: Tick(s.tick),
        money: money(s.money_bits, "money")?,
        company: CompanyState {
            level: CompanyLevel(s.company.level),
            line_slots: Level(s.company.line_slots),
            track_tiles: s.company.track_tiles,
        },
        tracks: sim::tracks_from_lines(spec.map.width, spec.map.height, &lines),
        station_plan: plan,
        stations,
        lines,
        trains,
        waiting,
        onboard,
        progress: Progress {
            unlocked: FeatureSet(s.progress.unlocked),
            upgraded_once: ParamSet(s.progress.upgraded_once),
            total_earned: money(s.total_earned_bits, "total_earned")?,
            max_money_unit: s.progress.max_money_unit,
            goal_reached: s.progress.goal_reached,
        },
        rebirth: RebirthState {
            count: s.rebirth.count,
            points: s.rebirth.points,
            fare_boost: s.rebirth.fare_boost,
            train_boost: s.rebirth.train_boost,
            line_boost: s.rebirth.line_boost,
        },
        stats: Stats {
            income_this_sec: money(sv.income_this_sec_bits, "income_this_sec")?,
            income_window,
            window_head: sv.window_head,
            arrivals: sv.arrivals,
            delivered,
            purchases: sv.purchases,
        },
        rng: RngStreams {
            passenger_dest: StreamRng::resume(
                s.seed,
                STREAM_PASSENGER_DEST,
                s.passenger_dest_words,
            ),
        },
        routing: Routing::default(),
        map_revision: s.map_revision,
        #[cfg(feature = "test-support")]
        ledger,
    };
    st.routing = Routing::new(&st, bal);
    Ok(st)
}

#[cfg(test)]
mod tests;
