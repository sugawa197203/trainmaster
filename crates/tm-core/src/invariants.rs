//! 不変条件の検査(10 §3.9、11 §9)と「減らせない」の検査。

use core::cmp::Ordering;

use tm_numeric::{BigNum, ja_unit_index};

use crate::balance::BalanceTable;
use crate::ids::{CompanyLevel, Level, LineId, StationId};
use crate::param::{
    Feature, FeatureSet, LineLevels, ParamKey, ParamOwner, ParamSet, StationLevels,
};
use crate::sim;
use crate::state::{GameState, Line, TrainPhase, lay_track};
use crate::time::Tick;
use crate::units::{Millis, Money, People, ms_to_ticks, train_capacity};

/// 破れた不変条件。
#[derive(Clone, Debug, PartialEq)]
pub struct InvariantViolation {
    /// `"INV-05"` などの ID(「減らせない」の検査は `"MONO"`)。
    pub id: &'static str,
    /// 何が破れたか。
    pub detail: String,
}

/// 「減らせない」の検査に使う、2 つの時点で比べる値 [Readme、02 §10.1]。
#[derive(Clone, Debug, PartialEq)]
pub struct MonotonicSnapshot {
    /// 最後に処理した tick。
    pub tick: Tick,
    /// 会社レベル。
    pub company_level: CompanyLevel,
    /// 駅の数。
    pub station_count: u32,
    /// 路線ごとの駅の列。
    pub line_stations: Vec<Vec<StationId>>,
    /// 列車の数。
    pub train_count: u32,
    /// 駅ごとのレベル。
    pub station_levels: Vec<StationLevels>,
    /// 路線ごとのレベル。
    pub line_levels: Vec<LineLevels>,
    /// R2 のレベル。
    pub line_slots: Level,
    /// これまでに線路を敷いたマスの数。
    pub track_tiles: u32,
    /// 解放済みの機能。
    pub unlocked: FeatureSet,
    /// 一度でも強化したパラメータ。
    pub upgraded_once: ParamSet,
    /// 累計収入。
    pub total_earned: Money,
    /// 所持金が届いた最大の単位。
    pub max_money_unit: u8,
    /// ゴールに届いたか。
    pub goal_reached: bool,
}

fn fail<T>(id: &'static str, detail: String) -> Result<T, InvariantViolation> {
    Err(InvariantViolation { id, detail })
}

fn ensure(
    ok: bool,
    id: &'static str,
    detail: impl FnOnce() -> String,
) -> Result<(), InvariantViolation> {
    if ok { Ok(()) } else { fail(id, detail()) }
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).expect("個数は u32")
}

pub(crate) fn snapshot(st: &GameState) -> MonotonicSnapshot {
    MonotonicSnapshot {
        tick: st.tick,
        company_level: st.company.level,
        station_count: count_u32(st.stations.len()),
        line_stations: st.lines.iter().map(|l| l.stations.clone()).collect(),
        train_count: count_u32(st.trains.len()),
        station_levels: st.stations.iter().map(|s| s.levels).collect(),
        line_levels: st.lines.iter().map(|l| l.levels).collect(),
        line_slots: st.company.line_slots,
        track_tiles: st.company.track_tiles,
        unlocked: st.progress.unlocked,
        upgraded_once: st.progress.upgraded_once,
        total_earned: st.progress.total_earned,
        max_money_unit: st.progress.max_money_unit,
        goal_reached: st.progress.goal_reached,
    }
}

fn levels_not_lower(before: &[Level], after: &[Level]) -> bool {
    before.iter().zip(after).all(|(b, a)| b <= a)
}

fn station_level_list(l: &StationLevels) -> [Level; 4] {
    [l.spawn, l.fare, l.capacity, l.platform]
}

fn line_level_list(l: &LineLevels) -> [Level; 7] {
    [
        l.speed,
        l.crowding,
        l.boarding,
        l.car_capacity,
        l.cars,
        l.max_trains,
        l.attractiveness,
    ]
}

