//! 出来事から音への対応表 [10 §6.3、01 §12.2〜12.3]。

use tm_assets::sound::{
    BGM_GAME_1, BGM_GAME_2, BGM_GAME_3, JINGLE_ALL_UNLOCKED, JINGLE_COMPANY_UP, JINGLE_GOAL,
    SE_BUY, SE_BUY_MAX, SE_CANCEL, SE_CANNOT_BUY, SE_COIN, SE_CONFIRM, SE_CURSOR, SE_ERROR,
    SE_LINE_EXTEND, SE_LINE_NEW, SE_MENU_CLOSE, SE_MENU_OPEN, SE_POPUP, SE_RESET, SE_STATION_NEW,
    SE_UNIT_UP,
};
use tm_core::{BuyMode, CommandError, GameEvent};
use tm_platform_api::SoundId;

/// 効果音の優先度 [01 §12.2]。宣言の順が高い順。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SePriority {
    /// 操作への返事。
    Response = 1,
    /// 出来事。
    Event = 2,
    /// `se_coin`。
    Coin = 3,
}

impl SePriority {
    /// `other` より優先度が高い(数が小さい)。
    pub(crate) fn is_higher_than(self, other: SePriority) -> bool {
        self < other
    }
}

/// 音の ID → 優先度。BGM とジングルは `None`。
pub(crate) fn se_priority(id: SoundId) -> Option<SePriority> {
    match id {
        SE_CONFIRM | SE_CANCEL | SE_CURSOR | SE_BUY | SE_BUY_MAX | SE_CANNOT_BUY | SE_ERROR
        | SE_MENU_OPEN | SE_MENU_CLOSE | SE_RESET => Some(SePriority::Response),
        SE_STATION_NEW | SE_LINE_NEW | SE_LINE_EXTEND | SE_UNIT_UP | SE_POPUP => {
            Some(SePriority::Event)
        }
        SE_COIN => Some(SePriority::Coin),
        _ => None,
    }
}

/// 同じ ID の間引きの最短間隔(ms)。`se_coin` は 150、ほかは 50。
pub(crate) fn se_min_interval_ms(id: SoundId) -> u16 {
    if id == SE_COIN { 150 } else { 50 }
}

/// 鳴らし方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SoundRequest {
    /// 効果音。
    Se(SoundId),
    /// 出したジングルが終わるまで遅らせる効果音(`se_station_new`)。
    SeAfterJingles(SoundId),
    /// ジングル。
    Jingle(SoundId),
    /// ジングルの後に曲を頭から流す(会社 Lv5・Lv14)。
    JingleThenBgm(SoundId, SoundId),
}

/// 段階の曲を流す時間(µs)[14 PL-D-31]。
const OWN_US: u64 = 360_000_000;
/// 前の段階の曲を流す時間(µs)。
const PREV_US: u64 = 180_000_000;
/// 曲の順番で次の曲へ移るときのクロスフェード。
pub(crate) const BGM_ROTATION_CROSSFADE_MS: u16 = 2000;

/// 段階ごとの曲の並びと流す時間 [14 §6.5]。先頭がその段階の曲。
const BGM_ROTATIONS: [&[(SoundId, u64)]; 3] = [
    &[(BGM_GAME_1, OWN_US)],
    &[(BGM_GAME_2, OWN_US), (BGM_GAME_1, PREV_US)],
    &[
        (BGM_GAME_3, OWN_US),
        (BGM_GAME_1, PREV_US),
        (BGM_GAME_3, OWN_US),
        (BGM_GAME_2, PREV_US),
    ],
];

/// 会社レベル → BGM の段階(0〜2)[01 §12.3]。
pub(crate) fn bgm_stage(level: u8) -> usize {
    match level {
        0..=4 => 0,
        5..=13 => 1,
        _ => 2,
    }
}

/// 段階の曲の並び。
pub(crate) fn bgm_rotation(stage: usize) -> &'static [(SoundId, u64)] {
    BGM_ROTATIONS[stage.min(BGM_ROTATIONS.len() - 1)]
}

/// 会社レベル → 段階の曲。
pub(crate) fn bgm_for_level(level: u8) -> SoundId {
    bgm_rotation(bgm_stage(level))[0].0
}

