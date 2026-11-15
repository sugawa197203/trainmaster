//! 毎回の検査(`test:nosdl`): 1 シード 40 分の通し(far と near)、行動間隔・建設の時間・停止、推定器とおすすめの一致、
//! 線路のお金と「ためる」、放置、区切りごとの作り直しと `resume`・リプレイ v2、再現性。

use std::sync::Arc;

use tm_core::{
    BalanceTable, Command, CommandError, Game, GameConfig, GameEvent, GameSnapshot, GridPos,
    ParamKey, ParamOwner, ReplayLog, StateHasher, StationId, Tick, replay,
};
use tm_simrun::check::Checker;
use tm_simrun::flow::{self, money_f64};
use tm_simrun::save::read_snapshot;
use tm_simrun::scenario::{CREATE_S, EXTEND_S, SAVE_S, TPS, hash_event, record_until};
use tm_simrun::{
    Decision, GameWorld, Idle, Pause, Player, Route, Runner, Scenario, World, decide_route,
};

const SEED_A: u64 = 1;
const MIN: u64 = 60 * TPS;

fn builtin() -> Arc<BalanceTable> {
    Arc::new(BalanceTable::builtin())
}

fn is_build(c: &Command) -> bool {
    matches!(c, Command::CreateLine { .. } | Command::ExtendLine { .. })
}

/// 40 分の通し: 全機能が 30 分以内、拒否 0、おすすめと一致、台帳、線路のお金の集計、再生と再実行の一致。
fn smoke_40_minutes(route: Route) {
    let bal = builtin();
    let sc = Scenario::new(Player::B, Pause::STANDARD).with_route(route);
    let mut r = Runner::new(SEED_A, Arc::clone(&bal), sc.clone());
    r.record();
    let mut checker = Checker::new(r.game());
    let (mut tiles, mut spent, mut builds) = (0u32, tm_core::Money::ZERO, 0usize);
    while r.wall() < 40 * MIN {
        r.tick();
        for ev in r.events() {
            if let GameEvent::LineCreated {
                cost, new_tiles, ..
            }
            | GameEvent::LineExtended {
                cost, new_tiles, ..
            } = *ev
            {
                tiles += new_tiles;
                spent = spent.saturating_add(cost);
                builds += 1;
            }
        }
        if r.game().now().is_second_boundary() && r.pause_left() == 0 {
            checker
                .check(r.game())
                .unwrap_or_else(|e| panic!("wall {}: {e}", r.wall()));
        }
    }
    let report = r.report();
    let row = report.csv_row();
    let g = report.game.as_ref().expect("実際のゲーム");
    assert!(
        report
            .milestones
            .all_features
            .is_some_and(|t| t <= 30 * MIN),
        "{row}"
    );
    assert!(report.milestones.all_params.is_some());
    assert_eq!(g.rejected, 0);
    assert!(g.rec_checked > 50, "{}", g.rec_checked);
    assert_eq!(g.rec_mismatch, 0, "{:?}", g.first_mismatch);
    assert!(g.ledger.is_conserved());
    assert!(g.routes_recomputed > 0);
    assert_eq!(report.route, route);
    assert_eq!((report.track_tiles, report.builds.len()), (tiles, builds));
    assert_eq!(report.track_spent.to_bits(), money_f64(spent).to_bits());
    assert!(spent.total_cmp(&tm_core::Money::ZERO).is_gt(), "{row}");
    assert!(report.builds.iter().all(|b| b.level >= 1 || b.cost == 0.0));
    assert!(
        report
            .builds
            .iter()
            .filter(|b| b.cost > 0.0)
            .all(|b| b.income > 0.0)
    );

    let rec = r.recording().expect("記録した");
    let end = r.game().now();
    assert_eq!(rec.log.checkpoints.len() as u64, end.0 / 600);
    let replayed = replay(&rec.log, Arc::clone(&bal), end, &mut Vec::new()).expect("再生できる");
    assert_eq!(replayed.state_hash(), r.game().state_hash());

    let mut again = Runner::new(SEED_A, bal, sc);
    while again.wall() < 40 * MIN {
        again.tick();
    }
    assert_eq!(again.game().state_hash(), r.game().state_hash());
    assert_eq!(again.actions(), r.actions());
}

