//! コマンドの検査(10 §5.3 の順)と適用(11 §2.2、§4.3〜4.4)。`validate` と `step` が同じ検査を使う。

use core::cmp::Ordering;

use super::SimScratch;
use super::company::settle_company_level;
use super::economy::{PurchasePlan, apply_purchase, plan_purchase, require_unlocked};
use super::movement::place_initial_train;
use super::rebirth::{self, RebirthPlan, UpgradePlan};
use super::topology::{
    PassChange, add_line_tracks, extend_shape, insert_serving_line, new_line, rebuild_line_derived,
    relocate_trains, snapshot_anchors, station_index,
};
use crate::balance::BalanceTable;
use crate::command::{Command, CommandError, CommandPreview, LineEnd};
use crate::event::GameEvent;
use crate::geom::{Bend, GridPos};
use crate::ids::{Level, LineId, StationId};
use crate::param::Feature;
use crate::state::{GameState, TileBits, TrackGrid, walk_track};
use crate::units::Money;

/// 検査を通ったコマンド。`check_command` が作り、`apply_command` がそのまま使う。
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "validate はメモリを確保しない(11 §1.1)"
)]
pub(crate) enum CheckedCommand {
    CreateLine(LinePlan),
    ExtendLine(ExtendPlan),
    LevelUp(PurchasePlan),
    Rebirth(RebirthPlan),
    RebirthUpgrade(UpgradePlan),
}

/// 線路の値段と新しいマスの数(11 §4.8、SIM-D-25)。検査で決め、適用はこの値を使う。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrackPrice {
    pub cost: Money,
    pub new_tiles: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LinePlan {
    pub track_tiles: u32,
    pub new_pass_through: u32,
    pub track: TrackPrice,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExtendPlan {
    pub new_len: u32,
    pub new_is_stop: bool,
    pub track: TrackPrice,
}

impl CheckedCommand {
    /// 11 §4.7 の表。
    pub fn preview(&self) -> CommandPreview {
        match self {
            CheckedCommand::CreateLine(p) => CommandPreview {
                cost: p.track.cost,
                steps: 0,
                raised: 0,
                track_tiles: p.track_tiles,
                new_track_tiles: p.track.new_tiles,
                new_pass_through: p.new_pass_through,
            },
            CheckedCommand::ExtendLine(p) => CommandPreview {
                cost: p.track.cost,
                steps: 0,
                raised: 0,
                track_tiles: p.new_len,
                new_track_tiles: p.track.new_tiles,
                new_pass_through: u32::from(!p.new_is_stop),
            },
            CheckedCommand::LevelUp(p) => p.preview(),
            CheckedCommand::Rebirth(_) | CheckedCommand::RebirthUpgrade(_) => CommandPreview {
                cost: Money::ZERO,
                steps: 0,
                raised: 0,
                track_tiles: 0,
                new_track_tiles: 0,
                new_pass_through: 0,
            },
        }
    }
}

/// 状態を変えずに検査する(10 §5.3。存在と形式 → 解放 → 対象 → お金)。メモリを確保しない。
pub(crate) fn check_command(
    st: &GameState,
    bal: &BalanceTable,
    cmd: &Command,
) -> Result<CheckedCommand, CommandError> {
    match cmd {
        Command::CreateLine { stations, bends } => {
            check_create(st, bal, stations, bends).map(CheckedCommand::CreateLine)
        }
        Command::ExtendLine {
            line,
            end,
            station,
            bend,
        } => check_extend(st, bal, (*line, *end, *station, *bend)).map(CheckedCommand::ExtendLine),
        Command::LevelUp { param, scope, mode } => {
            plan_purchase(st, bal, *param, *scope, *mode).map(CheckedCommand::LevelUp)
        }
        Command::Rebirth => rebirth::check_rebirth(st, bal).map(CheckedCommand::Rebirth),
        Command::RebirthUpgrade { upgrade } => {
            rebirth::check_upgrade(st, bal, *upgrade).map(CheckedCommand::RebirthUpgrade)
        }
    }
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).expect("個数は u32")
}

