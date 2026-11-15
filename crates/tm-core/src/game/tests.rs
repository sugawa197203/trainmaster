use std::sync::Arc;

use super::*;
use crate::balance::BalanceSpec;
use crate::command::LineEnd;
use crate::ids::{LineId, TrainId};
use crate::param::{BuyMode, Feature, FeatureSet, ParamKey, UpgradeScope};
use crate::time::Ticks;
use crate::units::PosUnits;

fn table(edit: impl FnOnce(&mut BalanceSpec)) -> Arc<BalanceTable> {
    let mut spec = BalanceTable::builtin().spec().clone();
    edit(&mut spec);
    Arc::new(BalanceTable::from_spec(spec).expect("検証を通る表"))
}

fn game(seed: u64, balance: Arc<BalanceTable>) -> Game {
    Game::new(GameConfig { seed, balance })
}

fn sid(i: u32) -> StationId {
    StationId(i)
}

fn create(stations: &[u32]) -> Command {
    Command::CreateLine {
        stations: stations.iter().map(|&i| sid(i)).collect(),
        bends: vec![Bend::StraightFirst; stations.len() - 1],
    }
}

fn level_up(param: ParamKey, scope: UpgradeScope) -> Command {
    Command::LevelUp {
        param,
        scope,
        mode: BuyMode::One,
    }
}

fn run(g: &mut Game, ticks: u64) -> Vec<GameEvent> {
    let mut events = Vec::new();
    for _ in 0..ticks {
        g.step(&mut events);
    }
    events
}

/// 路線・列車・乗客・強化のある状態(所持金の多い表)。
fn busy() -> Game {
    let mut g = game(7, table(|s| s.economy.initial_money_yen = 1_000_000_000));
    g.enqueue(create(&[0, 1, 2]));
    run(&mut g, 1);
    for p in [
        ParamKey::S1Spawn,
        ParamKey::T4CarCapacity,
        ParamKey::T1Speed,
    ] {
        let scope = match p {
            ParamKey::S1Spawn => UpgradeScope::AllStations,
            _ => UpgradeScope::AllLines,
        };
        g.enqueue(Command::LevelUp {
            param: p,
            scope,
            mode: BuyMode::Max,
        });
    }
    run(&mut g, 700);
    assert!(
        g.state.trains.iter().any(|t| t.riders.0 > 0),
        "乗客が乗っている"
    );
    assert!(g.state.stations.iter().any(|s| s.waiting_total.0 > 0));
    g
}

#[test]
fn new_builds_the_initial_state() {
    let bal = Arc::new(BalanceTable::builtin());
    let g = game(42, Arc::clone(&bal));
    let st = &g.state;
    assert_eq!((g.now(), g.next_tick()), (Tick(0), Tick(1)));
    assert_eq!(st.money, bal.initial_money());
    assert_eq!(st.station_plan.len(), 27);
    assert_eq!(st.stations.len(), 3);
    for (i, s) in st.stations.iter().enumerate() {
        assert_eq!(
            (s.id.index(), s.pos, s.spawned_at),
            (i, st.station_plan[i], Tick(0))
        );
    }
    assert!(st.lines.is_empty() && st.trains.is_empty());
    assert_eq!(
        st.progress.unlocked,
        sim::unlocked_at(&bal, CompanyLevel(0))
    );
    assert!(st.progress.unlocked.contains(Feature::LineExtend));
    assert!(!st.progress.unlocked.contains(Feature::Transfer));
    assert_eq!(st.progress.max_money_unit, 0);
    assert!(!st.routing.is_dirty());
    assert_eq!(st.routing.check_up_to_date(st, &bal), Ok(()));
    assert_eq!(st.waiting.rows(), 3);
    assert!(g.pending().is_empty());
    g.check_invariants().unwrap();
}

#[test]
fn new_uses_the_station_plan_of_the_seed() {
    let bal = Arc::new(BalanceTable::builtin());
    let (a, b, c) = (
        game(1, Arc::clone(&bal)),
        game(1, Arc::clone(&bal)),
        game(2, Arc::clone(&bal)),
    );
    let spec = bal.spec();
    assert_eq!(
        a.state.station_plan,
        plan_stations(1, &spec.map, &spec.company, 27)
    );
    assert_eq!(a.state_hash(), b.state_hash());
    assert_ne!(a.state.station_plan, c.state.station_plan);
    assert_ne!(a.state_hash(), c.state_hash());
}