fn is_contiguous_part(before: &[StationId], after: &[StationId]) -> bool {
    before.is_empty() || after.windows(before.len()).any(|w| w == before)
}

/// `before` → `after` で、時刻・会社レベル・数・駅の列・レベル・解放・累計収入などが減っていないか。
pub fn check_monotonic(
    before: &MonotonicSnapshot,
    after: &MonotonicSnapshot,
) -> Result<(), InvariantViolation> {
    const ID: &str = "MONO";
    let (b, a) = (before, after);
    ensure(a.tick >= b.tick, ID, || {
        format!("tick {:?} → {:?}", b.tick, a.tick)
    })?;
    ensure(a.company_level >= b.company_level, ID, || {
        format!("会社レベル {} → {}", b.company_level, a.company_level)
    })?;
    ensure(a.station_count >= b.station_count, ID, || {
        format!("駅の数 {} → {}", b.station_count, a.station_count)
    })?;
    ensure(a.line_stations.len() >= b.line_stations.len(), ID, || {
        format!(
            "路線の数 {} → {}",
            b.line_stations.len(),
            a.line_stations.len()
        )
    })?;
    ensure(a.train_count >= b.train_count, ID, || {
        format!("列車の数 {} → {}", b.train_count, a.train_count)
    })?;
    for (i, (bl, al)) in b.line_stations.iter().zip(&a.line_stations).enumerate() {
        ensure(is_contiguous_part(bl, al), ID, || {
            format!("L{i} の駅の列 {bl:?} → {al:?}")
        })?;
    }
    for (i, (bs, as_)) in b.station_levels.iter().zip(&a.station_levels).enumerate() {
        ensure(
            levels_not_lower(&station_level_list(bs), &station_level_list(as_)),
            ID,
            || format!("S{i} のレベル {bs:?} → {as_:?}"),
        )?;
    }
    for (i, (bl, al)) in b.line_levels.iter().zip(&a.line_levels).enumerate() {
        ensure(
            levels_not_lower(&line_level_list(bl), &line_level_list(al)),
            ID,
            || format!("L{i} のレベル {bl:?} → {al:?}"),
        )?;
    }
    ensure(a.line_slots >= b.line_slots, ID, || {
        format!("路線の数のレベル {} → {}", b.line_slots, a.line_slots)
    })?;
    ensure(a.track_tiles >= b.track_tiles, ID, || {
        format!("線路のマス数 {} → {}", b.track_tiles, a.track_tiles)
    })?;
    ensure(b.unlocked.is_subset(a.unlocked), ID, || {
        format!("解放 {:?} → {:?}", b.unlocked, a.unlocked)
    })?;
    ensure(b.upgraded_once.is_subset(a.upgraded_once), ID, || {
        format!("強化済み {:?} → {:?}", b.upgraded_once, a.upgraded_once)
    })?;
    ensure(
        a.total_earned.total_cmp(&b.total_earned) != Ordering::Less,
        ID,
        || format!("累計収入 {:?} → {:?}", b.total_earned, a.total_earned),
    )?;
    ensure(a.max_money_unit >= b.max_money_unit, ID, || {
        format!("単位 {} → {}", b.max_money_unit, a.max_money_unit)
    })?;
    ensure(!b.goal_reached || a.goal_reached, ID, || {
        "ゴールが戻った".to_string()
    })
}

/// INV-01〜INV-21 を検査し、最初に破れたものを返す(11 §9)。INV-17・INV-16 は INV-10・INV-13 の前提なので先に見る。
pub(crate) fn check_all(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    inv01_stations(st, bal)?;
    inv02_positions(st, bal)?;
    inv03_line_count(st, bal)?;
    inv04_line_shape(st)?;
    inv05_trains(st, bal)?;
    inv06_levels(st, bal)?;
    inv07_money(st)?;
    inv08_pax(st)?;
    inv09_capacity(st, bal)?;
    inv17_offsets(st)?;
    inv16_stops(st)?;
    inv10_train_state(st, bal)?;
    inv11_spawn_acc(st)?;
    inv12_company_settled(st, bal)?;
    inv13_derived(st, bal)?;
    inv14_unlocked(st, bal)?;
    inv15_money_progress(st, bal)?;
    inv18_reachable(st)?;
    inv19_unconnected(st)?;
    inv20_track_tiles(st)?;
    inv21_line_born_level(st)?;
    inv22_rebirth(st, bal)
}

