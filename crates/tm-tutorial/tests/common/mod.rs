//! 詰み検査の「自動の子ども」(13 §10.3)。`Game` と `Tutorial` を tm-app と同じ順で回す。
//! 使う tm-simrun の API: `decide`、`Player`、`Decision`、`GameWorld::{new, command, decide_saving}`、`strategy::{extend_nearer, all_scope}`。

#![allow(dead_code)]

use std::sync::Arc;

use tm_assets::text::{TextId, txt};
use tm_core::{
    BalanceTable, BuyMode, Command, CommandError, Game, GameConfig, GameEvent, LineEnd, LineId,
    ParamKey, StationId, UpgradeScope,
};
use tm_simrun::strategy::{all_scope, extend_nearer};
use tm_simrun::{GameWorld, Player, decide};
use tm_tutorial::{DirectiveKind, MenuAnchor, Tutorial, TutorialDirective, UiAnchor, UiEvent};

pub const FPS: u64 = 60;
pub const SEC: u64 = FPS;
pub const MIN: u64 = 60 * FPS;
/// 合格の条件: ゲームの時間でこの tick 数以内に終わる。
pub const LIMIT_TICKS: u64 = 40 * MIN;
/// 同じ directive のまま動かなければ詰みとみなす時間(序盤の制限のある段階)。
pub const STILL_LIMIT: u64 = 5 * MIN;

/// 決定論の乱数(splitmix64)。
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    pub fn percent(&mut self, p: u64) -> bool {
        self.below(100) < p
    }

    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
}

/// 「しない」の間の子どものふるまい。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffPlay {
    /// 何もしない。
    Idle,
    /// カーソルだけ駅に動かす。
    Cursor,
    /// 戦略どおりに遊ぶ(3 駅の路線を作ることもある)。
    Play,
}

/// directive が `at_change` 回目に変わったときに「しない」にし、`off` フレーム後に「する」に戻す。
#[derive(Clone, Copy, Debug)]
pub struct TogglePlan {
    pub at_change: usize,
    pub off: u64,
    pub play: OffPlay,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub show: bool,
    pub random_settings: bool,
    pub deviate_percent: u64,
    pub drop_fanfare_percent: u64,
    pub plan: Option<TogglePlan>,
    /// 平均この フレームに 1 回、`save` → `restore` で続きから始め直す(0 はしない)。
    pub resume_every: u64,
}

impl Options {
    /// 13 §10.3 の通しテストの子ども。
    pub const RANDOM: Options = Options {
        show: true,
        random_settings: true,
        deviate_percent: 20,
        drop_fanfare_percent: 5,
        plan: None,
        resume_every: 45 * SEC,
    };
}

