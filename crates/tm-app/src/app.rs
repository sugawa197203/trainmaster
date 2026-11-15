//! メインループ: `App`
// 1 フレームの処理
// `AppRequest` の処理 [02 §4.7、16 §1.4〜§1.10]

use std::time::Duration;

use log::{debug, info, warn};
use tm_core::{
    CHECKPOINT_INTERVAL_TICKS, Command, Game, GameConfig, GameEvent, REPLAY_FORMAT_VERSION,
    RULES_VERSION, ReplayHeader, ReplayLog, TICKS_PER_SEC,
};
use tm_input::InputMapper;
use tm_platform_api::{AssetBundle, AudioCmd, DrawList, MonoTime, PhysicalInput, Platform};
use tm_tutorial::Tutorial;
use tm_ui::{AppRequest, AppStatus, Ui};

use crate::clock::FrameClock;
use crate::config::{AppConfig, ExitAction};
use crate::error::AppError;
use crate::overlay;
use crate::profiler::{FrameProfiler, FrameTimes};
use crate::replay_out::ReplaySink;
use crate::save::{Loaded, Playing, SaveCause, SaveState};
use crate::seed::SeedSource;

/// 捨てた tick の警告の最短の間隔(µs)
const DROP_WARN_INTERVAL_US: u64 = 10_000_000;
/// 「ゲームを終わる」の BGM のフェード(14 PL-D-32)。
const QUIT_FADE_MS: u16 = 500;
/// 電源の箱を出し続ける時間(µs。保存の完了から)。
const POWER_BOX_US: u64 = 1_500_000;
/// 「ゲームを終わる」で保存を待つ上限(µs)。
const QUIT_SAVE_TIMEOUT_US: u64 = 5_000_000;
/// `shutdown(Quit)` で保存を待つ上限。
const SHUTDOWN_SAVE_WAIT: Duration = Duration::from_secs(1);

/// `run` の戻り値。bin が終了コードにする [16 §1.15]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppExit {
    /// 普通の終了(終了コード 0)。
    Normal,
    /// 電源を切る要求(終了コード `EXIT_POWER_OFF`)。
    PowerOff,
}

/// 電源を切る要求の終了コード。`tm-launch` が `sync; poweroff` する(03 §5.3)。
pub const EXIT_POWER_OFF: u8 = 10;

/// 「ゲームを終わる」の後の状態。
#[derive(Clone, Copy, Debug)]
struct QuitState {
    since: MonoTime,
    saved_at: Option<MonoTime>,
    gave_up: bool,
}

/// 1 回のプレイの状態
// リセットではこれを丸ごと捨てる [02 D30]
struct Session {
    game: Game,
    tutorial: Tutorial,
    replay: Option<ReplayLog>,
    seed: u64,
    no: u32,
    last_save_tick: u64,
}

fn replay_log(seed: u64, cfg: &AppConfig) -> ReplayLog {
    ReplayLog::new(ReplayHeader {
        format: REPLAY_FORMAT_VERSION,
        rules_version: RULES_VERSION,
        game_version: cfg.version.game_version.clone(),
        rustc_version: cfg.version.rustc_version.clone(),
        ticks_per_sec: TICKS_PER_SEC,
        seed,
        config_hash: cfg.balance.config_hash(),
    })
}

impl Session {
    fn new(seed: u64, no: u32, cfg: &AppConfig, tutorial: Tutorial, record: bool) -> Self {
        Session {
            game: Game::new(GameConfig {
                seed,
                balance: cfg.balance.clone(),
            }),
            tutorial,
            replay: record.then(|| replay_log(seed, cfg)),
            seed,
            no,
            last_save_tick: 0,
        }
    }

    /// 続きから。リプレイは読み込んだ写しから始める [10 §9.1]。
    fn resume(game: Game, tutorial: Tutorial, no: u32, cfg: &AppConfig, record: bool) -> Self {
        let seed = game.config().seed;
        let replay = record.then(|| {
            let mut log = replay_log(seed, cfg);
            log.start = Some(game.snapshot());
            log
        });
        Session {
            last_save_tick: game.now().0,
            game,
            tutorial,
            replay,
            seed,
            no,
        }
    }

