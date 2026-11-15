//! 転生のテスト(T1)。解放条件、リセットの範囲、ポイントと恒久の強化の効果。

use std::sync::Arc;

use crate::{
    BalanceTable, Bend, BuyMode, Command, CommandError, CompanyLevel, Game, GameConfig, GameEvent,
    Level, LineId, Money, ParamKey, RebirthUpgrade, StationId, UpgradeScope,
};
use tm_numeric::ja_unit_index;

/// 早く転生に届く表: 全部の強化が 1 円、初期所持金 1000 円、ゴールは累計 1000 円、
/// 転生ポイントは基礎点 20。
fn table() -> Arc<BalanceTable> {
    let mut spec = BalanceTable::builtin().spec().clone();
    for p in &mut spec.params {
        p.base_yen = 1;
        p.cost_ratio = 1;
        p.cost_steps.clear();
        p.unlock = 0;
    }
    spec.economy.initial_money_yen = 1000;
    spec.economy.goal_pow10 = 3;
    spec.rebirth.points_base = 20;
    BalanceTable::from_spec(spec).map(Arc::new).expect("表")
}

fn game(seed: u64, bal: &Arc<BalanceTable>) -> Game {
    Game::new(GameConfig {
        seed,
        balance: Arc::clone(bal),
    })
}

fn two_stations(a: u32, b: u32) -> Command {
    Command::CreateLine {
        stations: vec![StationId(a), StationId(b)],
        bends: vec![Bend::StraightFirst],
    }
}

/// 最初の路線を作ってから S2 を MAX まで上げ、ゴール(累計 1000 円)に届くまで進める。
fn reach_goal(g: &mut Game) {
    g.enqueue(two_stations(0, 1));
    g.enqueue(Command::LevelUp {
        param: ParamKey::S2Fare,
        scope: UpgradeScope::AllStations,
        mode: BuyMode::Max,
    });
    let mut events = Vec::new();
    for _ in 0..7_200 {
        events.clear();
        g.step(&mut events);
        if g.view().goal_reached() {
            return;
        }
    }
    panic!(
        "ゴールに届かない: 所持金 {:?} 累計 {:?}",
        g.view().money(),
        g.view().total_earned()
    );
}

/// 転生ができるまで進める。
fn ready_game() -> (Game, Arc<BalanceTable>) {
    let bal = table();
    let mut g = game(1, &bal);
    reach_goal(&mut g);
    assert!(g.view().rebirth().can_rebirth);
    (g, bal)
}

/// 転生を 1 tick で適用する。
fn do_rebirth(g: &mut Game) {
    g.enqueue(Command::Rebirth);
    let mut events = Vec::new();
    g.step(&mut events);
    let n = g.view().rebirth().count;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, GameEvent::Rebirth { count, .. } if *count == n && n >= 1)),
        "{events:?}"
    );
}

/// 検査: ゴールに届く前は `RebirthNotReady`、届いたら `Ok` でポイントが会社レベルで決まる。
#[test]
fn rebirth_unlocks_at_the_goal() {
    let bal = table();
    let mut g = game(1, &bal);
    assert_eq!(
        g.validate(&Command::Rebirth),
        Err(CommandError::RebirthNotReady)
    );
    reach_goal(&mut g);
    let preview = g.validate(&Command::Rebirth).expect("転生できる");
    assert!(preview.cost.is_zero());
    let next = g.view().rebirth().next_points;
    assert_eq!(next, bal.rebirth_points(g.view().company().level));
}

/// 転生すると、会社・駅・路線・電車・所持金・進行は最初の状態に戻り、転生の状態だけが残る。
#[test]
fn rebirth_resets_the_company_and_keeps_the_rebirth_state() {
    let (mut g, bal) = ready_game();
    let before_level = g.view().company().level;
    do_rebirth(&mut g);
    g.check_invariants()
        .map_err(|v| format!("{}: {}", v.id, v.detail))
        .unwrap();

    let v = g.view();
    assert_eq!(v.company().level, CompanyLevel(0));
    assert_eq!(
        v.stations().len(),
        usize::from(bal.spec().company.stations_base)
    );
    assert_eq!(v.lines().len(), 0);
    assert_eq!(v.trains().len(), 0);
    assert!(v.money() == bal.initial_money());
    assert_eq!(v.total_earned(), Money::ZERO);
    assert!(!v.goal_reached());
    assert_eq!(v.max_money_unit(), ja_unit_index(bal.initial_money()));
    let r = v.rebirth();
    assert_eq!(r.count, 1);
    assert_eq!(r.points, bal.rebirth_points(before_level));
    // 路線を作れる(経路表が作り直されている)。
    let cmd = two_stations(0, 1);
    assert!(v.validate(&cmd).is_ok());
    g.enqueue(cmd);
    g.step(&mut Vec::new());
    assert_eq!(g.view().lines().len(), 1);
    assert!(g.check_invariants().is_ok());
}

