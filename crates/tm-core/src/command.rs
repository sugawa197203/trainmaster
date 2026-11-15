use serde::{Deserialize, Serialize};

use crate::geom::Bend;
use crate::ids::{CompanyLevel, Level, LineId, StationId};
use crate::param::{BuyMode, Feature, LimitKind, ParamKey, RebirthUpgrade, UpgradeScope};
use crate::units::Money;

/// プレイヤーの操作(コアの状態を変えるものだけ。10 §5.1)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// 路線を作る。`bends[i]` は `stations[i]` → `stations[i+1]` の曲がり方。
    CreateLine {
        /// 選んだ順の駅(2 個以上、重複なし)。
        stations: Vec<StationId>,
        /// `stations.len() − 1` 個の曲がり方。
        bends: Vec<Bend>,
    },
    /// 路線の端に駅を 1 つ足す。
    ExtendLine {
        /// 延ばす路線。
        line: LineId,
        /// どちらの端か。
        end: LineEnd,
        /// 足す駅。
        station: StationId,
        /// 既存の端の駅から新しい駅へ向かう曲がり方。
        bend: Bend,
    },
    /// 強化。
    LevelUp {
        /// 上げるパラメータ。
        param: ParamKey,
        /// 対象。
        scope: UpgradeScope,
        /// 1 段か MAX か。
        mode: BuyMode,
    },
    /// 転生する (T1)。会社は最初からやり直し、転生ポイントをもらう。
    Rebirth,
    /// 転生ポイントで恒久の強化を買う (T1)。
    RebirthUpgrade {
        /// 買う強化。
        upgrade: RebirthUpgrade,
    },
}

/// 路線の端。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnd {
    /// `stations[0]` 側。
    Head,
    /// 末尾の側。
    Tail,
}

/// イベントとチュートリアル用のコマンドの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// `Command::CreateLine`。
    CreateLine,
    /// `Command::ExtendLine`。
    ExtendLine,
    /// `Command::LevelUp`。
    LevelUp {
        /// 上げるパラメータ。
        param: ParamKey,
        /// 1 段か MAX か。
        mode: BuyMode,
    },
    /// `Command::Rebirth`。
    Rebirth,
    /// `Command::RebirthUpgrade`。
    RebirthUpgrade {
        /// 買う強化。
        upgrade: RebirthUpgrade,
    },
}

impl Command {
    /// コマンドの種類。
    pub fn kind(&self) -> CommandKind {
        match self {
            Command::CreateLine { .. } => CommandKind::CreateLine,
            Command::ExtendLine { .. } => CommandKind::ExtendLine,
            Command::LevelUp { param, mode, .. } => CommandKind::LevelUp {
                param: *param,
                mode: *mode,
            },
            Command::Rebirth => CommandKind::Rebirth,
            Command::RebirthUpgrade { upgrade } => {
                CommandKind::RebirthUpgrade { upgrade: *upgrade }
            }
        }
    }
}

/// `validate` の結果(UI のコスト表示とグレー表示)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandPreview {
    /// いま実行したら払う額(MAX なら買える段の合計。路線のコマンドは線路の値段。10 TD-41)。
    pub cost: Money,
    /// 買える段の数(One は 1。路線のコマンドは 0)。
    pub steps: u32,
    /// 上がるレベルの延べ数。
    pub raised: u32,
    /// 新しい区間の線路の長さの合計(マス。既存の線路と重なる分を含む)。強化は 0。
    pub track_tiles: u32,
    /// そのうち、いま線路がないマスの数(値段がかかるマス。重複は 1 回)。強化は 0。
    pub new_track_tiles: u32,
    /// 新しく通過駅になる駅の数。
    pub new_pass_through: u32,
}