    fn finish(self, sink: &mut ReplaySink, why: &str) {
        info!(
            "ゲーム #{} を終える({why}) seed=0x{:016x} tick={}",
            self.no,
            self.seed,
            self.game.now().0
        );
        if let Some(log) = &self.replay {
            sink.write(log, self.no);
        }
    }
}

/// `App::frame` の結果
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameOutcome {
    /// 次のフレームへ
    Continue,
    /// 終わる(`QuitRequested`、`quit_after_frames`、「ゲームを終わる」)
    Quit(AppExit),
}

/// 終わり方
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownReason {
    /// 終了の要求
    Quit,
    /// エラー
    Error,
}

/// 統計
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppStats {
    /// 処理したフレームの数
    pub frames: u64,
    /// 追いつけずに捨てた tick の累計
    pub dropped_ticks: u64,
    /// 始めたゲームの数
    pub sessions_started: u32,
    /// 書き出したリプレイの数
    pub replays_written: u32,
    /// 書けたセーブの数
    pub saves_written: u32,
    /// 書けなかったセーブの数
    pub save_failures: u32,
}

/// 各クレートを組み立てたアプリ。`frame` を繰り返し呼ぶ
pub struct App {
    cfg: AppConfig,
    ui: Ui,
    mapper: InputMapper,
    session: Option<Session>,
    clock: FrameClock,
    seeds: SeedSource,
    replay_sink: ReplaySink,
    save: SaveState,
    quit: Option<QuitState>,
    resumed_this_frame: bool,
    profiler: FrameProfiler,
    ducked: bool,
    frame_no: u64,
    sessions_started: u32,
    last_drop_warn: Option<(MonoTime, u64)>,
    raw: Vec<PhysicalInput>,
    cmds: Vec<Command>,
    requests: Vec<AppRequest>,
    audio: Vec<AudioCmd>,
    events: Vec<GameEvent>,
    draw: DrawList,
}

impl App {
    /// 起動: アセットの束を検査して `load_assets` を 1 回だけ呼び、UI と入力と時計を作り、設定とセーブを読む
    pub fn new<P: Platform>(platform: &mut P, cfg: AppConfig) -> Result<Self, AppError> {
        let bundle = tm_assets::bundle().map_err(AppError::Assets)?;
        Self::with_bundle(platform, cfg, bundle)
    }

    fn with_bundle<P: Platform>(
        platform: &mut P,
        cfg: AppConfig,
        bundle: &'static AssetBundle,
    ) -> Result<Self, AppError> {
        let t0 = platform.now();
        platform.load_assets(bundle).map_err(AppError::Platform)?;
        info!(
            "load_assets: {}ms",
            platform.now().saturating_since(t0) / 1000
        );
        let t0 = platform.now();
        let mut ui = Ui::new();
        let (save, settings) = SaveState::open(&cfg.save, &cfg.balance, &cfg.version.game_version);
        if let Some(s) = settings {
            ui.set_settings(s);
        }
        info!(
            "セーブ({:?}): 読み込みと検査 {}µs、続きから {}",
            cfg.save,
            platform.now().saturating_since(t0),
            if save.has_save() { "あり" } else { "なし" }
        );
        let now = platform.now();
        Ok(App {
            ui,
            mapper: InputMapper::new(cfg.bindings.clone()),
            session: None,
            clock: FrameClock::new(now, cfg.time_scale),
            seeds: SeedSource::new(cfg.seed.clone()),
            replay_sink: ReplaySink::new(cfg.replay.clone()),
            save,
            quit: None,
            resumed_this_frame: false,
            profiler: FrameProfiler::new(cfg!(feature = "devtools")),
            ducked: false,
            frame_no: 0,
            sessions_started: 0,
            last_drop_warn: None,
            raw: Vec::new(),
            cmds: Vec::new(),
            requests: Vec::new(),
            audio: Vec::new(),
            events: Vec::new(),
            draw: DrawList::default(),
            cfg,
        })
    }