#[test]
fn initial_money_sets_the_money_unit_without_events() {
    let g = game(1, table(|s| s.economy.initial_money_yen = 123_456_789));
    assert_eq!(g.state.progress.max_money_unit, 2);
    g.check_invariants().unwrap();
}

#[test]
fn first_tick_event_order() {
    let mut g = game(3, Arc::new(BalanceTable::builtin()));
    g.enqueue(create(&[0, 1]));
    g.enqueue(create(&[1, 2]));
    assert_eq!(g.pending().len(), 2);
    let ev = run(&mut g, 1);
    assert!(g.pending().is_empty());
    assert_eq!(g.now(), Tick(1));
    let tiles = g.state.stations[0].pos.chebyshev(g.state.stations[1].pos) + 1;
    assert_eq!(
        ev[..4],
        [
            GameEvent::LineCreated {
                line: LineId(0),
                station_count: 2,
                cost: Money::ZERO,
                new_tiles: tiles
            },
            GameEvent::TrainAdded {
                train: TrainId(0),
                line: LineId(0)
            },
            GameEvent::CommandRejected {
                kind: crate::command::CommandKind::CreateLine,
                reason: CommandError::NoFreeLineSlot { used: 1, slots: 1 }
            },
            GameEvent::RoutesRecomputed { node_count: 8 },
        ]
    );
    // 置いた列車は同じ tick に最初の停車駅に着く(11 SIM-D-05)。
    assert!(matches!(
        ev[4],
        GameEvent::TrainArrived { train: TrainId(0), station, riders: People(0), .. } if station == sid(0)
    ));
    assert_eq!(ev.len(), 5);
    assert!(matches!(
        g.state.trains[0].phase,
        TrainPhase::Dwelling {
            at: 0,
            remaining: Ticks(59)
        }
    ));
}

