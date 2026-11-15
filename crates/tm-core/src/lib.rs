//! ゲームルールとシミュレーション(固定タイムステップ、コマンド、イベント、リプレイ)。

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod balance;
mod command;
mod event;
mod game;
mod geom;
mod hash;
mod ids;
mod invariants;
mod mapgen;
mod param;
mod recommend;
mod replay;
mod rng;
mod routing;
mod sim;
mod snapshot;
mod state;
mod time;
mod units;
mod view;

/// ルールの版。ルールを変えてゴールデン値を更新したら上げる(リプレイのヘッダと config hash に入る)。
/// 2 = 1 週間のペース・線路の値段・40×18 のマップ(17 GW-17)。
/// 3 = 駅の定員の伸び(S3 の `value_steps`)と、あとから出た駅・路線の強化の値段の下限(M8)。
/// 4 = 駅の S1 は S3 のレベルまで(M9)。5 = 固有上限のある設備にも値段の下限、下限は会社 Lv4 以降に出たものだけ(M10)。
/// 6 = 転生(T1)。state_hash に転生の状態を追加。
pub const RULES_VERSION: u32 = 6;

pub use balance::{
    BALANCE_FORMAT_VERSION, BUILTIN_BALANCE_RON, BalanceError, BalanceSpec, BalanceTable,
    CompanyRules, CostStep, EconomyRules, FeatureUnlock, MapRules, ParamDef, RebirthRules,
    RebirthUpgradeDef, TrackCostRules, TransitRules, UpgradeFloorRules, ValueFormula,
};
pub use command::{Command, CommandError, CommandKind, CommandPreview, LineEnd};
pub use event::GameEvent;
#[cfg(feature = "test-support")]
pub use game::PaxLedgerView;
pub use game::{Game, GameConfig};
pub use geom::{Bend, Dir8, GridPos, MapPoint, TrackMask, TrackSegment};
pub use hash::StateHasher;
pub use ids::{CompanyLevel, Level, LineId, StationId, TrainId};
pub use invariants::{InvariantViolation, MonotonicSnapshot, check_monotonic};
pub use param::{
    BuyMode, Feature, FeatureSet, LimitKind, LineLevels, ParamCategory, ParamKey, ParamOwner,
    ParamSet, RebirthUpgrade, StationLevels, UpgradeScope,
};
pub use recommend::{RecommendReason, Recommendation};
pub use replay::{
    CHECKPOINT_INTERVAL_TICKS, Checkpoint, REPLAY_FORMAT_VERSION, ReplayEntry, ReplayError,
    ReplayHeader, ReplayLog, replay,
};
pub use snapshot::{
    GameSnapshot, SNAPSHOT_FORMAT_VERSION, SnapCompany, SnapLine, SnapMatrix, SnapPhase,
    SnapProgress, SnapRebirth, SnapStation, SnapStats, SnapTrain, SnapshotError,
};
pub use state::INCOME_WINDOW_SECS;
pub use time::{POS_UNITS_PER_TILE, TICKS_PER_SEC, Tick, Ticks};
pub use units::{
    Cars, MilliPeople, Millis, Money, People, Permille, PosUnits, Quarter, RouteCost, SpeedMilli,
    Tiles, ceil_div, dwell_ms, fare_income, ms_to_ticks, ride_cost, round_div, section_length,
    spawn_per_sec, train_capacity,
};
pub use view::{
    CompanyView, GameView, GaugeView, LineView, RebirthUpgradeView, RebirthView, StationView,
    TrackGridView, TrainPhaseView, TrainView,
};