    /// 1 フレーム: 入力 → `observe_view` → `ui.update` → 要求 → 一時停止 → `step` → `render` → 音 → `present`
    pub fn frame<P: Platform>(&mut self, p: &mut P) -> Result<FrameOutcome, AppError> {
        let t_in = p.now();
        self.raw.clear();
        p.poll_input(&mut self.raw);
        if self
            .raw
            .iter()
            .any(|e| matches!(e, PhysicalInput::QuitRequested))
        {
            return Ok(FrameOutcome::Quit(AppExit::Normal));
        }
        for ev in &self.raw {
            self.mapper.feed(ev);
        }
        let now = p.now();
        let actions = self.mapper.end_frame(now);

        let t_up = p.now();
        let status = self.app_status();
        if let Some(q) = self.quit.as_mut()
            && q.saved_at.is_none()
            && !status.saving
        {
            q.saved_at = Some(now);
        }
        self.ui.set_app_status(status);
        match self.session.as_mut() {
            Some(s) => {
                s.tutorial.observe_view(s.game.view());
                self.ui.update(
                    now,
                    &actions,
                    Some(s.game.view()),
                    Some(&mut s.tutorial),
                    &mut self.cmds,
                    &mut self.requests,
                    &mut self.audio,
                );
            }
            None => self.ui.update(
                now,
                &actions,
                None,
                None,
                &mut self.cmds,
                &mut self.requests,
                &mut self.audio,
            ),
        }

        for i in 0..self.requests.len() {
            let r = self.requests[i];
            self.handle_request(r, now);
        }
        self.requests.clear();

        let paused = self
            .ui
            .is_paused(self.session.as_ref().map(|s| &s.tutorial));
        if paused != self.ducked {
            self.audio.push(AudioCmd::SetBgmDuck(paused));
            self.ducked = paused;
        }

        let t_sim = p.now();
        // 続きからのフレームは進めない(保存した tick のまま、次のフレームの observe_view から始める)
        let running =
            self.session.is_some() && !paused && !std::mem::take(&mut self.resumed_this_frame);
        let steps = self.clock.advance(now, running);
        let mut done = 0;
        match self.session.as_mut() {
            Some(s) if running => {
                for c in self.cmds.drain(..) {
                    if let Some(r) = s.replay.as_mut() {
                        r.record(s.game.next_tick(), &c);
                    }
                    s.game.enqueue(c);
                }
                while done < steps {
                    s.game.step(&mut self.events);
                    done += 1;
                    if let Some(r) = s.replay.as_mut()
                        && s.game.now().0 % CHECKPOINT_INTERVAL_TICKS == 0
                    {
                        r.checkpoint(&s.game);
                    }
                    for e in self.events.drain(..) {
                        if matches!(e, GameEvent::CompanyLevelUp { .. }) {
                            self.save.request(SaveCause::LevelUp);
                        }
                        if matches!(e, GameEvent::Rebirth { .. }) {
                            self.save.request(SaveCause::Rebirth);
                        }
                        s.tutorial.observe_game(&e);
                        self.ui.observe(now, &e, &mut self.audio);
                    }
                    // 止める説明が始まったら残りの step は捨てる(UI の箱は次のフレームから出る)
                    if self.ui.is_paused(Some(&s.tutorial)) {
                        break;
                    }
                }
                if done < steps {
                    debug!("説明で止めたので {} tick を進めない", steps - done);
                }
            }
            Some(_) => {
                debug_assert!(
                    self.cmds.is_empty(),
                    "止まっているフレームにコマンドが残った(APP-D-18): {:?}",
                    self.cmds
                );
            }
            None => {
                if !self.cmds.is_empty() {
                    warn!("タイトル画面でコマンド {} 件。捨てる", self.cmds.len());
                    self.cmds.clear();
                }
            }
        }
        self.warn_dropped(now);
        let t_save = p.now();
        let play = self.session.as_mut().map(|s| Playing {
            game: &s.game,
            tutorial: &s.tutorial,
            last_save_tick: &mut s.last_save_tick,
        });
        if self.save.on_frame(play, &self.ui) {
            self.save.note_encode_us(p.now().saturating_since(t_save));
        }

        let t_draw = p.now();
        self.draw.cmds.clear();
        match self.session.as_ref() {
            Some(s) => self
                .ui
                .render(Some(s.game.view()), Some(&s.tutorial), &mut self.draw),
            None => self.ui.render(None, None, &mut self.draw),
        }
        if self.cfg.overlay {
            overlay::append(&self.profiler, &mut self.draw);
        }

        let t_out = p.now();
        p.audio(&self.audio);
        self.audio.clear();
        p.present(&self.draw).map_err(AppError::Platform)?;
        if self.frame_no == 0 {
            info!("最初のフレーム: 起動から {}ms", p.now().0 / 1000);
        }

        let times = FrameTimes {
            input: t_in,
            update: t_up,
            sim: t_sim,
            draw: t_draw,
            out: t_out,
            end: p.now(),
        };
        self.profiler
            .record(times, done, running, self.clock.dropped_ticks(), p.stats());
        self.frame_no += 1;
        if let Some(exit) = self.quit_ready(p.now()) {
            return Ok(FrameOutcome::Quit(exit));
        }
        if Some(self.frame_no) == self.cfg.quit_after_frames {
            return Ok(FrameOutcome::Quit(AppExit::Normal));
        }
        Ok(FrameOutcome::Continue)
    }

