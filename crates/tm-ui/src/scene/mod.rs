//! 画面の状態(タイトル・プレイ)と、その更新と描画 [13 §3.2、§3.3]。

pub(crate) mod build;
pub(crate) mod explain;
pub(crate) mod panel;
mod play;
pub(crate) mod settings_menu;

use tm_assets::font::FontId;
use tm_assets::sound::{SE_CANCEL, SE_CONFIRM, SE_CURSOR};
use tm_assets::sprite;
use tm_assets::txt;
use tm_core::{
    Command, CommandKind, Dir8, GameEvent, GameView, GridPos, ParamCategory, Recommendation,
};
use tm_input::{Action, ActionFrame, InputDevice};
use tm_platform_api::{MonoTime, SoundId};
use tm_tutorial::{Tutorial, TutorialDirective, UiEvent};

pub(crate) use build::BuildState;
pub(crate) use explain::ExplainView;
pub(crate) use panel::{PanelState, PanelTab};
pub(crate) use play::UiSound;
use play::{Ctx, Gate, show_error};
pub(crate) use settings_menu::SettingsMenu;
use settings_menu::{Answer, ConfirmBox, answer};

use crate::anim::Anims;
use crate::camera::Camera;
use crate::draw::DrawBuilder;
use crate::draw::map::{self, MapCache};
use crate::draw::palette;
use crate::hud::{Band, BandChips, Hud, InfoRow};
use crate::input_ctx::{Guard, InputCtx};
use crate::layout::{
    BAND_RIGHT, BAND_ROW2, MAP_COLS, MAP_ROWS, POWER_BOX, SAVING_BOX, SCREEN_H, SCREEN_W, TILE,
    TITLE_ROWS, right,
};
use crate::request::{AppRequest, AppStatus};
use crate::settings::Settings;
use crate::text::numbers::NumText;
use crate::text::{DynText, TextCache, TextRef};
use crate::tutorial_bridge::Bridge;

/// タイトルが出てから決定を受け付けるまで [UI-D-13]。
const TITLE_GUARD_US: u64 = 500_000;
/// おすすめを取り直す間隔 [UI-D-28]。
const RECOMMEND_US: u64 = 500_000;

/// 画面の状態。
pub(crate) enum Scene {
    /// タイトル。
    Title(Box<TitleScene>),
    /// ゲーム中。
    Play(Box<PlayScene>),
}

/// タイトルのメニューの項目。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TitleItem {
    /// 続きから。
    Continue,
    /// 初めから。
    #[default]
    NewGame,
}

/// タイトルで決めたこと。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitleExit {
    /// まだ。
    Stay,
    /// 初めから(セーブがあれば確認の 2 回目の「はい」)。
    NewGame,
    /// 続きから。
    Continue,
}

/// 「初めから」の確認(セーブがあるとき 2 回)。初期は「いいえ」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TitleConfirm {
    /// 1 か 2。
    pub step: u8,
    /// 「はい」を選んでいる。
    pub yes: bool,
    guard: Guard,
}

/// タイトル画面 [13 §3.3、UI-D-32]。
#[derive(Default)]
pub(crate) struct TitleScene {
    guard_until: Option<MonoTime>,
    has_save: bool,
    /// 選んでいる項目。セーブがあれば「続きから」から。
    pub selected: TitleItem,
    /// 「初めから」の確認。
    pub confirm: Option<TitleConfirm>,
    subtitle: Option<TextRef>,
    rows: [Option<TextRef>; 2],
    hint: DynText,
    dialog: ConfirmBox,
    /// 操作の音。`Ui` が毎フレーム取り出す。
    pub sounds: Vec<SoundId>,
}

/// かざりの線路の上端と、マスの数(マップと同じ大きさで描く)。
const DECO_Y: i32 = 166;
const DECO_TILES: i32 = 10;
/// かざりの 1 マス目の左端(中央の駅が画面の中央に来る)。
const DECO_X: i32 = SCREEN_W / 2 - DECO_TILES / 2 * TILE - TILE / 2;
const TITLE_TEXT_W: u16 = SCREEN_W as u16;

