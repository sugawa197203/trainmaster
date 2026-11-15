use crate::command::{CommandError, CommandKind, LineEnd};
use crate::geom::GridPos;
use crate::ids::{CompanyLevel, LineId, StationId, TrainId};
use crate::param::{BuyMode, Feature, ParamKey, RebirthUpgrade, UpgradeScope};
use crate::time::Ticks;
use crate::units::{Money, People};

/// コアから外への通知。1 tick の中の順序は 10 §6.1 で固定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GameEvent {
    /// 路線を作った。
    LineCreated {
        /// 路線。
        line: LineId,
        /// 駅の数。
        station_count: u32,
        /// 払った線路の値段(10 TD-41。会社 Lv0 などでは 0)。
        cost: Money,
        /// 新しく線路を敷いたマスの数。
        new_tiles: u32,
    },
    /// 路線を延ばした。
    LineExtended {
        /// 路線。
        line: LineId,
        /// 延ばした端。
        end: LineEnd,
        /// 足した駅。
        station: StationId,
        /// 払った線路の値段(10 TD-41)。
        cost: Money,
        /// 新しく線路を敷いたマスの数。
        new_tiles: u32,
    },
    /// 列車が増えた(路線の作成と R1 の強化)。
    TrainAdded {
        /// 列車。
        train: TrainId,
        /// 路線。
        line: LineId,
    },
    /// 強化を買えた。
    Purchased {
        /// パラメータ。
        param: ParamKey,
        /// 対象。
        scope: UpgradeScope,
        /// 1 段か MAX か。
        mode: BuyMode,
        /// 買えた段の数。
        steps: u32,
        /// 上がったレベルの延べ数。
        raised: u32,
        /// 払った額。
        cost: Money,
    },
    /// 停車駅が通過駅になった [01 D-17]。
    PassThroughAdded {
        /// 路線。
        line: LineId,
        /// 駅。
        station: StationId,
    },
    /// 通過駅が停車駅になった。
    PassThroughResolved {
        /// 路線。
        line: LineId,
        /// 駅。
        station: StationId,
    },
    /// キューのコマンドを適用時に拒否した。
    CommandRejected {
        /// コマンドの種類。
        kind: CommandKind,
        /// 理由。
        reason: CommandError,
    },
    /// 会社レベルが 1 上がった。
    CompanyLevelUp {
        /// 新しい会社レベル。
        new_level: CompanyLevel,
        /// このレベルですべての `Feature` の解放がそろった。
        all_features_unlocked: bool,
    },
    /// 機能が解放された。
    FeatureUnlocked {
        /// 機能。
        feature: Feature,
        /// 解放された会社レベル。
        company_level: CompanyLevel,
    },
    /// 駅が出現した。
    StationSpawned {
        /// 駅。
        id: StationId,
        /// 位置。
        pos: GridPos,
    },
    /// 経路を再計算した。
    RoutesRecomputed {
        /// 経路グラフのノード数。
        node_count: u32,
    },
    /// 列車が駅に着いた(乗客 0 人・収入 0 でも出す)。
    TrainArrived {
        /// 列車。
        train: TrainId,
        /// 路線。
        line: LineId,
        /// 駅。
        station: StationId,
        /// 着いたときに乗っていた人数(払った人数)。
        riders: People,
        /// 降りた人数。
        alighted: People,
        /// 乗った人数。
        boarded: People,
        /// 運賃収入。
        income: Money,
        /// 停車時間。
        dwell: Ticks,
    },
    /// 所持金が初めて単位 k(1..=17)に届いた(10 TD-14)。
    MoneyUnitReached {
        /// 単位の番号。
        unit: u8,
    },
    /// 累計収入が初めて `10^goal_pow10` に届いた [01 D-08]。
    GoalReached {
        /// 届いたときの累計収入。
        total_earned: Money,
    },
    /// 転生した [T1]。会社は最初の状態に戻り、転生ポイントが増える。
    Rebirth {
        /// もらったポイント。
        points_earned: u64,
        /// 転生した回数(1 始まり)。
        count: u32,
    },
    /// 転生の強化を買えた [T1]。
    RebirthUpgraded {
        /// 強化。
        upgrade: RebirthUpgrade,
        /// 新しいレベル。
        new_level: u8,
    },
}
