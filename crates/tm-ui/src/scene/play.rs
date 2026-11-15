//! マップの操作と入力の振り分け [13 §2.8、§3.4]。

use tm_assets::sound::{SE_CONFIRM, SE_CURSOR, SE_ERROR, SE_MENU_CLOSE, SE_MENU_OPEN};
use tm_assets::text::{TextArg, TextId};
use tm_assets::txt;
use tm_core::{Command, CommandError, GameView, GridPos, LimitKind, ParamCategory, Recommendation};
use tm_input::{Action, ActionFrame, InputDevice};
use tm_platform_api::{AudioBus, MonoTime, SoundId};
use tm_tutorial::{DirectiveKind, MenuAnchor, Tutorial, TutorialDirective, UiEvent};

use super::build::BuildState;
use super::panel::{PanelExit, PanelState, PanelTab};
use super::settings_menu::{MenuExit, SettingsMenu};
use super::{PlayMode, PlayScene, map_tiles, world_size};
use crate::camera::Viewport;
use crate::hud::{Band, decimal};
use crate::input_ctx::{only_moves, without};
use crate::layout::{Cam, MAP_H, MAP_X, MAP_Y, bottom};
use crate::request::AppRequest;
use crate::settings::Settings;

/// UI の操作で鳴らす音。`Ui` が音の命令に変える。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum UiSound {
    /// 効果音。
    Se(SoundId),
    /// `validate` の `Err` の音(`sound_for_error`)。
    Error(CommandError),
    /// 音量を 1 段変えた。
    Volume(AudioBus, u8),
}

/// 操作の結果を書く先と、判断に使う値。
pub(crate) struct Ctx<'a, 'v> {
    pub now: MonoTime,
    pub view: GameView<'v>,
    pub dev: InputDevice,
    pub tutorial: Option<&'a mut Tutorial>,
    pub recommendation: Option<Recommendation>,
    /// カメラの目標の位置(確認ダイアログを上下のどちらに出すかに使う)。
    pub cam: Cam,
    pub commands: &'a mut Vec<Command>,
    pub requests: &'a mut Vec<AppRequest>,
    pub sounds: &'a mut Vec<UiSound>,
    pub events: &'a mut Vec<UiEvent>,
    pub band: &'a mut Band,
    #[cfg(test)]
    pub forbid: Option<fn(&Command) -> bool>,
}

/// `permits` を確かめる窓口。
#[derive(Clone, Copy)]
pub(crate) struct Gate<'t> {
    tutorial: Option<&'t Tutorial>,
    #[cfg(test)]
    forbid: Option<fn(&Command) -> bool>,
}

impl<'t> Gate<'t> {
    /// チュートリアル(なければすべて許す)の窓口。
    pub(crate) fn new(tutorial: Option<&'t Tutorial>) -> Gate<'t> {
        Gate {
            tutorial,
            #[cfg(test)]
            forbid: None,
        }
    }

    /// 許さない条件を足す(テスト用)。
    #[cfg(test)]
    pub(crate) fn with_forbid(mut self, forbid: Option<fn(&Command) -> bool>) -> Gate<'t> {
        self.forbid = forbid;
        self
    }

    /// チュートリアルがこのコマンドを許すか。
    pub(crate) fn permits(&self, cmd: &Command) -> bool {
        #[cfg(test)]
        if self.forbid.is_some_and(|f| !f(cmd)) {
            return false;
        }
        self.tutorial.is_none_or(|t| t.permits(cmd))
    }
}

impl Ctx<'_, '_> {
    /// 効果音。
    pub(crate) fn se(&mut self, id: SoundId) {
        self.sounds.push(UiSound::Se(id));
    }

    /// チュートリアルへの出来事。
    pub(crate) fn event(&mut self, ev: UiEvent) {
        self.events.push(ev);
    }

    /// 下の帯に案内を 2 秒出す。
    pub(crate) fn notice(&mut self, id: TextId) {
        self.band.show_error(self.now, id, &[], self.dev);
    }