impl TitleScene {
    /// 見せる項目(上から)。セーブがなければ「初めから」だけ。
    pub(crate) fn items(&self) -> &'static [TitleItem] {
        if self.has_save {
            &[TitleItem::Continue, TitleItem::NewGame]
        } else {
            &[TitleItem::NewGame]
        }
    }

    /// 入力を受ける。上下で選び、決定はガード(500ms)の後 [13 §3.4 の 6.]。
    pub(crate) fn update(
        &mut self,
        now: MonoTime,
        input: &ActionFrame,
        text: &mut TextCache,
        app: AppStatus,
    ) -> TitleExit {
        let dev = input.last_device;
        let first = self.guard_until.is_none();
        let guard = *self
            .guard_until
            .get_or_insert(MonoTime(now.0 + TITLE_GUARD_US));
        if first && app.has_save {
            self.selected = TitleItem::Continue;
        }
        self.has_save = app.has_save;
        if !self.has_save {
            self.selected = TitleItem::NewGame;
            self.confirm = None;
        }
        let exit = match self.confirm {
            Some(c) => self.handle_confirm(c, now, input),
            None => self.handle_menu(now, guard, input),
        };
        self.prepare(text, dev);
        exit
    }

    fn handle_menu(&mut self, now: MonoTime, guard: MonoTime, input: &ActionFrame) -> TitleExit {
        let items = self.items();
        let at = items.iter().position(|&i| i == self.selected).unwrap_or(0);
        let next = (at as i32 + play::step_y(input)).clamp(0, items.len() as i32 - 1) as usize;
        if next != at {
            self.selected = items[next];
            self.sounds.push(SE_CURSOR);
        }
        if now < guard || !input.pressed.contains(Action::Confirm) {
            return TitleExit::Stay;
        }
        match self.selected {
            TitleItem::Continue => TitleExit::Continue,
            TitleItem::NewGame if self.has_save => {
                self.sounds.push(SE_CONFIRM);
                self.confirm = Some(TitleConfirm {
                    step: 1,
                    yes: false,
                    guard: Guard::from(now),
                });
                TitleExit::Stay
            }
            TitleItem::NewGame => TitleExit::NewGame,
        }
    }

    fn handle_confirm(
        &mut self,
        mut c: TitleConfirm,
        now: MonoTime,
        input: &ActionFrame,
    ) -> TitleExit {
        let a = answer(input, &mut c.yes, c.guard, now, &mut self.sounds);
        self.confirm = Some(c);
        match a {
            Answer::None => {}
            Answer::Cancel => {
                self.sounds.push(SE_CANCEL);
                self.confirm = None;
            }
            Answer::No => {
                self.sounds.push(SE_CONFIRM);
                self.confirm = None;
            }
            Answer::Yes if c.step == 1 => {
                self.sounds.push(SE_CONFIRM);
                self.confirm = Some(TitleConfirm {
                    step: 2,
                    yes: false,
                    guard: Guard::from(now),
                });
            }
            Answer::Yes => {
                self.confirm = None;
                return TitleExit::NewGame;
            }
        }
        TitleExit::Stay
    }

    fn prepare(&mut self, text: &mut TextCache, dev: InputDevice) {
        self.subtitle = Some(text.get(txt::TITLE_SUBTITLE, FontId::Head16, TITLE_TEXT_W, dev));
        let items = self.items();
        for (i, slot) in self.rows.iter_mut().enumerate() {
            *slot = items.get(i).map(|&item| {
                let id = match item {
                    TitleItem::Continue => txt::TITLE_CONTINUE,
                    TitleItem::NewGame => txt::TITLE_NEW_GAME,
                };
                text.get(id, FontId::Head16, TITLE_TEXT_W, dev)
            });
        }
        let hint = match self.confirm {
            Some(c) => {
                self.dialog.prepare(
                    if c.step == 1 {
                        txt::TITLE_NEW_CONFIRM1
                    } else {
                        txt::RESET_CONFIRM2
                    },
                    dev,
                );
                txt::BUILD_CHOOSE_HINT
            }
            None => txt::TITLE_HINT,
        };
        self.hint
            .set(hint, &[], FontId::Body12, BAND_RIGHT.w as u16, dev);
    }

    /// いまの確認の文(テスト用)。
    #[cfg(test)]
    pub(crate) fn dialog_text(&self) -> Option<tm_assets::text::TextId> {
        self.confirm.and(self.dialog.text())
    }

    /// タイトルを描く。
    pub(crate) fn draw(&self, b: &mut DrawBuilder<'_>, text: &TextCache, now: MonoTime) {
        b.clear(palette::PANEL);
        b.sprite(sprite::TITLE_LOGO, SCREEN_W / 2, 64, palette::WHITE);
        if let Some(r) = self.subtitle {
            let blk = text.block(r);
            b.text(
                blk,
                (SCREEN_W - i32::from(blk.width)) / 2,
                108,
                palette::TEXT,
            );
        }
        for table in [&sprite::TRK_RAIL_O, &sprite::TRK_RAIL_F] {
            for x in 0..=DECO_TILES {
                let lx = DECO_X + x * TILE;
                if x < DECO_TILES {
                    b.map_sprite(table[Dir8::E as usize], lx, DECO_Y, palette::WHITE);
                }
                if x > 0 {
                    b.map_sprite(table[Dir8::W as usize], lx, DECO_Y, palette::WHITE);
                }
            }
        }
        for x in [0, DECO_TILES / 2, DECO_TILES] {
            b.map_sprite(
                sprite::STN_BODY,
                DECO_X + x * TILE + TILE / 2,
                DECO_Y + TILE / 2,
                palette::WHITE,
            );
        }
        let span = (DECO_TILES * TILE) as u64;
        let tx = DECO_X + TILE / 2 + ((now.0 / 12_000) % span) as i32;
        b.map_sprite(
            sprite::TRN[Dir8::E as usize],
            tx,
            DECO_Y + TILE / 2,
            palette::LINES[0],
        );
        b.map_sprite(
            sprite::TRN_GLASS[Dir8::E as usize],
            tx,
            DECO_Y + TILE / 2,
            palette::WHITE,
        );
        match self.confirm {
            Some(c) => self.dialog.draw(b, c.yes),
            None => self.draw_menu(b, text),
        }
        let hw = i32::from(self.hint.block.width);
        b.text(
            &self.hint.block,
            right(BAND_RIGHT) - hw,
            BAND_ROW2.y + (BAND_ROW2.h as i32 - self.hint.block.height()) / 2,
            palette::TEXT,
        );
    }

    fn draw_menu(&self, b: &mut DrawBuilder<'_>, text: &TextCache) {
        let items = self.items();
        for ((r, slot), item) in TITLE_ROWS.iter().zip(&self.rows).zip(items) {
            let Some(t) = slot else { continue };
            if *item == self.selected {
                b.nine_slice(&sprite::NS_UI_SEL, *r, palette::WHITE);
                b.sprite(
                    sprite::UI_MENU_CURSOR,
                    r.x + 4,
                    r.y + r.h as i32 / 2,
                    palette::WHITE,
                );
            }
            let blk = text.block(*t);
            b.text(
                blk,
                r.x + (r.w as i32 - i32::from(blk.width)) / 2,
                r.y + (r.h as i32 - blk.height()) / 2,
                palette::TEXT,
            );
        }
    }
}