/// 区間 (from の位置, to の位置, 保存値の向きの bend) のマスのうち、いま線路がないマスの数。
/// 同じコマンドの中で 2 回通るマスは 1 回だけ数える(10 TD-42)。
pub(crate) fn count_new_tiles(
    grid: &TrackGrid,
    segs: impl IntoIterator<Item = (GridPos, GridPos, Bend)>,
) -> u32 {
    let mut seen = TileBits::EMPTY;
    let mut k = 0;
    for (a, b, bend) in segs {
        for p in walk_track(a, b, bend) {
            if grid.get(p).is_empty() && seen.insert(grid.index(p)) {
                k += 1;
            }
        }
    }
    k
}

/// 線路の値段を決め、所持金が足りなければ `InsufficientFunds`(10 §5.3 の手順 4、TD-41)。
fn price_track(
    st: &GameState,
    bal: &BalanceTable,
    segs: impl IntoIterator<Item = (GridPos, GridPos, Bend)>,
) -> Result<TrackPrice, CommandError> {
    let new_tiles = count_new_tiles(&st.tracks, segs);
    let cost = bal.track_cost(st.company.level, st.company.track_tiles, new_tiles);
    if st.money.total_cmp(&cost) == Ordering::Less {
        return Err(CommandError::InsufficientFunds {
            required: cost,
            available: st.money,
        });
    }
    Ok(TrackPrice { cost, new_tiles })
}

/// 路線を作る途中で `selected` の次に `next` を足せるか(作成の手順 1 の存在・重複と手順 3 の枠)。
pub(crate) fn check_line_draft(
    st: &GameState,
    bal: &BalanceTable,
    selected: &[StationId],
    next: StationId,
) -> Result<(), CommandError> {
    let all = || selected.iter().chain(core::iter::once(&next));
    if let Some(&s) = all().find(|s| s.index() >= st.stations.len()) {
        return Err(CommandError::UnknownStation(s));
    }
    check_duplicate(selected)?;
    if selected.contains(&next) {
        return Err(CommandError::DuplicateStation(next));
    }
    check_slot(st, bal)
}

fn check_duplicate(stations: &[StationId]) -> Result<(), CommandError> {
    for (j, s) in stations.iter().enumerate() {
        if stations[..j].contains(s) {
            return Err(CommandError::DuplicateStation(*s));
        }
    }
    Ok(())
}

fn check_slot(st: &GameState, bal: &BalanceTable) -> Result<(), CommandError> {
    let used = count_u32(st.lines.len());
    let slots = bal.line_slots(st.company.line_slots) + rebirth::line_bonus(st, bal);
    if used >= slots {
        return Err(CommandError::NoFreeLineSlot { used, slots });
    }
    Ok(())
}

fn check_create(
    st: &GameState,
    bal: &BalanceTable,
    stations: &[StationId],
    bends: &[Bend],
) -> Result<LinePlan, CommandError> {
    if let Some(&s) = stations.iter().find(|s| s.index() >= st.stations.len()) {
        return Err(CommandError::UnknownStation(s));
    }
    if stations.len() < 2 {
        return Err(CommandError::TooFewStations {
            got: count_u32(stations.len()),
        });
    }
    check_duplicate(stations)?;
    if bends.len() != stations.len() - 1 {
        return Err(CommandError::BendCountMismatch {
            expected: count_u32(stations.len() - 1),
            got: count_u32(bends.len()),
        });
    }
    check_slot(st, bal)?;
    let pos = |s: &StationId| st.stations[s.index()].pos;
    let track = price_track(
        st,
        bal,
        stations
            .windows(2)
            .zip(bends)
            .map(|(w, &b)| (pos(&w[0]), pos(&w[1]), b)),
    )?;
    let track_tiles = stations
        .windows(2)
        .map(|w| pos(&w[0]).chebyshev(pos(&w[1])))
        .sum();
    let cars = bal.cars(Level::ZERO);
    let new_pass_through = count_u32(
        stations
            .iter()
            .filter(|s| bal.platform(st.stations[s.index()].levels.platform) < cars)
            .count(),
    );
    Ok(LinePlan {
        track_tiles,
        new_pass_through,
        track,
    })
}

