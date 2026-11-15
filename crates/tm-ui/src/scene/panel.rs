//! 強化パネル: タブ、対象の切り替え、項目、購入 [13 §3.8]。

use tm_assets::font::FontId;
use tm_assets::sound::{SE_CANCEL, SE_CURSOR, SE_ERROR};
use tm_assets::sprite;
use tm_assets::text::{TextArg, TextId};
use tm_assets::txt;
use tm_core::{
    BuyMode, Command, CommandError, GameView, LimitKind, LineId, ParamCategory, ParamKey,
    RebirthUpgrade, Recommendation, StationId, UpgradeScope,
};
use tm_input::{Action, ActionFrame, InputDevice};
use tm_numeric::FormatStyle;
use tm_platform_api::{MonoTime, Rect, SpriteId};
use tm_tutorial::{MenuAnchor, UiAnchor, UiEvent};

use super::play::{Ctx, Gate};
use crate::draw::palette::{self, line_color, with_alpha};
use crate::draw::{DrawBuilder, rect};
use crate::hud::decimal;
use crate::layout::PANEL;
use crate::text::DynText;
use crate::text::numbers::NumText;

/// パネルのタブ [01 §3.2、T1]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelTab {
    /// パラメータのタブ。
    Param(ParamCategory),
    /// てんせい (T1)。
    Rebirth,
}

impl PanelTab {
    /// パラメータのタブなら `ParamCategory`。
    pub(crate) const fn category(self) -> Option<ParamCategory> {
        match self {
            PanelTab::Param(c) => Some(c),
            PanelTab::Rebirth => None,
        }
    }
}

impl From<ParamCategory> for PanelTab {
    fn from(c: ParamCategory) -> Self {
        PanelTab::Param(c)
    }
}

/// タブの順(01 §3.2 + T1)。
pub(crate) const TABS: [PanelTab; 4] = [
    PanelTab::Param(ParamCategory::Station),
    PanelTab::Param(ParamCategory::Train),
    PanelTab::Param(ParamCategory::Line),
    PanelTab::Rebirth,
];
const MAX_ROWS: usize = 5;
/// タブ・対象の行・項目の高さ(13 §3.8 の表)。
const TAB_H: i32 = 20;
const SCOPE_H: i32 = 20;
const ROW_H: i32 = 38;
/// タブ 1 つの幅(4 つで内側の幅に収める)。
const TAB_W: i32 = 52;
/// 内側の左端と幅(208)。
const INNER_X: i32 = PANEL.x + 8;
pub(crate) const INNER_W: u16 = PANEL.w as u16 - 16;
/// 項目の文の左端(選んでいる印の後)と右端。
const TEXT_X: i32 = INNER_X + 14;
const EDGE: i32 = INNER_X + INNER_W as i32 - 4;
const TABS_Y: i32 = PANEL.y + 4;
const SCOPE_Y: i32 = TABS_Y + TAB_H;
const ROWS_Y: i32 = SCOPE_Y + SCOPE_H;
/// 項目の 2 行目の、行の上端からの距離。
const LINE2: i32 = 18;
/// 下の欄(説明と操作のヒントの 2 行)。
const FOOT_Y: i32 = ROWS_Y + ROW_H * MAX_ROWS as i32 + 2;
const FOOT_LINE: i32 = 18;
/// おすすめの枠の点めつの周期(2 Hz)。
const REC_BLINK_MS: u64 = 500;
/// 買った行を光らせて「レベルアップ!」を出す長さ [13 §3.12]。
pub(crate) const LEVEL_UP_US: u64 = 600_000;

/// タブの項目(01 §3.2 の表の順)。
pub(crate) fn items(tab: ParamCategory) -> &'static [ParamKey] {
    match tab {
        ParamCategory::Station => &[
            ParamKey::S1Spawn,
            ParamKey::S2Fare,
            ParamKey::S3StationCap,
            ParamKey::S4Platform,
        ],
        ParamCategory::Train => &[
            ParamKey::T1Speed,
            ParamKey::T2Crowding,
            ParamKey::T3Boarding,
            ParamKey::T4CarCapacity,
            ParamKey::T5Cars,
        ],
        ParamCategory::Line => &[
            ParamKey::R1MaxTrains,
            ParamKey::R2LineSlots,
            ParamKey::R3Attractiveness,
        ],
    }
}

fn tab_index(tab: PanelTab) -> usize {
    TABS.iter().position(|&t| t == tab).unwrap_or(0)
}

/// てんせいタブの行の数(転生する 1 行 + 強化 3 行)。
const REBIRTH_ROWS: usize = 1 + RebirthUpgrade::ALL.len();

/// てんせいタブの行 `i` の強化(`i ≥ 1`)。
fn rebirth_item(i: usize) -> RebirthUpgrade {
    RebirthUpgrade::ALL[i - 1]
}

fn name_id(p: ParamKey) -> TextId {
    match p {
        ParamKey::S1Spawn => txt::PARAM_S1_NAME,
        ParamKey::S2Fare => txt::PARAM_S2_NAME,
        ParamKey::S3StationCap => txt::PARAM_S3_NAME,
        ParamKey::S4Platform => txt::PARAM_S4_NAME,
        ParamKey::T1Speed => txt::PARAM_T1_NAME,
        ParamKey::T2Crowding => txt::PARAM_T2_NAME,
        ParamKey::T3Boarding => txt::PARAM_T3_NAME,
        ParamKey::T4CarCapacity => txt::PARAM_T4_NAME,
        ParamKey::T5Cars => txt::PARAM_T5_NAME,
        ParamKey::R1MaxTrains => txt::PARAM_R1_NAME,
        ParamKey::R2LineSlots => txt::PARAM_R2_NAME,
        ParamKey::R3Attractiveness => txt::PARAM_R3_NAME,
    }
}

fn desc_id(p: ParamKey) -> TextId {
    match p {
        ParamKey::S1Spawn => txt::PARAM_S1_DESC,
        ParamKey::S2Fare => txt::PARAM_S2_DESC,
        ParamKey::S3StationCap => txt::PARAM_S3_DESC,
        ParamKey::S4Platform => txt::PARAM_S4_DESC,
        ParamKey::T1Speed => txt::PARAM_T1_DESC,
        ParamKey::T2Crowding => txt::PARAM_T2_DESC,
        ParamKey::T3Boarding => txt::PARAM_T3_DESC,
        ParamKey::T4CarCapacity => txt::PARAM_T4_DESC,
        ParamKey::T5Cars => txt::PARAM_T5_DESC,
        ParamKey::R1MaxTrains => txt::PARAM_R1_DESC,
        ParamKey::R2LineSlots => txt::PARAM_R2_DESC,
        ParamKey::R3Attractiveness => txt::PARAM_R3_DESC,
    }
}