/// 終了中の表示(「ゲームを終わる」の 2 回目の「はい」の後。13 §3.3、UI-D-33)。
pub(crate) struct QuitView {
    /// 2 回目の「はい」を押した時刻。
    pub at: MonoTime,
    saving: DynText,
    power: DynText,
}

impl QuitView {
    pub(crate) fn new(at: MonoTime) -> QuitView {
        QuitView {
            at,
            saving: DynText::default(),
            power: DynText::default(),
        }
    }

    fn prepare(&mut self, dev: InputDevice) {
        self.saving.set(
            txt::QUIT_SAVING,
            &[],
            FontId::Body12,
            SAVING_BOX.w as u16,
            dev,
        );
        self.power.set(
            txt::QUIT_POWER_OFF,
            &[],
            FontId::Head16,
            POWER_BOX.w as u16 - 2 * QUIT_PAD as u16,
            dev,
        );
    }

    /// 出す箱。保存の間は保存の箱、保存の後(要求の次のフレームから)で電源を切るなら電源の箱。
    pub(crate) fn shown(&self, now: MonoTime, app: AppStatus) -> QuitBox {
        if app.saving {
            QuitBox::Saving
        } else if app.power_off_on_quit && now > self.at {
            QuitBox::PowerOff
        } else {
            QuitBox::None
        }
    }