fn check_extend(
    st: &GameState,
    bal: &BalanceTable,
    (l, end, s, bend): (LineId, LineEnd, StationId, Bend),
) -> Result<ExtendPlan, CommandError> {
    let Some(line) = st.lines.get(l.index()) else {
        return Err(CommandError::UnknownLine(l));
    };
    let Some(stn) = st.stations.get(s.index()) else {
        return Err(CommandError::UnknownStation(s));
    };
    if line.stations.contains(&s) {
        return Err(CommandError::StationAlreadyOnLine {
            line: l,
            station: s,
        });
    }
    require_unlocked(st, bal, Feature::LineExtend)?;
    let end_station = match end {
        LineEnd::Head => line.stations[0],
        LineEnd::Tail => *line.stations.last().expect("路線に駅がある"),
    };
    let end_pos = st.stations[end_station.index()].pos;
    let seg = match end {
        LineEnd::Tail => (end_pos, stn.pos, bend),
        LineEnd::Head => (stn.pos, end_pos, bend.reversed()),
    };
    let track = price_track(st, bal, [seg])?;
    Ok(ExtendPlan {
        new_len: end_pos.chebyshev(stn.pos),
        new_is_stop: bal.platform(stn.levels.platform) >= bal.cars(line.levels.cars),
        track,
    })
}

/// 手順1: その時点の状態でもう一度検査し、通れば適用する。失敗なら `CommandRejected` だけを出す(10 §5.4)。
pub(crate) fn apply_command(
    st: &mut GameState,
    bal: &BalanceTable,
    cmd: &Command,
    sc: &mut SimScratch,
    events: &mut Vec<GameEvent>,
) {
    let checked = match check_command(st, bal, cmd) {
        Ok(c) => c,
        Err(reason) => {
            events.push(GameEvent::CommandRejected {
                kind: cmd.kind(),
                reason,
            });
            return;
        }
    };
    match (cmd, checked) {
        (Command::CreateLine { stations, bends }, CheckedCommand::CreateLine(plan)) => {
            create_line(st, bal, stations, bends, plan.track, sc, events)
        }
        (
            Command::ExtendLine {
                line,
                end,
                station,
                bend,
            },
            CheckedCommand::ExtendLine(plan),
        ) => extend_line(
            st,
            bal,
            (*line, *end, *station, *bend),
            plan.track,
            sc,
            events,
        ),
        (Command::LevelUp { .. }, CheckedCommand::LevelUp(plan)) => apply_purchase(
            st,
            bal,
            &plan,
            &mut sc.anchors,
            &mut sc.coords,
            &mut sc.stops_before,
            events,
        ),
        (Command::Rebirth, CheckedCommand::Rebirth(plan)) => {
            rebirth::apply_rebirth(st, bal, plan.points_earned, events)
        }
        (Command::RebirthUpgrade { .. }, CheckedCommand::RebirthUpgrade(plan)) => {
            rebirth::apply_upgrade(st, &plan, sc, events)
        }
        _ => unreachable!("検査の結果とコマンドの種類が合わない"),
    }
    // 10 TD-10: 成功したコマンドの後だけ判定する。
    settle_company_level(st, bal, events);
}