fn inv01_stations(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-01";
    let c = &bal.spec().company;
    let want = u32::from(c.stations_base)
        + u32::from(st.company.level.0) * u32::from(c.stations_per_level);
    let n = count_u32(st.stations.len());
    ensure(n == want && n <= bal.max_stations(), ID, || {
        format!("駅の数 {n}、会社 Lv{} で {want}", st.company.level)
    })?;
    ensure(
        st.station_plan.len() == bal.max_stations() as usize,
        ID,
        || format!("station_plan の長さ {}", st.station_plan.len()),
    )?;
    for (i, s) in st.stations.iter().enumerate() {
        ensure(s.id.index() == i && s.pos == st.station_plan[i], ID, || {
            format!(
                "S{i}: id {}、pos {:?}、計画 {:?}",
                s.id, s.pos, st.station_plan[i]
            )
        })?;
    }
    Ok(())
}

fn inv02_positions(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-02";
    let m = &bal.spec().map;
    let (w, h, g) = (i32::from(m.width), i32::from(m.height), i32::from(m.margin));
    for (i, s) in st.stations.iter().enumerate() {
        let p = s.pos;
        ensure(
            (g..w - g).contains(&p.x) && (g..h - g).contains(&p.y),
            ID,
            || format!("S{i} の位置 {p:?} が外周の内側にない"),
        )?;
        ensure(!st.stations[..i].iter().any(|o| o.pos == p), ID, || {
            format!("S{i} の位置 {p:?} が重なる")
        })?;
    }
    Ok(())
}

fn inv03_line_count(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    let n = count_u32(st.lines.len());
    let bonus = sim::rebirth::line_bonus(st, bal);
    let slots = bal.line_slots(st.company.line_slots) + bonus;
    ensure(
        n <= slots && slots <= bal.max_lines() + bonus,
        "INV-03",
        || format!("路線 {n}、枠 {slots}、最大 {}", bal.max_lines()),
    )
}

fn inv04_line_shape(st: &GameState) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-04";
    let mut tiles = Vec::new();
    for l in &st.lines {
        let ss = &l.stations;
        ensure(ss.len() >= 2, ID, || format!("{}: 駅 {}", l.id, ss.len()))?;
        for (j, s) in ss.iter().enumerate() {
            ensure(s.index() < st.stations.len(), ID, || {
                format!("{}: 未出現の {s}", l.id)
            })?;
            ensure(!ss[..j].contains(s), ID, || format!("{}: {s} が重複", l.id))?;
        }
        ensure(l.segments.len() == ss.len() - 1, ID, || {
            format!("{}: 区間 {}、駅 {}", l.id, l.segments.len(), ss.len())
        })?;
        for (i, seg) in l.segments.iter().enumerate() {
            ensure(seg.from == ss[i] && seg.to == ss[i + 1], ID, || {
                format!("{}: segments[{i}] が {} → {}", l.id, seg.from, seg.to)
            })?;
            lay_track(
                st.stations[seg.from.index()].pos,
                st.stations[seg.to.index()].pos,
                seg.bend,
                &mut tiles,
            );
            ensure(
                seg.tiles == tiles && seg.len.0 as usize + 1 == tiles.len(),
                ID,
                || format!("{}: segments[{i}] のマスが lay_track と違う", l.id),
            )?;
        }
    }
    Ok(())
}