    fn draw(&self, b: &mut DrawBuilder<'_>, now: MonoTime, app: AppStatus) {
        match self.shown(now, app) {
            QuitBox::None => {}
            QuitBox::Saving => {
                let r = SAVING_BOX;
                b.nine_slice(&sprite::NS_UI_WIN, r, palette::WHITE);
                let blk = &self.saving.block;
                let w = SAVE_ICON + SAVE_ICON_GAP + i32::from(blk.width);
                let x = r.x + (r.w as i32 - w) / 2;
                b.sprite(
                    sprite::ICO_SAVE,
                    x,
                    r.y + (r.h as i32 - SAVE_ICON) / 2,
                    palette::WHITE,
                );
                b.text(
                    blk,
                    x + SAVE_ICON + SAVE_ICON_GAP,
                    r.y + (r.h as i32 - blk.height()) / 2,
                    palette::TEXT,
                );
            }
            QuitBox::PowerOff => {
                let r = POWER_BOX;
                b.nine_slice(&sprite::NS_UI_WIN, r, palette::WHITE);
                let blk = &self.power.block;
                b.text(
                    blk,
                    r.x + QUIT_PAD,
                    r.y + (r.h as i32 - blk.height()) / 2,
                    palette::TEXT,
                );
            }
        }
    }
}

/// 終了中に出す箱。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuitBox {
    /// うすい黒だけ。
    None,
    /// 保存しているよ。
    Saving,
    /// 電源を切るよ(実機)。
    PowerOff,
}

/// `ico_save` の大きさと文との間。
const SAVE_ICON: i32 = 12;
const SAVE_ICON_GAP: i32 = 4;
/// 電源の箱の左右の余白。
const QUIT_PAD: i32 = 16;

/// プレイ中のモード [13 §3.2]。
pub(crate) enum PlayMode {
    /// マップ。
    Map,
    /// 強化パネル。
    Panel(Box<PanelState>),
    /// ろせんモード。
    Build(Box<BuildState>),
}

/// ゲーム中の画面。
pub(crate) struct PlayScene {
    /// マップのカーソル。
    pub cursor: GridPos,
    cursor_ready: bool,
    /// このフレームで始めのカメラを置いた(最初の説明の箱に合わせて、動きを見せずに置き直せる)。
    camera_new: bool,
    /// カメラ。
    pub camera: Camera,
    /// モード。
    pub mode: PlayMode,
    /// せってい(開いているときだけ)。
    pub settings_menu: Option<SettingsMenu>,
    /// `map_revision` ごとに作り直すもの。
    pub map: Option<MapCache>,
    /// HUD。
    pub hud: Hud,
    info: InfoRow,
    info_cursor: Option<GridPos>,
    /// 下の帯。
    pub band: Band,
    /// 演出。
    pub anims: Anims,
    station_tags: Vec<NumText>,
    recommendation: Option<Recommendation>,
    rec_refresh_at: Option<MonoTime>,
    input: InputCtx,
    last_tab: PanelTab,
    dev: InputDevice,
    /// UI の操作で鳴らす音。`Ui` が毎フレーム取り出す。
    pub sounds: Vec<UiSound>,
    /// チュートリアルへの出来事。`Ui` が毎フレーム取り出して `observe_ui` に渡す。
    pub events: Vec<UiEvent>,
    /// カーソルの下と directive の変化の記録。
    pub bridge: Bridge,
    /// 説明と案内の表示データ。
    pub explain: ExplainView,
    /// 終了中(`None` でなければ入力をすべて無視する)。
    pub quit: Option<QuitView>,
    /// 静的な層を作り直した回数(テスト用)。
    #[cfg(test)]
    pub map_builds: u32,
    /// チュートリアルの directive の代わり(テスト用)。
    #[cfg(test)]
    pub fake_directive: Option<TutorialDirective>,
    /// `permits` の代わりに false を返す条件(テスト用)。
    #[cfg(test)]
    pub forbid: Option<fn(&Command) -> bool>,
}