/// `GameEvent` → 鳴らす音。鳴らさないなら `None`。
pub(crate) fn sound_for_event(ev: &GameEvent) -> Option<SoundRequest> {
    use SoundRequest::{Jingle, JingleThenBgm, Se, SeAfterJingles};
    match *ev {
        GameEvent::TrainArrived { income, .. } => (!income.is_zero()).then_some(Se(SE_COIN)),
        GameEvent::Purchased {
            mode: BuyMode::Max,
            steps: 2..,
            ..
        } => Some(Se(SE_BUY_MAX)),
        GameEvent::Purchased { .. } => Some(Se(SE_BUY)),
        GameEvent::CommandRejected { reason, .. } => Some(Se(sound_for_error(&reason))),
        GameEvent::StationSpawned { .. } => Some(SeAfterJingles(SE_STATION_NEW)),
        GameEvent::LineCreated { .. } => Some(Se(SE_LINE_NEW)),
        GameEvent::LineExtended { .. } => Some(Se(SE_LINE_EXTEND)),
        GameEvent::PassThroughAdded { .. } => Some(Se(SE_POPUP)),
        GameEvent::MoneyUnitReached { .. } => Some(Se(SE_UNIT_UP)),
        GameEvent::CompanyLevelUp {
            new_level,
            all_features_unlocked,
        } => {
            let jingle = if all_features_unlocked {
                JINGLE_ALL_UNLOCKED
            } else {
                JINGLE_COMPANY_UP
            };
            let bgm = bgm_for_level(new_level.0);
            if new_level.0 > 0 && bgm != bgm_for_level(new_level.0 - 1) {
                Some(JingleThenBgm(jingle, bgm))
            } else {
                Some(Jingle(jingle))
            }
        }
        GameEvent::GoalReached { .. } => Some(Jingle(JINGLE_GOAL)),
        GameEvent::Rebirth { .. } => Some(SeAfterJingles(SE_RESET)),
        GameEvent::RebirthUpgraded { .. } => Some(Se(SE_BUY)),
        GameEvent::TrainAdded { .. }
        | GameEvent::PassThroughResolved { .. }
        | GameEvent::FeatureUnlocked { .. }
        | GameEvent::RoutesRecomputed { .. } => None,
    }
}