    /// 「ゲームを終わる」の後、保存とフェード(電源を切るなら電源の箱)が済んだら終わり方を返す [16 §1.15]。
    fn quit_ready(&mut self, now: MonoTime) -> Option<AppExit> {
        let q = self.quit.as_mut()?;
        let exit = match self.cfg.exit_action {
            ExitAction::Exit => AppExit::Normal,
            ExitAction::PowerOff => AppExit::PowerOff,
        };
        let faded = now.saturating_since(q.since) >= u64::from(QUIT_FADE_MS) * 1000;
        let Some(saved) = q.saved_at else {
            if now.saturating_since(q.since) >= QUIT_SAVE_TIMEOUT_US {
                warn!("保存が 5 秒で終わらない。諦めて終える");
                q.gave_up = true;
                self.save.give_up();
                return Some(exit);
            }
            return None;
        };
        let ready = match exit {
            AppExit::Normal => faded,
            AppExit::PowerOff => faded && now.saturating_since(saved) >= POWER_BOX_US,
        };
        if ready {
            info!(
                "ゲームを終わる: 要求から {}ms",
                now.saturating_since(q.since) / 1000
            );
        }
        ready.then_some(exit)
    }

    fn warn_dropped(&mut self, now: MonoTime) {
        let total = self.clock.dropped_ticks();
        let (last_t, last_n) = self.last_drop_warn.unwrap_or((MonoTime(0), 0));
        if total > last_n
            && (self.last_drop_warn.is_none()
                || now.saturating_since(last_t) >= DROP_WARN_INTERVAL_US)
        {
            warn!("処理落ちで tick を捨てた: 累計 {total}");
            self.last_drop_warn = Some((now, total));
        }
    }