    /// できない操作: `se_error` と案内。
    pub(crate) fn refuse(&mut self, id: TextId) {
        self.se(SE_ERROR);
        self.notice(id);
    }

    /// `CommandError` の音と案内。
    pub(crate) fn error(&mut self, err: CommandError) {
        self.sounds.push(UiSound::Error(err));
        show_error(self.band, self.now, err, self.dev);
    }

    /// `permits` の窓口。
    pub(crate) fn gate(&self) -> Gate<'_> {
        let g = Gate::new(self.tutorial.as_deref());
        #[cfg(test)]
        let g = g.with_forbid(self.forbid);
        g
    }

    /// チュートリアルが許すか。許さなければ `se_error` と「いまは ここを えらんでね」。
    pub(crate) fn permitted(&mut self, cmd: &Command) -> bool {
        if self.gate().permits(cmd) {
            return true;
        }
        self.refuse(txt::ERR_TUTORIAL);
        false
    }

    /// `permits` → `validate` の順に確かめて積む [13 §1 の 4.]。積めなければ音と案内を出して false。
    pub(crate) fn submit(&mut self, cmd: Command) -> bool {
        if !self.permitted(&cmd) {
            return false;
        }
        match self.view.validate(&cmd) {
            Ok(_) => {
                self.commands.push(cmd);
                true
            }
            Err(e) => {
                self.error(e);
                false
            }
        }
    }
}

/// `CommandError` の案内の文と `{n}` [13 §7.4]。
pub(crate) fn error_text(err: CommandError) -> (TextId, Option<u64>) {
    match err {
        CommandError::InsufficientFunds { .. } => (txt::ERR_MONEY, None),
        CommandError::AtLevelLimit { kind, .. } => match kind {
            LimitKind::CompanyCap => (txt::ERR_LIMIT_COMPANY, None),
            LimitKind::Intrinsic => (txt::ERR_LIMIT_INTRINSIC, None),
            LimitKind::Platform => (txt::ERR_LIMIT_PLATFORM, None),
            LimitKind::Capacity => (txt::ERR_LIMIT_CAPACITY, None),
        },
        CommandError::Locked { required, .. } => (txt::ERR_LOCKED, Some(u64::from(required.0))),
        CommandError::NoFreeLineSlot { .. } => (txt::ERR_NO_SLOT, None),
        CommandError::StationNotConnected(_) => (txt::ERR_NOT_CONNECTED, None),
        CommandError::NoTarget { .. } => (txt::ERR_NO_LINE, None),
        CommandError::DuplicateStation(_) => (txt::ERR_DUPLICATE, None),
        CommandError::StationAlreadyOnLine { .. } => (txt::ERR_ALREADY_ON_LINE, None),
        CommandError::TooFewStations { .. } => (txt::ERR_TOO_FEW, None),
        CommandError::UnknownStation(_)
        | CommandError::UnknownLine(_)
        | CommandError::ScopeMismatch { .. }
        | CommandError::BendCountMismatch { .. } => (txt::ERR_GENERIC, None),
        CommandError::RebirthNotReady => (txt::ERR_REBIRTH_NOT_READY, None),
        CommandError::RebirthUpgradeMaxed { .. } => (txt::ERR_REBIRTH_MAXED, None),
        CommandError::RebirthUpgradePoints {
            required,
            available,
        } => {
            let _ = (required, available);
            (txt::ERR_REBIRTH_POINTS, None)
        }
    }
}

/// `CommandError` の案内を下の帯に出す。
pub(crate) fn show_error(band: &mut Band, now: MonoTime, err: CommandError, dev: InputDevice) {
    let (id, n) = error_text(err);
    let mut buf = [0u8; 4];
    match n {
        Some(n) => band.show_error(
            now,
            id,
            &[TextArg {
                name: "n",
                value: decimal(n, &mut buf),
            }],
            dev,
        ),
        None => band.show_error(now, id, &[], dev),
    }
}

fn axis(input: &ActionFrame, minus: Action, plus: Action) -> i32 {
    if input.held.contains(minus) && input.held.contains(plus) {
        return 0;
    }
    let step = |a| i32::from(input.pressed.contains(a) || input.repeated.contains(a));
    step(plus) - step(minus)
}

