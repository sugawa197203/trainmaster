use tm_numeric::BigNum;

use super::*;

fn def(
    key: ParamKey,
    base_yen: u64,
    cost_ratio: u32,
    intrinsic_max: Option<u8>,
    unlock: u8,
    value: ValueFormula,
) -> ParamDef {
    ParamDef {
        key,
        base_yen,
        cost_ratio,
        cost_steps: Vec::new(),
        intrinsic_max,
        unlock,
        value,
        value_steps: Vec::new(),
    }
}

/// S3 の値の区間の比(M8)。
fn s3_value_steps(mut d: ParamDef) -> ParamDef {
    d.value_steps = vec![
        CostStep { from: 10, ratio: 4 },
        CostStep { from: 14, ratio: 2 },
    ];
    d
}

/// 01 §3.1 の区間の比(17 GW-03)。
fn week_steps(mut d: ParamDef) -> ParamDef {
    d.cost_steps = vec![
        CostStep { from: 4, ratio: 26 },
        CostStep {
            from: 32,
            ratio: 20,
        },
    ];
    d
}

/// 01 と同じ転生の規則(T1)。
fn rebirth_01() -> RebirthRules {
    RebirthRules {
        points_base: 5,
        points_per_level: 2,
        fare_boost_ratio: 2,
        train_boost_trains: 1,
        line_boost_slots: 1,
        upgrades: vec![
            RebirthUpgradeDef {
                key: RebirthUpgrade::FareBoost,
                max: 8,
                base_cost: 2,
                cost_ratio: 2,
            },
            RebirthUpgradeDef {
                key: RebirthUpgrade::TrainBoost,
                max: 3,
                base_cost: 5,
                cost_ratio: 3,
            },
            RebirthUpgradeDef {
                key: RebirthUpgrade::LineBoost,
                max: 2,
                base_cost: 10,
                cost_ratio: 3,
            },
        ],
    }
}

fn lin(v0: u64, step: u64) -> ValueFormula {
    ValueFormula::Linear { v0, step }
}

fn geo(v0: u64, ratio: u64) -> ValueFormula {
    ValueFormula::Geometric { v0, ratio }
}

/// 01 §3.2(パラメータ)・§5(会社)・§6.1(マップ)・§6.4 と §7.2(運行)・§4.1 と D-08・D-20(経済)・D-40 を、RON とは別に手で写したもの。
fn spec_01() -> BalanceSpec {
    use ParamKey::*;
    BalanceSpec {
        format_version: 6,
        map: MapRules {
            width: 40,
            height: 18,
            margin: 1,
            initial_area: (16, 10),
            min_spacing: 4,
            max_nearest: 10,
            relax_spacings: vec![3, 2],
            attempts_per_stage: 1000,
        },
        company: CompanyRules {
            max_level: 24,
            cap_base: 4,
            cap_step: 2,
            gauge_num: 7,
            gauge_den: 10,
            stations_base: 3,
            stations_per_level: 1,
        },
        params: vec![
            week_steps(def(S1Spawn, 5, 10, None, 0, geo(500, 2))),
            week_steps(def(S2Fare, 5, 10, None, 0, geo(1, 10))),
            s3_value_steps(week_steps(def(S3StationCap, 50, 10, None, 1, geo(10, 2)))),
            def(S4Platform, 1_000, 3, Some(8), 2, lin(2, 1)),
            def(T1Speed, 10, 2, Some(10), 0, lin(1000, 500)),
            def(T2Crowding, 100_000, 3, Some(6), 4, lin(4, 1)),
            week_steps(def(T3Boarding, 100_000, 10, None, 4, geo(5, 2))),
            week_steps(def(T4CarCapacity, 10, 10, None, 0, geo(10, 2))),
            def(T5Cars, 1_000, 3, Some(9), 2, lin(1, 1)),
            def(R1MaxTrains, 10_000, 3, Some(7), 3, lin(1, 1)),
            def(R2LineSlots, 1_000, 10, Some(5), 1, lin(1, 1)),
            def(R3Attractiveness, 10_000, 3, Some(10), 3, lin(1000, 200)),
        ],
        features: vec![
            FeatureUnlock {
                feature: Feature::LineExtend,
                unlock: 0,
            },
            FeatureUnlock {
                feature: Feature::BulkBuy,
                unlock: 0,
            },
            FeatureUnlock {
                feature: Feature::Transfer,
                unlock: 1,
            },
        ],
        transit: TransitRules {
            dwell_base_ms: 1000,
            dwell_max_ms: 10_000,
            stop_allowance_s: 2,
            transfer_cost: 1000,
            waiting_cap_factor: 10,
            spawn_level_margin: 0,
        },
        economy: EconomyRules {
            initial_money_yen: 0,
            goal_pow10: 68,
            track: TrackCostRules {
                c1_yen: 50,
                level_ratio: 200,
                tile_ratio: (6, 5),
                tile_cap: 200,
                free_below_level: 1,
            },
            upgrade_floor: UpgradeFloorRules {
                behind: 3,
                from_level: 4,
            },
        },
        rebirth: rebirth_01(),
    }
}

fn rule_of(spec: BalanceSpec) -> &'static str {
    match BalanceTable::from_spec(spec) {
        Err(BalanceError::Invalid { rule, .. }) => rule,
        other => panic!("検証で落ちるはず: {other:?}"),
    }
}

fn detail_of(spec: BalanceSpec) -> (&'static str, String) {
    match BalanceTable::from_spec(spec) {
        Err(BalanceError::Invalid { rule, detail }) => (rule, detail),
        other => panic!("検証で落ちるはず: {other:?}"),
    }
}