fn tab_text(tab: PanelTab) -> TextId {
    match tab {
        PanelTab::Param(ParamCategory::Station) => txt::PANEL_TAB_STATION,
        PanelTab::Param(ParamCategory::Train) => txt::PANEL_TAB_TRAIN,
        PanelTab::Param(ParamCategory::Line) => txt::PANEL_TAB_LINE,
        PanelTab::Rebirth => txt::PANEL_TAB_REBIRTH,
    }
}

/// てんせいの強化の名前。
fn rebirth_name_id(k: RebirthUpgrade) -> TextId {
    match k {
        RebirthUpgrade::FareBoost => txt::PANEL_REBIRTH_NAME_FARE,
        RebirthUpgrade::TrainBoost => txt::PANEL_REBIRTH_NAME_TRAIN,
        RebirthUpgrade::LineBoost => txt::PANEL_REBIRTH_NAME_LINE,
    }
}

/// てんせいの強化の説明。
fn rebirth_desc_id(k: RebirthUpgrade) -> TextId {
    match k {
        RebirthUpgrade::FareBoost => txt::PANEL_REBIRTH_DESC_FARE,
        RebirthUpgrade::TrainBoost => txt::PANEL_REBIRTH_DESC_TRAIN,
        RebirthUpgrade::LineBoost => txt::PANEL_REBIRTH_DESC_LINE,
    }
}

/// 対象の並び `[ぜんぶ, 起点, 起点の次 …]` で `dir` だけ進める。端で折り返す [UI-D-05]。
pub(crate) fn cycle(ids: &[u32], origin: Option<u32>, cur: Option<u32>, dir: i32) -> Option<u32> {
    let start = origin
        .and_then(|o| ids.iter().position(|&i| i == o))
        .unwrap_or(0);
    let n = ids.len() as i32 + 1;
    let pos = |k: Option<u32>| -> i32 {
        k.and_then(|k| ids.iter().position(|&i| i == k))
            .map_or(0, |p| (p + ids.len() - start) as i32 % ids.len() as i32 + 1)
    };
    let next = (pos(cur) + dir).rem_euclid(n);
    (next != 0).then(|| ids[(start + next as usize - 1) % ids.len()])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Cost {
        affordable: bool,
    },
    Note(Option<SpriteId>),
    /// 転生ポイントの値段。星のアイコン (T1)。
    Points {
        affordable: bool,
    },
    /// 転生でもらえるポイント (T1)。
    Earn,
}

/// てんせいタブの行 (T1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RebRow {
    /// 「転生する」。
    Do,
    /// 恒久の強化。
    Upgrade(RebirthUpgrade),
}

#[derive(Default)]
struct Row {
    param: Option<ParamKey>,
    rebirth: Option<RebRow>,
    name: DynText,
    new: bool,
    level: DynText,
    show_level: bool,
    status: Option<Status>,
    cost: NumText,
    /// 転生のポイントの値段でもらえるポイントの文。
    pts: DynText,
    note: DynText,
    dim: bool,
    rec: Option<bool>,
}

/// パネルを閉じた後にすること。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelExit {
    /// 開いたまま。
    Stay,
    /// 閉じてマップへ。
    Close,
    /// 閉じてろせんモードへ。
    Build,
}

/// 強化パネルの状態。
pub(crate) struct PanelState {
    /// いまのタブ。
    pub tab: PanelTab,
    item: [usize; 4],
    /// 「えき」タブの対象(`None` = ぜんぶ)。
    pub target_station: Option<StationId>,
    /// 「電車」「ろせん」タブの対象(`None` = ぜんぶ)。
    pub target_line: Option<LineId>,
    origin_station: Option<StationId>,
    origin_line: Option<LineId>,
    buy_blocked: bool,
    /// 対象が不連続に変わった(カメラを中央へ)。
    pub jump: bool,
    dirty: bool,
    rows: [Row; MAX_ROWS],
    tabs: [DynText; 4],
    tab_new: [bool; 4],
    rec_tab: Option<(usize, bool)>,
    scope: DynText,
    scope_arrows: bool,
    foot: DynText,
    foot_need: bool,
    need: NumText,
    hint: DynText,
    new_tag: DynText,
    rec_tag: DynText,
    levelup: DynText,
    /// 買った項目と、その時刻(「レベルアップ!」を出す)。
    flash: Option<(ParamKey, MonoTime)>,
    /// 「転生する」の 2 段の確認(もう一度 Confirm で確定)(T1)。
    rebirth_confirm: bool,
    /// 止める説明を出している(`permits` による行の暗さを付けない。UI-D-31)。
    pub explaining: bool,
}

impl PanelState {
    /// 開く。`origin` は対象の候補の起点。
    pub(crate) fn open(
        tab: PanelTab,
        origin_station: Option<StationId>,
        origin_line: Option<LineId>,
        cx: &mut Ctx<'_, '_>,
    ) -> PanelState {
        let mut p = PanelState {
            tab,
            item: [0; 4],
            target_station: None,
            target_line: None,
            origin_station,
            origin_line,
            buy_blocked: false,
            jump: false,
            dirty: true,
            rows: Default::default(),
            tabs: Default::default(),
            tab_new: [false; 4],
            rec_tab: None,
            scope: DynText::default(),
            scope_arrows: false,
            foot: DynText::default(),
            foot_need: false,
            need: NumText::default(),
            hint: DynText::default(),
            new_tag: DynText::default(),
            rec_tag: DynText::default(),
            levelup: DynText::default(),
            flash: None,
            rebirth_confirm: false,
            explaining: false,
        };
        p.init_items(cx.view);
        match tab {
            PanelTab::Param(c) => {
                cx.event(UiEvent::MenuOpened(MenuAnchor::UpgradePanel));
                cx.event(UiEvent::MenuOpened(MenuAnchor::UpgradeTab(c)));
            }
            PanelTab::Rebirth => {
                cx.event(UiEvent::MenuOpened(MenuAnchor::UpgradePanel));
                cx.event(UiEvent::MenuOpened(MenuAnchor::UpgradeRebirth));
            }
        }
        p
    }

    /// タブごとの項目の数。
    fn item_count(&self) -> usize {
        match self.tab {
            PanelTab::Param(c) => items(c).len(),
            PanelTab::Rebirth => REBIRTH_ROWS,
        }
    }

    /// 開いたときの選択の位置を項目の数に収める(タブを記憶して開き直すとき)。
    fn init_items(&mut self, view: GameView<'_>) {
        let i = tab_index(self.tab);
        self.item[i] = self.item[i].min(self.item_count().saturating_sub(1));
        let _ = view;
    }

