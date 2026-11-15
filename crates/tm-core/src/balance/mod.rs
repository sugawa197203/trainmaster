//! 数値バランス表(10 §4)。01 の数値はすべてここに置き、コードに直接書かない。

mod derived;
mod load;
mod validate;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use tm_numeric::BigNum;

use crate::ids::{CompanyLevel, Level, StationId};
use crate::param::{Feature, ParamKey, ParamOwner, RebirthUpgrade};
use crate::units::{Cars, MilliPeople, Money, People, Permille, Quarter, SpeedMilli};

pub(crate) use derived::BalanceDerived;
pub use load::BUILTIN_BALANCE_RON;
pub use validate::BalanceError;

/// `BalanceSpec` の形式の版。フィールドを変えたら上げる(config hash にも入る)。
/// 3 = `value_steps` と `economy.upgrade_floor`(M8)。4 = `transit.spawn_level_margin`(M9)。
/// 5 = `upgrade_floor.from_level`(M10)。6 = `rebirth`(転生)。
pub const BALANCE_FORMAT_VERSION: u32 = 6;

/// 検証済みの表。`from_spec` / `from_ron_str` でしか作れない。
#[derive(Clone, Debug, PartialEq)]
pub struct BalanceTable {
    spec: BalanceSpec,
    derived: BalanceDerived,
}

/// RON にそのまま対応する生の表(整数だけ)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceSpec {
    /// `BALANCE_FORMAT_VERSION` と同じ値。
    pub format_version: u32,
    /// 駅の生成。
    pub map: MapRules,
    /// 会社レベル。
    pub company: CompanyRules,
    /// ちょうど 12 個、`ParamKey` の順。
    pub params: Vec<ParamDef>,
    /// `Param` 以外の `Feature` の解放レベル。
    pub features: Vec<FeatureUnlock>,
    /// 運行と経路。
    pub transit: TransitRules,
    /// 経済。
    pub economy: EconomyRules,
    /// 転生。
    pub rebirth: RebirthRules,
}

/// 駅の生成 [01 §6.1 D-18]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapRules {
    /// マップの横のマス数。
    pub width: u8,
    /// マップの縦のマス数。
    pub height: u8,
    /// 駅を置かない外周のマス数。
    pub margin: u8,
    /// 最初の駅を選ぶマップ中央の範囲 (横, 縦)。
    pub initial_area: (u8, u8),
    /// 駅どうしの最小のチェビシェフ距離。
    pub min_spacing: u8,
    /// 2 駅目以降、いちばん近い駅までの最大距離。
    pub max_nearest: u8,
    /// 見つからないときに緩める間隔の順。
    pub relax_spacings: Vec<u8>,
    /// 間隔ごとの試行回数。
    pub attempts_per_stage: u32,
}

/// 会社レベル [01 §5 D-06 D-07]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanyRules {
    /// 会社レベルの最大。
    pub max_level: u8,
    /// `cap(C) = cap_base + cap_step × C` の `cap_base`。
    pub cap_base: u8,
    /// `cap(C)` の `cap_step`。
    pub cap_step: u8,
    /// ゲージのしきい値の分子。
    pub gauge_num: u32,
    /// ゲージのしきい値の分母。
    pub gauge_den: u32,
    /// 会社 Lv0 の駅の数。
    pub stations_base: u8,
    /// 会社レベル 1 つで増える駅の数。
    pub stations_per_level: u8,
}

/// パラメータ 1 つ [01 §3.1 §3.2]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamDef {
    /// どのパラメータか。
    pub key: ParamKey,
    /// `cost(L) = base × Π_{i<L} ratio(i)` の base(円)。
    pub base_yen: u64,
    /// `cost_steps` に当たらない段の比。
    pub cost_ratio: u32,
    /// 区間ごとの比(10 TD-43)。`ratio(i)` は `from ≤ i` の最後の要素の比。省略は空。
    #[serde(default)]
    pub cost_steps: Vec<CostStep>,
    /// 固有上限(なし = `None`)。
    pub intrinsic_max: Option<u8>,
    /// 解放される会社レベル。
    pub unlock: u8,
    /// レベルごとの値(10 §3.2 の内部単位)。
    pub value: ValueFormula,
    /// `Geometric` の値の区間ごとの比。`L → L+1` の比は `from ≤ L` の最後の要素の比、なければ `ratio`。省略は空。
    #[serde(default)]
    pub value_steps: Vec<CostStep>,
}