fn assert_not_rule(spec: BalanceSpec, rule: &str) {
    if let Err(e) = BalanceTable::from_spec(spec) {
        assert!(
            !matches!(e, BalanceError::Invalid { rule: r, .. } if r == rule),
            "{e:?}"
        );
    }
}

fn param_mut(spec: &mut BalanceSpec, p: ParamKey) -> &mut ParamDef {
    &mut spec.params[p.index()]
}

#[test]
fn builtin_matches_01() {
    let t = BalanceTable::builtin();
    assert_eq!(t.spec(), &spec_01());
}

#[test]
fn builtin_derived_values() {
    let t = BalanceTable::builtin();
    let d = &t.derived;
    assert_eq!(d.max_stations, 27);
    assert_eq!(d.max_lines, 6);
    assert_eq!(d.max_trains_per_line, 8);
    assert_eq!(d.max_level_any, Level(52));
    assert_eq!(t.max_stations(), 27);
    assert_eq!(t.max_lines(), 6);
    assert_eq!(t.goal(), BigNum::pow10(68));
    assert_eq!(t.initial_money(), BigNum::ZERO);
}

#[test]
fn builtin_quick_reference_01_3_3() {
    let t = BalanceTable::builtin();
    let l = Level;
    assert_eq!(t.cost(ParamKey::S2Fare, l(0)), BigNum::from_u64(5));
    assert_eq!(t.cost(ParamKey::S2Fare, l(1)), BigNum::from_u64(50));
    assert_eq!(t.cost(ParamKey::S2Fare, l(5)), BigNum::from_u64(1_300_000));
    assert_eq!(
        t.cost(ParamKey::S2Fare, l(10)),
        BigNum::from_u64(5 * 10_000 * 26u64.pow(6))
    );
    let c51 = t.cost(ParamKey::S2Fare, l(51));
    assert!(c51 > BigNum::from_u64(10_909).saturating_mul(BigNum::pow10(65)));
    assert!(c51 < BigNum::from_u64(10_910).saturating_mul(BigNum::pow10(65)));
    assert_eq!(t.fare(l(0)), BigNum::from_u64(1));
    assert_eq!(t.fare(l(1)), BigNum::from_u64(10));
    assert_eq!(t.fare(l(6)), BigNum::from_u64(1_000_000));
    assert_eq!(t.fare(l(11)), BigNum::from_u64(100_000_000_000));
    assert_eq!(t.spawn_rate(l(0)), MilliPeople(500));
    assert_eq!(t.spawn_rate(l(6)), MilliPeople(32_000));
    assert_eq!(t.spawn_rate(l(11)), MilliPeople(1_024_000));
    assert_eq!(t.spawn_rate(l(52)), MilliPeople(500 << 52));
    assert_eq!(t.cost(ParamKey::T1Speed, l(0)), BigNum::from_u64(10));
    assert_eq!(t.cost(ParamKey::T1Speed, l(1)), BigNum::from_u64(20));
    assert_eq!(t.cost(ParamKey::T1Speed, l(5)), BigNum::from_u64(320));
    assert_eq!(
        t.cost(ParamKey::R1MaxTrains, l(0)),
        BigNum::from_u64(10_000)
    );
    assert_eq!(
        t.cost(ParamKey::R1MaxTrains, l(1)),
        BigNum::from_u64(30_000)
    );
    assert_eq!(
        t.cost(ParamKey::R1MaxTrains, l(5)),
        BigNum::from_u64(2_430_000)
    );
}

#[test]
fn builtin_values_01_3_2() {
    let t = BalanceTable::builtin();
    let l = Level;
    assert_eq!(t.station_capacity(l(0)), People(10));
    assert_eq!(t.station_capacity(l(10)), People(10 << 10));
    assert_eq!(t.station_capacity(l(11)), People(10 << 12));
    assert_eq!(t.station_capacity(l(14)), People(10 << 18));
    assert_eq!(t.station_capacity(l(52)), People(10 << 56));
    assert_eq!(t.platform(l(0)), Cars(2));
    assert_eq!(t.platform(l(8)), Cars(10));
    assert_eq!(t.speed(l(0)), SpeedMilli(1000));
    assert_eq!(t.speed(l(10)), SpeedMilli(6000));
    assert_eq!(t.crowding(l(0)), Quarter(4));
    assert_eq!(t.crowding(l(6)), Quarter(10));
    assert_eq!(t.boarding_rate(l(0)), People(5));
    assert_eq!(t.boarding_rate(l(52)), People(5 << 52));
    assert_eq!(t.car_capacity(l(0)), People(10));
    assert_eq!(t.cars(l(0)), Cars(1));
    assert_eq!(t.cars(l(9)), Cars(10));
    assert_eq!(t.max_trains(l(7)), 8);
    assert_eq!(t.line_slots(l(0)), 1);
    assert_eq!(t.line_slots(l(5)), 6);
    assert_eq!(t.attractiveness(l(0)), Permille(1000));
    assert_eq!(t.attractiveness(l(10)), Permille(3000));
}