#[derive(Clone, Copy, Debug)]
struct Settings {
    close_at: u64,
    toggle_at: Option<u64>,
    ack_at: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Log {
    pub refused: u32,
    pub toggles: u32,
    pub settings: u32,
    pub acks: u32,
    pub commands: u32,
    pub fanfares_dropped: u32,
    pub resumes: u32,
    pub changes: Vec<(u8, DirectiveKind, u8)>,
}

pub struct Kid {
    pub game: Game,
    pub t: Tutorial,
    pub show: bool,
    pub frame: u64,
    pub log: Log,
    events: Vec<GameEvent>,
    rng: Rng,
    resume_rng: Rng,
    player: Player,
    opts: Options,
    settings: Option<Settings>,
    ack_at: Option<u64>,
    act_at: u64,
    fanfare_at: Option<u64>,
    cursor: Option<StationId>,
    last: Option<TutorialDirective>,
    noted: Option<TutorialDirective>,
    still_since: u64,
    off_until: Option<u64>,
}

fn sample_cmds() -> [Command; 4] {
    [
        Command::CreateLine {
            stations: vec![StationId(0), StationId(1)],
            bends: vec![tm_core::Bend::StraightFirst],
        },
        Command::ExtendLine {
            line: LineId(0),
            end: LineEnd::Tail,
            station: StationId(2),
            bend: tm_core::Bend::StraightFirst,
        },
        Command::LevelUp {
            param: ParamKey::S1Spawn,
            scope: UpgradeScope::AllStations,
            mode: BuyMode::Max,
        },
        Command::LevelUp {
            param: ParamKey::S2Fare,
            scope: UpgradeScope::AllStations,
            mode: BuyMode::One,
        },
    ]
}

/// 案内の文から、買うように案内しているパラメータを引く。
fn guide_param(m: TextId) -> Option<ParamKey> {
    [
        (txt::TUT_S06_G1, ParamKey::S2Fare),
        (txt::TUT_S09_G1, ParamKey::R2LineSlots),
        (txt::TUT_S10_G1, ParamKey::S4Platform),
        (txt::TUT_S10_G2, ParamKey::T5Cars),
        (txt::TUT_S11_G1, ParamKey::R1MaxTrains),
        (txt::TUT_S11_G2, ParamKey::R3Attractiveness),
        (txt::TUT_S12_G1, ParamKey::T3Boarding),
        (txt::TUT_S12_G2, ParamKey::T2Crowding),
    ]
    .into_iter()
    .find(|&(t, _)| t == m)
    .map(|(_, p)| p)
}

/// 組み込みの表のゴールだけを近くした表(1 週間のゴールまでは通しで回さない。13 §10.3)。
pub fn near_goal_table() -> BalanceTable {
    let mut spec = BalanceTable::builtin().spec().clone();
    spec.economy.goal_pow10 = NEAR_GOAL_POW10;
    BalanceTable::from_spec(spec).expect("検証を通る表")
}

pub const NEAR_GOAL_POW10: u32 = 24;

impl Kid {
    pub fn new(seed: u64, opts: Options) -> Kid {
        let game = Game::new(GameConfig {
            seed,
            balance: Arc::new(near_goal_table()),
        });
        let mut rng = Rng::new(seed);
        let player = rng.pick(&Player::ALL);
        Kid {
            game,
            t: Tutorial::new(opts.show),
            show: opts.show,
            frame: 0,
            log: Log::default(),
            events: Vec::new(),
            rng,
            resume_rng: Rng::new(!seed),
            player,
            opts,
            settings: None,
            ack_at: None,
            act_at: 0,
            fanfare_at: None,
            cursor: None,
            last: None,
            noted: None,
            still_since: 0,
            off_until: None,
        }
    }

    pub fn ticks(&self) -> u64 {
        self.game.now().0
    }

    pub fn paused(&self) -> bool {
        self.settings.is_some() || self.t.directive().is_some_and(|d| d.pauses_game)
    }

    fn ui(&mut self, ev: UiEvent) {
        self.t.observe_ui(&ev);
        self.note();
    }

    fn set_show(&mut self, on: bool) {
        self.show = on;
        self.log.toggles += 1;
        self.t.set_show_explanations(on);
        self.note();
    }

    /// 1 フレーム(tm-app の順: observe_view → UI → コマンド → step → observe_game)。
    pub fn frame(&mut self) -> Result<(), String> {
        if self.opts.resume_every > 0 && self.resume_rng.below(self.opts.resume_every) == 0 {
            self.resume();
        }
        self.t.observe_view(self.game.view());
        self.note();
        self.update()?;
        if !self.paused() {
            self.game.step(&mut self.events);
            for e in std::mem::take(&mut self.events) {
                self.t.observe_game(&e);
                self.note();
                if matches!(
                    e,
                    GameEvent::CompanyLevelUp { .. } | GameEvent::GoalReached { .. }
                ) {
                    self.schedule_fanfare();
                }
            }
        }
        self.frame += 1;
        self.check_still()
    }

    /// `is_finished()` になるまで回す。ゲームの時間で 40 分を超えたら失敗。
    pub fn run_to_end(&mut self) -> Result<(), String> {
        while !self.t.is_finished() {
            if self.ticks() > LIMIT_TICKS || self.frame > LIMIT_TICKS + 30 * MIN {
                return Err(self.describe("40 分で終わらない"));
            }
            self.frame()?;
        }
        Ok(())
    }