    /// 選んでいるパラメータの項目(パラメータのタブだけ)。
    pub(crate) fn param(&self) -> ParamKey {
        let c = self.tab.category().expect("パラメータのタブ");
        let list = items(c);
        list[self.item[tab_index(self.tab)].min(list.len() - 1)]
    }

    /// てんせいタブで選んでいる行 (T1)。
    fn rebirth_row(&self) -> RebRow {
        let i = self.item[tab_index(self.tab)];
        if i == 0 {
            RebRow::Do
        } else {
            RebRow::Upgrade(rebirth_item(i))
        }
    }

    /// 選んでいる項目の購入の対象。R2 は常に会社 [13 §3.8]。
    pub(crate) fn scope(&self) -> UpgradeScope {
        scope_for(self.param(), self.target_station, self.target_line)
    }

    /// おすすめの項目へ(対象は「ぜんぶ」)。
    pub(crate) fn jump_to(&mut self, rec: Recommendation, cx: &mut Ctx<'_, '_>) {
        self.set_tab(PanelTab::Param(rec.param.category()), cx);
        let c = rec.param.category();
        let t = tab_index(self.tab);
        self.item[t] = items(c).iter().position(|&p| p == rec.param).unwrap_or(0);
        match self.tab {
            PanelTab::Param(ParamCategory::Station) => self.target_station = None,
            _ => self.target_line = None,
        }
        self.jump = true;
        self.dirty = true;
        cx.event(UiEvent::CursorOn(UiAnchor::Menu(MenuAnchor::UpgradeItem(
            rec.param,
        ))));
    }

    fn set_tab(&mut self, tab: PanelTab, cx: &mut Ctx<'_, '_>) {
        if tab == self.tab {
            return;
        }
        match self.tab {
            PanelTab::Param(c) => cx.event(UiEvent::MenuClosed(MenuAnchor::UpgradeTab(c))),
            PanelTab::Rebirth => cx.event(UiEvent::MenuClosed(MenuAnchor::UpgradeRebirth)),
        }
        self.tab = tab;
        self.rebirth_confirm = false;
        match tab {
            PanelTab::Param(c) => cx.event(UiEvent::MenuOpened(MenuAnchor::UpgradeTab(c))),
            PanelTab::Rebirth => cx.event(UiEvent::MenuOpened(MenuAnchor::UpgradeRebirth)),
        }
        self.dirty = true;
        self.jump = true;
    }

    /// パネルを閉じたときの出来事。
    pub(crate) fn close(&self, cx: &mut Ctx<'_, '_>) {
        match self.tab {
            PanelTab::Param(c) => cx.event(UiEvent::MenuClosed(MenuAnchor::UpgradeTab(c))),
            PanelTab::Rebirth => cx.event(UiEvent::MenuClosed(MenuAnchor::UpgradeRebirth)),
        }
        cx.event(UiEvent::MenuClosed(MenuAnchor::UpgradePanel));
    }

    /// 入力を受ける。
    pub(crate) fn handle(&mut self, input: &ActionFrame, cx: &mut Ctx<'_, '_>) -> PanelExit {
        if input.released.contains(Action::Confirm) || !input.held.contains(Action::Confirm) {
            self.buy_blocked = false;
        }
        let pressed = |a| input.pressed.contains(a);
        let step = |a| input.pressed.contains(a) || input.repeated.contains(a);
        if pressed(Action::Cancel) {
            cx.se(SE_CANCEL);
            return PanelExit::Close;
        }
        if pressed(Action::BuildMode) {
            return PanelExit::Build;
        }
        if pressed(Action::JumpRecommend) {
            match cx.recommendation {
                Some(rec) => {
                    self.jump_to(rec, cx);
                    cx.se(SE_CURSOR);
                }
                None => cx.se(SE_ERROR),
            }
            return PanelExit::Stay;
        }
        let tab_dir = i32::from(pressed(Action::TabNext)) - i32::from(pressed(Action::TabPrev));
        if tab_dir != 0 {
            let t = (tab_index(self.tab) as i32 + tab_dir).rem_euclid(TABS.len() as i32);
            self.set_tab(TABS[t as usize], cx);
            cx.se(SE_CURSOR);
            return PanelExit::Stay;
        }
        let both = |a, b| input.held.contains(a) && input.held.contains(b);
        let dy = if both(Action::MoveUp, Action::MoveDown) {
            0
        } else {
            i32::from(step(Action::MoveDown)) - i32::from(step(Action::MoveUp))
        };
        if dy != 0 {
            let t = tab_index(self.tab);
            let n = self.item_count() as i32;
            let next = (self.item[t] as i32 + dy).clamp(0, n - 1) as usize;
            if next != self.item[t] {
                self.item[t] = next;
                self.rebirth_confirm = false;
                self.dirty = true;
                cx.se(SE_CURSOR);
                if let Some(p) = self.selected_param() {
                    cx.event(UiEvent::CursorOn(UiAnchor::Menu(MenuAnchor::UpgradeItem(
                        p,
                    ))));
                }
            }
        }
        let dx = if both(Action::MoveLeft, Action::MoveRight) {
            0
        } else {
            i32::from(step(Action::MoveRight)) - i32::from(step(Action::MoveLeft))
        };
        if dx != 0
            && matches!(self.tab, PanelTab::Param(_))
            && self.param() != ParamKey::R2LineSlots
            && self.switch_target(dx, cx.view)
        {
            cx.se(SE_CURSOR);
            cx.event(UiEvent::CursorOn(UiAnchor::Menu(MenuAnchor::ScopeToggle)));
        }
        if pressed(Action::BuyMax) && matches!(self.tab, PanelTab::Param(_)) {
            self.buy(BuyMode::Max, cx);
        } else if pressed(Action::Confirm)
            || (input.repeated.contains(Action::Confirm) && !self.buy_blocked)
        {
            self.confirm(cx);
        }
        PanelExit::Stay
    }

    /// 選んでいる項目のパラメータ(てんせいタブは `None`)。
    fn selected_param(&self) -> Option<ParamKey> {
        self.tab.category().map(|_| self.param())
    }

    /// 決定ボタン (T1)。
    fn confirm(&mut self, cx: &mut Ctx<'_, '_>) {
        match self.tab {
            PanelTab::Param(_) => self.buy(BuyMode::One, cx),
            PanelTab::Rebirth => match self.rebirth_row() {
                RebRow::Do => {
                    // 2 段の確認: 1 回目で「もう一度」を出し、2 回目で転生する。
                    if !cx.view.rebirth().can_rebirth {
                        cx.se(SE_ERROR);
                        self.rebirth_confirm = false;
                        return;
                    }
                    if self.rebirth_confirm {
                        self.rebirth_confirm = false;
                        let cmd = Command::Rebirth;
                        if !cx.submit(cmd) {
                            self.rebirth_confirm = true;
                        }
                    } else {
                        self.rebirth_confirm = true;
                        self.dirty = true;
                        cx.se(SE_CURSOR);
                    }
                }
                RebRow::Upgrade(u) => {
                    self.rebirth_confirm = false;
                    let cmd = Command::RebirthUpgrade { upgrade: u };
                    if !cx.submit(cmd) {
                        self.buy_blocked = true;
                    }
                }
            },
        }
    }