/// 転生を何度しても、駅の位置の計画は変わらない(同じシードなら同じマップ)。
#[test]
fn repeated_rebirths_keep_the_map_plan() {
    let bal = table();
    let mut g = game(1, &bal);
    let plan: Vec<(i32, i32)> = g.view().stations().map(|s| (s.pos.x, s.pos.y)).collect();
    for _ in 0..3 {
        reach_goal(&mut g);
        do_rebirth(&mut g);
    }
    assert_eq!(g.view().rebirth().count, 3);
    assert!(g.check_invariants().is_ok());
    let fresh = game(1, &bal);
    let plan2: Vec<(i32, i32)> = fresh
        .view()
        .stations()
        .map(|s| (s.pos.x, s.pos.y))
        .collect();
    assert_eq!(plan, plan2);
}

/// 転生の強化はポイントを払って 1 レベル上がり、ポイントが足りなければ拒否される。
#[test]
fn upgrades_cost_points() {
    let (mut g, bal) = ready_game();
    do_rebirth(&mut g);
    let points = g.view().rebirth().points;
    assert_eq!(points, bal.rebirth_points(g.view().company().level));

    let fare = Command::RebirthUpgrade {
        upgrade: RebirthUpgrade::FareBoost,
    };
    let cost = bal.rebirth_cost(RebirthUpgrade::FareBoost, 0);
    g.enqueue(fare.clone());
    g.step(&mut Vec::new());
    assert_eq!(g.view().rebirth().points, points - cost);
    assert_eq!(
        g.view().rebirth().upgrades[RebirthUpgrade::FareBoost.index()].level,
        1
    );
    assert!(g.check_invariants().is_ok());

    // 2 つ目: ポイントが足りなければ拒否される。
    let train = Command::RebirthUpgrade {
        upgrade: RebirthUpgrade::TrainBoost,
    };
    let need = bal.rebirth_cost(RebirthUpgrade::TrainBoost, 0);
    if points - cost < need {
        assert_eq!(
            g.validate(&train),
            Err(CommandError::RebirthUpgradePoints {
                required: need,
                available: points - cost
            })
        );
    }
    // 転生の直後なので、また転生はまだできない。
    assert_eq!(
        g.validate(&Command::Rebirth),
        Err(CommandError::RebirthNotReady)
    );
}

/// 運賃倍率: 同じ列挙で収入がちょうど倍になる。
#[test]
fn fare_boost_doubles_the_income() {
    let income = |boost: bool| -> Money {
        let bal = table();
        let mut g = game(7, &bal);
        reach_goal(&mut g);
        do_rebirth(&mut g);
        if boost {
            g.enqueue(Command::RebirthUpgrade {
                upgrade: RebirthUpgrade::FareBoost,
            });
            g.step(&mut Vec::new());
        }
        g.enqueue(two_stations(0, 1));
        let mut total = Money::ZERO;
        let mut events = Vec::new();
        for _ in 0..3_600 {
            events.clear();
            g.step(&mut events);
            for e in &events {
                if let GameEvent::TrainArrived { income, .. } = e {
                    total = total.saturating_add(*income);
                }
            }
        }
        assert!(g.check_invariants().is_ok());
        total
    };
    let base = income(false);
    assert!(!base.is_zero(), "収入 0");
    assert_eq!(income(true), base.saturating_mul(Money::from_u64(2)));
}