#[test]
fn upgrade_floor_follows_birth_level_and_unlock_m8() {
    let t = BalanceTable::builtin();
    let (c, l) = (CompanyLevel, Level);
    let f = |p, born| t.floor_level(p, c(born));
    for born in 0..=3 {
        assert_eq!(
            f(ParamKey::S1Spawn, born),
            l(0),
            "Lv{born}: 下限は会社 Lv4 から(M10)"
        );
    }
    assert_eq!(f(ParamKey::S1Spawn, 4), l(6));
    assert_eq!(f(ParamKey::S2Fare, 4), l(6));
    assert_eq!(f(ParamKey::S3StationCap, 3), l(0));
    assert_eq!(f(ParamKey::S3StationCap, 4), l(6));
    assert_eq!(f(ParamKey::T3Boarding, 6), l(0));
    assert_eq!(f(ParamKey::T3Boarding, 7), l(12));
    assert_eq!(f(ParamKey::T4CarCapacity, 24), l(46));
    for p in [
        ParamKey::S4Platform,
        ParamKey::T1Speed,
        ParamKey::T2Crowding,
        ParamKey::T5Cars,
        ParamKey::R1MaxTrains,
        ParamKey::R2LineSlots,
        ParamKey::R3Attractiveness,
    ] {
        assert_eq!(f(p, 24), l(0), "{p:?}");
    }
    assert_eq!(
        t.upgrade_cost(ParamKey::S1Spawn, l(0), c(4)),
        t.cost(ParamKey::S1Spawn, l(6))
    );
    assert_eq!(
        t.upgrade_cost(ParamKey::S1Spawn, l(7), c(4)),
        t.cost(ParamKey::S1Spawn, l(7))
    );
    let u = |p, from, born| t.upgrade_cost(p, l(from), c(born));
    let era = |born: u8| t.cost(ParamKey::S1Spawn, l(t.cap(c(born - 3))));
    for p in [ParamKey::S4Platform, ParamKey::T1Speed, ParamKey::T5Cars] {
        assert_eq!(
            u(p, 0, 3),
            t.cost(p, l(0)),
            "{p:?}: 会社 Lv3 で生まれたものは元の値段"
        );
        assert_eq!(u(p, 0, 0), t.cost(p, l(0)), "{p:?}");
    }
    assert_eq!(
        u(ParamKey::T1Speed, 0, 4),
        era(4),
        "固有上限のある設備も基準の値段より安くならない(M10)"
    );
    assert_eq!(
        u(ParamKey::S4Platform, 0, 4),
        t.cost(ParamKey::S4Platform, l(0)),
        "S4 は会社 Lv2 で解放。4 − 3 < 2"
    );
    assert_eq!(u(ParamKey::S4Platform, 0, 5), era(5));
    assert_eq!(u(ParamKey::S4Platform, 7, 24), era(24));
    assert_eq!(u(ParamKey::T2Crowding, 5, 7), era(7));
    assert_eq!(u(ParamKey::R3Attractiveness, 0, 6), era(6));
    assert_eq!(
        u(ParamKey::R2LineSlots, 0, 24),
        t.cost(ParamKey::R2LineSlots, l(0)),
        "R2 は会社全体"
    );
    assert_eq!(u(ParamKey::R1MaxTrains, 6, 6), era(6));
    let mut spec = t.spec().clone();
    spec.params[ParamKey::T1Speed.index()].base_yen = 10_u64.pow(9);
    let big = BalanceTable::from_spec(spec).unwrap();
    assert_eq!(
        big.upgrade_cost(ParamKey::T1Speed, l(0), c(4)),
        big.cost(ParamKey::T1Speed, l(0)),
        "元の値段の方が高ければ元の値段"
    );
    let s = |i| t.station_born_level(crate::ids::StationId(i));
    assert_eq!(
        [s(0), s(2), s(3), s(4), s(26)],
        [c(0), c(0), c(1), c(2), c(24)]
    );
}

/// 値段の下限の基準(S1 の Lv cap(B − behind))は、検証を通る表ならいつも S1 の表の中にある。
#[test]
fn floor_reference_stays_inside_the_s1_table() {
    let mut spec = spec_01();
    param_mut(&mut spec, ParamKey::S1Spawn).intrinsic_max = Some(5);
    assert_eq!(rule_of(spec), "BV-22");
    let mut spec = spec_01();
    param_mut(&mut spec, ParamKey::S1Spawn).intrinsic_max = Some(46);
    let t = BalanceTable::from_spec(spec).expect("cap(24 − 3) = 46 なら通る");
    for born in 0..=24 {
        for p in ParamKey::ALL {
            let _ = t.upgrade_cost(p, Level(0), CompanyLevel(born));
        }
    }
    assert_eq!(
        t.upgrade_cost(ParamKey::T1Speed, Level(0), CompanyLevel(24)),
        t.cost(ParamKey::S1Spawn, Level(46))
    );
}

#[test]
fn value_steps_multiply_in_level_order_m8() {
    let t = BalanceTable::builtin();
    let s3 = t.param(ParamKey::S3StationCap);
    let ratios: Vec<u64> = (8..16).map(|l| s3.value_ratio_at(l).unwrap()).collect();
    assert_eq!(ratios, [2, 2, 4, 4, 4, 4, 2, 2]);
    assert_eq!(t.param(ParamKey::T1Speed).value_ratio_at(3), None);
    let mut v = 10u64;
    for l in 0..=52u8 {
        assert_eq!(t.station_capacity(Level(l)), People(v), "Lv{l}");
        v *= s3.value_ratio_at(l).unwrap();
    }
}