    fn switch_target(&mut self, dir: i32, view: GameView<'_>) -> bool {
        let changed = match self.tab {
            PanelTab::Param(ParamCategory::Station) => {
                let ids: Vec<u32> = view
                    .stations()
                    .filter(|s| s.connected)
                    .map(|s| s.id.0)
                    .collect();
                let next = cycle(
                    &ids,
                    self.origin_station.map(|s| s.0),
                    self.target_station.map(|s| s.0),
                    dir,
                )
                .map(StationId);
                std::mem::replace(&mut self.target_station, next) != next
            }
            _ => {
                let ids: Vec<u32> = view.lines().map(|l| l.id.0).collect();
                let next = cycle(
                    &ids,
                    self.origin_line.map(|l| l.0),
                    self.target_line.map(|l| l.0),
                    dir,
                )
                .map(LineId);
                std::mem::replace(&mut self.target_line, next) != next
            }
        };
        if changed {
            self.dirty = true;
            self.jump = true;
        }
        changed
    }

    fn buy(&mut self, mode: BuyMode, cx: &mut Ctx<'_, '_>) {
        let cmd = Command::LevelUp {
            param: self.param(),
            scope: self.scope(),
            mode,
        };
        if !cx.submit(cmd) {
            self.buy_blocked = true;
        }
    }

    /// 表示を作り直す(10 Hz と、状態が変わったとき)。
    pub(crate) fn refresh(
        &mut self,
        view: GameView<'_>,
        gate: Gate<'_>,
        rec: Option<Recommendation>,
        forced: bool,
        dev: InputDevice,
    ) {
        if !forced && !self.dirty {
            return;
        }
        self.dirty = false;
        self.fix_targets(view);
        for (i, t) in TABS.iter().enumerate() {
            self.tabs[i].set(tab_text(*t), &[], FontId::Body12, INNER_W, dev);
            self.tab_new[i] = match t.category() {
                Some(c) => items(c).iter().any(|&p| view.is_new(p)),
                None => view.rebirth().upgrades.iter().any(|u| u.affordable),
            };
        }
        self.rec_tab = rec.map(|r| (tab_index(PanelTab::Param(r.param.category())), r.affordable));
        match self.tab {
            PanelTab::Rebirth => self.refresh_rebirth(view, gate, dev),
            PanelTab::Param(_) => self.refresh_params(view, gate, rec, dev),
        }
    }

    /// パラメータのタブの表示 [13 §3.8]。
    fn refresh_params(
        &mut self,
        view: GameView<'_>,
        gate: Gate<'_>,
        rec: Option<Recommendation>,
        dev: InputDevice,
    ) {
        let c = self.tab.category().expect("パラメータのタブ");
        let r2 = self.param() == ParamKey::R2LineSlots;
        let (scope_id, arrows) = if r2 {
            (txt::PANEL_SCOPE_COMPANY, false)
        } else {
            match self.scope() {
                UpgradeScope::Station(_) => (txt::PANEL_SCOPE_STATION, true),
                UpgradeScope::Line(_) => (txt::PANEL_SCOPE_LINE, true),
                _ => (txt::PANEL_SCOPE_ALL, true),
            }
        };
        self.scope.set(scope_id, &[], FontId::Body12, INNER_W, dev);
        self.scope_arrows = arrows;
        self.hint
            .set(txt::PANEL_HINT, &[], FontId::Body12, INNER_W, dev);
        self.new_tag
            .set(txt::PANEL_NEW, &[], FontId::Body12, INNER_W, dev);
        self.rec_tag
            .set(txt::PANEL_RECOMMEND, &[], FontId::Body12, INNER_W, dev);
        self.levelup
            .set(txt::PANEL_LEVELUP, &[], FontId::Body12, INNER_W, dev);

        let list = items(c);
        let selected = self.param();
        let mut foot_err = None;
        for (i, row) in self.rows.iter_mut().enumerate() {
            let Some(&p) = list.get(i) else {
                row.param = None;
                row.rebirth = None;
                continue;
            };
            row.param = Some(p);
            row.rebirth = None;
            row.name.set(name_id(p), &[], FontId::Body12, INNER_W, dev);
            row.new = view.is_new(p);
            let scope = scope_for(p, self.target_station, self.target_line);
            row.show_level = set_level(&mut row.level, view, p, scope, dev);
            let cmd = Command::LevelUp {
                param: p,
                scope,
                mode: BuyMode::One,
            };
            let result = view.validate(&cmd);
            row.dim = !gate.permits(&cmd);
            row.rec = rec
                .filter(|r| r.param == p && r.scope == scope_for(p, None, None))
                .map(|r| r.affordable);
            row.status = Some(match result {
                Ok(preview) => {
                    row.cost
                        .set(preview.cost, &FormatStyle::DEFAULT, FontId::Body12);
                    Status::Cost { affordable: true }
                }
                Err(CommandError::InsufficientFunds {
                    required,
                    available,
                }) => {
                    row.cost
                        .set(required, &FormatStyle::DEFAULT, FontId::Body12);
                    if p == selected {
                        foot_err = Some(
                            required
                                .checked_sub(available)
                                .unwrap_or(tm_numeric::BigNum::ZERO),
                        );
                    }
                    Status::Cost { affordable: false }
                }
                Err(e) => {
                    let (id, icon, n) = status_note(e, c);
                    let mut buf = [0u8; 4];
                    match n {
                        Some(n) => row.note.set(
                            id,
                            &[TextArg {
                                name: "n",
                                value: decimal(n, &mut buf),
                            }],
                            FontId::Body12,
                            INNER_W,
                            dev,
                        ),
                        None => row.note.set(id, &[], FontId::Body12, INNER_W, dev),
                    };
                    Status::Note(icon)
                }
            });
        }
        match foot_err {
            Some(need) => {
                self.need.set(need, &FormatStyle::DEFAULT, FontId::Body12);
                self.foot.set(
                    txt::PANEL_NEED_MORE,
                    &[TextArg {
                        name: "money",
                        value: &self.need.text,
                    }],
                    FontId::Body12,
                    INNER_W,
                    dev,
                );
                self.foot_need = true;
            }
            None => {
                self.foot
                    .set(desc_id(selected), &[], FontId::Body12, INNER_W, dev);
                self.foot_need = false;
            }
        }
    }