/// 上下の 1 歩(反対向きの同時押しは打ち消す)。
pub(crate) fn step_y(input: &ActionFrame) -> i32 {
    axis(input, Action::MoveUp, Action::MoveDown)
}

/// 左右の 1 歩(反対向きの同時押しは打ち消す)。
pub(crate) fn step_x(input: &ActionFrame) -> i32 {
    axis(input, Action::MoveLeft, Action::MoveRight)
}

/// `pressed` と `repeated` で 1 マス。マップ(`size` マス)の範囲で止まる。上と右を同時に押すと斜め、反対向きは打ち消す [13 §3.7]。
pub(super) fn move_cursor(cursor: GridPos, input: &ActionFrame, size: (i32, i32)) -> GridPos {
    GridPos {
        x: (cursor.x + step_x(input)).clamp(0, size.0 - 1),
        y: (cursor.y + step_y(input)).clamp(0, size.1 - 1),
    }
}

fn explaining(d: Option<&TutorialDirective>) -> bool {
    d.is_some_and(|d| d.kind == DirectiveKind::Explanation)
}

/// 決定を受ける説明のページ。せってい の中では、いちばん上の画面の止めない説明(#15)だけ。外では止める説明 [13 §3.4 の 3.・4.]。
pub(super) fn visible_page(
    d: Option<&TutorialDirective>,
    menu: Option<&SettingsMenu>,
) -> Option<TutorialDirective> {
    d.filter(|d| {
        d.kind == DirectiveKind::Explanation && menu.is_none_or(|m| !d.pauses_game && m.is_top())
    })
    .copied()
}

impl PlayScene {
    /// 入力の振り分け [13 §3.4]。`input` は `require_release` を除いたもの。終了中は呼ばない(0.)。
    pub(super) fn dispatch(
        &mut self,
        input: &ActionFrame,
        directive: Option<TutorialDirective>,
        settings: &mut Settings,
        cx: &mut Ctx<'_, '_>,
    ) {
        let shown = visible_page(directive.as_ref(), self.settings_menu.as_ref());
        self.input.watch_page(cx.now, shown.as_ref());
        if input.pressed.contains(Action::Menu) {
            self.toggle_settings(cx);
            return;
        }
        if let Some(menu) = &mut self.settings_menu {
            let mut input = *input;
            if shown.is_some() {
                if input.pressed.contains(Action::Confirm) && self.input.page_ready(cx.now) {
                    acknowledge(cx);
                }
                input = without(&input, Action::Confirm);
            }
            match menu.handle(&input, settings, cx) {
                MenuExit::Stay => {}
                MenuExit::Close => self.close_settings(cx),
                MenuExit::Quit => self.start_quit(cx.now),
            }
            return;
        }
        if shown.is_some() {
            if input.pressed.contains(Action::Confirm) && self.input.page_ready(cx.now) {
                acknowledge(cx);
            }
            self.handle_mode(&only_moves(input), cx);
            return;
        }
        self.handle_mode(input, cx);
    }

    fn toggle_settings(&mut self, cx: &mut Ctx<'_, '_>) {
        match &mut self.settings_menu {
            None => {
                self.settings_menu = Some(SettingsMenu::default());
                cx.se(SE_MENU_OPEN);
                cx.event(UiEvent::MenuOpened(MenuAnchor::SettingsMenu));
            }
            Some(menu) => {
                if menu.back_to_top(cx) == MenuExit::Close {
                    self.close_settings(cx);
                }
            }
        }
    }

    fn close_settings(&mut self, cx: &mut Ctx<'_, '_>) {
        self.settings_menu = None;
        cx.se(SE_MENU_CLOSE);
        cx.event(UiEvent::MenuClosed(MenuAnchor::SettingsMenu));
    }

