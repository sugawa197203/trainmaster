use crate::ids::{CompanyLevel, Level};
use crate::param::{FeatureSet, ParamSet, RebirthUpgrade};
use crate::units::Money;

/// 「1びょうで ○円」の平均をとる秒数(10 TD-15)。
pub const INCOME_WINDOW_SECS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompanyState {
    pub level: CompanyLevel,
    /// R2。路線の数の上限 = `line_slots(L)`。
    pub line_slots: Level,
    /// これまでに線路を敷いたマスの数 N(10 TD-41)。`TrackGrid` の空でないマスの数(INV-20)。
    pub track_tiles: u32,
}

/// 転生の状態 [T1]。転生してもリセットされない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RebirthState {
    /// 転生した回数。
    pub count: u32,
    /// 持っている転生ポイント。
    pub points: u64,
    /// 運賃の倍率の強化のレベル。
    pub fare_boost: u8,
    /// 電車の数の強化のレベル。
    pub train_boost: u8,
    /// 路線の数の強化のレベル。
    pub line_boost: u8,
}

impl RebirthState {
    /// 強化のレベル。
    pub fn level(self, k: RebirthUpgrade) -> u8 {
        match k {
            RebirthUpgrade::FareBoost => self.fare_boost,
            RebirthUpgrade::TrainBoost => self.train_boost,
            RebirthUpgrade::LineBoost => self.line_boost,
        }
    }

    /// 強化のレベルを書く。
    pub fn set_level(&mut self, k: RebirthUpgrade, level: u8) {
        match k {
            RebirthUpgrade::FareBoost => self.fare_boost = level,
            RebirthUpgrade::TrainBoost => self.train_boost = level,
            RebirthUpgrade::LineBoost => self.line_boost = level,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Progress {
    pub unlocked: FeatureSet,
    pub upgraded_once: ParamSet,
    pub total_earned: Money,
    /// 所持金が届いたことのある最大の単位 k(0..=17。10 TD-14)。
    pub max_money_unit: u8,
    pub goal_reached: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stats {
    pub income_this_sec: Money,
    pub income_window: [Money; INCOME_WINDOW_SECS],
    pub window_head: u8,
    pub arrivals: u64,
    pub delivered: u128,
    /// 成功した `LevelUp` コマンドの数。
    pub purchases: u32,
}