    /// 序盤の制限(#2〜#6)を抜けて、「する」のまま止める説明もなく、すべて許される状態になるまで回す。
    pub fn run_past_restrictions(&mut self, max_ticks: u64) -> Result<(), String> {
        loop {
            let open = self.show
                && self.off_until.is_none()
                && self.settings.is_none()
                && self.t.directive().is_none_or(|d| d.stage.0 >= 7)
                && sample_cmds().iter().all(|c| self.t.permits(c));
            if open || self.t.is_finished() {
                return Ok(());
            }
            if self.ticks() > max_ticks {
                return Err(self.describe("序盤を抜けない"));
            }
            self.frame()?;
        }
    }

    /// 保存して終わり、続きから始めた(13 §6.8)。UI は閉じた状態から、ジングルは鳴らない。
    pub fn resume(&mut self) {
        self.log.resumes += 1;
        let text = ron::to_string(&self.t.save()).expect("書ける");
        self.t = Tutorial::restore(ron::from_str(&text).expect("読める"), self.show);
        self.settings = None;
        self.fanfare_at = None;
        self.ack_at = None;
        self.cursor = None;
        self.noted = None;
    }

    /// UI の状態を変えずに続きからにしても、同じ directive の列になるはずの時点(13 §6.8 の規則が効かない)。
    /// ファンファーレを落とさない子(`drop_fanfare_percent == 0`)でだけ使う。
    pub fn at_a_plain_point(&self) -> bool {
        self.settings.is_none()
            && self.fanfare_at.is_none()
            && self
                .t
                .directive()
                .is_none_or(|d| d.kind != DirectiveKind::Explanation)
    }

    pub fn describe(&self, what: &str) -> String {
        let v = self.game.view();
        format!(
            "{what}: tick {} frame {} show {} directive {:?} level {} money {:?} lines {} player {:?} log {:?}",
            self.ticks(),
            self.frame,
            self.show,
            self.t.directive(),
            v.company().level.0,
            v.money(),
            v.lines().len(),
            self.player,
            self.log
        )
    }

    fn schedule_fanfare(&mut self) {
        if self.rng.percent(self.opts.drop_fanfare_percent) {
            self.log.fanfares_dropped += 1;
            return;
        }
        let at = self.frame + self.rng.range(2 * SEC, 20 * SEC);
        self.fanfare_at = Some(self.fanfare_at.map_or(at, |f| f.max(at)));
    }

    fn check_still(&mut self) -> Result<(), String> {
        let d = self.t.directive().copied();
        if d != self.last {
            self.last = d;
            self.still_since = self.frame;
            return Ok(());
        }
        if d.is_some_and(|d| d.stage.0 <= 6) && self.frame - self.still_since > STILL_LIMIT {
            return Err(self.describe("同じ directive のまま 5 分動かない"));
        }
        Ok(())
    }

    /// directive が変わったら記録し、TogglePlan を当て、カーソルの下の駅を送り直す(13 §3.13 の最後の行)。
    fn note(&mut self) {
        loop {
            let d = self.t.directive().copied();
            if d == self.noted {
                return;
            }
            self.noted = d;
            let Some(d) = d else {
                continue;
            };
            self.log.changes.push((d.stage.0, d.kind, d.page));
            if let Some(p) = self.opts.plan
                && self.log.changes.len() == p.at_change + 1
                && self.show
            {
                self.set_show(false);
                self.off_until = Some(self.frame + p.off);
                continue;
            }
            if let Some(s) = self.cursor {
                self.t.observe_ui(&UiEvent::CursorOn(UiAnchor::Station(s)));
            }
        }
    }