    fn handle_mode(&mut self, input: &ActionFrame, cx: &mut Ctx<'_, '_>) {
        match &mut self.mode {
            PlayMode::Map => self.handle_map(input, cx),
            PlayMode::Panel(panel) => match panel.handle(input, cx) {
                PanelExit::Stay => {}
                PanelExit::Close => {
                    panel.close(cx);
                    self.last_tab = panel.tab;
                    self.mode = PlayMode::Map;
                }
                PanelExit::Build => {
                    panel.close(cx);
                    self.last_tab = panel.tab;
                    self.start_build(cx);
                }
            },
            PlayMode::Build(build) => {
                if build.uses_map_cursor() {
                    self.cursor = move_cursor(self.cursor, input, map_tiles(cx.view));
                }
                if build.handle(input, self.cursor, cx) {
                    self.mode = PlayMode::Map;
                }
            }
        }
    }

    fn handle_map(&mut self, input: &ActionFrame, cx: &mut Ctx<'_, '_>) {
        self.cursor = move_cursor(self.cursor, input, map_tiles(cx.view));
        let pressed = |a| input.pressed.contains(a);
        if pressed(Action::BuildMode) {
            self.start_build(cx);
        } else if pressed(Action::Confirm) {
            self.confirm_on_map(cx);
        } else if pressed(Action::JumpRecommend) {
            let mut panel = PanelState::open(self.last_tab, None, None, cx);
            match cx.recommendation {
                Some(rec) => panel.jump_to(rec, cx),
                None => panel.jump = true,
            }
            cx.se(SE_CURSOR);
            self.mode = PlayMode::Panel(Box::new(panel));
        } else if pressed(Action::TabPrev) || pressed(Action::TabNext) {
            let panel = PanelState::open(self.last_tab, None, None, cx);
            cx.se(SE_CURSOR);
            self.mode = PlayMode::Panel(Box::new(panel));
        } else if pressed(Action::BuyMax) {
            cx.notice(txt::ERR_MAX_OUTSIDE);
        }
    }

    /// 駅・線路の上の決定で強化パネルを開く [13 §3.8、UI-D-04]。
    fn confirm_on_map(&mut self, cx: &mut Ctx<'_, '_>) {
        let view = cx.view;
        if let Some(id) = view.station_at(self.cursor) {
            let Some(st) = view.station(id) else { return };
            if !st.connected {
                cx.refuse(txt::ERR_NOT_CONNECTED);
                return;
            }
            let line = st.lines.iter().min().copied();
            let panel =
                PanelState::open(PanelTab::Param(ParamCategory::Station), Some(id), line, cx);
            cx.se(SE_CONFIRM);
            self.mode = PlayMode::Panel(Box::new(panel));
        } else if let Some(line) = self.map.as_ref().and_then(|m| m.first_line_at(self.cursor)) {
            let panel =
                PanelState::open(PanelTab::Param(ParamCategory::Train), None, Some(line), cx);
            cx.se(SE_CONFIRM);
            self.mode = PlayMode::Panel(Box::new(panel));
        }
    }

    fn start_build(&mut self, cx: &mut Ctx<'_, '_>) {
        self.mode = PlayMode::Build(Box::new(BuildState::start(cx)));
    }

    /// カメラが見せる場面。表示域の幅はパネルで変わり、路線の確認ダイアログと止める説明の箱(前のフレームの位置)の帯には
    /// カーソルを置かない。
    pub(crate) fn viewport(&self, view: GameView<'_>) -> Viewport {
        let shown = self.explain.shown.filter(|s| s.boxed && !s.under_menu);
        self.viewport_with_box(view, shown.map(|s| s.area))
    }

    /// `viewport` の、止める説明の箱を `area` にしたもの。
    pub(crate) fn viewport_with_box(
        &self,
        view: GameView<'_>,
        area: Option<tm_platform_api::Rect>,
    ) -> Viewport {
        let mut vp = Viewport::new(self.map_w(), MAP_H, world_size(view));
        let band = |r: tm_platform_api::Rect| (r.y - MAP_Y - BOX_GAP, bottom(r) - MAP_Y + BOX_GAP);
        if let PlayMode::Build(b) = &self.mode
            && let Some((y0, y1)) = b.dialog_area().map(band)
        {
            vp = vp.without_band(y0, y1);
        }
        if let Some((x0, x1, (y0, y1))) =
            area.map(|r| (r.x - MAP_X, r.x + r.w as i32 - MAP_X, band(r)))
        {
            vp = vp.without_soft_band((x0, x1), y0, y1);
        }
        vp
    }