#[test]
fn smoke_40_minutes_player_b_standard() {
    smoke_40_minutes(Route::Far);
}

#[test]
fn smoke_40_minutes_player_b_standard_near() {
    smoke_40_minutes(Route::Near);
}

#[test]
fn near_and_far_connect_new_stations_differently() {
    let bal = builtin();
    let lines = |route| {
        let sc = Scenario::new(Player::B, Pause::NONE).with_route(route);
        let mut r = Runner::new(SEED_A, Arc::clone(&bal), sc);
        while r.wall() < 30 * MIN {
            r.tick();
        }
        let created: Vec<(StationId, StationId)> = r
            .actions()
            .iter()
            .filter_map(|a| match &a.cmd {
                Command::CreateLine { stations, .. } => Some((stations[0], stations[1])),
                _ => None,
            })
            .collect();
        (created, r.report().track_tiles)
    };
    let (far, far_tiles) = lines(Route::Far);
    let (near, near_tiles) = lines(Route::Near);
    assert!(far.len() >= 2 && near.len() >= 2);
    assert!(
        far[1..].iter().all(|&(_, with)| with == StationId(0)),
        "{far:?}"
    );
    assert_ne!(far, near);
    assert!(near_tiles < far_tiles, "{near_tiles} / {far_tiles}");
}

#[test]
fn game_world_prices_builds_like_validate() {
    let bal = builtin();
    let mut checked = [0u32; 2];
    for (i, route) in Route::ALL.into_iter().enumerate() {
        let sc = Scenario::new(Player::A, Pause::NONE).with_route(route);
        let mut r = Runner::new(SEED_A, Arc::clone(&bal), sc);
        while r.wall() < 30 * MIN {
            r.tick();
            if !r.wall().is_multiple_of(5 * TPS) {
                continue;
            }
            let v = r.game().view();
            let w = GameWorld::new(v);
            assert_eq!(
                w.earning().to_bits(),
                money_f64(v.income_per_sec()).to_bits()
            );
            let d = decide_route(Player::A, route, &w);
            let Some(cmd) = w.command(d).filter(|_| d.is_build()) else {
                assert_eq!(w.build_price(d), None);
                continue;
            };
            let want = match v.validate(&cmd) {
                Ok(p) => Some(money_f64(p.cost)),
                Err(CommandError::InsufficientFunds { required, .. }) => Some(money_f64(required)),
                Err(_) => None,
            };
            assert_eq!(w.build_price(d), want, "{cmd:?}");
            let saved = tm_simrun::decide_with(Player::A, route, SAVE_S, &w);
            if let Some(price) = want {
                let short = price - w.money();
                let expect_save = short > 0.0 && short <= w.earning() * f64::from(SAVE_S);
                assert_eq!(saved == Decision::Save, expect_save, "{cmd:?}");
                checked[i] += u32::from(short > 0.0);
            }
        }
    }
    assert!(checked.iter().all(|&n| n > 0), "{checked:?}");
}

#[test]
fn save_s_zero_never_waits_for_track() {
    let bal = builtin();
    let mut sc = Scenario::new(Player::B, Pause::NONE);
    let saving = tm_simrun::run(SEED_A, &bal, sc.clone(), 40);
    sc.save_s = 0;
    let eager = tm_simrun::run(SEED_A, &bal, sc, 40);
    assert!(saving.saved_s > 0, "{}", saving.csv_row());
    assert_eq!(eager.saved_s, 0);
    assert_eq!(saving.saved_s % u64::from(Player::B.interval_s()), 0);
}