fn inv05_trains(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-05";
    for (i, t) in st.trains.iter().enumerate() {
        ensure(
            t.id.index() == i && t.line.index() < st.lines.len(),
            ID,
            || format!("T{i}: id {}、路線 {}", t.id, t.line),
        )?;
    }
    let mut total = 0usize;
    for l in &st.lines {
        let want =
            (bal.max_trains(l.levels.max_trains) + sim::rebirth::train_bonus(st, bal)) as usize;
        ensure(l.trains.len() == want, ID, || {
            format!("{}: 列車 {}、R1 で {want}", l.id, l.trains.len())
        })?;
        ensure(l.trains.windows(2).all(|w| w[0] < w[1]), ID, || {
            format!("{}: 列車が昇順でない", l.id)
        })?;
        for t in &l.trains {
            ensure(
                t.index() < st.trains.len() && st.trains[t.index()].line == l.id,
                ID,
                || format!("{}: {t} の路線が違う", l.id),
            )?;
        }
        total += l.trains.len();
    }
    ensure(total == st.trains.len(), ID, || {
        format!("列車の総数 {}、路線の合計 {total}", st.trains.len())
    })
}

fn inv06_levels(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-06";
    let c = st.company.level;
    for p in ParamKey::ALL {
        let limit = bal.limit(p, c);
        let unlocked = st.progress.unlocked.contains(Feature::Param(p));
        let check = |owner: String, level: Level, connected: bool| {
            ensure(level <= limit, ID, || {
                format!("{owner} の {p:?} = {level} > 上限 {limit}")
            })?;
            ensure(unlocked || level == Level::ZERO, ID, || {
                format!("{owner} の未解放の {p:?} = {level}")
            })?;
            ensure(connected || level == Level::ZERO, ID, || {
                format!("路線に入っていない {owner} の {p:?} = {level}")
            })
        };
        match p.owner() {
            ParamOwner::Station => {
                for s in &st.stations {
                    let lv = s.levels.get(p).expect("駅のパラメータ");
                    check(s.id.to_string(), lv, s.is_connected())?;
                }
            }
            ParamOwner::Line => {
                for l in &st.lines {
                    let lv = l.levels.get(p).expect("路線のパラメータ");
                    check(l.id.to_string(), lv, true)?;
                }
            }
            ParamOwner::Company => check("会社".to_string(), st.company.line_slots, true)?,
        }
    }
    Ok(())
}

fn finite_non_negative(m: Money) -> bool {
    BigNum::from_bits_checked(m.to_bits()).is_some()
}

fn inv07_money(st: &GameState) -> Result<(), InvariantViolation> {
    ensure(
        finite_non_negative(st.money) && finite_non_negative(st.progress.total_earned),
        "INV-07",
        || {
            format!(
                "所持金 {:?}、累計収入 {:?}",
                st.money, st.progress.total_earned
            )
        },
    )
}

fn inv08_pax(st: &GameState) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-08";
    let n = st.stations.len();
    ensure(
        st.waiting.rows() as usize == n && st.onboard.rows() as usize == st.trains.len(),
        ID,
        || {
            format!(
                "行数 waiting {}、onboard {}",
                st.waiting.rows(),
                st.onboard.rows()
            )
        },
    )?;
    for s in &st.stations {
        let row = st.waiting.row(s.id.0);
        ensure(st.waiting.row_total(s.id.0) == s.waiting_total, ID, || {
            format!("{}: waiting_total {:?} != 行の合計", s.id, s.waiting_total)
        })?;
        ensure(row[s.id.index()] == 0, ID, || {
            format!("{}: 自分の駅が目的駅", s.id)
        })?;
        ensure(row[n..].iter().all(|&v| v == 0), ID, || {
            format!("{}: 未出現の目的駅", s.id)
        })?;
    }
    for t in &st.trains {
        ensure(st.onboard.row_total(t.id.0) == t.riders, ID, || {
            format!("{}: riders {:?} != 行の合計", t.id, t.riders)
        })?;
        ensure(
            st.onboard.row(t.id.0)[n..].iter().all(|&v| v == 0),
            ID,
            || format!("{}: 未出現の目的駅", t.id),
        )?;
    }
    Ok(())
}