    /// カメラの目標を決めて 1 フレーム進める。マップとろせんモードではカーソルを追い、highlight のマップの対象は
    /// カーソルと一緒に見せられるときだけ入れる。パネルでは、選んでいる対象、なければ highlight の対象、なければカーソルを追い、
    /// 対象を追うときは highlight かカーソルを一緒に見せられれば入れる。
    pub(super) fn update_camera(&mut self, view: GameView<'_>) {
        let vp = self.viewport(view);
        let paths = self.map.as_ref().map_or(&[][..], |m| &m.paths[..]);
        let mark = self.explain.focus(view, paths);
        let mut keep = None;
        match &mut self.mode {
            PlayMode::Panel(panel) => {
                let target = panel.focus(view, paths);
                if std::mem::take(&mut panel.jump)
                    && let Some(t) = target
                {
                    self.camera.jump(t, vp);
                } else if let Some(t) = target {
                    self.camera.follow(t, mark, vp);
                } else if let Some(m) = mark {
                    self.camera.follow(m, Some(self.cursor), vp);
                } else {
                    self.camera.follow(self.cursor, None, vp);
                    keep = Some(self.cursor);
                }
            }
            PlayMode::Map | PlayMode::Build(_) => {
                self.camera.follow(self.cursor, mark, vp);
                keep = Some(self.cursor);
            }
        }
        self.camera.step();
        if let Some(c) = keep {
            self.camera.keep_visible(c, vp);
        }
    }

    /// 画面(層)の番号。変わったときに押しっぱなしを持ち越さない(13 §3.4 の 1.)。
    pub(super) fn layer_key(&self, directive: Option<&TutorialDirective>) -> u32 {
        let mode = match &self.mode {
            PlayMode::Map => 0,
            PlayMode::Panel(_) => 1,
            PlayMode::Build(b) => 2 + b.layer_key(),
        };
        let menu = self.settings_menu.as_ref().map_or(0, |m| 1 + m.layer_key());
        let explain = u32::from(self.settings_menu.is_none() && explaining(directive));
        let quit = u32::from(self.quitting());
        mode | menu << 8 | explain << 16 | quit << 24
    }
}

/// 確認ダイアログ・説明の箱と、カーソルのマスの間(カーソルの絵はマスから `RING` はみ出す)。
const BOX_GAP: i32 = crate::layout::RING;

/// 説明の 1 ページを読んだ。
fn acknowledge(cx: &mut Ctx<'_, '_>) {
    cx.se(SE_CONFIRM);
    cx.event(UiEvent::MessageAcknowledged);
}

#[cfg(test)]
mod tests {
    use tm_input::ActionSet;

    use super::*;

    fn frame(pressed: &[Action], held: &[Action]) -> ActionFrame {
        let mut f = ActionFrame::default();
        for &a in pressed {
            f.pressed.insert(a);
        }
        for &a in held {
            f.held.insert(a);
        }
        f
    }