#[test]
fn actions_follow_interval_and_build_times() {
    let bal = builtin();
    for pause in [Pause::NONE, Pause::STANDARD] {
        for player in Player::ALL {
            let mut r = Runner::new(SEED_A, Arc::clone(&bal), Scenario::new(player, pause));
            let mut created = Vec::new();
            let mut extended = Vec::new();
            while r.wall() < 10 * MIN {
                r.tick();
                for ev in r.events() {
                    match ev {
                        GameEvent::LineCreated { .. } => created.push(r.wall()),
                        GameEvent::LineExtended { .. } => extended.push(r.wall()),
                        _ => {}
                    }
                }
            }
            let ctx = format!("{player:?} {pause:?}");
            let iv = u64::from(player.interval_s()) * TPS;
            let start = u64::from(pause.start_s) * TPS;
            let acts = r.actions();
            assert!(acts.len() > 5, "{ctx}");
            assert_eq!(created[0], start + u64::from(CREATE_S) * TPS, "{ctx}");
            assert_eq!(
                extended[0],
                start + u64::from(CREATE_S + EXTEND_S) * TPS,
                "{ctx}"
            );
            for (i, a) in acts.iter().enumerate() {
                if i >= 2 {
                    assert_eq!(a.decided_wall % iv, 0, "{ctx} {a:?}");
                }
                let wait = match &a.cmd {
                    Command::CreateLine { .. } => u64::from(CREATE_S) * TPS,
                    Command::ExtendLine { .. } => u64::from(EXTEND_S) * TPS,
                    Command::LevelUp { .. } | Command::Rebirth | Command::RebirthUpgrade { .. } => {
                        1
                    }
                };
                assert_eq!(a.tick.0 - a.decided_tick.0, wait, "{ctx} {a:?}");
                if pause == Pause::NONE {
                    let wall_wait = if is_build(&a.cmd) { wait } else { 0 };
                    assert_eq!(a.wall - a.decided_wall, wall_wait, "{ctx} {a:?}");
                }
                let busy = a.decided_wall + 1..a.wall;
                assert!(
                    !acts.iter().any(|b| busy.contains(&b.decided_wall)),
                    "{ctx} {a:?}"
                );
            }
            if pause != Pause::NONE {
                assert!(
                    acts.iter().skip(2).any(|a| a.decided_tick.0 % iv != 0),
                    "停止の分だけゲームの時刻はずれる: {ctx}"
                );
            }
            assert_eq!(
                created.len() + extended.len(),
                acts.iter().filter(|a| is_build(&a.cmd)).count(),
                "{ctx}"
            );
        }
    }
}

#[test]
fn tutorial_pause_stops_the_game() {
    let pause = Pause::STANDARD;
    let unlock = u64::from(pause.unlock_s) * TPS;
    let mut r = Runner::new(SEED_A, builtin(), Scenario::new(Player::B, pause));
    let start = u64::from(pause.start_s) * TPS;
    while r.wall() < start {
        r.tick();
        assert_eq!(r.game().now(), Tick(0));
    }
    let mut created = None;
    let mut levels = Vec::new();
    while !levels.contains(&5) {
        let (paused, now) = (r.pause_left() > 0, r.game().now());
        r.tick();
        if paused {
            assert_eq!(r.game().now(), now, "停止中は進まない");
        } else {
            assert_eq!(r.game().now(), Tick(now.0 + 1));
        }
        let mut stops = 0;
        for ev in r.events() {
            match *ev {
                GameEvent::LineCreated { .. } => {
                    created.get_or_insert(r.wall());
                }
                GameEvent::CompanyLevelUp { new_level, .. } => {
                    levels.push(new_level.0);
                    stops += u64::from(new_level.0 <= 4);
                }
                _ => {}
            }
        }
        if !r.events().is_empty() {
            assert_eq!(r.pause_left(), stops * unlock, "Lv {levels:?}");
        }
        assert!(r.wall() < 40 * MIN);
    }
    assert_eq!(created, Some(start + u64::from(CREATE_S) * TPS));
    assert_eq!(levels, [1, 2, 3, 4, 5]);
}

#[test]
fn players_b_c_agree_with_recommendation() {
    let bal = builtin();
    for player in [Player::B, Player::C] {
        let report = tm_simrun::run(SEED_A, &bal, Scenario::new(player, Pause::NONE), 20);
        let g = report.game.expect("実際のゲーム");
        assert!(g.rec_checked > 50, "{player:?}: {}", g.rec_checked);
        assert_eq!(g.rec_mismatch, 0, "{player:?}: {:?}", g.first_mismatch);
    }
}

