use serde::{Deserialize, Serialize};

use crate::ids::{Level, LineId, StationId};

/// レベルを持つ 12 のパラメータ。宣言の順 = 01 §3.2 の表の順 = おすすめの同点の順 [01 D-25]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ParamKey {
    /// S1 お客さんが来る頻度。
    S1Spawn = 0,
    /// S2 お客さんが落とすお金。
    S2Fare,
    /// S3 お客さんの最大人数。
    S3StationCap,
    /// S4 ホーム長。
    S4Platform,
    /// T1 速度。
    T1Speed,
    /// T2 混雑率。
    T2Crowding,
    /// T3 乗り降り速度。
    T3Boarding,
    /// T4 1 車両の人数。
    T4CarCapacity,
    /// T5 両数。
    T5Cars,
    /// R1 1 路線の最大列車数。
    R1MaxTrains,
    /// R2 路線の数(会社全体)。
    R2LineSlots,
    /// R3 魅力度。
    R3Attractiveness,
}

impl ParamKey {
    /// 宣言の順のすべてのパラメータ。
    pub const ALL: [ParamKey; 12] = [
        ParamKey::S1Spawn,
        ParamKey::S2Fare,
        ParamKey::S3StationCap,
        ParamKey::S4Platform,
        ParamKey::T1Speed,
        ParamKey::T2Crowding,
        ParamKey::T3Boarding,
        ParamKey::T4CarCapacity,
        ParamKey::T5Cars,
        ParamKey::R1MaxTrains,
        ParamKey::R2LineSlots,
        ParamKey::R3Attractiveness,
    ];

    /// `ALL` の中の位置。
    pub const fn index(self) -> usize {
        self as usize
    }

    /// 持ち主(駅・路線・会社)。
    pub const fn owner(self) -> ParamOwner {
        match self {
            ParamKey::S1Spawn
            | ParamKey::S2Fare
            | ParamKey::S3StationCap
            | ParamKey::S4Platform => ParamOwner::Station,
            ParamKey::R2LineSlots => ParamOwner::Company,
            _ => ParamOwner::Line,
        }
    }

    /// 画面のタブ。
    pub const fn category(self) -> ParamCategory {
        match self {
            ParamKey::S1Spawn
            | ParamKey::S2Fare
            | ParamKey::S3StationCap
            | ParamKey::S4Platform => ParamCategory::Station,
            ParamKey::R1MaxTrains | ParamKey::R2LineSlots | ParamKey::R3Attractiveness => {
                ParamCategory::Line
            }
            _ => ParamCategory::Train,
        }
    }
}

/// 転生ポイントで買う恒久の強化(転生しても残る)。宣言の順 = 表示の順。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum RebirthUpgrade {
    /// 運賃の倍率。
    FareBoost = 0,
    /// 電車の数。
    TrainBoost,
    /// 路線の数。
    LineBoost,
}

impl RebirthUpgrade {
    /// 宣言の順のすべての強化。
    pub const ALL: [RebirthUpgrade; 3] = [
        RebirthUpgrade::FareBoost,
        RebirthUpgrade::TrainBoost,
        RebirthUpgrade::LineBoost,
    ];

    /// `ALL` の中の位置。
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// パラメータの持ち主。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParamOwner {
    /// 駅ごと。
    Station,
    /// 路線ごと。
    Line,
    /// 会社全体(R2 だけ)。
    Company,
}

/// 画面のタブ「えき」「電車」「ろせん」[01 §11.1]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ParamCategory {
    /// えき。
    Station,
    /// 電車。
    Train,
    /// ろせん。
    Line,
}

/// 駅ごとのレベル。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StationLevels {
    /// S1。
    pub spawn: Level,
    /// S2。
    pub fare: Level,
    /// S3。
    pub capacity: Level,
    /// S4。
    pub platform: Level,
}

impl StationLevels {
    /// パラメータのレベル。駅のパラメータ以外は `None`。
    pub fn get(&self, p: ParamKey) -> Option<Level> {
        self.slot(p).copied()
    }

    /// パラメータのレベルへの可変参照。駅のパラメータ以外は `None`。
    pub fn get_mut(&mut self, p: ParamKey) -> Option<&mut Level> {
        match p {
            ParamKey::S1Spawn => Some(&mut self.spawn),
            ParamKey::S2Fare => Some(&mut self.fare),
            ParamKey::S3StationCap => Some(&mut self.capacity),
            ParamKey::S4Platform => Some(&mut self.platform),
            _ => None,
        }
    }

