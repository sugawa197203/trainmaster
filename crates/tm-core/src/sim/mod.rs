//! 1 tick の手順の並び(11 §2.1)と、tick の中で使い回す作業領域。

mod apply;
mod boarding;
mod company;
mod economy;
mod movement;
pub(crate) mod rebirth;
mod spawn;
mod topology;

pub(crate) use apply::{check_command, check_line_draft};
pub(crate) use boarding::waiting_limit;
pub(crate) use company::{gauge, gauge_view, unlocked_at};
pub(crate) use economy::{income_per_sec, recommendation};
pub(crate) use movement::{departure_index, next_stop};
pub(crate) use spawn::max_stopping_attractiveness;
pub(crate) use topology::{make_segment, rebuild_line_derived, tracks_from_lines};

use crate::balance::BalanceTable;
use crate::command::Command;
use crate::event::GameEvent;
use crate::ids::TrainId;
use crate::state::GameState;
use topology::{PassChange, TrainAnchor};

/// 1 tick の中で使い回すバッファ(11 §1.3、SIM-D-02)。tick の外に意味を持たない。
#[derive(Clone, Debug, Default)]
pub(crate) struct SimScratch {
    arrived: Vec<TrainId>,
    pass_changes: Vec<PassChange>,
    anchors: Vec<TrainAnchor>,
    coords: Vec<(u64, TrainId)>,
    stops_before: Vec<bool>,
}

/// 1 tick を進める(02 D27、10 §6.1)。`queue` は空にして領域を返す。
pub(crate) fn run_tick(
    st: &mut GameState,
    bal: &BalanceTable,
    queue: &mut Vec<Command>,
    sc: &mut SimScratch,
    events: &mut Vec<GameEvent>,
) {
    st.tick = st.tick.next();

    for cmd in queue.iter() {
        apply::apply_command(st, bal, cmd, sc, events);
    }
    queue.clear();

    // 表は Arc なので複製は参照カウントだけ(11 §2.1 の注)。
    let mut r = st.routing.clone();
    let recomputed = r.recompute_if_dirty(st, bal);
    st.routing = r;
    if let Some(node_count) = recomputed {
        events.push(GameEvent::RoutesRecomputed { node_count });
    }

    movement::advance_all(st, bal, &mut sc.arrived);

    for &t in &sc.arrived {
        boarding::serve_arrival(st, bal, t, events);
    }
    economy::check_money_milestones(st, bal, events);

    if st.tick.is_second_boundary() {
        economy::roll_income_window(st);
        spawn::spawn_all(st, bal);
    }

    #[cfg(debug_assertions)]
    if let Err(v) = crate::invariants::check_all(st, bal) {
        panic!("{} が破れた(tick {}): {}", v.id, st.tick.0, v.detail);
    }
}
