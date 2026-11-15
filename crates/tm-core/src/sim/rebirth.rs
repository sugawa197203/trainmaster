//! 転生と転生の強化 (T1)。解放条件は累計収入がゴール(1 無量大数円)に届いたこと。
//! 転生すると会社・駅・路線・電車・所持金は最初からやり直し、転生ポイントと恒久の強化だけが残る。

use tm_numeric::ja_unit_index;

use super::SimScratch;
use super::movement;
use crate::balance::BalanceTable;
use crate::command::CommandError;
use crate::event::GameEvent;
use crate::ids::{CompanyLevel, LineId, StationId};
use crate::param::{ParamSet, RebirthUpgrade};
use crate::sim::company::unlocked_at;
use crate::state::{
    CompanyState, GameState, INCOME_WINDOW_SECS, PaxMatrix, Progress, Station, Stats, TrackGrid,
};
use crate::units::MilliPeople;

/// 転生の計画(検査の結果)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RebirthPlan {
    /// 転生でもらえるポイント。
    pub points_earned: u64,
}

/// 転生の強化の計画(検査の結果)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UpgradePlan {
    /// どの強化か。
    pub upgrade: RebirthUpgrade,
    /// 払うポイント。
    pub cost: u64,
    /// 買う前のレベル。
    pub from: u8,
    /// 買った後のレベル。
    pub to: u8,
}

/// 転生ができるか(累計収入がゴールに届いたか)。
pub(crate) fn check_rebirth(
    st: &GameState,
    bal: &BalanceTable,
) -> Result<RebirthPlan, CommandError> {
    if !st.progress.goal_reached {
        return Err(CommandError::RebirthNotReady);
    }
    Ok(RebirthPlan {
        points_earned: bal.rebirth_points(st.company.level),
    })
}

/// 転生を適用する: ポイントをもらい、会社を最初の状態に戻す。駅の位置の計画と乱数の位置はそのまま。
pub(crate) fn apply_rebirth(
    st: &mut GameState,
    bal: &BalanceTable,
    points_earned: u64,
    events: &mut Vec<GameEvent>,
) {
    st.rebirth.points = st.rebirth.points.saturating_add(points_earned);
    st.rebirth.count = st.rebirth.count.saturating_add(1);

    let spec = bal.spec();
    let base = usize::from(spec.company.stations_base);
    st.stations = st.station_plan[..base]
        .iter()
        .enumerate()
        .map(|(i, &pos)| Station {
            id: StationId::from_index(i),
            pos,
            spawned_at: st.tick,
            levels: Default::default(),
            spawn_acc: MilliPeople::ZERO,
            waiting_total: Default::default(),
            serving_lines: Vec::new(),
        })
        .collect();
    let mut waiting = PaxMatrix::new(bal.max_stations());
    for _ in &st.stations {
        waiting.push_row();
    }
    st.waiting = waiting;
    st.onboard = PaxMatrix::new(bal.max_stations());
    st.lines.clear();
    st.trains.clear();
    st.tracks = TrackGrid::new(spec.map.width, spec.map.height);
    st.money = bal.initial_money();
    st.company = CompanyState {
        level: CompanyLevel::ZERO,
        line_slots: Default::default(),
        track_tiles: 0,
    };
    st.progress = Progress {
        unlocked: unlocked_at(bal, CompanyLevel::ZERO),
        upgraded_once: ParamSet::default(),
        total_earned: crate::units::Money::ZERO,
        max_money_unit: ja_unit_index(st.money),
        goal_reached: false,
    };
    st.stats = Stats {
        income_this_sec: crate::units::Money::ZERO,
        income_window: [crate::units::Money::ZERO; INCOME_WINDOW_SECS],
        window_head: 0,
        arrivals: 0,
        delivered: 0,
        purchases: 0,
    };
    st.map_revision += 1;
    st.routing = crate::routing::Routing::new(st, bal);
    events.push(GameEvent::Rebirth {
        points_earned,
        count: st.rebirth.count,
    });
}

/// 転生の強化が買えるか。
pub(crate) fn check_upgrade(
    st: &GameState,
    bal: &BalanceTable,
    upgrade: RebirthUpgrade,
) -> Result<UpgradePlan, CommandError> {
    let def = bal.rebirth_upgrade(upgrade);
    let from = st.rebirth.level(upgrade);
    if from >= def.max {
        return Err(CommandError::RebirthUpgradeMaxed { upgrade });
    }
    let cost = bal.rebirth_cost(upgrade, from);
    if st.rebirth.points < cost {
        return Err(CommandError::RebirthUpgradePoints {
            required: cost,
            available: st.rebirth.points,
        });
    }
    Ok(UpgradePlan {
        upgrade,
        cost,
        from,
        to: from + 1,
    })
}

/// 転生の強化を適用する: ポイントを払い、レベルを 1 上げる。電車の数の強化は全路線に電車を 1 本足す。
pub(crate) fn apply_upgrade(
    st: &mut GameState,
    plan: &UpgradePlan,
    sc: &mut SimScratch,
    events: &mut Vec<GameEvent>,
) {
    st.rebirth.points -= plan.cost;
    st.rebirth.set_level(plan.upgrade, plan.to);
    events.push(GameEvent::RebirthUpgraded {
        upgrade: plan.upgrade,
        new_level: plan.to,
    });
    if plan.upgrade == RebirthUpgrade::TrainBoost {
        for i in 0..st.lines.len() {
            let line = LineId::from_index(i);
            let train = movement::place_added_train(st, line, &mut sc.coords);
            events.push(GameEvent::TrainAdded { train, line });
        }
    }
}

/// 運賃の倍率(転生の強化の分。1 = 倍率なし)。
pub(crate) fn fare_mul(st: &GameState, bal: &BalanceTable) -> u64 {
    bal.fare_boost_mul(st.rebirth.fare_boost)
}

/// 1 路線の初期の電車と R1 の上限に足す分。
pub(crate) fn train_bonus(st: &GameState, bal: &BalanceTable) -> u32 {
    bal.train_boost_trains(st.rebirth.train_boost)
}

/// 路線の枠に足す分。
pub(crate) fn line_bonus(st: &GameState, bal: &BalanceTable) -> u32 {
    bal.line_boost_slots(st.rebirth.line_boost)
}

/// 転生の強化のレベルが表の最大を超えていないか(INV-22)。
pub(crate) fn check_levels(st: &GameState, bal: &BalanceTable) -> Result<(), String> {
    for k in RebirthUpgrade::ALL {
        let level = st.rebirth.level(k);
        let max = bal.rebirth_upgrade(k).max;
        if level > max {
            return Err(format!("{k:?} = {level} > 最大 {max}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
