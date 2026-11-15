//! UI 上の場所の抽象名 [10 §8.4、02 §4.5]。

use tm_core::{LineId, ParamCategory, ParamKey, StationId, TrainId};

/// チュートリアルが参照するメニュー・画面の場所の抽象名。実際の構造との対応は UI が持つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAnchor {
    /// 強化パネル。
    UpgradePanel,
    /// 「えき」「電車」「ろせん」のタブ。
    UpgradeTab(ParamCategory),
    /// 「てんせい」のタブ (T1)。チュートリアルは案内しないが、場所の記録のために出す。
    UpgradeRebirth,
    /// パネルの各項目。
    UpgradeItem(ParamKey),
    /// 「この えきだけ / この ろせんだけ」と「ぜんぶ」の切り替え。
    ScopeToggle,
    /// MAX の案内。
    BuyMaxHint,
    /// ろせんモード。
    BuildMode,
    /// ろせんモードの「作る」の選択肢。
    BuildCreate,
    /// ろせんモードの「のばす」の選択肢。
    BuildExtend,
    /// 「あとで けせないよ。いい?」。
    LineConfirm,
    /// せってい メニュー。
    SettingsMenu,
    /// せってい の項目。
    SettingsItem(SettingsItem),
    /// リセットの確認。`step` は 1 か 2。
    ResetConfirm {
        /// 確認の段。
        step: u8,
    },
    /// 「ゲームを終わる」の確認。`step` は 1 か 2。
    QuitConfirm {
        /// 確認の段。
        step: u8,
    },
}

/// せってい メニューの項目(01 §11.3 の順)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsItem {
    /// つづける。
    Continue,
    /// おんがく の音量。
    MusicVolume,
    /// こうかおん の音量。
    SoundVolume,
    /// せつめいを もう いちど 見る。
    ReplayExplanations,
    /// せつめいを 出す。
    ShowExplanations,
    /// ゲームを終わる(リセットの 1 つ上)。
    Quit,
    /// さいしょから やりなおす。
    Reset,
}

/// UI 上の「ここを見て」の抽象名。画面上の位置は UI が解決する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAnchor {
    /// 駅。
    Station(StationId),
    /// 路線。
    Line(LineId),
    /// 列車。
    Train(TrainId),
    /// メニューの場所。
    Menu(MenuAnchor),
    /// 所持金の表示。
    MoneyDisplay,
    /// 収入の表示。
    IncomeDisplay,
    /// 会社レベル。
    CompanyLevel,
    /// 会社ゲージ。
    CompanyGauge,
    /// おすすめの印。
    RecommendMark,
}