impl PlayScene {
    /// 新しいゲームの画面。
    pub(crate) fn new() -> PlayScene {
        PlayScene {
            cursor: GridPos { x: 0, y: 0 },
            cursor_ready: false,
            camera_new: false,
            camera: Camera::default(),
            mode: PlayMode::Map,
            settings_menu: None,
            map: None,
            hud: Hud::default(),
            info: InfoRow::default(),
            info_cursor: None,
            band: Band::default(),
            anims: Anims::default(),
            station_tags: Vec::new(),
            recommendation: None,
            rec_refresh_at: None,
            input: InputCtx::default(),
            last_tab: PanelTab::Param(ParamCategory::Station),
            dev: InputDevice::Keyboard,
            sounds: Vec::new(),
            events: Vec::new(),
            bridge: Bridge::default(),
            explain: ExplainView::default(),
            quit: None,
            #[cfg(test)]
            map_builds: 0,
            #[cfg(test)]
            fake_directive: None,
            #[cfg(test)]
            forbid: None,
        }
    }

    /// いまの directive(テストでは代わりのものがあればそれ)。
    pub(crate) fn directive(&self, tutorial: Option<&Tutorial>) -> Option<TutorialDirective> {
        #[cfg(test)]
        if self.fake_directive.is_some() {
            return self.fake_directive;
        }
        tutorial.and_then(|t| t.directive().copied())
    }

    /// 1 フレームの更新。`game` がまだないフレームは何もしない(13 §3.3)。
    #[allow(clippy::too_many_arguments, reason = "Ui::update の引数をそのまま渡す")]
    pub(crate) fn update(
        &mut self,
        now: MonoTime,
        raw: &ActionFrame,
        game: Option<GameView<'_>>,
        mut tutorial: Option<&mut Tutorial>,
        settings: &mut Settings,
        app: AppStatus,
        commands: &mut Vec<Command>,
        requests: &mut Vec<AppRequest>,
        layer_versions: &mut u64,
    ) {
        let Some(view) = game else {
            return;
        };
        self.hud.set_saving(now, app.saving);
        let dev = raw.last_device;
        self.dev = dev;
        if !self.cursor_ready {
            self.cursor = initial_cursor(view);
            self.cursor_ready = true;
            let at = view.stations().map(|s| s.pos);
            self.camera.start(at, self.cursor, self.viewport(view));
            self.camera_new = true;
        }
        // 出現の演出が終わった駅を層に入れるときも作り直す [13 §3.11]
        if self.map.as_ref().is_none_or(|m| {
            m.revision != view.map_revision()
                || !m.hidden.iter().copied().eq(self.anims.hidden_stations(now))
        }) {
            *layer_versions += 1;
            let hidden = self.anims.hidden_stations(now).collect();
            self.map = Some(MapCache::build(view, *layer_versions, hidden));
            #[cfg(test)]
            {
                self.map_builds += 1;
            }
        }
        if self.rec_refresh_at.is_none_or(|t| now >= t) {
            self.recommendation = view.recommendation();
            self.rec_refresh_at = Some(MonoTime(now.0 + RECOMMEND_US));
        }

        let input = self.input.filter(raw);
        let directive = self.directive(tutorial.as_deref());
        let mut band = std::mem::take(&mut self.band);
        let mut sounds = std::mem::take(&mut self.sounds);
        let mut events = std::mem::take(&mut self.events);
        let mut cx = Ctx {
            now,
            view,
            dev,
            tutorial: tutorial.as_deref_mut(),
            recommendation: self.recommendation,
            cam: self.camera.target,
            commands,
            requests,
            sounds: &mut sounds,
            events: &mut events,
            band: &mut band,
            #[cfg(test)]
            forbid: self.forbid,
        };
        if self.quit.is_none() {
            self.dispatch(&input, directive, settings, &mut cx);
        }
        self.band = band;
        self.sounds = sounds;
        self.events = events;
        self.update_camera(view);

        let refreshed = self.hud.update(now, view, dev);
        if refreshed {
            self.refresh_tags(view);
        }
        if (refreshed || self.info_cursor != Some(self.cursor))
            && let Some(map) = &self.map
        {
            self.info.update(view, map, self.cursor, dev);
            self.info_cursor = Some(self.cursor);
        }
        let gate = Gate::new(tutorial.as_deref());
        #[cfg(test)]
        let gate = gate.with_forbid(self.forbid);
        let (mode_text, chips) = match &mut self.mode {
            PlayMode::Map => (None, BandChips::Map),
            PlayMode::Panel(p) => {
                p.refresh(view, gate, self.recommendation, refreshed, dev);
                (None, BandChips::Map)
            }
            PlayMode::Build(b) => {
                b.prepare(view, self.cursor, refreshed, dev);
                b.band_mode()
            }
        };
        let chips = self
            .settings_menu
            .as_ref()
            .map_or(chips, SettingsMenu::band_hint);
        let chips = if self.quitting() {
            BandChips::None
        } else {
            chips
        };
        self.band.set_mode(mode_text, chips, dev);
        if let Some(m) = &mut self.settings_menu {
            m.prepare(settings, app.save_enabled, dev);
        }
        if let Some(q) = &mut self.quit {
            q.prepare(dev);
        }
        let star = self.recommendation.is_some_and(|r| r.affordable);
        self.band.update(now, dev, star);
        self.anims.prune(now);
        self.input
            .end_frame(self.layer_key(directive.as_ref()), raw.held);
    }