    fn handle_request(&mut self, r: AppRequest, now: MonoTime) {
        match r {
            AppRequest::StartNewGame => {
                if self.session.is_some() {
                    warn!("ゲーム中の StartNewGame を無視");
                    return;
                }
                let seed = self.seeds.next_seed(now);
                let tutorial = Tutorial::new(self.ui.show_explanations());
                self.sessions_started += 1;
                info!("新しいゲーム #{} seed=0x{seed:016x}", self.sessions_started);
                self.session = Some(Session::new(
                    seed,
                    self.sessions_started,
                    &self.cfg,
                    tutorial,
                    self.replay_sink.enabled(),
                ));
                self.save.clear_slots();
                self.save.request(SaveCause::NewGame);
            }
            AppRequest::ContinueGame => {
                if self.session.is_some() {
                    warn!("ゲーム中の ContinueGame を無視");
                    return;
                }
                let Some(c) = self.save.take_candidate() else {
                    warn!("続きからの候補がない。タイトルに戻す");
                    self.ui.go_to_title(&mut self.audio);
                    return;
                };
                let Loaded { game, tutorial, .. } = c;
                let tutorial = Tutorial::restore(tutorial, self.ui.show_explanations());
                self.sessions_started += 1;
                info!(
                    "続きから #{} seed=0x{:016x} tick={}",
                    self.sessions_started,
                    game.config().seed,
                    game.now().0
                );
                self.session = Some(Session::resume(
                    game,
                    tutorial,
                    self.sessions_started,
                    &self.cfg,
                    self.replay_sink.enabled(),
                ));
                self.resumed_this_frame = true;
            }
            AppRequest::ResetToTitle => {
                let Some(s) = self.session.take() else {
                    warn!("タイトルでの ResetToTitle を無視");
                    return;
                };
                s.finish(&mut self.replay_sink, "リセット");
                self.save.clear_slots();
                self.cmds.clear();
                self.events.clear();
                self.mapper.reset();
                self.ui.go_to_title(&mut self.audio);
                info!("タイトルに戻った");
            }
            AppRequest::QuitGame => {
                if self.quit.is_some() {
                    return;
                }
                self.save.write_settings_if_changed(&self.ui);
                if let Some(s) = &self.session {
                    self.save.request_final(&s.game);
                }
                self.audio.push(AudioCmd::StopBgm {
                    fade_ms: QUIT_FADE_MS,
                });
                self.quit = Some(QuitState {
                    since: now,
                    saved_at: None,
                    gave_up: false,
                });
                info!("ゲームを終わる({:?})", self.cfg.exit_action);
            }
        }
    }

    /// 終了処理: `Quit` ならゲーム中の保存を 1 回試みて最大 1 秒待ち、リプレイを書き出し、統計をログに出す。
    pub fn shutdown(&mut self, reason: ShutdownReason) {
        if reason == ShutdownReason::Quit {
            self.save.write_settings_if_changed(&self.ui);
            if self.quit.is_none()
                && let Some(s) = self.session.as_mut()
            {
                self.save.request_final(&s.game);
                let play = Playing {
                    game: &s.game,
                    tutorial: &s.tutorial,
                    last_save_tick: &mut s.last_save_tick,
                };
                self.save.on_frame(Some(play), &self.ui);
            }
            if !self.quit.is_some_and(|q| q.gave_up) {
                self.save.finish(SHUTDOWN_SAVE_WAIT);
            }
        }
        if let Some(s) = self.session.take() {
            let why = match reason {
                ShutdownReason::Quit => "終了",
                ShutdownReason::Error => "エラー",
            };
            s.finish(&mut self.replay_sink, why);
        }
        self.save.log_summary();
        let st = self.stats();
        let (snapped, unsnapped) = self.profiler.shadow_steps();
        info!(
            "終了({reason:?}): フレーム {}、ゲーム {}、リプレイ {}、捨てた tick {}",
            st.frames, st.sessions_started, st.replays_written, st.dropped_ticks
        );
        info!(
            "step の回数 0/1/2/3/4+: 実際 {:?}、全フレームで進めた場合 スナップあり {snapped:?} なし {unsnapped:?}、間隔のずれ ≤100/≤500/≤1000/それ以上 µs {:?}",
            self.profiler.total_steps(),
            self.profiler.interval_deviation()
        );
    }

    /// いまのゲーム(タイトル画面では `None`)
    pub fn game(&self) -> Option<&Game> {
        self.session.as_ref().map(|s| &s.game)
    }

    /// いまのチュートリアル(タイトル画面では `None`)
    pub fn tutorial(&self) -> Option<&Tutorial> {
        self.session.as_ref().map(|s| &s.tutorial)
    }

    /// UI。
    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    /// いまのゲームの混ぜた後のシード。
    pub fn seed(&self) -> Option<u64> {
        self.session.as_ref().map(|s| s.seed)
    }

    /// いまのゲームのリプレイログ(記録していなければ `None`)。
    pub fn replay(&self) -> Option<&ReplayLog> {
        self.session.as_ref().and_then(|s| s.replay.as_ref())
    }

    /// 計測。
    pub fn profiler(&self) -> &FrameProfiler {
        &self.profiler
    }

    /// 直前のフレームの描画コマンド。
    pub fn draw_list(&self) -> &DrawList {
        &self.draw
    }