/// コマンドを拒否した理由。検査の順は 10 §5.3(存在と形式 → 解放 → 対象 → お金)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CommandError {
    /// 出現していない駅。
    UnknownStation(StationId),
    /// 存在しない路線。
    UnknownLine(LineId),
    /// `scope` がパラメータの持ち主と合わない。
    ScopeMismatch {
        /// パラメータ。
        param: ParamKey,
        /// 指定された対象。
        scope: UpgradeScope,
    },
    /// `bends` の数が `stations.len() − 1` でない。
    BendCountMismatch {
        /// 期待した数。
        expected: u32,
        /// 実際の数。
        got: u32,
    },
    /// `CreateLine` の駅が 2 未満。
    TooFewStations {
        /// 実際の数。
        got: u32,
    },
    /// 同じ駅をもう一度選んだ。
    DuplicateStation(StationId),
    /// 延伸先がすでにその路線にある。
    StationAlreadyOnLine {
        /// 路線。
        line: LineId,
        /// 駅。
        station: StationId,
    },
    /// 未解放。
    Locked {
        /// 必要な機能。
        feature: Feature,
        /// 解放される会社レベル。
        required: CompanyLevel,
    },
    /// 路線の空き枠がない。
    NoFreeLineSlot {
        /// 使っている数。
        used: u32,
        /// 枠の数。
        slots: u32,
    },
    /// 路線に入っていない駅は強化できない [01 D-26]。
    StationNotConnected(StationId),
    /// 「ぜんぶ」の対象が 1 つもない。
    NoTarget {
        /// パラメータ。
        param: ParamKey,
    },
    /// 上限に届いている。
    AtLevelLimit {
        /// パラメータ。
        param: ParamKey,
        /// その対象の上限。
        limit: Level,
        /// 上限の種類(10 TD-12)。
        kind: LimitKind,
    },
    /// お金が足りない。
    InsufficientFunds {
        /// 強化の 1 段のコストか、線路の値段。
        required: Money,
        /// 所持金。
        available: Money,
    },
    /// 転生の条件(累計収入がゴールの 1 無量大数円)に届いていない (T1)。
    RebirthNotReady,
    /// 転生の強化がすでに最大レベル (T1)。
    RebirthUpgradeMaxed {
        /// 強化。
        upgrade: RebirthUpgrade,
    },
    /// 転生ポイントが足りない (T1)。
    RebirthUpgradePoints {
        /// 必要なポイント。
        required: u64,
        /// 持っているポイント。
        available: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_of_each_command() {
        let c = Command::CreateLine {
            stations: vec![StationId(0), StationId(1)],
            bends: vec![Bend::StraightFirst],
        };
        assert_eq!(c.kind(), CommandKind::CreateLine);
        let e = Command::ExtendLine {
            line: LineId(0),
            end: LineEnd::Head,
            station: StationId(2),
            bend: Bend::DiagonalFirst,
        };
        assert_eq!(e.kind(), CommandKind::ExtendLine);
        let u = Command::LevelUp {
            param: ParamKey::S2Fare,
            scope: UpgradeScope::AllStations,
            mode: BuyMode::Max,
        };
        assert_eq!(
            u.kind(),
            CommandKind::LevelUp {
                param: ParamKey::S2Fare,
                mode: BuyMode::Max
            }
        );
    }

    #[test]
    fn ron_representation_10_9_1() {
        let u = Command::LevelUp {
            param: ParamKey::S2Fare,
            scope: UpgradeScope::AllStations,
            mode: BuyMode::Max,
        };
        assert_eq!(
            ron::to_string(&u).unwrap(),
            "LevelUp(param:S2Fare,scope:AllStations,mode:Max)"
        );
        let cmds = [
            u,
            Command::LevelUp {
                param: ParamKey::S1Spawn,
                scope: UpgradeScope::Station(StationId(3)),
                mode: BuyMode::One,
            },
            Command::CreateLine {
                stations: vec![StationId(0), StationId(2), StationId(1)],
                bends: vec![Bend::StraightFirst, Bend::DiagonalFirst],
            },
            Command::ExtendLine {
                line: LineId(1),
                end: LineEnd::Tail,
                station: StationId(4),
                bend: Bend::StraightFirst,
            },
        ];
        for c in cmds {
            let s = ron::to_string(&c).unwrap();
            assert_eq!(ron::from_str::<Command>(&s).unwrap(), c, "{s}");
        }
        let parsed: Command =
            ron::from_str("LevelUp(param: S1Spawn, scope: Station(3), mode: One)").unwrap();
        assert_eq!(
            parsed,
            Command::LevelUp {
                param: ParamKey::S1Spawn,
                scope: UpgradeScope::Station(StationId(3)),
                mode: BuyMode::One
            }
        );
    }
}