    fn refresh_tags(&mut self, view: GameView<'_>) {
        if self.station_tags.len() < view.stations().len() {
            self.station_tags
                .resize_with(view.stations().len(), NumText::default);
        }
        for st in view.stations() {
            if map::waiting_marks(st.waiting.0).1 {
                self.station_tags[st.id.index()].set_people(st.waiting.0, FontId::Body12);
            }
        }
    }

    /// せってい が開いているか(開いている間はゲームを止める。01 §11.3)。
    pub(crate) fn settings_open(&self) -> bool {
        self.settings_menu.is_some()
    }

    /// 終了中か。
    pub(crate) fn quitting(&self) -> bool {
        self.quit.is_some()
    }

    /// 終わる確認の 2 回目の「はい」: せってい を閉じて終了中にする(音と UiEvent は出さない)。
    pub(super) fn start_quit(&mut self, now: MonoTime) {
        self.settings_menu = None;
        self.quit = Some(QuitView::new(now));
    }

    /// コアのイベントを演出に変え、路線の送信の結果を受ける(音は IMP-208)。
    pub(crate) fn observe(&mut self, now: MonoTime, ev: &GameEvent) {
        if let PlayMode::Build(b) = &mut self.mode
            && b.observe(ev, now, &mut self.band, self.dev, &mut self.events)
        {
            self.mode = PlayMode::Map;
        }
        match *ev {
            GameEvent::TrainArrived {
                station, income, ..
            } => self.anims.income(now, station, income),
            GameEvent::Purchased { param, scope, .. } => {
                if let PlayMode::Panel(p) = &mut self.mode {
                    p.purchased(param, now);
                }
                self.hud.refresh.force();
                self.rec_refresh_at = None;
                self.anims.level_up(now, scope);
            }
            GameEvent::CommandRejected { kind, reason } => {
                self.hud.refresh.force();
                if matches!(kind, CommandKind::LevelUp { .. }) {
                    show_error(&mut self.band, now, reason, self.dev);
                }
            }
            GameEvent::CompanyLevelUp { new_level, .. } => self.anims.company_up(now, new_level.0),
            GameEvent::FeatureUnlocked { company_level, .. } => {
                self.anims.unlocked(company_level.0)
            }
            GameEvent::MoneyUnitReached { unit } => self.anims.unit_reached(now, unit),
            GameEvent::PassThroughAdded { .. } => self.anims.pass_through(now),
            GameEvent::GoalReached { .. } => self.anims.goal(now),
            GameEvent::LineCreated { line, .. } | GameEvent::LineExtended { line, .. } => {
                self.anims.line_done(now, line)
            }
            GameEvent::StationSpawned { id, .. } => self.anims.station_spawned(now, id),
            GameEvent::TrainAdded { train, .. } => self.anims.train_added(now, train),
            GameEvent::Rebirth { points_earned, .. } => {
                self.hud.refresh.force();
                self.anims.rebirth(now, points_earned);
            }
            GameEvent::RebirthUpgraded { .. } => self.hud.refresh.force(),
            GameEvent::PassThroughResolved { .. } | GameEvent::RoutesRecomputed { .. } => {}
        }
    }