    /// てんせいタブの表示 (T1)。
    fn refresh_rebirth(&mut self, view: GameView<'_>, gate: Gate<'_>, dev: InputDevice) {
        let r = view.rebirth();
        // 対象行の代わりに、ポイントの所持数を出す。
        let mut points_buf = [0u8; 8];
        self.scope.set(
            txt::PANEL_REBIRTH_POINTS,
            &[TextArg {
                name: "n",
                value: decimal(r.points, &mut points_buf),
            }],
            FontId::Body12,
            INNER_W,
            dev,
        );
        self.scope_arrows = false;
        let selected = self.item[tab_index(self.tab)];
        self.hint
            .set(txt::PANEL_REBIRTH_HINT, &[], FontId::Body12, INNER_W, dev);
        self.new_tag
            .set(txt::PANEL_NEW, &[], FontId::Body12, INNER_W, dev);
        self.levelup
            .set(txt::PANEL_LEVELUP, &[], FontId::Body12, INNER_W, dev);
        let mut buf_a = [0u8; 8];
        let mut buf_b = [0u8; 8];
        for (i, row) in self.rows.iter_mut().enumerate() {
            if i >= REBIRTH_ROWS {
                row.param = None;
                row.rebirth = None;
                continue;
            }
            row.param = None;
            row.rec = None;
            if i == 0 {
                row.rebirth = Some(RebRow::Do);
                row.name
                    .set(txt::PANEL_REBIRTH_DO, &[], FontId::Body12, INNER_W, dev);
                row.show_level = false;
                row.new = false;
                row.dim = !gate.permits(&Command::Rebirth);
                row.status = Some(if !r.can_rebirth {
                    row.note
                        .set(txt::PANEL_REBIRTH_LOCK, &[], FontId::Body12, INNER_W, dev);
                    Status::Note(Some(sprite::ICO_LOCK))
                } else if self.rebirth_confirm && selected == 0 {
                    row.note.set(
                        txt::PANEL_REBIRTH_DO_AGAIN,
                        &[],
                        FontId::Body12,
                        INNER_W,
                        dev,
                    );
                    Status::Note(None)
                } else {
                    row.pts.set(
                        txt::PANEL_REBIRTH_EARN,
                        &[TextArg {
                            name: "n",
                            value: decimal(r.next_points, &mut buf_a),
                        }],
                        FontId::Body12,
                        INNER_W,
                        dev,
                    );
                    Status::Earn
                });
                continue;
            }
            let k = rebirth_item(i);
            let uv = r.upgrades[k.index()];
            row.rebirth = Some(RebRow::Upgrade(k));
            row.name
                .set(rebirth_name_id(k), &[], FontId::Body12, INNER_W, dev);
            row.new = false;
            row.dim = !gate.permits(&Command::RebirthUpgrade { upgrade: k });
            if uv.level >= uv.max {
                row.show_level = false;
                row.note
                    .set(txt::PANEL_REBIRTH_LV_MAX, &[], FontId::Body12, INNER_W, dev);
                row.status = Some(Status::Note(Some(sprite::ICO_CROWN)));
            } else {
                row.show_level = true;
                let a = decimal(u64::from(uv.level), &mut buf_a);
                let b = decimal(u64::from(uv.max), &mut buf_b);
                row.level.set(
                    txt::PANEL_REBIRTH_LV,
                    &[
                        TextArg {
                            name: "a",
                            value: a,
                        },
                        TextArg {
                            name: "b",
                            value: b,
                        },
                    ],
                    FontId::Body12,
                    INNER_W,
                    dev,
                );
                let cost = uv.cost.unwrap_or(0);
                row.pts.set(
                    txt::PANEL_REBIRTH_EARN,
                    &[TextArg {
                        name: "n",
                        value: decimal(cost, &mut buf_a),
                    }],
                    FontId::Body12,
                    INNER_W,
                    dev,
                );
                row.status = Some(Status::Points {
                    affordable: uv.affordable,
                });
            }
        }
        // 下の欄: 選んでいる行の説明。
        self.foot_need = false;
        let foot = match selected {
            0 => txt::PANEL_REBIRTH_DESC_REBIRTH,
            i => rebirth_desc_id(rebirth_item(i)),
        };
        self.foot.set(foot, &[], FontId::Body12, INNER_W, dev);
    }

    fn fix_targets(&mut self, view: GameView<'_>) {
        if self
            .target_station
            .is_some_and(|s| view.station(s).is_none_or(|st| !st.connected))
        {
            self.target_station = None;
        }
        if self.target_line.is_some_and(|l| view.line(l).is_none()) {
            self.target_line = None;
        }
    }

    /// 選んでいる対象のマス(カメラの注目点。路線は真ん中のマス)。「ぜんぶ」なら `None`。
    pub(crate) fn focus(
        &self,
        view: GameView<'_>,
        paths: &[Vec<tm_core::GridPos>],
    ) -> Option<tm_core::GridPos> {
        match self.tab {
            PanelTab::Param(ParamCategory::Station) => self
                .target_station
                .and_then(|s| view.station(s))
                .map(|s| s.pos),
            PanelTab::Param(_) => self
                .target_line
                .and_then(|l| paths.get(l.index()))
                .and_then(|p| p.get(p.len() / 2))
                .copied(),
            PanelTab::Rebirth => None,
        }
    }