impl ParamDef {
    /// `L → L+1` の値の比(`Geometric` だけ。`Linear` は `None`)。
    pub fn value_ratio_at(&self, l: u8) -> Option<u64> {
        match self.value {
            ValueFormula::Linear { .. } => None,
            ValueFormula::Geometric { ratio, .. } => Some(
                self.value_steps
                    .iter()
                    .rev()
                    .find(|s| s.from <= l)
                    .map_or(ratio, |s| u64::from(s.ratio)),
            ),
        }
    }
}

/// コストの区間 1 つ: レベル `from` 以上の段の比。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostStep {
    /// 区間の始まりのレベル。
    pub from: u8,
    /// この区間の 1 段の比。
    pub ratio: u32,
}

/// レベルごとの値の式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueFormula {
    /// `v0 + step × L`。
    Linear {
        /// Lv0 の値。
        v0: u64,
        /// 1 レベルの増分。
        step: u64,
    },
    /// `v0 × ratio^L`(S2 は `Money` として計算する)。
    Geometric {
        /// Lv0 の値。
        v0: u64,
        /// 1 レベルの倍率。
        ratio: u64,
    },
}

/// `Param` 以外の `Feature` の解放レベル。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureUnlock {
    /// 対象。
    pub feature: Feature,
    /// 解放される会社レベル。
    pub unlock: u8,
}

/// 運行と経路 [01 §6.4 §7.2 D-13]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitRules {
    /// 停車時間の基本(ms)。
    pub dwell_base_ms: u32,
    /// 停車時間の上限(ms)。
    pub dwell_max_ms: u32,
    /// 走行コストの停車の目安(秒)。
    pub stop_allowance_s: u32,
    /// 乗り換え辺のコスト(秒 × 100)。
    pub transfer_cost: u64,
    /// 駅の待ち人数の合計の上限 = 定員 × この値 [01 D-40](10 TD-37)。
    pub waiting_cap_factor: u32,
    /// S3 の解放後、駅の S1 のレベルは S3 のレベル + この値まで(M9)。
    pub spawn_level_margin: u8,
}

/// 経済 [01 §4 D-08 D-20]。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomyRules {
    /// 初期所持金(円)。
    pub initial_money_yen: u64,
    /// ゴールの累計収入 `10^goal_pow10` 円。
    pub goal_pow10: u32,
    /// 線路の値段。
    pub track: TrackCostRules,
    /// あとから出た駅・作った路線の強化の値段の下限。
    pub upgrade_floor: UpgradeFloorRules,
}

/// あとから出た駅・作った路線の強化の値段の下限(M8、M10)。会社レベル `B`(`from_level` 以上)で生まれた駅・
/// 路線の、`B − behind` までに解放済みのパラメータが対象。基準の値段 = S1 の Lv `cap(B − behind)` の値段。
/// 固有上限のないものは Lv `cap(B − behind)` の値段より、固有上限のあるものは基準の値段より安くならない。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeFloorRules {
    /// 生まれた会社レベルから何レベル前の上限を下限のレベルにするか(1 以上)。
    pub behind: u8,
    /// この会社レベル以上で生まれた駅・路線だけが対象。
    pub from_level: u8,
}

/// 線路の値段 [01 §4.1 D-20](10 TD-41)。1 マス `c1 × level_ratio^(C−1) × tile_ratio^min(N, tile_cap)`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackCostRules {
    /// 会社 Lv1・N = 0 の 1 マスの値段(円)。
    pub c1_yen: u64,
    /// 会社レベル 1 つの倍率。
    pub level_ratio: u32,
    /// 線路 1 マスごとの倍率 (分子, 分母)。
    pub tile_ratio: (u32, u32),
    /// N の頭打ち。
    pub tile_cap: u32,
    /// 会社レベルがこれ未満の間はただ。
    pub free_below_level: u8,
}

/// 転生 [T1]。解放条件は累計収入がゴール(`10^goal_pow10` 円 = 1 無量大数)に届いたこと。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebirthRules {
    /// 転生 1 回のポイントの基礎点。
    pub points_base: u32,
    /// そのときの会社レベル 1 ごとに増えるポイント。
    pub points_per_level: u32,
    /// 運賃倍率の強化 1 レベルごとの倍率。
    pub fare_boost_ratio: u64,
    /// 電車の数の強化 1 レベルごとに増える電車。
    pub train_boost_trains: u32,
    /// 路線の数の強化 1 レベルごとに増える枠。
    pub line_boost_slots: u32,
    /// ちょうど 3 個、`RebirthUpgrade` の順。
    pub upgrades: Vec<RebirthUpgradeDef>,
}

