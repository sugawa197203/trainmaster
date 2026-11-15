//! 到着した列車の処理: 運賃 → 降車 → 乗車 → 停車時間(11 §6.3)。

use super::economy::book_income;
use super::movement::{arrival_index, begin_dwell, turn_at_arrival};
use super::rebirth;
use crate::balance::BalanceTable;
use crate::event::GameEvent;
use crate::ids::{StationId, TrainId};
use crate::routing::RiderAction;
use crate::state::{GameState, Station};
use crate::time::Ticks;
use crate::units::{Millis, Money, People, dwell_ms, fare_income, ms_to_ticks, train_capacity};

/// 1 回の到着の結果(11 §1.3)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ArrivalOutcome {
    pub riders: People,
    pub delivered: People,
    pub returned: People,
    pub boarded: People,
    pub income: Money,
    pub dwell: Ticks,
}

/// 手順4: 着いた列車の運賃・降車・乗車・停車時間を到着の tick にまとめて決め、`TrainArrived` を出す。
pub(crate) fn serve_arrival(
    st: &mut GameState,
    bal: &BalanceTable,
    tid: TrainId,
    events: &mut Vec<GameEvent>,
) {
    let tr = &st.trains[tid.index()];
    let (li, station) = {
        let line = &st.lines[tr.line.index()];
        (
            tr.line,
            line.stations[arrival_index(line, tr.section, tr.dir)],
        )
    };
    let out = arrive(st, bal, tid);
    events.push(GameEvent::TrainArrived {
        train: tid,
        line: li,
        station,
        riders: out.riders,
        alighted: People(out.delivered.0 + out.returned.0),
        boarded: out.boarded,
        income: out.income,
        dwell: out.dwell,
    });
}

fn arrive(st: &mut GameState, bal: &BalanceTable, tid: TrainId) -> ArrivalOutcome {
    let ti = tid.index();
    let line = &st.lines[st.trains[ti].line.index()];
    let l = line.id;
    let at = arrival_index(line, st.trains[ti].section, st.trains[ti].dir);
    debug_assert!(line.stops[at], "停車駅でない駅に着いた");
    let s = line.stations[at];
    let lv = line.levels;
    let ns = turn_at_arrival(line, &mut st.trains[ti]);

    let riders = st.trains[ti].riders;
    let income = fare_income(riders, bal.fare(st.stations[s.index()].levels.fare))
        .mul_small(rebirth::fare_mul(st, bal));
    book_income(st, income);

    let cols = st.onboard.cols();
    let (mut delivered, mut returned) = (0u64, 0u64);
    for t in (0..cols).map(StationId) {
        let n = st.onboard.get(tid.0, t);
        if n.0 == 0 {
            continue;
        }
        match st.routing.rider_action(l, s, t, ns) {
            RiderAction::Stay => {}
            RiderAction::Arrive => {
                st.onboard.take(tid.0, t, n);
                delivered += n.0;
            }
            RiderAction::AlightToWait => {
                st.onboard.take(tid.0, t, n);
                add_waiting_return(st, bal, s, t, n.0);
                returned += n.0;
            }
        }
    }
    let alighted = delivered + returned;
    let after_alight = riders.sub_floor(People(alighted)).0;

    let cap = train_capacity(
        bal.car_capacity(lv.car_capacity),
        bal.cars(lv.cars),
        bal.crowding(lv.crowding),
    );
    let rate = bal.boarding_rate(lv.boarding);
    let transit = &bal.spec().transit;
    let budget = u64::try_from(
        u128::from(transit.dwell_max_ms - transit.dwell_base_ms) * u128::from(rate.0) / 1000,
    )
    .unwrap_or(u64::MAX);
    let mut room = cap
        .sub_floor(People(after_alight))
        .0
        .min(budget.saturating_sub(alighted));
    let mut boarded = 0u64;
    for t in (0..cols).map(StationId) {
        if room == 0 {
            break;
        }
        let n = st.waiting.get(s.0, t).0;
        if n == 0 || !st.routing.wants_to_board(s, t, l, ns) {
            continue;
        }
        let k = st.waiting.take(s.0, t, People(n.min(room)));
        let w = &mut st.stations[s.index()].waiting_total;
        *w = w.sub_floor(k);
        st.onboard.add(tid.0, t, k);
        room -= k.0;
        boarded += k.0;
    }

    let ms = dwell_ms(
        People(alighted.saturating_add(boarded)),
        rate,
        Millis(transit.dwell_base_ms),
        Millis(transit.dwell_max_ms),
    );
    let dwell = ms_to_ticks(ms);
    let line = &st.lines[l.index()];
    let tr = &mut st.trains[ti];
    tr.riders = People(after_alight + boarded);
    begin_dwell(line, tr, dwell);

    st.stats.arrivals += 1;
    st.stats.delivered += u128::from(delivered);
    #[cfg(feature = "test-support")]
    {
        st.ledger.delivered += u128::from(delivered);
    }
    ArrivalOutcome {
        riders,
        delivered: People(delivered),
        returned: People(returned),
        boarded: People(boarded),
        income,
        dwell,
    }
}

/// 駅の待ち人数の合計の上限 = 定員 × `waiting_cap_factor`。`u128` で掛けて `u64` に飽和する(10 TD-37)。
pub(crate) fn waiting_limit(bal: &BalanceTable, stn: &Station) -> u64 {
    let cap = u128::from(bal.station_capacity(stn.levels.capacity).0);
    let v = cap * u128::from(bal.spec().transit.waiting_cap_factor);
    u64::try_from(v).unwrap_or(u64::MAX)
}

/// 降りて駅に戻る `n` 人を、合計が上限を超えない分だけマスと合計の両方に足す。超えた分は消える(11 §6.4、SIM-D-18)。
pub(crate) fn add_waiting_return(
    st: &mut GameState,
    bal: &BalanceTable,
    s: StationId,
    t: StationId,
    n: u64,
) {
    let limit = waiting_limit(bal, &st.stations[s.index()]);
    let total = st.stations[s.index()].waiting_total.0;
    debug_assert!(
        total <= limit,
        "駅 {s} の待ち人数 {total} が上限 {limit} を超えている"
    );
    let n2 = n.min(limit.saturating_sub(total));
    st.waiting.add(s.0, t, People(n2));
    st.stations[s.index()].waiting_total = People(total + n2);
    #[cfg(feature = "test-support")]
    {
        st.ledger.waiting_cap_loss += u128::from(n - n2);
    }
}

#[cfg(test)]
pub(crate) mod fixtures;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod flow_tests;