    /// マップの上に、選んでいる駅・路線の太い枠を描く(層 3)。
    pub(crate) fn draw_targets(
        &self,
        b: &mut DrawBuilder<'_>,
        view: GameView<'_>,
        cam: crate::layout::Cam,
    ) {
        let frame = |b: &mut DrawBuilder<'_>, pos: tm_core::GridPos| {
            b.nine_slice_scaled(
                &sprite::NS_UI_SEL,
                crate::layout::tile_ring(pos, cam),
                palette::WHITE,
                crate::layout::MAP_SCALE,
            );
        };
        match self.tab {
            PanelTab::Param(ParamCategory::Station) => {
                if let Some(st) = self.target_station.and_then(|s| view.station(s)) {
                    frame(b, st.pos);
                }
            }
            PanelTab::Param(_) => {
                if let Some(line) = self.target_line.and_then(|l| view.line(l)) {
                    for st in line.stations.iter().filter_map(|&s| view.station(s)) {
                        frame(b, st.pos);
                        let (x, y) = crate::layout::tile_screen(st.pos, cam);
                        b.map_sprite(
                            sprite::LINE_NO[line.id.index() % sprite::LINE_NO.len()],
                            x + 18 * crate::layout::MAP_SCALE,
                            y - 2 * crate::layout::MAP_SCALE,
                            line_color(line.id),
                        );
                    }
                }
            }
            PanelTab::Rebirth => {}
        }
    }

    /// パネルを描く(層 4)。
    pub(crate) fn draw(&self, b: &mut DrawBuilder<'_>, now: MonoTime) {
        b.nine_slice(&sprite::NS_UI_WIN, PANEL, palette::WHITE);
        for (i, t) in self.tabs.iter().enumerate() {
            let r = tab_rect(i);
            let on = TABS[i] == self.tab;
            if on {
                b.nine_slice(&sprite::NS_UI_SEL, r, palette::WHITE);
            }
            let c = if on {
                palette::TEXT
            } else {
                palette::TEXT_DISABLED
            };
            let mut cx = r.x + 6;
            b.text(&t.block, cx, r.y + 2, c);
            cx += i32::from(t.block.width) + 2;
            if self.tab_new[i] {
                b.sprite(sprite::ICO_NEW, cx, r.y + 6, palette::WHITE);
                cx += 18;
            }
            if let Some((rt, affordable)) = self.rec_tab
                && rt == i
            {
                b.sprite(star(affordable), cx, r.y + 4, palette::WHITE);
            }
        }

        let sw = i32::from(self.scope.block.width);
        let sx = PANEL.x + (PANEL.w as i32 - sw) / 2;
        b.text(&self.scope.block, sx, SCOPE_Y + 2, palette::TEXT);
        if self.scope_arrows {
            b.sprite(sprite::ICO_TRI_LEFT, sx - 16, SCOPE_Y + 6, palette::WHITE);
            b.sprite(
                sprite::ICO_TRI_RIGHT,
                sx + sw + 8,
                SCOPE_Y + 6,
                palette::WHITE,
            );
            if let (PanelTab::Param(ParamCategory::Train | ParamCategory::Line), Some(l)) =
                (self.tab, self.target_line)
            {
                b.sprite(
                    sprite::LINE_NO[l.index() % sprite::LINE_NO.len()],
                    sx - 26,
                    SCOPE_Y + SCOPE_H / 2,
                    line_color(l),
                );
            }
        }

        let selected = self.item[tab_index(self.tab)];
        for (i, row) in self.rows.iter().enumerate() {
            if row.param.is_none() && row.rebirth.is_none() {
                continue;
            }
            let r = row_rect(i);
            let y = r.y;
            if i == selected {
                b.nine_slice(&sprite::NS_UI_SEL, r, palette::WHITE);
                b.sprite(sprite::UI_MENU_CURSOR, INNER_X + 4, y + 10, palette::WHITE);
            } else if row.rec.is_some() {
                let a = blink_alpha(now);
                b.nine_slice(&sprite::NS_UI_SEL, r, with_alpha(palette::HIGHLIGHT, a));
            }
            let grey = !matches!(
                row.status,
                Some(Status::Cost { affordable: true } | Status::Points { affordable: true })
                    | Some(Status::Earn)
            );
            let name_c = if grey {
                palette::TEXT_DISABLED
            } else {
                palette::TEXT
            };
            let mut x = TEXT_X;
            if row.new {
                b.text(&self.new_tag.block, x, y + 2, palette::MONEY);
                x += i32::from(self.new_tag.block.width) + 4;
            }
            b.text(&row.name.block, x, y + 2, name_c);
            if row.show_level {
                b.text(
                    &row.level.block,
                    EDGE - i32::from(row.level.block.width),
                    y + 2,
                    name_c,
                );
            }
            let y2 = y + LINE2;
            let mut left_end = EDGE;
            let flash = self.flashing(now).filter(|(p, _)| row.param == Some(*p));
            if let Some((_, t)) = flash {
                let a = 96 * (LEVEL_UP_US - t) / LEVEL_UP_US;
                b.fill(r, with_alpha(palette::HIGHLIGHT, a as u8));
                let w = i32::from(self.levelup.block.width);
                b.text(&self.levelup.block, EDGE - w, y2, palette::MONEY);
                left_end = EDGE - w;
            }
            match row.status.filter(|_| flash.is_none()) {
                Some(Status::Cost { affordable }) => {
                    let w = i32::from(row.cost.block.width);
                    let (icon, c) = if affordable {
                        (sprite::ICO_COIN, palette::TEXT)
                    } else {
                        (sprite::ICO_COIN_X, palette::TEXT_DISABLED)
                    };
                    b.text(&row.cost.block, EDGE - w, y2, c);
                    left_end = EDGE - w - 16;
                    b.sprite(icon, left_end, y2 + 2, palette::WHITE);
                }
                Some(Status::Points { affordable }) => {
                    let w = i32::from(row.pts.block.width);
                    let c = if affordable {
                        palette::TEXT
                    } else {
                        palette::TEXT_DISABLED
                    };
                    b.text(&row.pts.block, EDGE - w, y2, c);
                    left_end = EDGE - w - 16;
                    b.sprite(star(affordable), left_end, y2 + 2, palette::WHITE);
                }
                Some(Status::Earn) => {
                    let w = i32::from(row.pts.block.width);
                    b.text(&row.pts.block, EDGE - w, y2, palette::TEXT);
                    left_end = EDGE - w - 16;
                    b.sprite(sprite::ICO_STAR, left_end, y2 + 2, palette::WHITE);
                }
                Some(Status::Note(icon)) => {
                    let w = i32::from(row.note.block.width);
                    b.text(&row.note.block, EDGE - w, y2, palette::TEXT_DISABLED);
                    left_end = EDGE - w;
                    if let Some(icon) = icon {
                        left_end -= 16;
                        b.sprite(icon, left_end, y2 + 2, palette::WHITE);
                    }
                }
                None => {}
            }
            if let Some(affordable) = row.rec {
                b.sprite(star(affordable), TEXT_X, y2 + 2, palette::WHITE);
                let tag_end = TEXT_X + 16 + i32::from(self.rec_tag.block.width);
                if tag_end + 8 <= left_end {
                    b.text(&self.rec_tag.block, TEXT_X + 16, y2, palette::HIGHLIGHT);
                }
            }
            if row.dim && !self.explaining {
                b.fill(r, with_alpha(palette::BLACK, 112));
            }
        }

        let c = if self.foot_need {
            palette::ERROR
        } else {
            palette::TEXT
        };
        b.text(&self.foot.block, INNER_X + 4, FOOT_Y, c);
        b.text(
            &self.hint.block,
            INNER_X + 4,
            FOOT_Y + FOOT_LINE,
            palette::TEXT,
        );
    }

    /// `Purchased` を受けた: その項目の行を光らせる。
    pub(crate) fn purchased(&mut self, p: ParamKey, now: MonoTime) {
        self.flash = Some((p, now));
    }

    /// 「レベルアップ!」を出している項目と、出してからの時間(µs)。
    fn flashing(&self, now: MonoTime) -> Option<(ParamKey, u64)> {
        let (p, at) = self.flash?;
        let t = now.0.saturating_sub(at.0);
        (t < LEVEL_UP_US).then_some((p, t))
    }

    /// highlight を置くパネルの部分。見えていなければ `None` [13 §3.13]。
    pub(crate) fn part_rect(&self, part: MenuAnchor) -> Option<Rect> {
        match part {
            MenuAnchor::UpgradePanel => Some(PANEL),
            MenuAnchor::UpgradeRebirth => Some(tab_rect(tab_index(PanelTab::Rebirth))),
            MenuAnchor::UpgradeTab(c) => Some(tab_rect(tab_index(PanelTab::Param(c)))),
            MenuAnchor::UpgradeItem(p) => {
                let i = self.rows.iter().position(|r| r.param == Some(p))?;
                Some(row_rect(i))
            }
            MenuAnchor::ScopeToggle => {
                Some(rect(INNER_X, SCOPE_Y, u32::from(INNER_W), SCOPE_H as u32))
            }
            MenuAnchor::BuyMaxHint => {
                let glyphs = &self.hint.block.glyphs;
                let max = [sprite::BTN_KEY_C, sprite::BTN_PAD_Y];
                let start = glyphs.iter().position(|g| max.contains(&g.sprite))?;
                let end = glyphs[start + 1..]
                    .iter()
                    .take_while(|g| !g.plain)
                    .chain(std::iter::once(&glyphs[start]))
                    .map(|g| i32::from(g.x) + i32::from(g.w))
                    .max()?;
                let x = INNER_X + 4 + i32::from(glyphs[start].x);
                Some(rect(
                    x - 2,
                    FOOT_Y + FOOT_LINE - 2,
                    (INNER_X + 4 + end - x + 4) as u32,
                    20,
                ))
            }
            _ => None,
        }
    }

    /// おすすめの星(行か、タブ)。パネルの中になければ `None`。
    pub(crate) fn rec_rect(&self) -> Option<Rect> {
        if let Some(i) = self.rows.iter().position(|r| r.rec.is_some()) {
            let y2 = row_rect(i).y + LINE2;
            let w = 16 + i32::from(self.rec_tag.block.width);
            return Some(rect(TEXT_X - 4, y2 - 2, (w + 8) as u32, 20));
        }
        let (t, _) = self.rec_tab?;
        self.part_rect(MenuAnchor::UpgradeTab(
            TABS[t].category().expect("おすすめはパラメータのタブ"),
        ))
    }

    /// 選んでいる項目の番号(テスト用)。
    #[cfg(test)]
    pub(crate) fn item_index(&self) -> usize {
        self.item[tab_index(self.tab)]
    }

    /// 項目の行の状態(テスト用): (パラメータ, 灰色, 暗い, おすすめ)。
    #[cfg(test)]
    pub(crate) fn row_states(&self) -> Vec<(ParamKey, bool, bool, Option<bool>)> {
        self.rows
            .iter()
            .filter_map(|r| {
                r.param.map(|p| {
                    (
                        p,
                        !matches!(r.status, Some(Status::Cost { affordable: true })),
                        r.dim && !self.explaining,
                        r.rec,
                    )
                })
            })
            .collect()
    }

    /// 項目の行の 2 行目の文(テスト用)。
    #[cfg(test)]
    pub(crate) fn row_note(&self, i: usize) -> Option<TextId> {
        match self.rows[i].status {
            Some(Status::Note(_)) => self.rows[i].note.id(),
            _ => None,
        }
    }

    /// 下の欄の 1 行目の文(テスト用)。
    #[cfg(test)]
    pub(crate) fn foot_text(&self) -> Option<TextId> {
        self.foot.id()
    }

    /// Lv の表示(テスト用)。
    #[cfg(test)]
    pub(crate) fn row_level(&self, i: usize) -> Option<TextId> {
        self.rows[i]
            .show_level
            .then(|| self.rows[i].level.id())
            .flatten()
    }
}