/// 恒久の強化 1 つ [T1]。コストは `base_cost × cost_ratio^レベル` のポイント。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebirthUpgradeDef {
    /// どの強化か。
    pub key: RebirthUpgrade,
    /// レベルの最大。
    pub max: u8,
    /// Lv0 → Lv1 のコスト(ポイント)。
    pub base_cost: u32,
    /// 1 レベルごとの倍率。
    pub cost_ratio: u32,
}

impl BalanceTable {
    /// 検証して前計算する(10 §4.4)。
    pub fn from_spec(spec: BalanceSpec) -> Result<Self, BalanceError> {
        let derived = validate::validate(&spec)?;
        Ok(BalanceTable { spec, derived })
    }

    /// 生の表。
    pub fn spec(&self) -> &BalanceSpec {
        &self.spec
    }

    /// パラメータの定義。
    pub fn param(&self, p: ParamKey) -> &ParamDef {
        &self.spec.params[p.index()]
    }

    /// `from` → `from + 1` のコスト。
    /// `from ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn cost(&self, p: ParamKey, from: Level) -> Money {
        self.derived.cost[p.index()][usize::from(from.0)]
    }

    /// 値段の下限の基準の会社レベル `B − behind`。`born` で生まれた `p` が対象でなければ `None`。
    fn floor_company(&self, p: ParamKey, born: CompanyLevel) -> Option<CompanyLevel> {
        let rules = &self.spec.economy.upgrade_floor;
        let c = born.0.checked_sub(rules.behind)?;
        let owned = p.owner() != ParamOwner::Company;
        (owned && born.0 >= rules.from_level && self.param(p).unlock <= c)
            .then_some(CompanyLevel(c))
    }

    /// 会社レベル `born` で生まれた駅・路線の `p`(固有上限なし)の値段の下限のレベル。下限がなければ 0。
    pub fn floor_level(&self, p: ParamKey, born: CompanyLevel) -> Level {
        match self.floor_company(p, born) {
            Some(c) if self.param(p).intrinsic_max.is_none() => Level(self.cap(c)),
            _ => Level::ZERO,
        }
    }

    /// 会社レベル `born` で生まれた駅・路線の `p` を `from` → `from + 1` にする値段(下限つき。`UpgradeFloorRules`)。
    pub fn upgrade_cost(&self, p: ParamKey, from: Level, born: CompanyLevel) -> Money {
        let def = self.param(p);
        let base = self.cost(p, from.max(self.floor_level(p, born)));
        let Some(c) = self
            .floor_company(p, born)
            .filter(|_| def.intrinsic_max.is_some())
        else {
            return base;
        };
        let era = self.cost(ParamKey::S1Spawn, Level(self.cap(c)));
        if era.total_cmp(&base) == core::cmp::Ordering::Greater {
            era
        } else {
            base
        }
    }

    /// 駅が出る会社レベル(最初の `stations_base` 駅は 0、そのあとは 1 レベルに `stations_per_level` 駅)。
    pub fn station_born_level(&self, id: StationId) -> CompanyLevel {
        let c = &self.spec.company;
        let Some(late) = id.0.checked_sub(u32::from(c.stations_base)) else {
            return CompanyLevel::ZERO;
        };
        let lv = late / u32::from(c.stations_per_level.max(1)) + 1;
        CompanyLevel(u8::try_from(lv).unwrap_or(u8::MAX))
    }

    /// 会社レベル `c`、敷く前の線路のマス数 `n_before` で、新しいマスを `new_tiles` 個敷く値段(10 TD-41)。
    /// `c ≤ max_level` が前提(超えると panic)。
    pub fn track_cost(&self, c: CompanyLevel, n_before: u32, new_tiles: u32) -> Money {
        self.derived
            .track_cost(&self.spec.economy.track, c, n_before, new_tiles)
    }

    /// `min(固有上限, cap(C))`。
    pub fn limit(&self, p: ParamKey, c: CompanyLevel) -> Level {
        derived::limit(&self.spec, p, c)
    }

    /// `cap_base + cap_step × C`。
    pub fn cap(&self, c: CompanyLevel) -> u8 {
        derived::cap(&self.spec.company, c)
    }

    /// 解放される会社レベル。
    pub fn unlock_level(&self, f: Feature) -> CompanyLevel {
        match f {
            Feature::Param(p) => CompanyLevel(self.param(p).unlock),
            _ => self.derived.feature_unlock(f),
        }
    }

    pub(crate) fn value_u64(&self, p: ParamKey, l: Level) -> u64 {
        self.derived.values[p.index()][usize::from(l.0)]
    }

    fn value_u32(&self, p: ParamKey, l: Level) -> u32 {
        u32::try_from(self.value_u64(p, l)).expect("検証済みの表では u32 に収まる")
    }

    /// S1 お客さんが来る頻度(1/1000 人/秒)。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn spawn_rate(&self, l: Level) -> MilliPeople {
        MilliPeople(self.value_u64(ParamKey::S1Spawn, l))
    }

    /// S2 運賃(円/人・回)。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn fare(&self, l: Level) -> Money {
        self.derived.fare[usize::from(l.0)]
    }

    /// S3 駅の定員。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn station_capacity(&self, l: Level) -> People {
        People(self.value_u64(ParamKey::S3StationCap, l))
    }

    /// S4 ホーム長。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn platform(&self, l: Level) -> Cars {
        Cars(self.value_u32(ParamKey::S4Platform, l))
    }

    /// T1 速度。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn speed(&self, l: Level) -> SpeedMilli {
        SpeedMilli(self.value_u32(ParamKey::T1Speed, l))
    }

    /// T2 混雑率。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn crowding(&self, l: Level) -> Quarter {
        Quarter(self.value_u32(ParamKey::T2Crowding, l))
    }

    /// T3 乗り降り速度(人/秒)。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn boarding_rate(&self, l: Level) -> People {
        People(self.value_u64(ParamKey::T3Boarding, l))
    }

    /// T4 1 車両の人数。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn car_capacity(&self, l: Level) -> People {
        People(self.value_u64(ParamKey::T4CarCapacity, l))
    }

    /// T5 両数。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn cars(&self, l: Level) -> Cars {
        Cars(self.value_u32(ParamKey::T5Cars, l))
    }

    /// R1 1 路線の最大列車数。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn max_trains(&self, l: Level) -> u32 {
        self.value_u32(ParamKey::R1MaxTrains, l)
    }

    /// R2 路線の数の上限。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn line_slots(&self, l: Level) -> u32 {
        self.value_u32(ParamKey::R2LineSlots, l)
    }

    /// R3 魅力度。
    /// `l ≤ limit(p, max_level)` が前提(超えると panic)。
    pub fn attractiveness(&self, l: Level) -> Permille {
        Permille(self.value_u32(ParamKey::R3Attractiveness, l))
    }

    /// 駅の数の最大(01 では 27)。
    pub fn max_stations(&self) -> u32 {
        self.derived.max_stations
    }

    /// 路線の数の最大(01 では 6)。
    pub fn max_lines(&self) -> u32 {
        self.derived.max_lines
    }

    /// ゴールの累計収入 `10^goal_pow10`。
    pub fn goal(&self) -> Money {
        self.derived.goal
    }

    /// 初期所持金。
    pub fn initial_money(&self) -> Money {
        BigNum::from_u64(self.spec.economy.initial_money_yen)
    }

    /// 転生 1 回でもらえるポイント(そのときの会社レベルで決まる)[T1]。
    pub fn rebirth_points(&self, c: CompanyLevel) -> u64 {
        let r = &self.spec.rebirth;
        u64::from(r.points_base) + u64::from(r.points_per_level) * u64::from(c.0)
    }

    /// 恒久の強化 1 つの定義。
    pub fn rebirth_upgrade(&self, k: RebirthUpgrade) -> &RebirthUpgradeDef {
        &self.spec.rebirth.upgrades[k.index()]
    }

    /// 恒久の強化の `from` → `from + 1` のコスト(ポイント)。`from < max` が前提。
    pub fn rebirth_cost(&self, k: RebirthUpgrade, from: u8) -> u64 {
        let def = self.rebirth_upgrade(k);
        debug_assert!(
            from < def.max,
            "rebirth_cost: {k:?} のレベル {from} は最大以上"
        );
        u64::from(def.cost_ratio)
            .checked_pow(u32::from(from))
            .and_then(|r| u64::from(def.base_cost).checked_mul(r))
            .expect("検証済みの表ではあふれない")
    }

    /// 運賃の倍率の強化の `level` レベル分の倍率(`fare_boost_ratio^level`)。
    pub fn fare_boost_mul(&self, level: u8) -> u64 {
        self.spec
            .rebirth
            .fare_boost_ratio
            .checked_pow(u32::from(level))
            .expect("検証済みの表ではあふれない")
    }

    /// 電車の数の強化の `level` レベル分の増加。
    pub fn train_boost_trains(&self, level: u8) -> u32 {
        self.spec.rebirth.train_boost_trains * u32::from(level)
    }

    /// 路線の数の強化の `level` レベル分の増加。
    pub fn line_boost_slots(&self, level: u8) -> u32 {
        self.spec.rebirth.line_boost_slots * u32::from(level)
    }
}