#[test]
fn builtin_limits_and_unlocks_01_5_2() {
    let t = BalanceTable::builtin();
    let c = CompanyLevel;
    assert_eq!(t.cap(c(0)), 4);
    assert_eq!(t.cap(c(4)), 12);
    assert_eq!(t.cap(c(23)), 50);
    assert_eq!(t.cap(c(24)), 52);
    assert_eq!(t.limit(ParamKey::S2Fare, c(0)), Level(4));
    assert_eq!(t.limit(ParamKey::S2Fare, c(24)), Level(52));
    assert_eq!(t.limit(ParamKey::T1Speed, c(24)), Level(10));
    assert_eq!(t.limit(ParamKey::T1Speed, c(2)), Level(8));
    assert_eq!(t.limit(ParamKey::T5Cars, c(1)), Level(6));
    assert_eq!(t.limit(ParamKey::T5Cars, c(3)), Level(9));
    assert_eq!(t.limit(ParamKey::R2LineSlots, c(0)), Level(4));
    assert_eq!(t.limit(ParamKey::R2LineSlots, c(1)), Level(5));
    let unlock = |f| t.unlock_level(f).0;
    let p = Feature::Param;
    assert_eq!(unlock(p(ParamKey::S1Spawn)), 0);
    assert_eq!(unlock(p(ParamKey::S3StationCap)), 1);
    assert_eq!(unlock(p(ParamKey::R2LineSlots)), 1);
    assert_eq!(unlock(Feature::Transfer), 1);
    assert_eq!(unlock(p(ParamKey::S4Platform)), 2);
    assert_eq!(unlock(p(ParamKey::T5Cars)), 2);
    assert_eq!(unlock(p(ParamKey::R1MaxTrains)), 3);
    assert_eq!(unlock(p(ParamKey::R3Attractiveness)), 3);
    assert_eq!(unlock(p(ParamKey::T2Crowding)), 4);
    assert_eq!(unlock(p(ParamKey::T3Boarding)), 4);
    assert_eq!(unlock(Feature::LineExtend), 0);
    assert_eq!(unlock(Feature::BulkBuy), 0);
    assert_eq!(Feature::all_in_order().map(unlock).max(), Some(4));
}

#[test]
fn parse_error_is_reported() {
    let err = BalanceTable::from_ron_str("BalanceSpec(format_version: 1.5)").unwrap_err();
    assert!(matches!(err, BalanceError::Parse { .. }), "{err:?}");
}

#[test]
fn spec_ron_round_trip() {
    let s = ron::to_string(&spec_01()).unwrap();
    let back = BalanceTable::from_ron_str(&s).unwrap();
    assert_eq!(back.spec(), &spec_01());
}