/// タブ `i` の枠(3 つ × 68)。
fn tab_rect(i: usize) -> Rect {
    let left = INNER_X + (i32::from(INNER_W) - TAB_W * TABS.len() as i32) / 2;
    rect(left + TAB_W * i as i32, TABS_Y, TAB_W as u32, TAB_H as u32)
}

/// 項目 `i` の行の枠(行の間に 2px)。
fn row_rect(i: usize) -> Rect {
    rect(
        INNER_X,
        ROWS_Y + ROW_H * i as i32,
        u32::from(INNER_W),
        ROW_H as u32 - 2,
    )
}

fn star(affordable: bool) -> SpriteId {
    if affordable {
        sprite::ICO_STAR
    } else {
        sprite::ICO_STAR_OFF
    }
}

fn blink_alpha(now: MonoTime) -> u8 {
    let t = (now.0 / 1000) % REC_BLINK_MS;
    let half = REC_BLINK_MS / 2;
    (128 + 127 * half.abs_diff(t) / half) as u8
}

/// 項目の購入の対象。R2 は常に会社、ほかは対象がなければ「ぜんぶ」[13 §3.8]。
fn scope_for(p: ParamKey, station: Option<StationId>, line: Option<LineId>) -> UpgradeScope {
    if p == ParamKey::R2LineSlots {
        UpgradeScope::Company
    } else if p.category() == ParamCategory::Station {
        station.map_or(UpgradeScope::AllStations, UpgradeScope::Station)
    } else {
        line.map_or(UpgradeScope::AllLines, UpgradeScope::Line)
    }
}

fn status_note(e: CommandError, tab: ParamCategory) -> (TextId, Option<SpriteId>, Option<u64>) {
    match e {
        CommandError::AtLevelLimit { kind, .. } => match kind {
            LimitKind::CompanyCap => (txt::PANEL_LIMIT_COMPANY, Some(sprite::ICO_UP), None),
            LimitKind::Intrinsic => (txt::PANEL_LIMIT_INTRINSIC, Some(sprite::ICO_CROWN), None),
            LimitKind::Platform => (txt::PANEL_LIMIT_PLATFORM, Some(sprite::ICO_STATION), None),
            LimitKind::Capacity => (txt::PANEL_LIMIT_CAPACITY, Some(sprite::ICO_STATION), None),
        },
        CommandError::Locked { required, .. } => (
            txt::PANEL_LOCKED,
            Some(sprite::ICO_LOCK),
            Some(u64::from(required.0)),
        ),
        CommandError::NoTarget { .. } | CommandError::StationNotConnected(_) => {
            if tab == ParamCategory::Station {
                (txt::PANEL_NO_STATION, None, None)
            } else {
                (txt::PANEL_NO_LINE, None, None)
            }
        }
        _ => (txt::ERR_GENERIC, None, None),
    }
}