/// `CommandError` → 効果音。お金とポイントが足りないときだけ `se_cannot_buy`。
pub(crate) fn sound_for_error(err: &CommandError) -> SoundId {
    match err {
        CommandError::InsufficientFunds { .. } | CommandError::RebirthUpgradePoints { .. } => {
            SE_CANNOT_BUY
        }
        _ => SE_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use tm_assets::sound::{SOUND_COUNT, SOUND_ENTRIES, name};
    use tm_core::{
        CommandKind, CompanyLevel, Feature, GridPos, Level, LimitKind, LineEnd, LineId, Money,
        ParamKey, People, StationId, Ticks, TrainId, UpgradeScope,
    };
    use tm_platform_api::SoundKind;

    use super::*;
    use SoundRequest::{Jingle, JingleThenBgm, Se, SeAfterJingles};

    fn sid(name_: &str) -> SoundId {
        let i = SOUND_ENTRIES
            .iter()
            .position(|e| e.name == name_)
            .expect("音がある");
        SoundId(i as u16)
    }

    #[test]
    fn priority_table_matches_01() {
        let table: [(&str, SePriority); 16] = [
            ("se_confirm", SePriority::Response),
            ("se_cancel", SePriority::Response),
            ("se_cursor", SePriority::Response),
            ("se_buy", SePriority::Response),
            ("se_buy_max", SePriority::Response),
            ("se_cannot_buy", SePriority::Response),
            ("se_error", SePriority::Response),
            ("se_menu_open", SePriority::Response),
            ("se_menu_close", SePriority::Response),
            ("se_reset", SePriority::Response),
            ("se_station_new", SePriority::Event),
            ("se_line_new", SePriority::Event),
            ("se_line_extend", SePriority::Event),
            ("se_unit_up", SePriority::Event),
            ("se_popup", SePriority::Event),
            ("se_coin", SePriority::Coin),
        ];
        for (n, p) in table {
            assert_eq!(se_priority(sid(n)), Some(p), "{n}");
        }
        let mut se = 0;
        for (i, e) in SOUND_ENTRIES.iter().enumerate() {
            let is_se = e.kind == SoundKind::Se;
            assert_eq!(
                se_priority(SoundId(i as u16)).is_some(),
                is_se,
                "{}",
                e.name
            );
            se += usize::from(is_se);
        }
        assert_eq!(se, 16);
        let count = |k| SOUND_ENTRIES.iter().filter(|e| e.kind == k).count();
        assert_eq!((count(SoundKind::Bgm), count(SoundKind::Jingle)), (4, 3));
        assert_eq!(se_priority(SoundId(SOUND_COUNT as u16)), None);
    }

    #[test]
    fn priority_order() {
        use SePriority::{Coin, Event, Response};
        assert!(Response.is_higher_than(Event) && Event.is_higher_than(Coin));
        assert!(Response.is_higher_than(Coin));
        for p in [Response, Event, Coin] {
            assert!(!p.is_higher_than(p));
        }
        assert!(!Coin.is_higher_than(Event) && !Event.is_higher_than(Response));
    }

    #[test]
    fn min_interval() {
        for i in 0..SOUND_COUNT {
            let id = SoundId(i as u16);
            if se_priority(id).is_some() {
                let want = if id == SE_COIN { 150 } else { 50 };
                assert_eq!(se_min_interval_ms(id), want, "{:?}", name(id));
            }
        }
    }

    #[test]
    fn bgm_by_company_level() {
        for lv in 0..=24u8 {
            let (stage, want) = match lv {
                0..=4 => (0, "bgm_game_1"),
                5..=13 => (1, "bgm_game_2"),
                _ => (2, "bgm_game_3"),
            };
            assert_eq!(bgm_stage(lv), stage, "Lv{lv}");
            assert_eq!(bgm_for_level(lv), sid(want), "Lv{lv}");
        }
        assert_eq!(bgm_for_level(u8::MAX), sid("bgm_game_3"));
    }

    #[test]
    fn bgm_rotation_order_and_time() {
        let names = |stage| -> Vec<(&str, u64)> {
            bgm_rotation(stage)
                .iter()
                .map(|&(id, us)| (name(id).expect("名前"), us / 1_000_000))
                .collect()
        };
        assert_eq!(names(0), [("bgm_game_1", 360)]);
        assert_eq!(names(1), [("bgm_game_2", 360), ("bgm_game_1", 180)]);
        assert_eq!(
            names(2),
            [
                ("bgm_game_3", 360),
                ("bgm_game_1", 180),
                ("bgm_game_3", 360),
                ("bgm_game_2", 180)
            ]
        );
        assert_eq!(bgm_rotation(3), bgm_rotation(2));
        assert_eq!(BGM_ROTATION_CROSSFADE_MS, 2000);
        for stage in 0..3 {
            let seq = bgm_rotation(stage);
            for i in 0..seq.len() {
                let next = seq[(i + 1) % seq.len()].0;
                assert!(seq.len() == 1 || seq[i].0 != next, "同じ曲を続けない");
            }
        }
    }

    fn level_up(lv: u8, all: bool) -> GameEvent {
        GameEvent::CompanyLevelUp {
            new_level: CompanyLevel(lv),
            all_features_unlocked: all,
        }
    }

    #[test]
    fn company_level_up_jingles() {
        for lv in 1..=24u8 {
            let all = lv == 4;
            let j = if all {
                sid("jingle_all_unlocked")
            } else {
                sid("jingle_company_up")
            };
            let want = match lv {
                5 => JingleThenBgm(j, sid("bgm_game_2")),
                14 => JingleThenBgm(j, sid("bgm_game_3")),
                _ => Jingle(j),
            };
            assert_eq!(sound_for_event(&level_up(lv, all)), Some(want), "Lv{lv}");
        }
        assert_eq!(
            sound_for_event(&level_up(0, false)),
            Some(Jingle(sid("jingle_company_up")))
        );
        assert_eq!(
            sound_for_event(&level_up(24, false)),
            Some(Jingle(sid("jingle_company_up")))
        );
    }

    fn arrived(riders: u64, income: Money) -> GameEvent {
        GameEvent::TrainArrived {
            train: TrainId(0),
            line: LineId(0),
            station: StationId(1),
            riders: People(riders),
            alighted: People(0),
            boarded: People(0),
            income,
            dwell: Ticks(60),
        }
    }

    fn purchased(mode: BuyMode, steps: u32) -> GameEvent {
        GameEvent::Purchased {
            param: ParamKey::S2Fare,
            scope: UpgradeScope::AllStations,
            mode,
            steps,
            raised: steps,
            cost: Money::ONE,
        }
    }

    #[test]
    fn events_to_sounds() {
        let line = LineId(0);
        let st = StationId(2);
        let cases = [
            (arrived(3, Money::from_u64(3)), Some(Se(sid("se_coin")))),
            (arrived(0, Money::ZERO), None),
            (purchased(BuyMode::One, 1), Some(Se(sid("se_buy")))),
            (purchased(BuyMode::Max, 1), Some(Se(sid("se_buy")))),
            (purchased(BuyMode::Max, 2), Some(Se(sid("se_buy_max")))),
            (purchased(BuyMode::Max, 9), Some(Se(sid("se_buy_max")))),
            (
                GameEvent::StationSpawned {
                    id: st,
                    pos: GridPos { x: 1, y: 1 },
                },
                Some(SeAfterJingles(sid("se_station_new"))),
            ),
            (
                GameEvent::LineCreated {
                    line,
                    station_count: 2,
                    cost: tm_core::Money::ZERO,
                    new_tiles: 0,
                },
                Some(Se(sid("se_line_new"))),
            ),
            (
                GameEvent::LineExtended {
                    line,
                    end: LineEnd::Tail,
                    station: st,
                    cost: tm_core::Money::ZERO,
                    new_tiles: 0,
                },
                Some(Se(sid("se_line_extend"))),
            ),
            (
                GameEvent::PassThroughAdded { line, station: st },
                Some(Se(sid("se_popup"))),
            ),
            (
                GameEvent::MoneyUnitReached { unit: 1 },
                Some(Se(sid("se_unit_up"))),
            ),
            (
                GameEvent::GoalReached {
                    total_earned: Money::ONE,
                },
                Some(Jingle(sid("jingle_goal"))),
            ),
            (
                GameEvent::TrainAdded {
                    train: TrainId(0),
                    line,
                },
                None,
            ),
            (GameEvent::PassThroughResolved { line, station: st }, None),
            (
                GameEvent::FeatureUnlocked {
                    feature: Feature::LineExtend,
                    company_level: CompanyLevel(2),
                },
                None,
            ),
            (GameEvent::RoutesRecomputed { node_count: 3 }, None),
        ];
        for (ev, want) in cases {
            assert_eq!(sound_for_event(&ev), want, "{ev:?}");
        }
    }

    #[test]
    fn errors_to_sounds() {
        let funds = CommandError::InsufficientFunds {
            required: Money::from_u64(10),
            available: Money::ONE,
        };
        assert_eq!(sound_for_error(&funds), sid("se_cannot_buy"));
        let others = [
            CommandError::UnknownStation(StationId(9)),
            CommandError::UnknownLine(LineId(9)),
            CommandError::TooFewStations { got: 1 },
            CommandError::DuplicateStation(StationId(1)),
            CommandError::NoFreeLineSlot { used: 1, slots: 1 },
            CommandError::StationNotConnected(StationId(1)),
            CommandError::NoTarget {
                param: ParamKey::S2Fare,
            },
            CommandError::AtLevelLimit {
                param: ParamKey::S2Fare,
                limit: Level(10),
                kind: LimitKind::Platform,
            },
        ];
        for e in others {
            assert_eq!(sound_for_error(&e), sid("se_error"), "{e:?}");
        }
        let kind = CommandKind::LevelUp {
            param: ParamKey::S2Fare,
            mode: BuyMode::One,
        };
        for (reason, want) in [(funds, "se_cannot_buy"), (others[0], "se_error")] {
            assert_eq!(
                sound_for_event(&GameEvent::CommandRejected { kind, reason }),
                Some(Se(sid(want)))
            );
        }
    }
}