    #[test]
    fn steps_clamps_and_cancels() {
        use crate::layout::{MAP_COLS, MAP_ROWS};
        let size = (MAP_COLS, MAP_ROWS);
        let p = GridPos { x: 5, y: 5 };
        assert_eq!(
            move_cursor(p, &frame(&[Action::MoveRight], &[]), size),
            GridPos { x: 6, y: 5 }
        );
        assert_eq!(
            move_cursor(p, &frame(&[Action::MoveUp, Action::MoveRight], &[]), size),
            GridPos { x: 6, y: 4 }
        );
        assert_eq!(
            move_cursor(
                p,
                &frame(&[Action::MoveLeft], &[Action::MoveLeft, Action::MoveRight]),
                size
            ),
            p
        );
        let corner = GridPos {
            x: 0,
            y: MAP_ROWS - 1,
        };
        assert_eq!(
            move_cursor(
                corner,
                &frame(&[Action::MoveLeft, Action::MoveDown], &[]),
                size
            ),
            corner
        );
        let edge = GridPos { x: 31, y: 3 };
        assert_eq!(
            move_cursor(edge, &frame(&[Action::MoveRight], &[]), (32, 18)),
            edge,
            "表示域より小さいマップでは、マップの右端で止まる"
        );
        assert_eq!(
            move_cursor(edge, &frame(&[Action::MoveRight], &[]), size),
            GridPos { x: 32, y: 3 }
        );
        let mut repeated = ActionSet::default();
        repeated.insert(Action::MoveDown);
        let rep = ActionFrame {
            repeated,
            ..ActionFrame::default()
        };
        assert_eq!(move_cursor(p, &rep, size), GridPos { x: 5, y: 6 });
        assert_eq!(
            move_cursor(p, &frame(&[], &[Action::MoveDown]), size),
            p,
            "held だけでは動かない"
        );
    }

    #[test]
    fn every_error_has_a_text() {
        use tm_core::{CompanyLevel, Feature, Level, LineId, ParamKey, StationId, UpgradeScope};
        let money = tm_numeric::BigNum::from_u64(1);
        let cases = [
            (
                CommandError::InsufficientFunds {
                    required: money,
                    available: money,
                },
                txt::ERR_MONEY,
            ),
            (
                CommandError::AtLevelLimit {
                    param: ParamKey::T1Speed,
                    limit: Level(1),
                    kind: LimitKind::CompanyCap,
                },
                txt::ERR_LIMIT_COMPANY,
            ),
            (
                CommandError::AtLevelLimit {
                    param: ParamKey::T1Speed,
                    limit: Level(1),
                    kind: LimitKind::Intrinsic,
                },
                txt::ERR_LIMIT_INTRINSIC,
            ),
            (
                CommandError::AtLevelLimit {
                    param: ParamKey::T5Cars,
                    limit: Level(1),
                    kind: LimitKind::Platform,
                },
                txt::ERR_LIMIT_PLATFORM,
            ),
            (
                CommandError::AtLevelLimit {
                    param: ParamKey::S1Spawn,
                    limit: Level(1),
                    kind: LimitKind::Capacity,
                },
                txt::ERR_LIMIT_CAPACITY,
            ),
            (
                CommandError::Locked {
                    feature: Feature::LineExtend,
                    required: CompanyLevel(3),
                },
                txt::ERR_LOCKED,
            ),
            (
                CommandError::NoFreeLineSlot { used: 1, slots: 1 },
                txt::ERR_NO_SLOT,
            ),
            (
                CommandError::StationNotConnected(StationId(0)),
                txt::ERR_NOT_CONNECTED,
            ),
            (
                CommandError::NoTarget {
                    param: ParamKey::S1Spawn,
                },
                txt::ERR_NO_LINE,
            ),
            (
                CommandError::DuplicateStation(StationId(0)),
                txt::ERR_DUPLICATE,
            ),
            (
                CommandError::StationAlreadyOnLine {
                    line: LineId(0),
                    station: StationId(0),
                },
                txt::ERR_ALREADY_ON_LINE,
            ),
            (CommandError::TooFewStations { got: 1 }, txt::ERR_TOO_FEW),
            (CommandError::UnknownStation(StationId(9)), txt::ERR_GENERIC),
            (
                CommandError::ScopeMismatch {
                    param: ParamKey::S1Spawn,
                    scope: UpgradeScope::Company,
                },
                txt::ERR_GENERIC,
            ),
        ];
        for (e, want) in cases {
            assert_eq!(error_text(e).0, want, "{e:?}");
        }
        assert_eq!(
            error_text(CommandError::Locked {
                feature: Feature::LineExtend,
                required: CompanyLevel(3)
            })
            .1,
            Some(3)
        );
    }
}