#[test]
fn recompute_happens_once_after_all_commands() {
    let mut g = game(3, table(|s| s.economy.initial_money_yen = 1_000_000));
    g.enqueue(create(&[0, 1]));
    g.enqueue(level_up(ParamKey::T1Speed, UpgradeScope::Line(LineId(0))));
    g.enqueue(Command::ExtendLine {
        line: LineId(0),
        end: LineEnd::Tail,
        station: sid(2),
        bend: Bend::DiagonalFirst,
    });
    let ev = run(&mut g, 1);
    let recomputed: Vec<usize> = ev
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, GameEvent::RoutesRecomputed { .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(recomputed.len(), 1);
    let last_cmd = ev
        .iter()
        .rposition(|e| matches!(e, GameEvent::LineExtended { .. }))
        .unwrap();
    let first_arrival = ev
        .iter()
        .position(|e| matches!(e, GameEvent::TrainArrived { .. }))
        .unwrap();
    assert!(last_cmd < recomputed[0] && recomputed[0] < first_arrival);
    assert!(run(&mut g, 1).is_empty(), "印がなければ再計算しない");
}

#[test]
fn passengers_spawn_on_second_boundaries_after_boarding() {
    let mut g = game(3, Arc::new(BalanceTable::builtin()));
    g.enqueue(create(&[0, 1]));
    run(&mut g, 119);
    assert_eq!(g.state.rng.passenger_dest.words(), 0);
    assert_eq!(g.state.stations[0].spawn_acc, MilliPeople(500));
    assert!(g.state.stations.iter().all(|s| s.waiting_total.0 == 0));
    run(&mut g, 1);
    assert_eq!(g.now(), Tick(120));
    let waiting: Vec<u64> = g.state.stations.iter().map(|s| s.waiting_total.0).collect();
    assert_eq!(waiting, [1, 1, 0], "路線に入っていない駅は発生しない");
    assert_eq!(g.state.rng.passenger_dest.words(), 2);
    assert_eq!(g.state.stats.income_window[1], Money::ZERO);
    assert_eq!(g.state.stats.window_head, 2);
}

#[test]
fn milestones_follow_arrivals_in_the_same_tick() {
    let bal = table(|s| {
        s.params[1].value = crate::balance::ValueFormula::Geometric {
            v0: 100_000,
            ratio: 10,
        }
    });
    let mut g = game(3, bal);
    g.enqueue(create(&[0, 1]));
    let mut events = Vec::new();
    while !events
        .iter()
        .any(|e| matches!(e, GameEvent::MoneyUnitReached { .. }))
    {
        events.clear();
        g.step(&mut events);
        assert!(g.now().0 < 5000);
    }
    let unit = events
        .iter()
        .position(|e| matches!(e, GameEvent::MoneyUnitReached { unit: 1 }))
        .unwrap();
    let arrived = events
        .iter()
        .rposition(|e| matches!(e, GameEvent::TrainArrived { income, .. } if !income.is_zero()))
        .unwrap();
    assert!(arrived < unit);
}

#[test]
fn clone_continues_identically() {
    let mut a = busy();
    let mut b = a.clone();
    let (ea, eb) = (run(&mut a, 500), run(&mut b, 500));
    assert_eq!(ea, eb);
    assert_eq!(a.state, b.state);
    assert_eq!(a.state_hash(), b.state_hash());
}

type Edit = fn(&mut GameState);

fn hashed_fields() -> Vec<(&'static str, Edit)> {
    vec![
        ("tick", |s| s.tick.0 += 1),
        ("money", |s| s.money = s.money.saturating_add(Money::ONE)),
        ("total_earned", |s| {
            s.progress.total_earned = s.progress.total_earned.saturating_add(Money::ONE)
        }),
        ("company.level", |s| s.company.level.0 += 1),
        ("company.line_slots", |s| s.company.line_slots.0 += 1),
        ("company.track_tiles", |s| s.company.track_tiles += 1),
        ("station_plan.x", |s| s.station_plan[18].x += 1),
        ("station_plan.y", |s| s.station_plan[18].y += 1),
        ("station_plan.len", |s| {
            s.station_plan.push(GridPos { x: 0, y: 0 })
        }),
        ("station.id", |s| s.stations[1].id.0 += 7),
        ("station.spawned_at", |s| s.stations[1].spawned_at.0 += 1),
        ("station.spawn", |s| s.stations[1].levels.spawn.0 += 1),
        ("station.fare", |s| s.stations[1].levels.fare.0 += 1),
        ("station.capacity", |s| s.stations[1].levels.capacity.0 += 1),
        ("station.platform", |s| s.stations[1].levels.platform.0 += 1),
        ("station.spawn_acc", |s| s.stations[1].spawn_acc.0 += 1),
        ("line.id", |s| s.lines[0].id.0 += 1),
        ("line.created_at", |s| s.lines[0].created_at.0 += 1),
        ("line.stations", |s| s.lines[0].stations.swap(0, 1)),
        ("line.bend", |s| {
            let b = &mut s.lines[0].segments[1].bend;
            *b = b.reversed()
        }),
        ("line.speed", |s| s.lines[0].levels.speed.0 += 1),
        ("line.crowding", |s| s.lines[0].levels.crowding.0 += 1),
        ("line.boarding", |s| s.lines[0].levels.boarding.0 += 1),
        ("line.car_capacity", |s| {
            s.lines[0].levels.car_capacity.0 += 1
        }),
        ("line.cars", |s| s.lines[0].levels.cars.0 += 1),
        ("line.max_trains", |s| s.lines[0].levels.max_trains.0 += 1),
        ("line.attractiveness", |s| {
            s.lines[0].levels.attractiveness.0 += 1
        }),
        ("line.trains", |s| s.lines[0].trains.push(TrainId(9))),
        ("train.id", |s| s.trains[0].id.0 += 5),
        ("train.line", |s| s.trains[0].line.0 += 1),
        ("train.section", |s| s.trains[0].section += 1),
        ("train.dir", |s| {
            s.trains[0].dir = match s.trains[0].dir {
                Direction::Forward => Direction::Backward,
                Direction::Backward => Direction::Forward,
            }
        }),
        ("train.pos", |s| {
            s.trains[0].pos = PosUnits(s.trains[0].pos.0 + 1)
        }),
        ("train.phase", |s| {
            s.trains[0].phase = match s.trains[0].phase {
                TrainPhase::Running => TrainPhase::Dwelling {
                    at: 0,
                    remaining: Ticks(0),
                },
                TrainPhase::Dwelling { .. } => TrainPhase::Running,
            }
        }),
        ("waiting", |s| s.waiting.add(0, sid(2), People(1))),
        ("waiting.rows", |s| s.waiting.push_row()),
        ("onboard", |s| s.onboard.add(0, sid(2), People(1))),
        ("onboard.rows", |s| s.onboard.push_row()),
        ("unlocked", |s| {
            s.progress.unlocked.insert(Feature::Transfer)
        }),
        ("upgraded_once", |s| {
            s.progress.upgraded_once.insert(ParamKey::R3Attractiveness)
        }),
        ("max_money_unit", |s| s.progress.max_money_unit += 1),
        ("goal_reached", |s| s.progress.goal_reached = true),
        ("income_this_sec", |s| {
            s.stats.income_this_sec = s.stats.income_this_sec.saturating_add(Money::ONE)
        }),
        ("income_window", |s| {
            s.stats.income_window[9] = s.stats.income_window[9].saturating_add(Money::ONE)
        }),
        ("window_head", |s| s.stats.window_head += 1),
        ("arrivals", |s| s.stats.arrivals += 1),
        ("delivered", |s| s.stats.delivered += 1 << 70),
        ("purchases", |s| s.stats.purchases += 1),
        ("rng.words", |s| {
            s.rng.passenger_dest.next_u32();
        }),
        ("rng.stream", |s| {
            s.rng.passenger_dest = crate::rng::StreamRng::new(7, 9)
        }),
        ("map_revision", |s| s.map_revision += 1),
    ]
}

#[test]
fn hash_covers_every_listed_field() {
    let g = busy();
    let base = g.state_hash();
    for (name, edit) in hashed_fields() {
        let mut st = g.state.clone();
        edit(&mut st);
        assert_ne!(state_hash(&st), base, "{name} が state_hash に入っていない");
    }
}

#[test]
fn hash_ignores_derived_data_and_work_areas() {
    let mut g = busy();
    let base = g.state_hash();
    let edits: Vec<(&str, Edit)> = vec![
        ("tracks", |s| s.tracks.masks[0].0 ^= 0xff),
        ("stops", |s| s.lines[0].stops[0] = !s.lines[0].stops[0]),
        ("sections", |s| s.lines[0].sections.clear()),
        ("offsets", |s| s.lines[0].offsets[1] += 1),
        ("serving_lines", |s| s.stations[0].serving_lines.clear()),
        ("waiting_total", |s| s.stations[0].waiting_total.0 += 1),
        ("riders", |s| s.trains[0].riders.0 += 1),
        ("routing", |s| s.routing.mark_dirty()),
        ("segment.tiles", |s| s.lines[0].segments[0].tiles.clear()),
    ];
    for (name, edit) in edits {
        let mut st = g.state.clone();
        edit(&mut st);
        assert_eq!(state_hash(&st), base, "{name} は state_hash に入れない");
    }
    g.enqueue(create(&[0, 2]));
    g.scratch = SimScratch::default();
    #[cfg(feature = "test-support")]
    {
        g.state.ledger.spawned += 1;
    }
    assert_eq!(g.state_hash(), base, "キュー・作業領域・台帳は入れない");
}

/// 10 §9.2 の表を写した参照の実装(本体の書く順の入れ替えを検出する)。
fn reference_hash(st: &GameState) -> u64 {
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
    st.station_plan.iter().for_each(|p| {
        h.write_i32(p.x);
        h.write_i32(p.y)
    });
    h.write_len(st.stations.len());
    for s in &st.stations {
        h.write_u32(s.id.0);
        h.write_u64(s.spawned_at.0);
        h.write_u8(s.levels.spawn.0);
        h.write_u8(s.levels.fare.0);
        h.write_u8(s.levels.capacity.0);
        h.write_u8(s.levels.platform.0);
        h.write_u64(s.spawn_acc.0);
    }
    h.write_len(st.lines.len());
    for l in &st.lines {
        h.write_u32(l.id.0);
        h.write_u64(l.created_at.0);
        h.write_u8(l.born_level.0);
        h.write_len(l.stations.len());
        l.stations.iter().for_each(|s| h.write_u32(s.0));
        h.write_len(l.segments.len());
        l.segments
            .iter()
            .for_each(|g| h.write_u8(u8::from(g.bend == Bend::DiagonalFirst)));
        let v = l.levels;
        h.write_u8(v.speed.0);
        h.write_u8(v.crowding.0);
        h.write_u8(v.boarding.0);
        h.write_u8(v.car_capacity.0);
        h.write_u8(v.cars.0);
        h.write_u8(v.max_trains.0);
        h.write_u8(v.attractiveness.0);
        h.write_len(l.trains.len());
        l.trains.iter().for_each(|t| h.write_u32(t.0));
    }
    h.write_len(st.trains.len());
    for t in &st.trains {
        h.write_u32(t.id.0);
        h.write_u32(t.line.0);
        h.write_u16(t.section);
        h.write_u8(u8::from(t.dir == Direction::Backward));
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
    for m in [&st.waiting, &st.onboard] {
        h.write_u32(m.rows());
        h.write_u32(m.cols());
        m.as_slice().iter().for_each(|&v| h.write_u64(v));
    }
    h.write_u32(st.progress.unlocked.0);
    h.write_u16(st.progress.upgraded_once.0);
    h.write_u8(st.progress.max_money_unit);
    h.write_bool(st.progress.goal_reached);
    h.write_u32(st.rebirth.count);
    h.write_u64(st.rebirth.points);
    h.write_u8(st.rebirth.fare_boost);
    h.write_u8(st.rebirth.train_boost);
    h.write_u8(st.rebirth.line_boost);
    h.write_money(st.stats.income_this_sec);
    st.stats
        .income_window
        .iter()
        .for_each(|&m| h.write_money(m));
    h.write_u8(st.stats.window_head);
    h.write_u64(st.stats.arrivals);
    h.write_u128(st.stats.delivered);
    h.write_u32(st.stats.purchases);
    h.write_u64(st.rng.passenger_dest.stream());
    h.write_u64(st.rng.passenger_dest.words());
    h.write_u64(st.map_revision);
    h.finish()
}

#[test]
fn hash_follows_the_specified_order() {
    let mut g = busy();
    assert_eq!(g.state_hash(), reference_hash(&g.state));
    run(&mut g, 300);
    assert_eq!(g.state_hash(), reference_hash(&g.state));
}

/// ゴールデン(16 §2.7)。書く順・ルール・表を変えたら更新する。
#[test]
fn hash_golden() {
    let g = game(1, Arc::new(BalanceTable::builtin()));
    assert_eq!(
        g.state_hash(),
        INITIAL_HASH_GOLDEN,
        "{:#018x}",
        g.state_hash()
    );
    let b = busy();
    assert_eq!(b.state_hash(), BUSY_HASH_GOLDEN, "{:#018x}", b.state_hash());
}

const INITIAL_HASH_GOLDEN: u64 = 0xa6d6_364d_1245_59c8;
const BUSY_HASH_GOLDEN: u64 = 0x5baf_2bd7_2fa6_b0a8;

#[test]
fn unlocked_matches_company_level_zero() {
    let g = game(5, Arc::new(BalanceTable::builtin()));
    let want = [
        Feature::Param(ParamKey::S1Spawn),
        Feature::Param(ParamKey::S2Fare),
        Feature::Param(ParamKey::T1Speed),
        Feature::Param(ParamKey::T4CarCapacity),
        Feature::LineExtend,
        Feature::BulkBuy,
    ];
    let mut set = FeatureSet::default();
    want.iter().for_each(|&f| set.insert(f));
    assert_eq!(g.state.progress.unlocked, set);
}

#[cfg(feature = "test-support")]
#[test]
fn ledger_is_conserved_and_readable() {
    let mut g = busy();
    let l = g.pax_ledger();
    assert!(l.spawned > 0 && l.is_conserved(), "{l:?}");
    run(&mut g, 1200);
    let l2 = g.pax_ledger();
    assert!(l2.is_conserved() && l2.spawned > l.spawned, "{l2:?}");
    assert_eq!(l2.delivered, g.state.stats.delivered);
}

#[cfg(feature = "test-support")]
#[test]
fn with_station_plan_uses_the_given_positions() {
    let bal = Arc::new(BalanceTable::builtin());
    let plan: Vec<GridPos> = (0..27)
        .map(|i| GridPos {
            x: 1 + i,
            y: 1 + i % 5,
        })
        .collect();
    let g = Game::with_station_plan(
        GameConfig {
            seed: 1,
            balance: bal,
        },
        plan.clone(),
    );
    assert_eq!(g.state.station_plan, plan);
    assert_eq!(g.view().station(sid(2)).unwrap().pos, plan[2]);
}

#[cfg(feature = "test-support")]
#[test]
fn bench_hooks_keep_invariants() {
    let mut g = busy();
    g.put_all_trains_before_arrival();
    g.check_invariants().unwrap();
    let n = g.state.trains.len();
    let ev = run(&mut g, 1);
    let arrivals = ev
        .iter()
        .filter(|e| matches!(e, GameEvent::TrainArrived { .. }))
        .count();
    assert_eq!(arrivals, n);
    g.mark_routes_dirty();
    assert!(
        run(&mut g, 1)
            .iter()
            .any(|e| matches!(e, GameEvent::RoutesRecomputed { .. }))
    );
}