/// 11 §4.3。イベントは `LineCreated` → `TrainAdded` → `PassThroughAdded`(10 §6.1)。
fn create_line(
    st: &mut GameState,
    bal: &BalanceTable,
    stations: &[StationId],
    bends: &[Bend],
    track: TrackPrice,
    sc: &mut SimScratch,
    events: &mut Vec<GameEvent>,
) {
    pay_track(st, track);
    let id = LineId::from_index(st.lines.len());
    let line = new_line(
        id,
        stations,
        bends,
        &st.stations,
        bal,
        (st.tick, st.company.level),
    );
    for seg in &line.segments {
        add_line_tracks(&mut st.tracks, &seg.tiles);
    }
    sc.pass_changes.clear();
    for (idx, &s) in line.stations.iter().enumerate() {
        insert_serving_line(&mut st.stations[s.index()].serving_lines, id);
        if !line.stops[idx] {
            sc.pass_changes.push(pass_change(id, idx, s));
        }
    }
    st.lines.push(line);
    events.push(GameEvent::LineCreated {
        line: id,
        station_count: count_u32(stations.len()),
        cost: track.cost,
        new_tiles: track.new_tiles,
    });
    let n = bal.max_trains(Level::ZERO) + rebirth::train_bonus(st, bal);
    for k in 0..n {
        let train = place_initial_train(st, id, k, n);
        events.push(GameEvent::TrainAdded { train, line: id });
    }
    flush_pass_changes(sc, events);
    st.routing.mark_dirty();
    st.map_revision += 1;
}

/// 11 §4.4。延伸は列車の位置を変えない(移し替え。11 §4.6)。
fn extend_line(
    st: &mut GameState,
    bal: &BalanceTable,
    (l, end, s, bend): (LineId, LineEnd, StationId, Bend),
    track: TrackPrice,
    sc: &mut SimScratch,
    events: &mut Vec<GameEvent>,
) {
    pay_track(st, track);
    let line = &mut st.lines[l.index()];
    snapshot_anchors(line, &st.trains, &mut sc.anchors);
    let k = extend_shape(line, &st.stations, end, s, bend);
    add_line_tracks(&mut st.tracks, &line.segments[k].tiles);
    rebuild_line_derived(line, &st.stations, bal);
    relocate_trains(line, &sc.anchors, &mut st.trains);
    let idx = station_index(line, s);
    let is_stop = line.stops[idx];
    insert_serving_line(&mut st.stations[s.index()].serving_lines, l);
    events.push(GameEvent::LineExtended {
        line: l,
        end,
        station: s,
        cost: track.cost,
        new_tiles: track.new_tiles,
    });
    sc.pass_changes.clear();
    if !is_stop {
        sc.pass_changes.push(pass_change(l, idx, s));
    }
    flush_pass_changes(sc, events);
    // 通過駅でも区間のつながりが変わりうるので付ける(10 §5.3)。
    st.routing.mark_dirty();
    st.map_revision += 1;
}

/// 検査で決めた値段を払い、線路のマス数を足す(11 §4.8。線路を敷くのは呼び出し側)。
fn pay_track(st: &mut GameState, track: TrackPrice) {
    st.money = st
        .money
        .checked_sub(track.cost)
        .expect("検査で所持金 ≥ 線路の値段");
    st.company.track_tiles += track.new_tiles;
}

fn pass_change(line: LineId, idx: usize, station: StationId) -> PassChange {
    PassChange {
        line,
        idx: u16::try_from(idx).expect("添字は u16"),
        station,
        now_stop: false,
    }
}

/// `(LineId, 添字)` の昇順に出す(SIM-D-12)。
fn flush_pass_changes(sc: &mut SimScratch, events: &mut Vec<GameEvent>) {
    sc.pass_changes.sort_by_key(|c| (c.line, c.idx));
    for c in &sc.pass_changes {
        events.push(if c.now_stop {
            GameEvent::PassThroughResolved {
                line: c.line,
                station: c.station,
            }
        } else {
            GameEvent::PassThroughAdded {
                line: c.line,
                station: c.station,
            }
        });
    }
    sc.pass_changes.clear();
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod track_tests;