fn inv09_capacity(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    for t in &st.trains {
        let lv = st.lines[t.line.index()].levels;
        let cap = train_capacity(
            bal.car_capacity(lv.car_capacity),
            bal.cars(lv.cars),
            bal.crowding(lv.crowding),
        );
        ensure(t.riders <= cap, "INV-09", || {
            format!("{}: {:?} > 定員 {cap:?}", t.id, t.riders)
        })?;
    }
    Ok(())
}

fn inv10_train_state(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-10";
    let max_dwell = ms_to_ticks(Millis(bal.spec().transit.dwell_max_ms));
    for t in &st.trains {
        let line = &st.lines[t.line.index()];
        let k = usize::from(t.section);
        ensure(k < line.sections.len(), ID, || {
            format!("{}: 区間 {k}", t.id)
        })?;
        let len = line.sections[k].length;
        match t.phase {
            TrainPhase::Running => {
                ensure(t.pos < len, ID, || {
                    format!("{}: 走行中 pos {:?} ≥ {len:?}", t.id, t.pos)
                })?;
            }
            TrainPhase::Dwelling { at, remaining } => {
                let dep = sim::departure_index(line, t.section, t.dir);
                ensure(
                    t.pos.0 == 0 && usize::from(at) == dep && remaining < max_dwell,
                    ID,
                    || {
                        format!(
                            "{}: 停車中 pos {:?}、at {at}(出発側 {dep})、残り {remaining:?}",
                            t.id, t.pos
                        )
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn inv11_spawn_acc(st: &GameState) -> Result<(), InvariantViolation> {
    for s in &st.stations {
        ensure(s.spawn_acc.0 < 1000, "INV-11", || {
            format!("{}: spawn_acc {:?}", s.id, s.spawn_acc)
        })?;
    }
    Ok(())
}

fn inv12_company_settled(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    let rules = &bal.spec().company;
    if st.company.level.0 >= rules.max_level {
        return Ok(());
    }
    let (sum, lim) = sim::gauge(st, bal);
    let ready = lim > 0 && u64::from(rules.gauge_den) * sum >= u64::from(rules.gauge_num) * lim;
    ensure(!ready, "INV-12", || {
        format!("ゲージ {sum}/{lim} でレベルアップしていない")
    })
}

fn inv17_offsets(st: &GameState) -> Result<(), InvariantViolation> {
    for l in &st.lines {
        let mut acc = 0u32;
        let ok = l.offsets.len() == l.stations.len()
            && l.offsets.iter().enumerate().all(|(i, &o)| {
                let here = o == acc;
                if let Some(seg) = l.segments.get(i) {
                    acc += seg.len.0;
                }
                here
            });
        ensure(ok, "INV-17", || {
            format!("{}: offsets {:?}", l.id, l.offsets)
        })?;
    }
    Ok(())
}

fn inv13_derived(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-13";
    for l in &st.lines {
        let mut fresh: Line = l.clone();
        sim::rebuild_line_derived(&mut fresh, &st.stations, bal);
        ensure(
            fresh.stops == l.stops && fresh.sections == l.sections && fresh.offsets == l.offsets,
            ID,
            || format!("{}: stops・sections・offsets が作り直したものと違う", l.id),
        )?;
    }
    for s in &st.stations {
        let want: Vec<LineId> = st
            .lines
            .iter()
            .filter(|l| l.stations.contains(&s.id))
            .map(|l| l.id)
            .collect();
        ensure(s.serving_lines == want, ID, || {
            format!(
                "{}: serving_lines {:?}、正しくは {want:?}",
                s.id, s.serving_lines
            )
        })?;
    }
    let grid = sim::tracks_from_lines(st.tracks.width, st.tracks.height, &st.lines);
    ensure(grid == st.tracks, ID, || {
        "tracks が全路線から作り直したものと違う".to_string()
    })?;
    ensure(!st.routing.is_dirty(), ID, || {
        "経路表に要再計算の印が残っている".to_string()
    })?;
    st.routing
        .check_up_to_date(st, bal)
        .or_else(|detail| fail(ID, detail))
}

fn inv14_unlocked(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    let want = sim::unlocked_at(bal, st.company.level);
    ensure(st.progress.unlocked == want, "INV-14", || {
        format!(
            "解放 {:?}、会社 Lv{} で {want:?}",
            st.progress.unlocked, st.company.level
        )
    })
}

fn inv15_money_progress(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    const ID: &str = "INV-15";
    let p = &st.progress;
    let k = ja_unit_index(st.money);
    ensure(p.max_money_unit >= k, ID, || {
        format!("単位 {} < いまの単位 {k}", p.max_money_unit)
    })?;
    let reached = p.total_earned.total_cmp(&bal.goal()) != Ordering::Less;
    ensure(p.goal_reached == reached, ID, || {
        format!(
            "goal_reached {}、累計収入 {:?}",
            p.goal_reached, p.total_earned
        )
    })
}

fn inv16_stops(st: &GameState) -> Result<(), InvariantViolation> {
    for l in &st.lines {
        let stops = l.stops.iter().filter(|&&s| s).count();
        ensure(stops >= 2 && !l.sections.is_empty(), "INV-16", || {
            format!("{}: 停車駅 {stops}、区間 {}", l.id, l.sections.len())
        })?;
    }
    Ok(())
}

fn inv18_reachable(st: &GameState) -> Result<(), InvariantViolation> {
    for s in &st.stations {
        for (t, &n) in st.waiting.row(s.id.0)[..st.stations.len()]
            .iter()
            .enumerate()
        {
            let t = StationId::from_index(t);
            ensure(
                n == 0 || st.routing.station_cost(s.id, t).is_some(),
                "INV-18",
                || format!("{} で待つ {n} 人の目的駅 {t} に行けない", s.id),
            )?;
        }
    }
    Ok(())
}

fn inv19_unconnected(st: &GameState) -> Result<(), InvariantViolation> {
    for s in st.stations.iter().filter(|s| !s.is_connected()) {
        ensure(
            s.spawn_acc.0 == 0 && s.waiting_total == People::ZERO,
            "INV-19",
            || {
                format!(
                    "路線に入っていない {}: spawn_acc {:?}、待ち {:?}",
                    s.id, s.spawn_acc, s.waiting_total
                )
            },
        )?;
    }
    Ok(())
}

fn inv21_line_born_level(st: &GameState) -> Result<(), InvariantViolation> {
    for (i, l) in st.lines.iter().enumerate() {
        ensure(l.born_level <= st.company.level, "INV-21", || {
            format!(
                "{}: 作ったときの会社 {}、いま {}",
                l.id, l.born_level, st.company.level
            )
        })?;
        if let Some(prev) = i.checked_sub(1).map(|j| &st.lines[j]) {
            ensure(prev.born_level <= l.born_level, "INV-21", || {
                format!(
                    "{} の作ったときの会社 {} が前の {} の {} より小さい",
                    l.id, l.born_level, prev.id, prev.born_level
                )
            })?;
        }
    }
    Ok(())
}

fn inv20_track_tiles(st: &GameState) -> Result<(), InvariantViolation> {
    let n = st.tracks.occupied();
    ensure(st.company.track_tiles == n, "INV-20", || {
        format!(
            "track_tiles {} と線路のあるマス {n}",
            st.company.track_tiles
        )
    })
}

/// 転生の強化のレベルが表の最大を超えていない [T1]。
fn inv22_rebirth(st: &GameState, bal: &BalanceTable) -> Result<(), InvariantViolation> {
    sim::rebirth::check_levels(st, bal).map_err(|detail| InvariantViolation {
        id: "INV-22",
        detail,
    })
}

#[cfg(test)]
mod tests;