    fn slot(&self, p: ParamKey) -> Option<&Level> {
        match p {
            ParamKey::S1Spawn => Some(&self.spawn),
            ParamKey::S2Fare => Some(&self.fare),
            ParamKey::S3StationCap => Some(&self.capacity),
            ParamKey::S4Platform => Some(&self.platform),
            _ => None,
        }
    }
}

/// 路線ごとのレベル(電車と路線のパラメータ。R2 を除く)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineLevels {
    /// T1。
    pub speed: Level,
    /// T2。
    pub crowding: Level,
    /// T3。
    pub boarding: Level,
    /// T4。
    pub car_capacity: Level,
    /// T5。
    pub cars: Level,
    /// R1。
    pub max_trains: Level,
    /// R3。
    pub attractiveness: Level,
}

impl LineLevels {
    /// パラメータのレベル。路線のパラメータ以外は `None`。
    pub fn get(&self, p: ParamKey) -> Option<Level> {
        self.slot(p).copied()
    }

    /// パラメータのレベルへの可変参照。路線のパラメータ以外は `None`。
    pub fn get_mut(&mut self, p: ParamKey) -> Option<&mut Level> {
        match p {
            ParamKey::T1Speed => Some(&mut self.speed),
            ParamKey::T2Crowding => Some(&mut self.crowding),
            ParamKey::T3Boarding => Some(&mut self.boarding),
            ParamKey::T4CarCapacity => Some(&mut self.car_capacity),
            ParamKey::T5Cars => Some(&mut self.cars),
            ParamKey::R1MaxTrains => Some(&mut self.max_trains),
            ParamKey::R3Attractiveness => Some(&mut self.attractiveness),
            _ => None,
        }
    }

    fn slot(&self, p: ParamKey) -> Option<&Level> {
        match p {
            ParamKey::T1Speed => Some(&self.speed),
            ParamKey::T2Crowding => Some(&self.crowding),
            ParamKey::T3Boarding => Some(&self.boarding),
            ParamKey::T4CarCapacity => Some(&self.car_capacity),
            ParamKey::T5Cars => Some(&self.cars),
            ParamKey::R1MaxTrains => Some(&self.max_trains),
            ParamKey::R3Attractiveness => Some(&self.attractiveness),
            _ => None,
        }
    }
}

/// 解放の対象 [01 §5.2]。`Ord` は `Param(S1)`…`Param(R3)`、`LineExtend`、`BulkBuy`、`Transfer`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Feature {
    /// そのパラメータを強化できる。
    Param(ParamKey),
    /// 路線の延伸。
    LineExtend,
    /// 「ぜんぶ」と MAX。
    BulkBuy,
    /// 乗り換え。表示と NEW のためだけで、何も制限しない(10 TD-13)。
    Transfer,
}

impl Feature {
    /// すべての `Feature` を `Ord` の順に返す。
    pub fn all_in_order() -> impl Iterator<Item = Feature> {
        ParamKey::ALL.into_iter().map(Feature::Param).chain([
            Feature::LineExtend,
            Feature::BulkBuy,
            Feature::Transfer,
        ])
    }

    /// `FeatureSet` のビット番号。
    pub const fn bit(self) -> u32 {
        match self {
            Feature::Param(p) => p as u32,
            Feature::LineExtend => 12,
            Feature::BulkBuy => 13,
            Feature::Transfer => 14,
        }
    }
}

/// `Feature` の集合(ビット番号は `Feature::bit`)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeatureSet(pub u32);

impl FeatureSet {
    /// 含むか。
    pub fn contains(self, f: Feature) -> bool {
        self.0 & (1 << f.bit()) != 0
    }

    /// 足す。
    pub fn insert(&mut self, f: Feature) {
        self.0 |= 1 << f.bit();
    }

    /// 和集合。
    pub fn union(self, other: FeatureSet) -> FeatureSet {
        FeatureSet(self.0 | other.0)
    }

    /// `other` に含まれるか。
    pub fn is_subset(self, other: FeatureSet) -> bool {
        self.0 & !other.0 == 0
    }
}

/// `ParamKey` の集合(ビット番号は `ParamKey::index`)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParamSet(pub u16);