#[test]
fn each_rule_rejects_a_broken_value() {
    use ParamKey::*;
    type Mutation = Box<dyn Fn(&mut BalanceSpec)>;
    let cases: Vec<(&str, Mutation)> = vec![
        ("BV-01", Box::new(|s| s.format_version = 4)),
        ("BV-01", Box::new(|s| s.format_version = 7)),
        ("BV-02", Box::new(|s| s.params.swap(0, 1))),
        (
            "BV-02",
            Box::new(|s| {
                s.params.pop();
            }),
        ),
        ("BV-03", Box::new(|s| param_mut(s, S2Fare).base_yen = 0)),
        ("BV-03", Box::new(|s| param_mut(s, T1Speed).cost_ratio = 0)),
        (
            "BV-04",
            Box::new(|s| param_mut(s, S1Spawn).value = geo(500, 0)),
        ),
        (
            "BV-04",
            Box::new(|s| param_mut(s, T1Speed).value = lin(0, 500)),
        ),
        (
            "BV-04",
            Box::new(|s| param_mut(s, R3Attractiveness).value = lin(0, 200)),
        ),
        (
            "BV-04",
            Box::new(|s| param_mut(s, T3Boarding).value = geo(0, 2)),
        ),
        (
            "BV-04",
            Box::new(|s| param_mut(s, R2LineSlots).value = lin(0, 1)),
        ),
        (
            "BV-04",
            Box::new(|s| param_mut(s, R1MaxTrains).value = lin(0, 1)),
        ),
        ("BV-05", Box::new(|s| param_mut(s, T2Crowding).unlock = 25)),
        ("BV-05", Box::new(|s| s.features[2].unlock = 25)),
        (
            "BV-05",
            Box::new(|s| param_mut(s, T1Speed).intrinsic_max = Some(53)),
        ),
        ("BV-05", Box::new(|s| s.company.cap_step = 16)),
        (
            "BV-06",
            Box::new(|s| param_mut(s, S4Platform).intrinsic_max = Some(7)),
        ),
        ("BV-07", Box::new(|s| s.company.gauge_num = 10)),
        ("BV-07", Box::new(|s| s.company.gauge_num = 0)),
        ("BV-08", Box::new(|s| s.company.stations_base = 1)),
        (
            "BV-08",
            Box::new(|s| s.map.initial_area = (s.map.width - 2 * s.map.margin + 1, 10)),
        ),
        (
            "BV-08",
            Box::new(|s| {
                s.map.width = 4;
                s.map.height = 4;
            }),
        ),
        (
            "BV-09",
            Box::new(|s| param_mut(s, R2LineSlots).intrinsic_max = Some(11)),
        ),
        (
            "BV-10",
            Box::new(|s| param_mut(s, S3StationCap).value = geo(10, 3)),
        ),
        (
            "BV-10",
            Box::new(|s| param_mut(s, T4CarCapacity).value = geo(1_000, 2)),
        ),
        (
            "BV-10",
            Box::new(|s| param_mut(s, S3StationCap).value_steps[0].ratio = 16),
        ),
        (
            "BV-11",
            Box::new(|s| param_mut(s, S2Fare).value = geo(1, u64::MAX)),
        ),
        (
            "BV-11",
            Box::new(|s| {
                param_mut(s, S1Spawn).cost_ratio = u32::MAX;
                param_mut(s, S1Spawn).cost_steps.clear();
            }),
        ),
        (
            "BV-11",
            Box::new(|s| {
                param_mut(s, T3Boarding).cost_steps = vec![CostStep {
                    from: 4,
                    ratio: u32::MAX,
                }]
            }),
        ),
        ("BV-12", Box::new(|s| s.transit.dwell_base_ms = 10_001)),
        ("BV-12", Box::new(|s| s.transit.transfer_cost = 0)),
        ("BV-12", Box::new(|s| s.transit.waiting_cap_factor = 0)),
        (
            "BV-13",
            Box::new(|s| {
                s.features.pop();
            }),
        ),
        (
            "BV-13",
            Box::new(|s| {
                s.features.push(FeatureUnlock {
                    feature: Feature::BulkBuy,
                    unlock: 0,
                })
            }),
        ),
        (
            "BV-13",
            Box::new(|s| {
                s.features.push(FeatureUnlock {
                    feature: Feature::Param(S1Spawn),
                    unlock: 0,
                })
            }),
        ),
        (
            "BV-14",
            Box::new(|s| param_mut(s, T1Speed).value = lin(1000, 6000)),
        ),
        ("BV-15", Box::new(|s| s.transit.dwell_base_ms = 0)),
        ("BV-16", Box::new(|s| s.map.min_spacing = 0)),
        ("BV-16", Box::new(|s| s.map.relax_spacings = vec![3, 0])),
        ("BV-16", Box::new(|s| s.map.attempts_per_stage = 0)),
        (
            "BV-17",
            Box::new(|s| param_mut(s, S4Platform).value = lin(0, 3)),
        ),
        (
            "BV-18",
            Box::new(|s| param_mut(s, S2Fare).cost_steps[0].from = 0),
        ),
        (
            "BV-18",
            Box::new(|s| param_mut(s, S2Fare).cost_steps[1].from = 53),
        ),
        (
            "BV-18",
            Box::new(|s| param_mut(s, T4CarCapacity).cost_steps[1].from = 4),
        ),
        (
            "BV-18",
            Box::new(|s| param_mut(s, T3Boarding).cost_steps.swap(0, 1)),
        ),
        (
            "BV-18",
            Box::new(|s| param_mut(s, S1Spawn).cost_steps[1].ratio = 0),
        ),
        ("BV-19", Box::new(|s| s.economy.track.c1_yen = 0)),
        ("BV-19", Box::new(|s| s.economy.track.level_ratio = 0)),
        ("BV-19", Box::new(|s| s.economy.track.tile_ratio = (6, 0))),
        ("BV-19", Box::new(|s| s.economy.track.tile_ratio = (4, 5))),
        ("BV-19", Box::new(|s| s.economy.track.free_below_level = 0)),
        ("BV-19", Box::new(|s| s.economy.track.free_below_level = 25)),
        ("BV-19", Box::new(|s| s.economy.track.tile_cap = 1025)),
        (
            "BV-19",
            Box::new(|s| s.economy.track.tile_ratio = (1000, 1)),
        ),
        (
            "BV-19",
            Box::new(|s| {
                s.economy.track.c1_yen = u64::MAX;
                s.economy.track.level_ratio = u32::MAX;
                s.economy.track.tile_ratio = (3, 1);
            }),
        ),
        (
            "BV-20",
            Box::new(|s| {
                s.map.width = 65;
                s.map.height = 16;
            }),
        ),
        (
            "BV-21",
            Box::new(|s| param_mut(s, S3StationCap).value_steps[0].from = 0),
        ),
        (
            "BV-21",
            Box::new(|s| param_mut(s, S3StationCap).value_steps.swap(0, 1)),
        ),
        (
            "BV-21",
            Box::new(|s| param_mut(s, S3StationCap).value_steps[1].from = 53),
        ),
        (
            "BV-21",
            Box::new(|s| param_mut(s, S3StationCap).value_steps[1].ratio = 0),
        ),
        (
            "BV-21",
            Box::new(|s| param_mut(s, S2Fare).value_steps = vec![CostStep { from: 5, ratio: 2 }]),
        ),
        (
            "BV-21",
            Box::new(|s| {
                param_mut(s, R1MaxTrains).value_steps = vec![CostStep { from: 2, ratio: 2 }]
            }),
        ),
        ("BV-22", Box::new(|s| s.economy.upgrade_floor.behind = 0)),
        (
            "BV-22",
            Box::new(|s| param_mut(s, S1Spawn).intrinsic_max = Some(45)),
        ),
    ];
    for (rule, mutate) in cases {
        let mut s = spec_01();
        mutate(&mut s);
        assert_eq!(rule_of(s), rule);
    }
}

#[test]
fn each_bv10_check_rejects_overflow() {
    use ParamKey::*;
    type Mutation = Box<dyn Fn(&mut BalanceSpec)>;
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "S3StationCap Lv",
            Box::new(|s| param_mut(s, S3StationCap).value = geo(10, 3)),
        ),
        (
            "S2Fare Lv1 の値が u64",
            Box::new(|s| param_mut(s, S2Fare).value = lin(1, u64::MAX)),
        ),
        (
            "S4Platform の値が u32",
            Box::new(|s| param_mut(s, S4Platform).value = lin(2, 1 << 32)),
        ),
        (
            "発生の累積器",
            Box::new(|s| param_mut(s, R3Attractiveness).value = lin(1000, 1000)),
        ),
        (
            "列車の定員",
            Box::new(|s| param_mut(s, T4CarCapacity).value = geo(1_000, 2)),
        ),
        (
            "走行コストの N",
            Box::new(|s| {
                s.transit.stop_allowance_s = u32::MAX;
                param_mut(s, T1Speed).value = lin(1000, 5899);
            }),
        ),
        ("乗車の枠", Box::new(|s| s.transit.dwell_max_ms = 1_000_000)),
    ];
    for (what, mutate) in cases {
        let mut s = spec_01();
        mutate(&mut s);
        let (rule, detail) = detail_of(s);
        assert_eq!(rule, "BV-10", "{detail}");
        assert!(detail.contains(what), "{what}: {detail}");
    }
}