    fn update(&mut self) -> Result<(), String> {
        if self.fanfare_at.is_some_and(|f| self.frame >= f) {
            self.fanfare_at = None;
            self.ui(UiEvent::FanfareFinished);
        }
        if let Some(until) = self.off_until {
            if self.frame >= until {
                self.off_until = None;
                self.set_show(true);
            } else {
                return self.while_off();
            }
        }
        if let Some(s) = self.settings {
            return self.in_settings(s);
        }
        if self.opts.random_settings && self.rng.below(90 * SEC) == 0 {
            self.open_settings();
            return Ok(());
        }
        if let Some(d) = self.t.directive().copied()
            && d.kind == DirectiveKind::Explanation
        {
            return self.read(d);
        }
        if self.frame >= self.act_at {
            self.act()?;
            let i = u64::from(self.player.interval_s());
            self.act_at = self.frame + self.rng.range(SEC / 2, i * SEC * 3 / 2);
        }
        Ok(())
    }

    fn while_off(&mut self) -> Result<(), String> {
        if self.frame < self.act_at {
            return Ok(());
        }
        self.act_at = self.frame + self.rng.range(SEC, 4 * SEC);
        match self.opts.plan.map(|p| p.play) {
            Some(OffPlay::Cursor) => self.move_cursor(),
            Some(OffPlay::Play) => {
                if self.rng.percent(30) {
                    self.move_cursor();
                }
                if let Some(c) = self.strategy() {
                    self.submit(c);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn open_settings(&mut self) {
        self.log.settings += 1;
        let close_at = self.frame + self.rng.range(SEC, 10 * SEC);
        let toggle_at = self
            .rng
            .percent(40)
            .then(|| self.rng.range(self.frame, close_at));
        let ack_at = self.frame + self.rng.range(SEC / 2, 4 * SEC);
        self.settings = Some(Settings {
            close_at,
            toggle_at,
            ack_at,
        });
        self.ui(UiEvent::MenuOpened(MenuAnchor::SettingsMenu));
    }

    fn in_settings(&mut self, mut s: Settings) -> Result<(), String> {
        if let Some(d) = self.t.directive()
            && d.stage.0 == 15
        {
            if d.pauses_game {
                return Err(self.describe("#15 が止める説明になっている"));
            }
            if self.frame >= s.ack_at {
                self.ui(UiEvent::MessageAcknowledged);
                s.ack_at = self.frame + self.rng.range(SEC / 2, 4 * SEC);
            }
        }
        if s.toggle_at == Some(self.frame) {
            self.set_show(!self.show);
        }
        if self.frame >= s.close_at {
            self.settings = None;
            self.ui(UiEvent::MenuClosed(MenuAnchor::SettingsMenu));
        } else {
            self.settings = Some(s);
        }
        Ok(())
    }

    fn read(&mut self, d: TutorialDirective) -> Result<(), String> {
        if !d.pauses_game {
            return Err(self.describe("本線の説明が止めない"));
        }
        if self.frame.is_multiple_of(30) {
            if let Some(c) = sample_cmds().iter().find(|c| self.t.permits(c)) {
                return Err(self.describe(&format!("止める説明の間に {c:?} を許した")));
            }
            if self.rng.percent(30) {
                self.move_cursor();
            }
        }
        match self.ack_at {
            None if self.fanfare_at.is_some() => {
                return Err(self.describe("ジングルの間に止める説明が出た"));
            }
            None => self.ack_at = Some(self.frame + self.rng.range(SEC * 3 / 10, 8 * SEC)),
            Some(at) if self.frame >= at => {
                self.ack_at = None;
                self.log.acks += 1;
                self.ui(UiEvent::MessageAcknowledged);
                if self.t.directive() == Some(&d) {
                    return Err(self.describe("決定で説明が進まない"));
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    fn move_cursor(&mut self) {
        let n = self.game.view().stations().len() as u64;
        let s = StationId(self.rng.below(n) as u32);
        self.cursor = Some(s);
        self.ui(UiEvent::CursorOn(UiAnchor::Station(s)));
    }

    fn act(&mut self) -> Result<(), String> {
        let d = self.t.directive().copied();
        let cmd = match d {
            Some(d) if d.kind == DirectiveKind::Guide => {
                if self.rng.percent(self.opts.deviate_percent) {
                    self.deviate()
                } else {
                    self.follow(d)?
                }
            }
            _ => {
                if self.rng.percent(10) {
                    self.move_cursor();
                }
                self.strategy()
            }
        };
        if let Some(c) = cmd {
            self.submit(c);
        }
        Ok(())
    }

    /// 案内どおりの操作。序盤(#2〜#6)の操作は、いつでも許され、実行できること(13 §6.9)。
    fn follow(&mut self, d: TutorialDirective) -> Result<Option<Command>, String> {
        let m = d.message;
        if m == txt::TUT_S02_G1 {
            self.move_cursor();
            return Ok(None);
        }
        if m == txt::TUT_S04_W || m == txt::TUT_S06_W {
            return Ok(None);
        }
        let Some(planned) = self.planned(m) else {
            return Ok(self.strategy());
        };
        let Some(c) = planned else {
            return Ok(None);
        };
        let res = self.game.view().validate(&c);
        if d.stage.0 <= 6 {
            if matches!(res, Err(CommandError::InsufficientFunds { .. })) && self.t.permits(&c) {
                return Ok(None);
            }
            if res.is_err() || !self.t.permits(&c) {
                return Err(self.describe(&format!(
                    "案内の操作 {c:?} が実行できない(validate {res:?}、permits {})",
                    self.t.permits(&c)
                )));
            }
            return Ok(Some(c));
        }
        Ok(if res.is_ok() {
            Some(c)
        } else {
            self.strategy()
        })
    }

    /// 案内の文に合う操作。`None` は案内に決まった操作がない(戦略で動く)。
    fn planned(&mut self, m: TextId) -> Option<Option<Command>> {
        let v = self.game.view();
        if m == txt::TUT_S03_G1 {
            let n = v.stations().len() as u64;
            let a = StationId(self.rng.below(n) as u32);
            let b = StationId(((u64::from(a.0) + 1 + self.rng.below(n - 1)) % n) as u32);
            return Some(Some(Command::CreateLine {
                stations: vec![a, b],
                bends: vec![v.default_bend(a, b)],
            }));
        }
        if m == txt::TUT_S05_G1 {
            return Some(
                v.stations()
                    .find(|s| !s.connected)
                    .map(|s| extend_nearer(&v, LineId(0), s.id)),
            );
        }
        if m == txt::TUT_S06_G1 {
            let connected: Vec<StationId> =
                v.stations().filter(|s| s.connected).map(|s| s.id).collect();
            let scope = if self.rng.percent(50) {
                UpgradeScope::AllStations
            } else {
                UpgradeScope::Station(self.rng.pick(&connected))
            };
            let mode = self.rng.pick(&[BuyMode::One, BuyMode::Max]);
            return Some(Some(Command::LevelUp {
                param: ParamKey::S2Fare,
                scope,
                mode,
            }));
        }
        if m == txt::TUT_S09_G2 {
            let w = GameWorld::new(v);
            let d = decide(self.player, &w);
            if matches!(d, tm_simrun::Decision::CreateLine { .. }) {
                return Some(w.command(d));
            }
            let last = StationId(v.stations().len() as u32 - 1);
            return Some(Some(Command::CreateLine {
                stations: vec![StationId(0), last],
                bends: vec![v.default_bend(StationId(0), last)],
            }));
        }
        let p = guide_param(m)?;
        let mode = self.rng.pick(&[BuyMode::One, BuyMode::Max]);
        Some(Some(Command::LevelUp {
            param: p,
            scope: all_scope(p),
            mode,
        }))
    }

    fn deviate(&mut self) -> Option<Command> {
        let v = self.game.view();
        let n = v.stations().len() as u32;
        let pick = |rng: &mut Rng| StationId(rng.below(u64::from(n)) as u32);
        match self.rng.below(5) {
            0 if n >= 3 => Some(Command::CreateLine {
                stations: vec![StationId(0), StationId(1), StationId(2)],
                bends: vec![tm_core::Bend::StraightFirst; 2],
            }),
            1 => {
                let (a, b) = (pick(&mut self.rng), pick(&mut self.rng));
                Some(Command::CreateLine {
                    stations: vec![a, b],
                    bends: vec![tm_core::Bend::StraightFirst],
                })
            }
            2 if v.lines().len() > 0 => {
                let s = pick(&mut self.rng);
                Some(Command::ExtendLine {
                    line: LineId(0),
                    end: self.rng.pick(&[LineEnd::Head, LineEnd::Tail]),
                    station: s,
                    bend: tm_core::Bend::StraightFirst,
                })
            }
            3 => {
                let p = self.rng.pick(&ParamKey::ALL);
                Some(Command::LevelUp {
                    param: p,
                    scope: all_scope(p),
                    mode: BuyMode::One,
                })
            }
            _ => self.strategy(),
        }
    }

    fn strategy(&mut self) -> Option<Command> {
        let v = self.game.view();
        if v.lines().len() == 0 {
            let three = !self.show && v.stations().len() >= 3 && self.rng.percent(30);
            let stations: Vec<StationId> = (0..if three { 3 } else { 2 }).map(StationId).collect();
            let bends = stations
                .windows(2)
                .map(|w| v.default_bend(w[0], w[1]))
                .collect();
            return Some(Command::CreateLine { stations, bends });
        }
        let w = GameWorld::new(v);
        // 子どもは建設のお金が足りない間も、ためずに強化する(A.3 の「ためる」規則は使わない)。
        w.command(w.decide_saving(self.player, 0))
    }

    fn submit(&mut self, c: Command) {
        match &c {
            Command::CreateLine { stations, .. } => {
                self.ui(UiEvent::MenuOpened(MenuAnchor::BuildMode));
                self.ui(UiEvent::MenuOpened(MenuAnchor::BuildCreate));
                for &s in stations {
                    self.cursor = Some(s);
                    self.ui(UiEvent::CursorOn(UiAnchor::Station(s)));
                }
            }
            Command::ExtendLine { station, .. } => {
                self.ui(UiEvent::MenuOpened(MenuAnchor::BuildMode));
                self.ui(UiEvent::MenuOpened(MenuAnchor::BuildExtend));
                self.cursor = Some(*station);
                self.ui(UiEvent::CursorOn(UiAnchor::Station(*station)));
            }
            Command::LevelUp { param, .. } => {
                self.ui(UiEvent::MenuOpened(MenuAnchor::UpgradePanel));
                self.ui(UiEvent::MenuOpened(MenuAnchor::UpgradeTab(
                    param.category(),
                )));
                self.ui(UiEvent::CursorOn(UiAnchor::Menu(MenuAnchor::UpgradeItem(
                    *param,
                ))));
            }
            Command::Rebirth | Command::RebirthUpgrade { .. } => {}
        }
        if !self.t.permits(&c) {
            self.log.refused += 1;
        } else if self.game.view().validate(&c).is_ok() {
            if !matches!(c, Command::LevelUp { .. }) {
                self.ui(UiEvent::MenuOpened(MenuAnchor::LineConfirm));
            }
            self.log.commands += 1;
            self.game.enqueue(c.clone());
        }
        match c {
            Command::CreateLine { .. } | Command::ExtendLine { .. } => {
                self.ui(UiEvent::MenuClosed(MenuAnchor::LineConfirm));
                self.ui(UiEvent::MenuClosed(MenuAnchor::BuildMode));
            }
            Command::LevelUp { param, .. } => {
                self.ui(UiEvent::MenuClosed(MenuAnchor::UpgradeTab(
                    param.category(),
                )));
                self.ui(UiEvent::MenuClosed(MenuAnchor::UpgradePanel));
            }
            Command::Rebirth | Command::RebirthUpgrade { .. } => {}
        }
    }
}
