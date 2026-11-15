//! ゲーム状態の内部構造(10 §3)。`Game` が唯一の所有者。

mod line;
mod pax;
mod progress;
mod station;
mod track;
mod train;

pub(crate) use line::{Line, Section};
pub(crate) use pax::PaxMatrix;
pub use progress::INCOME_WINDOW_SECS;
pub(crate) use progress::{CompanyState, Progress, RebirthState, Stats};
pub(crate) use station::Station;
pub(crate) use track::{TileBits, TrackGrid, lay_track, walk_track};
pub(crate) use train::{Direction, Train, TrainPhase};

use crate::geom::GridPos;
use crate::rng::RngStreams;
use crate::routing::Routing;
use crate::time::Tick;
use crate::units::Money;

/// 人数の保存のテスト用の台帳(11 §1.3、§6.5)。state_hash と不変条件の対象外。
#[cfg(feature = "test-support")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PaxLedger {
    pub spawned: u128,
    /// 定員のため生まれなかった人数(保存の等式に入れない)。
    pub capacity_loss: u128,
    pub delivered: u128,
    /// 降りて戻る乗客のうち、駅の待ち人数の上限で消えた人数(01 D-40)。
    pub waiting_cap_loss: u128,
}

/// ゲーム状態の全体。導出のフィールドは state_hash に入れない(10 §9.2)。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GameState {
    pub tick: Tick,
    pub money: Money,
    pub company: CompanyState,
    pub station_plan: Vec<GridPos>,
    pub stations: Vec<Station>,
    pub lines: Vec<Line>,
    pub trains: Vec<Train>,
    pub waiting: PaxMatrix,
    pub onboard: PaxMatrix,
    pub tracks: TrackGrid,
    pub progress: Progress,
    /// 転生の状態 [T1]。
    pub rebirth: RebirthState,
    pub stats: Stats,
    pub rng: RngStreams,
    /// 導出: 経路表と要再計算の印。
    pub routing: Routing,
    pub map_revision: u64,
    /// 人数の台帳(SIM-CR-08)。
    #[cfg(feature = "test-support")]
    pub ledger: PaxLedger,
}