/// BV-10 の走行コストの N: 区間の長さの上限は、マップの長いほうの辺から決める。
#[test]
fn ride_cost_bound_uses_the_longer_side_of_the_map() {
    let k = u64::MAX / 100_000;
    for (w, h, area) in [(40, 18, (16, 10)), (18, 40, (10, 16))] {
        let with_allowance = |allowance: u32| {
            let mut s = spec_01();
            s.map.width = w;
            s.map.height = h;
            s.map.initial_area = area;
            param_mut(&mut s, ParamKey::T1Speed).value = lin(1000, 5000);
            s.transit.stop_allowance_s = allowance;
            s
        };
        let t = BalanceTable::from_spec(with_allowance(0)).expect("余裕 0 は通る");
        let top = t.limit(ParamKey::T1Speed, CompanyLevel(t.spec().company.max_level));
        let v = u64::from(t.speed(top).0);
        let track = 1000 * u64::from(t.max_stations() - 1) * u64::from(w.max(h));
        let a = u32::try_from((k - track) / v).expect("u32 に収まる");
        BalanceTable::from_spec(with_allowance(a)).expect("N = u64 の上限以下は通る");
        let (rule, detail) = detail_of(with_allowance(a + 1));
        assert_eq!(rule, "BV-10", "{w} × {h}: {detail}");
        assert!(detail.contains("走行コストの N"), "{detail}");
    }
}

#[test]
fn boundary_values_pass_and_next_fails() {
    use ParamKey::*;
    let with = |f: &dyn Fn(&mut BalanceSpec)| {
        let mut s = spec_01();
        f(&mut s);
        s
    };

    BalanceTable::from_spec(with(&|s| s.company.stations_base = 8))
        .expect("ノード数 32 × (2 + 6) = 256 は通る");
    assert_eq!(rule_of(with(&|s| s.company.stations_base = 9)), "BV-09");

    BalanceTable::from_spec(with(&|s| param_mut(s, T1Speed).value = lin(1999, 5800)))
        .expect("速度 59999 は通る");
    assert_eq!(
        rule_of(with(&|s| param_mut(s, T1Speed).value = lin(2000, 5800))),
        "BV-14"
    );

    let platform_limit = |m: u8| {
        with(&move |s| {
            param_mut(s, S4Platform).intrinsic_max = Some(m);
            param_mut(s, T5Cars).intrinsic_max = None;
        })
    };
    BalanceTable::from_spec(platform_limit(51)).expect("C24 でホーム長 53 ≥ 両数 53");
    let (rule, detail) = detail_of(platform_limit(49));
    assert_eq!(
        (rule, detail.starts_with("C24:")),
        ("BV-06", true),
        "{detail}"
    );

    assert_not_rule(with(&|s| s.company.cap_base = 206), "BV-05");
    assert_eq!(rule_of(with(&|s| s.company.cap_base = 207)), "BV-05");
    BalanceTable::from_spec(with(&|s| param_mut(s, T1Speed).intrinsic_max = Some(52)))
        .expect("固有上限 52 = cap(24) は通る");

    let big_map = |base: u8| {
        with(&move |s| {
            s.map.width = 32;
            s.map.height = 32;
            s.map.margin = 0;
            s.company.stations_base = base;
        })
    };
    assert_not_rule(big_map(231), "BV-08");
    assert_eq!(rule_of(big_map(231)), "BV-09");
    assert_eq!(rule_of(big_map(232)), "BV-08");

    let map = |w: u8, h: u8| {
        with(&move |s| {
            s.map.width = w;
            s.map.height = h;
        })
    };
    BalanceTable::from_spec(map(64, 16)).expect("1024 マスは通る");
    assert_eq!(rule_of(map(64, 17)), "BV-20");
    BalanceTable::from_spec(with(&|s| s.economy.track.tile_cap = 1024)).expect("tile_cap = 1024");
    BalanceTable::from_spec(with(&|s| {
        s.economy.track.c1_yen = u64::MAX;
        s.economy.track.level_ratio = u32::MAX;
        s.economy.track.tile_ratio = (2, 1);
    }))
    .expect("建設の値段の上限 約 10^304 は飽和しない");
    let (rule, detail) = detail_of(with(&|s| {
        s.economy.track.c1_yen = u64::MAX;
        s.economy.track.level_ratio = u32::MAX;
        s.economy.track.tile_ratio = (2, 1);
        s.economy.track.tile_cap = 220;
    }));
    assert_eq!(
        rule, "BV-19",
        "1 マスは約 10^307、マス数を掛けると飽和: {detail}"
    );
    BalanceTable::from_spec(with(&|s| s.economy.track.tile_ratio = (5, 5)))
        .expect("tile_ratio 1 倍は通る");
    BalanceTable::from_spec(with(&|s| s.economy.track.free_below_level = 24))
        .expect("free_below_level = max_level");

    let steps = |a: u8, b: u8| {
        with(&move |s| {
            param_mut(s, S2Fare).cost_steps = vec![
                CostStep { from: a, ratio: 1 },
                CostStep { from: b, ratio: 1 },
            ]
        })
    };
    BalanceTable::from_spec(steps(1, 52)).expect("from = 1 と max_level_any、比 1 は通る");
    assert_eq!(rule_of(steps(5, 5)), "BV-18");

    let narrow_map = |width: u8| {
        with(&move |s| {
            s.map.width = width;
            s.map.height = 3;
            s.map.initial_area = (16, 1);
        })
    };
    BalanceTable::from_spec(narrow_map(29)).expect("置けるマス 27 × 1 に 27 駅は通る");
    assert_eq!(rule_of(narrow_map(28)), "BV-08");
}