impl ParamSet {
    /// 含むか。
    pub fn contains(self, p: ParamKey) -> bool {
        self.0 & (1 << p as u16) != 0
    }

    /// 足す。
    pub fn insert(&mut self, p: ParamKey) {
        self.0 |= 1 << p as u16;
    }

    /// 和集合。
    pub fn union(self, other: ParamSet) -> ParamSet {
        ParamSet(self.0 | other.0)
    }

    /// `other` に含まれるか。
    pub fn is_subset(self, other: ParamSet) -> bool {
        self.0 & !other.0 == 0
    }
}

/// 強化の対象の選び方。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpgradeScope {
    /// この えきだけ(駅のパラメータ)。
    Station(StationId),
    /// ぜんぶ(路線に入っている全駅)。
    AllStations,
    /// この ろせんだけ(電車・路線のパラメータ)。
    Line(LineId),
    /// ぜんぶ(全路線)。
    AllLines,
    /// 会社全体(R2 だけ)。
    Company,
}

/// 1 段だけ買うか、買えるだけ買うか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuyMode {
    /// 1 段。
    One,
    /// 買えなくなるか、上げられる対象がなくなるまで [01 D-02]。
    Max,
}

/// 上限の種類(エラーと UI の文言の出し分け)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    /// 固有上限(速度 Lv10 など)。
    Intrinsic,
    /// 会社レベルの上限 cap(C)。
    CompanyCap,
    /// 両数がホーム長に届いている [01 D-17]。
    Platform,
    /// 駅のお客さんの数(S1)が、駅で待てる人数(S3)のレベルに届いている(M9)。
    Capacity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_key_table() {
        for (i, p) in ParamKey::ALL.into_iter().enumerate() {
            assert_eq!(p.index(), i);
        }
        assert_eq!(ParamKey::S4Platform.owner(), ParamOwner::Station);
        assert_eq!(ParamKey::T5Cars.owner(), ParamOwner::Line);
        assert_eq!(ParamKey::R1MaxTrains.owner(), ParamOwner::Line);
        assert_eq!(ParamKey::R2LineSlots.owner(), ParamOwner::Company);
        assert_eq!(ParamKey::T1Speed.category(), ParamCategory::Train);
        assert_eq!(ParamKey::R2LineSlots.category(), ParamCategory::Line);
        assert_eq!(ParamKey::S2Fare.category(), ParamCategory::Station);
    }

    #[test]
    fn levels_cover_owner() {
        let mut s = StationLevels::default();
        let mut l = LineLevels::default();
        for p in ParamKey::ALL {
            assert_eq!(s.get(p).is_some(), p.owner() == ParamOwner::Station);
            assert_eq!(l.get(p).is_some(), p.owner() == ParamOwner::Line);
            if let Some(v) = s.get_mut(p) {
                *v = Level(p as u8 + 1);
            }
            if let Some(v) = l.get_mut(p) {
                *v = Level(p as u8 + 1);
            }
        }
        for p in ParamKey::ALL {
            let got = s.get(p).or(l.get(p));
            let expected = (p != ParamKey::R2LineSlots).then_some(Level(p as u8 + 1));
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn feature_order_and_bits() {
        let all: Vec<Feature> = Feature::all_in_order().collect();
        assert_eq!(all.len(), 15);
        assert!(all.windows(2).all(|w| w[0] < w[1]));
        for (i, f) in all.iter().enumerate() {
            assert_eq!(f.bit() as usize, i);
        }
        let mut set = FeatureSet::default();
        set.insert(Feature::Transfer);
        set.insert(Feature::Param(ParamKey::T1Speed));
        assert_eq!(set, FeatureSet((1 << 14) | (1 << 4)));
        assert!(set.contains(Feature::Transfer));
        assert!(!set.contains(Feature::BulkBuy));
        assert!(FeatureSet(1 << 4).is_subset(set));
        assert!(!set.is_subset(FeatureSet(1 << 4)));
        assert_eq!(FeatureSet(1).union(FeatureSet(2)), FeatureSet(3));

        let mut ps = ParamSet::default();
        ps.insert(ParamKey::R3Attractiveness);
        assert_eq!(ps, ParamSet(1 << 11));
        assert!(ps.contains(ParamKey::R3Attractiveness));
        assert!(ps.is_subset(ps.union(ParamSet(1))));
    }
}