/// Lv の表示を作る。この〜だけ ならその対象、ぜんぶ なら最小と最大 [13 §3.8]。出さないなら false。
fn set_level(
    out: &mut DynText,
    view: GameView<'_>,
    p: ParamKey,
    scope: UpgradeScope,
    dev: InputDevice,
) -> bool {
    let levels: Vec<u8> = match scope {
        UpgradeScope::Company => vec![view.line_slot_level().0],
        UpgradeScope::Station(s) => view
            .station(s)
            .and_then(|st| st.levels.get(p))
            .map(|l| l.0)
            .into_iter()
            .collect(),
        UpgradeScope::AllStations => view
            .stations()
            .filter(|s| s.connected)
            .filter_map(|s| s.levels.get(p))
            .map(|l| l.0)
            .collect(),
        UpgradeScope::Line(l) => view
            .line(l)
            .and_then(|line| line.levels.get(p))
            .map(|l| l.0)
            .into_iter()
            .collect(),
        UpgradeScope::AllLines => view
            .lines()
            .filter_map(|l| l.levels.get(p))
            .map(|l| l.0)
            .collect(),
    };
    let (Some(&lo), Some(&hi)) = (levels.iter().min(), levels.iter().max()) else {
        return false;
    };
    let (mut a, mut c) = ([0u8; 4], [0u8; 4]);
    if lo == hi {
        out.set(
            txt::PANEL_LEVEL,
            &[TextArg {
                name: "n",
                value: decimal(u64::from(lo), &mut a),
            }],
            FontId::Body12,
            INNER_W,
            dev,
        );
    } else {
        out.set(
            txt::PANEL_LEVEL_RANGE,
            &[
                TextArg {
                    name: "a",
                    value: decimal(u64::from(lo), &mut a),
                },
                TextArg {
                    name: "b",
                    value: decimal(u64::from(hi), &mut c),
                },
            ],
            FontId::Body12,
            INNER_W,
            dev,
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_items_follow_01_order() {
        assert_eq!(
            items(ParamCategory::Station),
            [
                ParamKey::S1Spawn,
                ParamKey::S2Fare,
                ParamKey::S3StationCap,
                ParamKey::S4Platform
            ]
        );
        assert_eq!(items(ParamCategory::Train).len(), 5);
        assert_eq!(
            items(ParamCategory::Line),
            [
                ParamKey::R1MaxTrains,
                ParamKey::R2LineSlots,
                ParamKey::R3Attractiveness
            ]
        );
        for t in TABS {
            if let Some(c) = t.category() {
                assert!(items(c).iter().all(|&p| p.category() == c));
            }
        }
    }

    #[test]
    fn each_validate_error_has_its_row_text() {
        use tm_core::{CompanyLevel, Feature, Level};
        let lim = |kind| CommandError::AtLevelLimit {
            param: ParamKey::T1Speed,
            limit: Level(1),
            kind,
        };
        let st = ParamCategory::Station;
        let tr = ParamCategory::Train;
        assert_eq!(
            status_note(lim(LimitKind::CompanyCap), tr),
            (txt::PANEL_LIMIT_COMPANY, Some(sprite::ICO_UP), None)
        );
        assert_eq!(
            status_note(lim(LimitKind::Intrinsic), tr),
            (txt::PANEL_LIMIT_INTRINSIC, Some(sprite::ICO_CROWN), None)
        );
        assert_eq!(
            status_note(lim(LimitKind::Platform), tr).0,
            txt::PANEL_LIMIT_PLATFORM
        );
        assert_eq!(
            status_note(lim(LimitKind::Capacity), st),
            (txt::PANEL_LIMIT_CAPACITY, Some(sprite::ICO_STATION), None)
        );
        assert_eq!(
            status_note(
                CommandError::Locked {
                    feature: Feature::Param(ParamKey::S3StationCap),
                    required: CompanyLevel(1)
                },
                st
            ),
            (txt::PANEL_LOCKED, Some(sprite::ICO_LOCK), Some(1))
        );
        let none = CommandError::NoTarget {
            param: ParamKey::S1Spawn,
        };
        assert_eq!(status_note(none, st).0, txt::PANEL_NO_STATION);
        assert_eq!(status_note(none, tr).0, txt::PANEL_NO_LINE);
    }

    #[test]
    fn targets_start_from_origin_and_wrap() {
        let ids = [0, 2, 5, 7];
        let seq = |origin| {
            let mut v = vec![];
            let mut cur = None;
            for _ in 0..6 {
                cur = cycle(&ids, origin, cur, 1);
                v.push(cur);
            }
            v
        };
        assert_eq!(
            seq(Some(5)),
            [Some(5), Some(7), Some(0), Some(2), None, Some(5)]
        );
        assert_eq!(
            seq(None),
            [Some(0), Some(2), Some(5), Some(7), None, Some(0)]
        );
        assert_eq!(cycle(&ids, Some(5), None, -1), Some(2), "左は逆の順");
        assert_eq!(cycle(&ids, Some(5), Some(5), -1), None);
        assert_eq!(
            cycle(&[], None, None, 1),
            None,
            "対象がなければ ぜんぶ のまま"
        );
    }

    #[test]
    fn r2_level_comes_from_the_game() {
        use std::sync::Arc;
        let mut spec = tm_core::BalanceTable::builtin().spec().clone();
        for p in &mut spec.params {
            p.base_yen = 1;
            p.cost_ratio = 1;
            p.unlock = 0;
        }
        spec.economy.initial_money_yen = 1_000_000;
        let table = tm_core::BalanceTable::from_spec(spec).expect("表");
        let mut g = tm_core::Game::new(tm_core::GameConfig {
            seed: 1,
            balance: Arc::new(table),
        });
        let lv = |g: &tm_core::Game| {
            let mut t = DynText::default();
            set_level(
                &mut t,
                g.view(),
                ParamKey::R2LineSlots,
                UpgradeScope::Company,
                InputDevice::Keyboard,
            );
            t.block
        };
        let want = |n: &str| {
            let mut t = DynText::default();
            t.set(
                txt::PANEL_LEVEL,
                &[TextArg {
                    name: "n",
                    value: n,
                }],
                FontId::Body12,
                INNER_W,
                InputDevice::Keyboard,
            );
            t.block
        };
        assert_eq!(lv(&g), want("0"));
        g.enqueue(Command::LevelUp {
            param: ParamKey::R2LineSlots,
            scope: UpgradeScope::Company,
            mode: BuyMode::One,
        });
        g.step(&mut Vec::new());
        assert_eq!(g.view().line_slot_level().0, 1);
        assert_eq!(lv(&g), want("1"), "line_slot_level の値を出す(10 TD-39)");
    }
}