/// 上位 4 桁(`v ≥ 1000`)。
fn top4(v: Money) -> u64 {
    let e = (0..300)
        .find(|&e| BigNum::from_u64(10_000).saturating_mul(BigNum::pow10(e)) > v)
        .expect("10^304 未満");
    let scale = BigNum::pow10(e);
    (1000..10_000)
        .rev()
        .find(|&d| BigNum::from_u64(d).saturating_mul(scale) <= v)
        .expect("v ≥ 1000")
}

fn close(a: Money, b: Money) -> bool {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    hi.checked_sub(lo)
        .is_some_and(|d| d <= hi.div_small(1_000_000_000_000))
}

#[test]
fn cost_steps_boundaries_11_7_2() {
    use ParamKey::*;
    let t = BalanceTable::builtin();
    for p in [S1Spawn, S2Fare, S3StationCap, T3Boarding, T4CarCapacity] {
        let c = |l: u8| t.cost(p, Level(l));
        for (l, r) in [
            (0, 10),
            (2, 10),
            (3, 10),
            (4, 26),
            (31, 26),
            (32, 20),
            (51, 20),
        ] {
            assert_eq!(c(l + 1), c(l).mul_small(r), "{p:?} Lv{l}→{}", l + 1);
        }
    }
    for p in [
        S4Platform,
        T1Speed,
        T2Crowding,
        T5Cars,
        R1MaxTrains,
        R2LineSlots,
        R3Attractiveness,
    ] {
        let d = t.param(p);
        assert!(d.cost_steps.is_empty());
        for l in 0..=t.derived.max_reach[p.index()].0 {
            let want = BigNum::pow_u32(BigNum::from_u64(u64::from(d.cost_ratio)), u32::from(l))
                .mul_small(d.base_yen);
            assert!(close(t.cost(p, Level(l)), want), "{p:?} Lv{l}");
        }
    }
    assert!(close(
        t.cost(T3Boarding, Level(52)),
        BigNum::from_u64(100_000)
            .mul_small(10_000)
            .saturating_mul(BigNum::pow_u32(BigNum::from_u64(26), 28))
            .saturating_mul(BigNum::pow_u32(BigNum::from_u64(20), 20))
    ));
}

#[test]
fn cost_table_multiplies_ratio_in_level_order() {
    let mut spec = spec_01();
    param_mut(&mut spec, ParamKey::S2Fare).cost_steps = vec![
        CostStep { from: 2, ratio: 3 },
        CostStep { from: 5, ratio: 7 },
        CostStep { from: 6, ratio: 1 },
    ];
    let t = BalanceTable::from_spec(spec).unwrap();
    let want = [5u64, 50, 500, 1500, 4500, 13_500, 94_500, 94_500];
    for (l, w) in want.into_iter().enumerate() {
        assert_eq!(
            t.cost(ParamKey::S2Fare, Level(l as u8)),
            BigNum::from_u64(w)
        );
    }
    for p in ParamKey::ALL {
        let d = t.param(p);
        let mut c = BigNum::from_u64(d.base_yen);
        for l in 0..=t.derived.max_reach[p.index()].0 {
            assert_eq!(t.cost(p, Level(l)).to_bits(), c.to_bits(), "{p:?} Lv{l}");
            let r = d
                .cost_steps
                .iter()
                .filter(|s| s.from <= l)
                .next_back()
                .map_or(d.cost_ratio, |s| s.ratio);
            c = c.mul_small(u64::from(r));
        }
    }
}

#[test]
fn omitted_cost_steps_is_empty_and_v1_is_rejected() {
    let no_steps = BUILTIN_BALANCE_RON.replace(
        "cost_steps: [(from: 4, ratio: 26), (from: 32, ratio: 20)], ",
        "",
    );
    let t = BalanceTable::from_ron_str(&no_steps).unwrap();
    assert!(t.spec().params.iter().all(|p| p.cost_steps.is_empty()));
    assert_eq!(
        t.cost(ParamKey::S2Fare, Level(10)),
        BigNum::from_u64(50_000_000_000)
    );

    for old in [
        "format_version: 1",
        "format_version: 2",
        "format_version: 3",
        "format_version: 4",
        "format_version: 5",
    ] {
        let v = BUILTIN_BALANCE_RON.replace("format_version: 6", old);
        assert!(matches!(
            BalanceTable::from_ron_str(&v),
            Err(BalanceError::Invalid { rule: "BV-01", .. })
        ));
    }
    let start = BUILTIN_BALANCE_RON
        .find(",\n              // 会社 Lv B")
        .unwrap();
    let end = BUILTIN_BALANCE_RON[start..].find(")),").unwrap() + start + 1;
    let no_floor = format!(
        "{}{}",
        &BUILTIN_BALANCE_RON[..start],
        &BUILTIN_BALANCE_RON[end..]
    );
    assert!(!no_floor.contains("upgrade_floor"), "{no_floor}");
    assert!(matches!(
        BalanceTable::from_ron_str(&no_floor),
        Err(BalanceError::Parse { .. })
    ));
    let start = BUILTIN_BALANCE_RON.find(",\n              track:").unwrap();
    let end = BUILTIN_BALANCE_RON[start..].find(")),").unwrap() + start + 1;
    let no_track = format!(
        "{}{}",
        &BUILTIN_BALANCE_RON[..start],
        &BUILTIN_BALANCE_RON[end..]
    );
    assert!(no_track.contains("goal_pow10: 68),"), "{no_track}");
    assert!(matches!(
        BalanceTable::from_ron_str(&no_track),
        Err(BalanceError::Parse { .. })
    ));
}