/// 電車の数: 買うと全路線に 1 本足り、新しい路線にも最初から増えている。
#[test]
fn train_boost_adds_trains() {
    let (mut g, bal) = ready_game();
    do_rebirth(&mut g);
    g.enqueue(two_stations(0, 1));
    g.step(&mut Vec::new());
    assert_eq!(g.view().trains().len(), 1);

    // ポイントが足りるまで転生する(転生の強化は残る)。
    let need = bal.rebirth_cost(RebirthUpgrade::TrainBoost, 0);
    while g.view().rebirth().points < need {
        reach_goal(&mut g);
        do_rebirth(&mut g);
        // 転生で路線は消えるので作り直す。
        g.enqueue(two_stations(0, 1));
        g.step(&mut Vec::new());
    }
    g.enqueue(Command::RebirthUpgrade {
        upgrade: RebirthUpgrade::TrainBoost,
    });
    let mut events = Vec::new();
    g.step(&mut events);
    assert!(events.iter().any(|e| matches!(
        e,
        GameEvent::TrainAdded {
            line: LineId(0),
            ..
        }
    )));
    assert_eq!(g.view().trains().len(), 2);
    assert!(g.check_invariants().is_ok());

    // 新しい路線にも増えた数の電車が最初からいる(転生して路線を消してから作る)。
    reach_goal(&mut g);
    do_rebirth(&mut g);
    g.enqueue(two_stations(0, 2));
    g.step(&mut Vec::new());
    let base = bal.max_trains(Level(0)) as usize;
    assert_eq!(g.view().trains().len(), base + 1);
    assert!(g.check_invariants().is_ok());
}

/// 路線の数: 増えた枠だけ路線を作れる。
#[test]
fn line_boost_adds_line_slots() {
    let (mut g, bal) = ready_game();
    do_rebirth(&mut g);
    let base_slots = bal.line_slots(Level(0));
    // 基礎の枠を全部使う。
    let mut used = 0;
    for k in 0..base_slots {
        let cmd = two_stations(k % 2, 2 - k % 2);
        assert!(g.view().validate(&cmd).is_ok(), "{k} 本目");
        g.enqueue(cmd);
        g.step(&mut Vec::new());
        used += 1;
    }
    assert_eq!(used, base_slots);
    assert!(
        g.view().validate(&two_stations(0, 1)).is_err(),
        "枠が空いている"
    );
    // ゴールへ再び届いてから、転生 → 路線の数の強化。
    reach_goal(&mut g);
    do_rebirth(&mut g);
    let points = g.view().rebirth().points;
    let need = bal.rebirth_cost(RebirthUpgrade::LineBoost, 0);
    assert!(points >= need, "ポイント {points} < {need}");
    g.enqueue(Command::RebirthUpgrade {
        upgrade: RebirthUpgrade::LineBoost,
    });
    g.step(&mut Vec::new());
    assert_eq!(g.view().line_slots().1, base_slots + 1);
    assert!(g.check_invariants().is_ok());
    // 転生で路線は消えているので、また作れる。
    assert!(g.view().validate(&two_stations(0, 1)).is_ok());
}

/// 最大レベルを超えて買おうとすると拒否される。ポイントが尽きたら転生してまた積む。
#[test]
fn upgrades_stop_at_the_max() {
    let (mut g, bal) = ready_game();
    let fare = Command::RebirthUpgrade {
        upgrade: RebirthUpgrade::FareBoost,
    };
    while !matches!(
        g.validate(&fare),
        Err(CommandError::RebirthUpgradeMaxed {
            upgrade: RebirthUpgrade::FareBoost
        })
    ) {
        if g.validate(&fare).is_ok() {
            g.enqueue(fare.clone());
            g.step(&mut Vec::new());
            continue;
        }
        // ポイントが足りない: 転生して積む。
        reach_goal(&mut g);
        do_rebirth(&mut g);
    }
    let max = bal.rebirth_upgrade(RebirthUpgrade::FareBoost).max;
    assert_eq!(
        g.view().rebirth().upgrades[RebirthUpgrade::FareBoost.index()].level,
        max
    );
    assert!(g.check_invariants().is_ok());
}

/// 恒久の強化は転生しても消えない。
#[test]
fn upgrades_survive_the_rebirth() {
    let (mut g, _) = ready_game();
    do_rebirth(&mut g);
    let fare = Command::RebirthUpgrade {
        upgrade: RebirthUpgrade::FareBoost,
    };
    g.enqueue(fare);
    g.step(&mut Vec::new());
    let level = g.view().rebirth().upgrades[RebirthUpgrade::FareBoost.index()].level;
    assert_eq!(level, 1);
    reach_goal(&mut g);
    do_rebirth(&mut g);
    assert_eq!(
        g.view().rebirth().upgrades[RebirthUpgrade::FareBoost.index()].level,
        1
    );
    assert_eq!(g.view().rebirth().count, 2);
}

/// 恒久の強化のレベルの検査(INV-22 の本体)。
#[test]
fn check_levels_passes_a_fresh_game() {
    let bal = table();
    let g = game(1, &bal);
    assert!(crate::sim::rebirth::check_levels(&g.state, &bal).is_ok());
}