    /// 統計。
    pub fn stats(&self) -> AppStats {
        let (saves_written, save_failures) = self.save.counts();
        AppStats {
            frames: self.frame_no,
            dropped_ticks: self.clock.dropped_ticks(),
            sessions_started: self.sessions_started,
            replays_written: self.replay_sink.written(),
            saves_written,
            save_failures,
        }
    }

    /// 毎フレーム `ui.set_app_status` に渡す値(13 UI-IF-08)。`saving` は書き込みスレッドの今の状態。
    pub fn app_status(&self) -> AppStatus {
        AppStatus {
            save_enabled: self.save.enabled(),
            has_save: self.save.has_save(),
            saving: self.save.saving(),
            power_off_on_quit: self.cfg.exit_action == ExitAction::PowerOff,
        }
    }
}

/// 02 §4.7 の `run`: `App::new` → `frame` の繰り返し → `shutdown`。
pub fn run<P: Platform>(platform: &mut P, cfg: AppConfig) -> Result<AppExit, AppError> {
    let mut app = App::new(platform, cfg)?;
    let result = loop {
        match app.frame(platform) {
            Ok(FrameOutcome::Continue) => {}
            Ok(FrameOutcome::Quit(exit)) => break Ok(exit),
            Err(e) => break Err(e),
        }
    };
    app.shutdown(if result.is_ok() {
        ShutdownReason::Quit
    } else {
        ShutdownReason::Error
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_platform_api::{HeadlessConfig, HeadlessPlatform, PlatformError, SpriteId, SpriteRect};

    use crate::seed::SeedPolicy;

    fn headless() -> HeadlessPlatform {
        HeadlessPlatform::new(HeadlessConfig::default())
    }

    fn cfg() -> AppConfig {
        AppConfig {
            seed: SeedPolicy::Fixed(1),
            ..AppConfig::default()
        }
    }

    fn snapshot(app: &App) -> (bool, Option<u64>, u32, bool, usize) {
        (
            app.session.is_some(),
            app.seed(),
            app.sessions_started,
            app.ducked,
            app.audio.len(),
        )
    }

    #[test]
    fn app_t_12_start_during_game_and_reset_on_title_are_ignored() {
        let mut p = headless();
        let mut app = App::new(&mut p, cfg()).unwrap();
        let before = snapshot(&app);
        app.handle_request(AppRequest::ResetToTitle, MonoTime(0));
        assert_eq!(snapshot(&app), before, "タイトルでの ResetToTitle");
        assert!(app.audio.is_empty(), "go_to_title を呼ばない");

        app.handle_request(AppRequest::StartNewGame, MonoTime(0));
        let started = snapshot(&app);
        assert!(started.0 && started.2 == 1);
        let tick = app.game().unwrap().now();
        let hash = app.game().unwrap().state_hash();
        app.handle_request(AppRequest::StartNewGame, MonoTime(5));
        assert_eq!(snapshot(&app), started, "ゲーム中の StartNewGame");
        assert_eq!(app.game().unwrap().now(), tick);
        assert_eq!(app.game().unwrap().state_hash(), hash);
    }

    #[test]
    fn app_t_12_continue_without_a_candidate_goes_back_to_the_title() {
        let mut p = headless();
        let mut app = App::new(&mut p, cfg()).unwrap();
        let before = snapshot(&app);
        app.handle_request(AppRequest::ContinueGame, MonoTime(0));
        assert_eq!(snapshot(&app).0, before.0, "ゲームを作らない");
        assert!(
            app.audio.contains(&AudioCmd::StopBgm { fade_ms: 0 }),
            "go_to_title: {:?}",
            app.audio
        );
        app.audio.clear();
        app.handle_request(AppRequest::StartNewGame, MonoTime(0));
        let started = snapshot(&app);
        app.handle_request(AppRequest::ContinueGame, MonoTime(5));
        assert_eq!(snapshot(&app), started, "ゲーム中の ContinueGame");
        assert!(!app.app_status().save_enabled, "既定は保存しない");
    }

    #[test]
    fn quit_game_once_and_no_save_with_pending_commands() {
        let store = crate::save::MemStore::new();
        let mut p = headless();
        let c = AppConfig {
            save: crate::save::SavePolicy::Memory(store),
            exit_action: ExitAction::PowerOff,
            ..cfg()
        };
        let mut app = App::new(&mut p, c).unwrap();
        app.handle_request(AppRequest::StartNewGame, MonoTime(0));
        assert!(app.app_status().saving, "初めからの保存を待っている");
        app.session
            .as_mut()
            .unwrap()
            .game
            .enqueue(tm_core::Command::LevelUp {
                param: tm_core::ParamKey::S1Spawn,
                scope: tm_core::UpgradeScope::AllStations,
                mode: tm_core::BuyMode::One,
            });
        app.handle_request(AppRequest::QuitGame, MonoTime(7));
        app.handle_request(AppRequest::QuitGame, MonoTime(9));
        let stops: Vec<_> = app
            .audio
            .iter()
            .filter(|c| matches!(c, AudioCmd::StopBgm { .. }))
            .collect();
        assert_eq!(stops, [&AudioCmd::StopBgm { fade_ms: 500 }]);
        assert_eq!(app.quit.map(|q| q.since), Some(MonoTime(7)));
        app.save.finish(Duration::from_secs(5));
        assert!(
            !app.app_status().saving,
            "写しを取れないので待たない(前の保存が残る)"
        );
        assert!(app.app_status().power_off_on_quit);
    }

    #[test]
    fn quit_game_writes_changed_settings_in_that_frame() {
        let store = crate::save::MemStore::new();
        let mut p = headless();
        let c = AppConfig {
            save: crate::save::SavePolicy::Memory(store.clone()),
            ..cfg()
        };
        let mut app = App::new(&mut p, c).unwrap();
        let s = tm_ui::Settings {
            music: 3,
            ..tm_ui::Settings::default()
        };
        app.ui.set_settings(s);
        app.handle_request(AppRequest::QuitGame, MonoTime(0));
        app.save.finish(Duration::from_secs(5));
        let got = store.settings().map(|b| crate::save::decode_settings(&b));
        assert_eq!(got, Some(Ok(s)));
    }

    #[test]
    fn seed_mixes_the_time_of_the_confirm_frame() {
        let mut p = headless();
        p.tap(40, tm_platform_api::Key::Z, 1);
        let mut app = App::new(&mut p, cfg()).unwrap();
        app.seeds = SeedSource::with_entropy(SeedPolicy::Os, || Ok(0x0123_4567_89AB_CDEF));
        while app.session.is_none() {
            app.frame(&mut p).unwrap();
        }
        assert_eq!(p.frame_index(), 41);
        let now = MonoTime(40 * 1_000_000 / 60);
        assert_eq!(
            app.seed(),
            Some(crate::seed::mix(0x0123_4567_89AB_CDEF, now))
        );
    }

    static BROKEN_SPRITES: [SpriteRect; 2] = [
        SpriteRect {
            atlas: 0,
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        },
        SpriteRect {
            atlas: 0,
            x: 3,
            y: 3,
            w: 2,
            h: 2,
        },
    ];

    fn broken_bundle() -> &'static AssetBundle {
        static BROKEN: std::sync::OnceLock<AssetBundle> = std::sync::OnceLock::new();
        BROKEN.get_or_init(|| {
            let min = AssetBundle::test_minimal();
            AssetBundle {
                sprites: &BROKEN_SPRITES,
                white: SpriteId(0),
                ..*min
            }
        })
    }

    #[test]
    fn app_t_16_broken_bundle_stops_before_any_frame() {
        assert!(AssetBundle::test_minimal().validate().is_ok());
        assert!(
            broken_bundle().validate().is_err(),
            "ページの外のスプライト"
        );
        let mut p = headless();
        let r = App::with_bundle(&mut p, cfg(), broken_bundle());
        assert!(
            matches!(r, Err(AppError::Platform(PlatformError::Assets(_)))),
            "{:?}",
            r.err()
        );
        assert_eq!(p.frame_index(), 0, "フレームを回さない");
        assert!(p.audio_log().is_empty());
        assert!(p.assets_summary().is_none());
    }
}