#[test]
fn track_tables_10_4_2() {
    let t = BalanceTable::builtin();
    let d = &t.derived;
    assert_eq!(d.track_pow_c.len(), 25);
    assert_eq!(d.track_pow_n.len(), 201);
    assert_eq!(d.track_pow_c[0], BigNum::ZERO);
    assert_eq!(d.track_pow_c[1], BigNum::from_u64(50));
    assert_eq!(d.track_pow_c[2], BigNum::from_u64(10_000));
    assert_eq!(d.track_pow_c[3], BigNum::from_u64(2_000_000));
    assert!(close(
        d.track_pow_c[24],
        BigNum::pow_u32(BigNum::from_u64(200), 23).mul_small(50)
    ));
    assert_eq!(top4(d.track_pow_c[24]), 4194);
    assert_eq!(d.track_pow_n[0], BigNum::ONE);
    assert_eq!(d.track_pow_n[1], BigNum::from_u64(6).div_small(5));
    assert_eq!(
        d.track_pow_n[2],
        BigNum::from_u64(6).div_small(5).mul_small(6).div_small(5)
    );
    assert_eq!(top4(d.track_pow_n[200]), 6858);
    for w in d.track_pow_n.windows(2) {
        assert_eq!(w[1], w[0].mul_small(6).div_small(5));
    }
    for w in d.track_pow_c[1..].windows(2) {
        assert_eq!(w[1], w[0].mul_small(200));
    }
}

#[test]
fn track_cost_examples_11_4_8() {
    let t = BalanceTable::builtin();
    let c = CompanyLevel;
    for k in [0, 1, 8, 720] {
        assert_eq!(t.track_cost(c(0), 14, k), BigNum::ZERO);
    }
    assert_eq!(t.track_cost(c(5), 14, 0), BigNum::ZERO);
    assert_eq!(t.track_cost(c(1), 0, 1), BigNum::from_u64(50));
    let r = BigNum::from_u64(6).div_small(5);
    assert_eq!(
        t.track_cost(c(1), 0, 2),
        BigNum::from_u64(50).saturating_mul(BigNum::ONE.saturating_add(r))
    );
    assert_eq!(
        t.track_cost(c(2), 1, 1),
        BigNum::from_u64(10_000).saturating_mul(r)
    );
    for (lv, n, k, digits) in [
        (1, 14, 8, 1059),
        (4, 40, 7, 7593),
        (12, 90, 5, 1019),
        (18, 120, 6, 2066),
        (24, 150, 6, 3139),
    ] {
        assert_eq!(
            top4(t.track_cost(c(lv), n, k)),
            digits,
            "Lv{lv} N={n} k={k}"
        );
    }
    let unit_cap = t.track_cost(c(10), 200, 1);
    assert_eq!(t.track_cost(c(10), 250, 1), unit_cap);
    let p200 = t.derived.track_pow_n[200];
    assert_eq!(
        t.track_cost(c(10), u32::MAX, 3),
        t.derived.track_pow_c[10].saturating_mul(p200.saturating_add(p200).saturating_add(p200))
    );
    assert_eq!(
        t.track_cost(c(10), 199, 3),
        t.derived.track_pow_c[10].saturating_mul(
            t.derived.track_pow_n[199]
                .saturating_add(t.derived.track_pow_n[200])
                .saturating_add(t.derived.track_pow_n[200])
        )
    );
    for (lv, n, k) in [(4, 40, 7), (1, 0, 10)] {
        let add = |s: BigNum, i: usize| s.saturating_add(t.derived.track_pow_n[n + i]);
        let up = (0..k).fold(BigNum::ZERO, add);
        let down = (0..k).rev().fold(BigNum::ZERO, add);
        let pow_c = t.derived.track_pow_c[usize::from(lv)];
        assert_ne!(
            pow_c.saturating_mul(up).to_bits(),
            pow_c.saturating_mul(down).to_bits()
        );
        assert_eq!(
            t.track_cost(c(lv), n as u32, k as u32).to_bits(),
            pow_c.saturating_mul(up).to_bits(),
            "Lv{lv} N={n} k={k}: i の昇順に足す(11 §4.8)"
        );
    }

    let mut spec = spec_01();
    spec.economy.track.free_below_level = 3;
    let t3 = BalanceTable::from_spec(spec).unwrap();
    assert_eq!(t3.track_cost(c(2), 0, 5), BigNum::ZERO);
    assert_eq!(t3.track_cost(c(3), 0, 1), BigNum::from_u64(2_000_000));
}

/// 表のビット列のゴールデン(11 §11.3)。`balance.ron` か表の作り方を変えたら更新する(16 §2.7)。
#[test]
fn tables_bits_golden() {
    let t = BalanceTable::builtin();
    let d = &t.derived;
    let mut h = crate::hash::StateHasher::new();
    for table in &d.cost {
        h.write_len(table.len());
        table.iter().for_each(|&m| h.write_money(m));
    }
    for table in [&d.track_pow_c, &d.track_pow_n] {
        h.write_len(table.len());
        table.iter().for_each(|&m| h.write_money(m));
    }
    assert_eq!(h.finish(), TABLES_GOLDEN, "{:#018x}", h.finish());
}

const TABLES_GOLDEN: u64 = 0xd69d_74a2_dad0_95bc;