#[test]
fn targets_match_validate_cost() {
    let bal = builtin();
    let mut r = Runner::new(SEED_A, bal, Scenario::new(Player::B, Pause::NONE));
    let mut checked = 0;
    while r.wall() < 15 * MIN {
        r.tick();
        if !r.wall().is_multiple_of(30 * TPS) {
            continue;
        }
        let v = r.game().view();
        let w = GameWorld::new(v);
        for p in ParamKey::ALL
            .into_iter()
            .filter(|p| p.owner() != ParamOwner::Company)
        {
            if let Some(step) = w.step_cost(p) {
                assert_eq!(flow::targets(&v, p).step.to_bits(), step.to_bits(), "{p:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 100);
}

fn line_6_tiles_apart() -> Game {
    let bal = builtin();
    let n = bal.max_stations() as i32;
    let mut plan = vec![
        GridPos { x: 2, y: 2 },
        GridPos { x: 8, y: 2 },
        GridPos { x: 14, y: 2 },
    ];
    plan.extend((0..n - 3).map(|i| GridPos {
        x: 2 + (i % 8) * 3,
        y: 8 + (i / 8) * 3,
    }));
    let mut g = Game::with_station_plan(
        GameConfig {
            seed: 7,
            balance: bal,
        },
        plan,
    );
    g.enqueue(Command::CreateLine {
        stations: vec![StationId(0), StationId(1), StationId(2)],
        bends: vec![tm_core::Bend::StraightFirst; 2],
    });
    g.step(&mut Vec::new());
    g
}

#[test]
fn view_estimate_is_hand_value_on_6_tile_line() {
    let g = line_6_tiles_apart();
    let v = g.view();
    let base = flow::estimate_view(&v, None);
    assert!((base - 5.0 / 7.0).abs() <= 1e-12, "{base}");
    let w = GameWorld::new(v);
    assert_eq!(w.income(None).to_bits(), base.to_bits());
    let fare = flow::targets(&v, ParamKey::S2Fare);
    assert_eq!(fare.stations, vec![true, true, true]);
    assert_eq!(fare.step, 15.0);
    assert!((w.income(Some(ParamKey::S2Fare)) - 50.0 / 7.0).abs() <= 1e-12);
    assert_eq!(money_f64(v.money()), 0.0);
}

/// 600 tick ごとの (tick, 値)。
type Series = Vec<(Tick, u64)>;

/// tm-core の `tests/common` の `play` と同じ手順で、記録したログを独立に再生する(state_hash とイベント列のハッシュ)。
fn play(log: &ReplayLog, bal: &Arc<BalanceTable>, until: u64) -> (Series, Series) {
    let mut g = Game::new(GameConfig {
        seed: log.header.seed,
        balance: Arc::clone(bal),
    });
    let (mut states, mut hashes) = (Vec::new(), Vec::new());
    let mut events = Vec::new();
    let mut eh = StateHasher::new();
    let mut entries = log.entries.iter().peekable();
    while g.now().0 < until {
        let t = g.next_tick();
        while let Some(e) = entries.next_if(|e| e.tick == t) {
            g.enqueue(e.cmd.clone());
        }
        events.clear();
        g.step(&mut events);
        events.iter().for_each(|e| hash_event(&mut eh, e));
        if g.now().0.is_multiple_of(600) {
            states.push((g.now(), g.state_hash()));
            hashes.push((g.now(), eh.finish()));
            eh = StateHasher::new();
        }
    }
    (states, hashes)
}

#[test]
fn recording_can_replace_scenario_2_golden() {
    let bal = builtin();
    let ticks = 10 * MIN;
    let r = record_until(
        2,
        Arc::clone(&bal),
        Scenario::new(Player::B, Pause::NONE),
        ticks,
    );
    let rec = r.recording().expect("記録した");
    let log = ReplayLog::from_ron(&rec.ron_text()).expect("RON を読める");
    assert_eq!(log, rec.log);
    assert_eq!(log.checkpoints.len() as u64, ticks / 600);
    let (states, hashes) = play(&log, &bal, ticks);
    let want: Vec<(Tick, u64)> = log
        .checkpoints
        .iter()
        .map(|c| (c.tick, c.state_hash))
        .collect();
    assert_eq!(states, want);
    assert_eq!(hashes, rec.event_hashes);
    let text = rec.events_text();
    assert_eq!(text.lines().count(), hashes.len());
    assert_eq!(
        text.lines().next(),
        Some(format!("600 {:016x}", hashes[0].1).as_str())
    );
    assert!(
        log.entries
            .iter()
            .any(|e| matches!(e.cmd, Command::LevelUp { .. }))
    );
    let golden = |ext: &str| {
        let path = format!(
            "{}/../../crates/tm-core/tests/golden/scenario_recommend.{ext}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    };
    assert_eq!(
        rec.ron_text(),
        golden("ron"),
        "tm-core の recommend のゴールデン"
    );
    assert_eq!(text, golden("events"));
}

/// ゴールだけを近くし、線路をただにした表(1 週間のゴールまでは毎回の検査で回さない)。
fn near_goal() -> Arc<BalanceTable> {
    let mut spec = BalanceTable::builtin().spec().clone();
    spec.economy.goal_pow10 = 24;
    spec.economy.track.free_below_level = spec.company.max_level;
    Arc::new(BalanceTable::from_spec(spec).expect("検証を通る表"))
}

#[test]
fn milestones_match_events() {
    let bal = near_goal();
    let mut levels = vec![None; usize::from(bal.spec().company.max_level) + 1];
    let mut r = Runner::new(SEED_A, bal, Scenario::new(Player::A, Pause::STANDARD));
    let mut first_buy = [None; 12];
    let mut goal = None;
    while r.wall() < 20 * MIN {
        r.tick();
        for ev in r.events() {
            match *ev {
                GameEvent::CompanyLevelUp { new_level, .. } => {
                    levels[usize::from(new_level.0)].get_or_insert(r.wall());
                }
                GameEvent::Purchased { param, .. } => {
                    first_buy[param.index()].get_or_insert(r.wall());
                }
                GameEvent::GoalReached { .. } => goal = Some(r.wall()),
                _ => {}
            }
        }
    }
    let m = r.milestones();
    assert_eq!(m.level, levels);
    assert_eq!(m.all_features, levels[4]);
    let all: Option<Vec<u64>> = first_buy.iter().copied().collect();
    assert_eq!(m.all_params, all.and_then(|v| v.into_iter().max()));
    assert!(m.all_params.is_some());
    assert_eq!(m.goal, goal);
    assert!(goal.is_some());
    let mut sc = Scenario::new(Player::A, Pause::STANDARD);
    sc.stop_at_goal = true;
    let stopped = tm_simrun::run(SEED_A, &near_goal(), sc, 20);
    assert_eq!(Some(stopped.end_wall), goal);
}

/// `scenario` で `minutes` 分回した結果(壁時計の分で区切らない)。
fn runner_for(sc: Scenario, minutes: u64) -> Runner {
    let mut r = Runner::new(SEED_A, builtin(), sc);
    while r.wall() < minutes * MIN {
        r.tick();
    }
    r
}

#[test]
fn idle_window_stops_actions_and_company_growth() {
    let mut sc = Scenario::new(Player::B, Pause::STANDARD);
    sc.idle = vec![Idle {
        start_s: 10 * 60,
        end_s: 20 * 60,
    }];
    let r = runner_for(sc.clone(), 30);
    let (from, to) = (10 * MIN, 20 * MIN);
    assert!(
        r.actions()
            .iter()
            .all(|a| !(from..to).contains(&a.decided_wall))
    );
    assert!(r.actions().iter().any(|a| a.decided_wall >= to));
    let lv = |r: &Runner| r.milestones().level.clone();
    let grown: Vec<u64> = lv(&r)
        .into_iter()
        .flatten()
        .filter(|&t| t > from + MIN && t <= to)
        .collect();
    assert!(grown.is_empty(), "{grown:?}");

    sc.idle = vec![Idle::after(5 * 60)];
    let r = runner_for(sc, 60);
    let at_idle = lv(&r)
        .iter()
        .flatten()
        .filter(|&&t| t <= 5 * MIN + MIN)
        .count();
    assert_eq!(usize::from(r.report().end_level), at_idle);
    assert!(lv(&r).iter().flatten().all(|&t| t <= 5 * MIN + MIN));
}

#[test]
fn sessions_restore_without_changing_results() {
    let bal = builtin();
    let plain = Scenario::new(Player::B, Pause::STANDARD);
    let a = runner_for(plain.clone(), 40);
    let mut sc = plain;
    sc.session_s = Some(10 * 60);
    let mut b = Runner::new(SEED_A, Arc::clone(&bal), sc);
    b.record();
    while b.wall() < 40 * MIN {
        b.tick();
    }
    assert_eq!(b.restores(), 4);
    assert_eq!(b.game().state_hash(), a.game().state_hash());
    assert_eq!(b.actions(), a.actions());
    let (ra, rb) = (a.report(), b.report());
    assert!(ra.reach.is_empty());
    assert_eq!(
        rb.reach.iter().map(|x| x.wall).collect::<Vec<_>>(),
        [10, 20, 30, 40].map(|m| m * MIN)
    );
    for x in &rb.reach {
        let reached = ra
            .milestones
            .level
            .iter()
            .flatten()
            .filter(|&&t| t <= x.wall);
        assert_eq!(usize::from(x.level), reached.count(), "{x:?}");
        assert_eq!(x.stations, 3 + u32::from(x.level), "{x:?}");
    }
    let v = a.game().view();
    let last = rb.reach.last().expect("区切り");
    assert_eq!(
        (last.level, last.unit),
        (v.company().level.0, v.max_money_unit())
    );
    assert!(rb.reach.windows(2).all(|w| w[0].unit <= w[1].unit));
    let cut = |row: String| row.rsplit_once(',').expect("reach の列").0.to_string();
    assert_eq!(cut(rb.csv_row()), cut(ra.csv_row()));
    assert_eq!(rb.game, ra.game);
    let rec = b.recording().expect("記録した");
    let end = b.game().now();
    let replayed =
        replay(&rec.log, bal, end, &mut Vec::new()).expect("作り直しを挟んだ記録も再生できる");
    assert_eq!(replayed.state_hash(), b.game().state_hash());
}

/// (b)・停止なしで `ready` になった後の、`align` の倍数で建設中でない tick まで進めた元と、そこで取った写しから
/// 作り直した続き。写しの tick は行動を選ばない tick(行動間隔の倍数でない)にして、元と続きが同じ行動を選ぶようにする。
fn split_for_resume(ready: impl Fn(&Runner) -> bool, align: u64) -> (Runner, Runner, GameSnapshot) {
    let bal = builtin();
    let sc = Scenario::new(Player::B, Pause::NONE);
    let mut a = Runner::new(SEED_A, Arc::clone(&bal), sc.clone());
    let iv = u64::from(Player::B.interval_s()) * TPS;
    let mut snap: Option<GameSnapshot> = None;
    while snap.is_none() {
        let want = ready(&a);
        let mut taken = None;
        a.tick_observing(|g| {
            let t = g.now().0;
            if want && t.is_multiple_of(align) && !t.is_multiple_of(iv) {
                taken = Some(g.snapshot());
            }
        });
        if !a.is_building() {
            snap = taken;
        }
    }
    let snap = snap.expect("写し");
    assert_eq!(snap.tick, a.wall());
    let text = ron::to_string(&snap).expect("RON");
    let loaded = read_snapshot(&text).expect("読める");
    let game = Game::restore(bal, &loaded.snapshot).expect("作り直せる");
    (a, Runner::resume(game, sc), snap)
}

#[test]
fn resume_counts_unlocked_but_unbought_params_as_new() {
    let (mut a, mut b, snap) = split_for_resume(|r| r.milestones().all_features.is_some(), 1);
    assert_eq!(
        a.milestones().all_params,
        None,
        "写しの時点では未強化のパラメータがある"
    );
    while a.wall() < 20 * MIN {
        a.tick();
    }
    while b.wall() < 20 * MIN {
        b.tick();
    }
    let all = a.milestones().all_params;
    assert!(all.is_some_and(|t| t > snap.tick), "{all:?}");
    assert_eq!(b.milestones().all_params, all);
}

#[test]
fn resume_keeps_upgrades_bought_before_the_save() {
    let (mut a, mut b, snap) = split_for_resume(|r| r.game().now().0 >= 6 * MIN, 600);
    while a.wall() < 20 * MIN {
        a.tick();
    }
    while b.wall() < 20 * MIN {
        b.tick();
    }
    let all = a.milestones().all_params;
    assert!(all.is_some_and(|t| t > snap.tick), "{all:?}");
    assert_eq!(b.milestones().all_params, all);
}

#[test]
fn resume_continues_like_the_original_and_replays_v2() {
    let bal = builtin();
    let (mut a, mut b, snap) = split_for_resume(|r| r.game().now().0 >= 20 * MIN, 600);
    let at = snap.tick;
    b.record();
    assert_eq!(b.wall(), at);
    let end = at + 10 * MIN;
    while a.wall() < end {
        a.tick();
    }
    while b.wall() < end {
        b.tick();
    }
    assert_eq!(b.game().state_hash(), a.game().state_hash());
    let after: Vec<_> = a
        .actions()
        .iter()
        .filter(|x| x.decided_wall > at)
        .cloned()
        .collect();
    assert!(!after.is_empty());
    assert_eq!(b.actions(), after.as_slice());
    let m = b.report().milestones;
    assert!(
        m.level[..=usize::from(snap.company.level)]
            .iter()
            .all(Option::is_none)
    );
    assert!(a.milestones().all_params.is_some_and(|t| t < at));
    assert_eq!(m.all_params, None, "再開の前にそろっていた");

    let rec = b.recording().expect("記録した");
    let log = ReplayLog::from_ron(&rec.ron_text()).expect("RON を読める");
    assert_eq!(log.start.as_ref().map(|s| s.tick), Some(at));
    assert_eq!(log.checkpoints.len() as u64, 10 * MIN / 600);
    let replayed =
        replay(&log, bal, b.game().now(), &mut Vec::new()).expect("v2 のログを再生できる");
    assert_eq!(replayed.state_hash(), b.game().state_hash());
    while b.wall() < end + 50 * MIN {
        b.tick();
    }
    assert!(b.report().end_level > snap.company.level);
    assert_eq!(b.report().milestones.all_params, None);
}

#[test]
fn distance_is_chebyshev_and_equal_ends_extend_tail() {
    let bal = builtin();
    let n = bal.max_stations() as i32;
    let mut plan = vec![
        GridPos { x: 2, y: 2 },
        GridPos { x: 8, y: 9 },
        GridPos { x: 14, y: 2 },
    ];
    plan.extend((0..n - 3).map(|i| GridPos {
        x: 18 + (i % 6) * 3,
        y: 6 + (i / 6) * 3,
    }));
    let mut g = Game::with_station_plan(
        GameConfig {
            seed: 7,
            balance: bal,
        },
        plan,
    );
    g.enqueue(Command::CreateLine {
        stations: vec![StationId(0), StationId(2)],
        bends: vec![tm_core::Bend::StraightFirst],
    });
    g.step(&mut Vec::new());
    let v = g.view();
    let w = GameWorld::new(v);
    assert_eq!(w.distance(StationId(0), StationId(1)), 7);
    assert_eq!(w.distance(StationId(2), StationId(1)), 7);
    assert_eq!(w.distance(StationId(0), StationId(2)), 12);
    let Command::ExtendLine { end, .. } =
        tm_simrun::strategy::extend_nearer(&v, tm_core::LineId(0), StationId(1))
    else {
        panic!("延伸");
    };
    assert_eq!(end, tm_core::LineEnd::Tail, "端までの距離が同じなら Tail");
}