    /// 描く。`game` がない、または表示データがまだないフレームは背景だけ。
    pub(crate) fn draw(
        &self,
        b: &mut DrawBuilder<'_>,
        game: Option<GameView<'_>>,
        now: MonoTime,
        settings: &Settings,
        app: AppStatus,
    ) {
        b.clear(palette::BLACK);
        let (Some(view), Some(map)) = (game, self.map.as_ref()) else {
            return;
        };
        let cam = self.camera.pos;
        let view_w = self.map_w();
        map::draw_static(b, map, cam, view_w);
        self.anims.draw_glow(b, &map.paths, now, cam, view_w);
        if let PlayMode::Build(build) = &self.mode {
            build.draw_map(b, view, now, cam);
        }
        map::draw_trains(b, view, cam, view_w);
        let popups = self.anims.popups.layout(view, now, cam, view_w);
        map::draw_waiting(b, view, &self.station_tags, &popups, cam, view_w);
        self.anims.draw_fx(b, view, now, cam, view_w, &popups);
        if let PlayMode::Panel(panel) = &self.mode {
            panel.draw_targets(b, view, cam);
        }
        map::draw_cursor(b, self.cursor, now, cam);
        if let PlayMode::Panel(panel) = &self.mode {
            panel.draw(b, now);
        }
        self.hud.draw(b, now);
        if !matches!(&self.mode, PlayMode::Build(build) if build.draw_cost(b)) {
            self.info.draw(b);
        }
        self.band.draw(b);
        if let PlayMode::Build(build) = &self.mode {
            build.draw_ui(b);
        }
        self.anims.draw_overlay(b, now);
        if let PlayMode::Build(build) = &self.mode {
            build.draw_dialog(b);
        }
        if let Some(q) = &self.quit {
            b.fill(SCREEN, DIM);
            q.draw(b, now, app);
            return;
        }
        self.draw_explain(b, now, settings);
    }
}

/// 止める説明と せってい の下に重ねる うすい黒(層 7)。
const DIM: tm_platform_api::Color = palette::with_alpha(palette::BLACK, 128);
const SCREEN: tm_platform_api::Rect = crate::draw::rect(0, 0, SCREEN_W as u32, SCREEN_H as u32);

/// マップのマス数 (横, 縦)。上限(40 × 18)より大きければ上限に切り詰める。
pub(crate) fn map_tiles(view: GameView<'_>) -> (i32, i32) {
    let (w, h) = view.map_size();
    debug_assert!(i32::from(w) <= MAP_COLS && i32::from(h) <= MAP_ROWS);
    (i32::from(w).min(MAP_COLS), i32::from(h).min(MAP_ROWS))
}

/// マップ全体の大きさ(層の座標の px)。
pub(crate) fn world_size(view: GameView<'_>) -> (i32, i32) {
    let (w, h) = map_tiles(view);
    (w * TILE, h * TILE)
}

/// マップの中央のマス(40 × 18 なら (20, 9))。
pub(crate) fn cursor_start(view: GameView<'_>) -> GridPos {
    let (w, h) = map_tiles(view);
    GridPos { x: w / 2, y: h / 2 }
}

/// マップの中央。駅なら左へずらし、駅でない最初のマスにする [UI-D-27]。
fn initial_cursor(view: GameView<'_>) -> GridPos {
    let start = cursor_start(view);
    let mut p = start;
    while view.station_at(p).is_some() && p.x > 0 {
        p.x -= 1;
    }
    if view.station_at(p).is_some() {
        p = start;
        while view.station_at(p).is_some() && p.x + 1 < map_tiles(view).0 {
            p.x += 1;
        }
    }
    p
}
