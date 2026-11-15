//! セーブ: 置き場所、起動時の読み込み、続きからの候補、保存のきっかけ、`settings.ron` [16 §1.20、APP-D-26]。

mod file;
mod settings;
mod store;
mod writer;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use log::{debug, info, warn};
use tm_core::{BalanceTable, Game};
use tm_tutorial::Tutorial;
use tm_ui::{Settings, Ui};

pub use file::{
    Loaded, MAX_SAVE_BYTES, MAX_SEQ, SAVE_FORMAT_VERSION, SAVE_MAGIC, SaveBody, SaveError,
    SaveFile, SaveHeader, body_fnv,
};
pub use settings::{decode_settings, encode_settings};
pub use store::{
    FsStore, MAX_SETTINGS_BYTES, MemCounts, MemStore, MemWrite, NullStore, SETTINGS_FILE,
    SaveStore, Slot, WriteStage,
};
use writer::Saver;

/// ゲーム時間でこの tick ごとに保存する(60 秒)。
pub const SAVE_INTERVAL_TICKS: u64 = 3_600;
/// 続けてこの回数だけ書き込みに失敗したら、その起動の間は保存をやめる。
pub const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// 保存するか、どこへ保存するか(`TM_SAVE_DIR`)。
#[derive(Clone, Debug)]
pub enum SavePolicy {
    /// 保存しない(`off`)。`settings.ron` も読み書きしない。
    Off,
    /// このディレクトリの `FsStore`。
    Dir(PathBuf),
    /// メモリ(テスト用)。
    Memory(MemStore),
}

/// 保存のきっかけ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveCause {
    Interval,
    LevelUp,
    /// 転生した(T1)。
    Rebirth,
    SettingsOpened,
    NewGame,
    Quit,
}

/// 保存するゲームと、最後に保存した tick。
pub(crate) struct Playing<'a> {
    pub game: &'a Game,
    pub tutorial: &'a Tutorial,
    pub last_save_tick: &'a mut u64,
}

/// 起動時に 2 つのスロットを調べた結果。
pub(crate) struct Scan {
    pub candidate: Option<(Slot, Loaded)>,
    pub rejected: Vec<(Slot, SaveError)>,
    pub max_seq: u64,
    pub next_slot: Slot,
}

/// ヘッダの `seq` の大きい方から `SaveFile::load` を試し、最初に通ったものを候補にする。
pub(crate) fn scan_slots(slots: &[(Slot, Vec<u8>)], balance: &Arc<BalanceTable>) -> Scan {
    let mut rejected = Vec::new();
    let mut heads = Vec::new();
    for (slot, bytes) in slots {
        match SaveFile::header(bytes) {
            Ok(h) => heads.push((*slot, h.seq, bytes.as_slice())),
            Err(e) => rejected.push((*slot, e)),
        }
    }
    let max_seq = heads.iter().map(|h| h.1).max().unwrap_or(0);
    heads.sort_by_key(|h| (std::cmp::Reverse(h.1), h.0));
    let mut candidate = None;
    for &(slot, _, bytes) in &heads {
        if candidate.is_some() {
            break;
        }
        match SaveFile::load(bytes, balance) {
            Ok(l) => candidate = Some((slot, l)),
            Err(e) => rejected.push((slot, e)),
        }
    }
    let next_slot = match &candidate {
        Some((slot, _)) => slot.other(),
        None => Slot::ALL
            .into_iter()
            .min_by_key(|s| {
                let seq = heads.iter().find(|h| h.0 == *s).map(|h| h.1);
                let present = slots.iter().any(|(x, _)| x == s);
                (present, seq.is_some(), seq)
            })
            .unwrap_or(Slot::A),
    };
    Scan {
        candidate,
        rejected,
        max_seq,
        next_slot,
    }
}

/// 保存の数と時間(`AppStats` とログ)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SaveStats {
    pub encoded: u32,
    pub encode_us_max: u64,
    pub encode_us_total: u64,
    pub bytes_max: usize,
}

/// `App` が持つセーブの状態。
pub(crate) struct SaveState {
    saver: Option<Saver>,
    stopped: bool,
    has_save: bool,
    candidate: Option<Loaded>,
    next_seq: u64,
    requested: Option<SaveCause>,
    settings_open: bool,
    settings: Settings,
    version: String,
    stats: SaveStats,
}

impl SaveState {
    /// 起動時の 5a・5b: `settings.ron` とスロットを読み、書き込みスレッドを作る。返す設定は `ui.set_settings` に渡す。
    pub(crate) fn open(
        policy: &SavePolicy,
        balance: &Arc<BalanceTable>,
        game_version: &str,
    ) -> (SaveState, Option<Settings>) {
        let (mut store, enabled): (Box<dyn SaveStore>, bool) = match policy {
            SavePolicy::Off => (Box::new(NullStore), false),
            SavePolicy::Dir(dir) => match FsStore::open(dir) {
                Ok(s) => (Box::new(s), true),
                Err(e) => {
                    warn!("保存の場所 {} を使えない: {e}。保存しない", dir.display());
                    (Box::new(NullStore), false)
                }
            },
            SavePolicy::Memory(m) => (Box::new(m.clone()), true),
        };
        let mut state = SaveState {
            saver: None,
            stopped: false,
            has_save: false,
            candidate: None,
            next_seq: 1,
            requested: None,
            settings_open: false,
            settings: Settings::default(),
            version: game_version.to_owned(),
            stats: SaveStats::default(),
        };
        if !enabled {
            return (state, None);
        }
        let loaded = store
            .load_settings()
            .and_then(|b| match decode_settings(&b) {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!("{SETTINGS_FILE} が壊れている: {e}。初期値を使う");
                    None
                }
            });
        if let Some(s) = loaded {
            state.settings = s;
        }
        let scan = scan_slots(&store.load_slots(), balance);
        for (slot, e) in &scan.rejected {
            warn!("セーブ {slot:?} を使わない: {e}");
        }
        if let Some((slot, l)) = scan.candidate {
            info!(
                "続きからの候補: {slot:?} seq={} tick={} seed=0x{:016x}",
                l.seq,
                l.game.now().0,
                l.seed
            );
            if l.game_version != game_version {
                info!(
                    "セーブの版 {} は今の版 {game_version} と違うが使う",
                    l.game_version
                );
            }
            state.has_save = true;
            state.candidate = Some(l);
        }
        state.next_seq = scan.max_seq.saturating_add(1);
        match Saver::spawn(store, scan.next_slot) {
            Ok(s) => state.saver = Some(s),
            Err(e) => warn!("書き込みスレッドを作れない: {e}。保存しない"),
        }
        (state, loaded)
    }

    /// 保存する設定で、書き込みスレッドが動いていて、失敗でやめていない。
    pub(crate) fn enabled(&self) -> bool {
        self.saver.is_some() && !self.stopped
    }

    /// 続きからのセーブがある。
    pub(crate) fn has_save(&self) -> bool {
        self.has_save
    }

    /// 保存を待っているか、書き込み中。
    pub(crate) fn saving(&self) -> bool {
        self.enabled() && (self.requested.is_some() || self.saver.as_ref().is_some_and(Saver::busy))
    }

    /// きっかけを覚える。フレームの終わりの `on_frame` で書く。
    pub(crate) fn request(&mut self, cause: SaveCause) {
        if self.enabled() && self.requested.is_none() {
            self.requested = Some(cause);
        }
    }

    /// 終わるときの保存を頼む。キューにコマンドが残っていれば写しを取れないので、頼まずに前の保存を残す。
    pub(crate) fn request_final(&mut self, game: &Game) {
        if game.pending().is_empty() {
            self.request(SaveCause::Quit);
        } else if self.enabled() {
            warn!(
                "キューにコマンドが {} 件残っているので保存しない。前の保存が残る",
                game.pending().len()
            );
            self.requested = None;
        }
    }

    /// 書き込みスレッドが空くまで `timeout` だけ待つ。空かなければ諦める(`Drop` でも待たない)。
    pub(crate) fn finish(&mut self, timeout: Duration) {
        let Some(w) = self.saver.as_mut() else {
            return;
        };
        if !w.wait_idle(timeout) {
            warn!(
                "保存が {}ms で終わらない。待たずに終える",
                timeout.as_millis()
            );
            w.abandon();
        }
    }

    /// 書き込みを諦める(`Drop` で待たない)。
    pub(crate) fn give_up(&mut self) {
        if let Some(w) = self.saver.as_mut() {
            w.abandon();
        }
    }

    /// 続きからの候補を渡す(1 回だけ)。
    pub(crate) fn take_candidate(&mut self) -> Option<Loaded> {
        self.candidate.take()
    }

    /// 両方のスロットを消す(初めから・リセット)。候補と待っている保存も捨てる。
    pub(crate) fn clear_slots(&mut self) {
        self.candidate = None;
        self.has_save = false;
        self.requested = None;
        if let Some(s) = &self.saver {
            s.clear();
        }
    }

    /// せってい を閉じたときと終わるとき: 前に書いた値と違えば `settings.ron` を書く。
    pub(crate) fn write_settings_if_changed(&mut self, ui: &Ui) {
        let s = ui.settings();
        if s == self.settings || !self.enabled() {
            return;
        }
        if let Some(w) = &self.saver {
            w.submit_settings(encode_settings(s).into_bytes());
            self.settings = s;
            debug!("設定を保存する: {s:?}");
        }
    }

    /// フレームの終わり(§1.6 の 5b)。せってい の開閉と 60 秒ごとを見て、要るなら写しを取って書き込みスレッドへ渡す。
    /// 写しを取ったら true。
    pub(crate) fn on_frame(&mut self, play: Option<Playing<'_>>, ui: &Ui) -> bool {
        self.check_failures();
        let open = ui.is_settings_open();
        if open != self.settings_open {
            self.settings_open = open;
            if !open {
                self.write_settings_if_changed(ui);
            } else if play.is_some() {
                self.request(SaveCause::SettingsOpened);
            }
        }
        let Some(p) = play else {
            return false;
        };
        if p.game.now().0 >= *p.last_save_tick + SAVE_INTERVAL_TICKS {
            self.request(SaveCause::Interval);
        }
        let Some(cause) = self.requested else {
            return false;
        };
        if !p.game.pending().is_empty() {
            // step が 0 回のフレームの後: 写しを取れないので頼まない(60 秒ごとは空になったフレームでまた来る)
            debug!("キューにコマンドが残っているので {cause:?} の保存をしない");
            self.requested = None;
            return false;
        }
        let Some(w) = &self.saver else {
            return false;
        };
        let snap = p.game.snapshot();
        let text = SaveFile::encode(self.next_seq, &snap, &p.tutorial.save(), &self.version);
        debug!(
            "保存 seq={} ({cause:?}) tick={} {} B",
            self.next_seq,
            snap.tick,
            text.len()
        );
        self.stats.bytes_max = self.stats.bytes_max.max(text.len());
        self.stats.encoded += 1;
        w.submit(text.into_bytes());
        self.next_seq = self.next_seq.saturating_add(1);
        *p.last_save_tick = snap.tick;
        self.requested = None;
        self.has_save = true;
        true
    }

    /// `on_frame` が写しを取ったフレームの、メインスレッドの時間。
    pub(crate) fn note_encode_us(&mut self, us: u64) {
        self.stats.encode_us_max = self.stats.encode_us_max.max(us);
        self.stats.encode_us_total += us;
    }

    fn check_failures(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(w) = &self.saver
            && !w.alive()
        {
            warn!("セーブの書き込みスレッドが止まった。この起動の間は保存しない");
            self.stopped = true;
            self.requested = None;
            return;
        }
        if let Some(w) = &self.saver
            && w.stats().consecutive_failures >= MAX_CONSECUTIVE_FAILURES
        {
            warn!(
                "セーブの書き込みに {MAX_CONSECUTIVE_FAILURES} 回続けて失敗した。この起動の間は保存しない"
            );
            self.stopped = true;
            self.requested = None;
        }
    }

    /// (成功した書き込み、失敗した書き込み)。
    pub(crate) fn counts(&self) -> (u32, u32) {
        self.saver.as_ref().map_or((0, 0), |w| {
            let s = w.stats();
            (s.saved, s.failed)
        })
    }

    /// 終わるときのログ。
    pub(crate) fn log_summary(&self) {
        let Some(w) = &self.saver else {
            return;
        };
        let ws = w.stats();
        let s = self.stats;
        info!(
            "保存: 写し {} 回(メインスレッド 平均 {}µs 最大 {}µs、最大 {} B)、書き込み 成功 {} 失敗 {}(平均 {}µs 最大 {}µs)",
            s.encoded,
            s.encode_us_total / u64::from(s.encoded.max(1)),
            s.encode_us_max,
            s.bytes_max,
            ws.saved,
            ws.failed,
            ws.write_us_total / u64::from(ws.saved.max(1)),
            ws.write_us_max
        );
    }
}

#[cfg(test)]
mod tests {
    use super::file::tests::{encoded, game};
    use super::*;

    fn balance() -> Arc<BalanceTable> {
        Arc::new(BalanceTable::builtin())
    }

    #[test]
    fn scan_prefers_the_larger_valid_seq() {
        let g1 = game(1, 100);
        let g2 = game(1, 200);
        let slots = vec![(Slot::A, encoded(4, &g1)), (Slot::B, encoded(5, &g2))];
        let s = scan_slots(&slots, &balance());
        let (slot, l) = s.candidate.expect("候補");
        assert_eq!((slot, l.seq, l.game.now().0), (Slot::B, 5, 200));
        assert!(s.rejected.is_empty());
        assert_eq!((s.max_seq, s.next_slot), (5, Slot::A));

        let mut broken = encoded(6, &g2);
        let n = broken.len();
        broken[n - 5] ^= 1;
        let slots = vec![(Slot::A, encoded(4, &g1)), (Slot::B, broken)];
        let s = scan_slots(&slots, &balance());
        let (slot, l) = s.candidate.expect("古い方");
        assert_eq!((slot, l.seq), (Slot::A, 4));
        assert_eq!(s.rejected.len(), 1);
        assert!(matches!(
            s.rejected[0],
            (Slot::B, SaveError::BodyFnv { .. })
        ));
        assert_eq!(
            (s.max_seq, s.next_slot),
            (6, Slot::B),
            "読めなかった方に書く"
        );
    }

    #[test]
    fn scan_without_a_candidate_writes_the_missing_or_older_slot() {
        let g = game(1, 50);
        let s = scan_slots(&[], &balance());
        assert!(s.candidate.is_none());
        assert_eq!((s.max_seq, s.next_slot), (0, Slot::A));

        let mut bad = encoded(9, &g);
        let n = bad.len();
        bad[n - 5] ^= 1;
        let s = scan_slots(&[(Slot::A, bad.clone())], &balance());
        assert!(s.candidate.is_none());
        assert_eq!((s.max_seq, s.next_slot), (9, Slot::B), "ない方");

        let mut older = encoded(3, &g);
        let n = older.len();
        older[n - 5] ^= 1;
        let s = scan_slots(&[(Slot::A, bad.clone()), (Slot::B, older)], &balance());
        assert_eq!((s.max_seq, s.next_slot), (9, Slot::B), "seq の小さい方");
        let s = scan_slots(&[(Slot::A, b"junk".to_vec()), (Slot::B, bad)], &balance());
        assert_eq!(s.rejected.len(), 2);
        assert_eq!((s.max_seq, s.next_slot), (9, Slot::A), "ヘッダが読めない方");
    }

    fn state(store: &MemStore) -> SaveState {
        SaveState::open(&SavePolicy::Memory(store.clone()), &balance(), "v").0
    }

    fn wait(s: &SaveState) {
        assert!(
            s.saver
                .as_ref()
                .unwrap()
                .wait_idle(std::time::Duration::from_secs(5))
        );
    }

    fn play<'a>(game: &'a Game, tutorial: &'a Tutorial, last: &'a mut u64) -> Playing<'a> {
        Playing {
            game,
            tutorial,
            last_save_tick: last,
        }
    }

    fn cmd() -> tm_core::Command {
        tm_core::Command::LevelUp {
            param: tm_core::ParamKey::S1Spawn,
            scope: tm_core::UpgradeScope::AllStations,
            mode: tm_core::BuyMode::One,
        }
    }

    #[test]
    fn app_t_25_no_snapshot_while_commands_are_pending() {
        let store = MemStore::new();
        let mut s = state(&store);
        let ui = Ui::new();
        let tut = Tutorial::new(false);
        let mut g = game(1, 10);
        let mut last = 0;
        g.enqueue(cmd());
        s.request(SaveCause::SettingsOpened);
        assert!(
            !s.on_frame(Some(play(&g, &tut, &mut last)), &ui),
            "キューが空でなければ写しを取らない"
        );
        assert!(!s.saving(), "頼まないので保存中の印も出さない");
        g.step(&mut Vec::new());
        assert!(
            !s.on_frame(Some(play(&g, &tut, &mut last)), &ui),
            "捨てたきっかけは残らない"
        );
        s.request(SaveCause::LevelUp);
        assert!(s.on_frame(Some(play(&g, &tut, &mut last)), &ui));
        assert_eq!(last, 11);
        wait(&s);
        assert_eq!(store.history().len(), 1);
        assert!(
            !s.on_frame(Some(play(&g, &tut, &mut last)), &ui),
            "きっかけがなければ書かない"
        );
        let mut later = game(1, 11 + SAVE_INTERVAL_TICKS);
        let mut last = 12;
        assert!(!s.on_frame(Some(play(&later, &tut, &mut last)), &ui));
        let mut last = 11;
        later.enqueue(cmd());
        assert!(!s.on_frame(Some(play(&later, &tut, &mut last)), &ui));
        assert!(!s.saving());
        later.step(&mut Vec::new());
        assert!(
            s.on_frame(Some(play(&later, &tut, &mut last)), &ui),
            "3,600 tick ごとは次に空になったフレームで書く"
        );
        s.request(SaveCause::Quit);
        assert!(!s.on_frame(None, &ui), "ゲームがなければ書かない");
        wait(&s);
        s.clear_slots();
        wait(&s);
        assert!(!s.saving());
        assert!(store.slot(Slot::A).is_none() && store.slot(Slot::B).is_none());
        assert!(!s.has_save());
        assert_eq!(s.counts(), (2, 0));
    }

    #[test]
    fn clear_slots_lets_go_of_the_candidate() {
        let store = MemStore::new();
        store.set_slot(Slot::A, Some(encoded(3, &game(1, 50))));
        let mut s = state(&store);
        assert!(s.has_save() && s.candidate.is_some());
        s.clear_slots();
        assert!(s.candidate.is_none(), "候補の Game を手放す(16 §1.19)");
        assert!(s.take_candidate().is_none());
    }

    #[test]
    fn huge_seq_is_not_used_and_does_not_overflow() {
        let good = encoded(5, &game(1, 50));
        let text = String::from_utf8(encoded(7, &game(1, 60))).unwrap();
        let huge = text
            .replacen("seq: 7,", &format!("seq: {},", u64::MAX), 1)
            .into_bytes();
        let s = scan_slots(&[(Slot::A, huge.clone()), (Slot::B, good)], &balance());
        assert_eq!(
            s.candidate.as_ref().map(|c| (c.0, c.1.seq)),
            Some((Slot::B, 5))
        );
        assert_eq!(s.max_seq, 5);
        assert!(matches!(
            s.rejected[..],
            [(Slot::A, SaveError::Seq { found: u64::MAX })]
        ));
        let store = MemStore::new();
        store.set_slot(Slot::A, Some(huge));
        let s = state(&store);
        assert!(!s.has_save());
        assert_eq!(s.next_seq, 1);
    }

    #[test]
    fn a_panicking_writer_stops_saving() {
        let store = MemStore::new();
        let mut s = state(&store);
        let ui = Ui::new();
        let tut = Tutorial::new(false);
        let g = game(1, 10);
        let mut last = 0;
        store.panic_next_write();
        s.request(SaveCause::NewGame);
        assert!(s.on_frame(Some(play(&g, &tut, &mut last)), &ui));
        let t = std::time::Instant::now();
        while s.saver.as_ref().unwrap().alive() {
            assert!(t.elapsed() < std::time::Duration::from_secs(5));
            std::thread::yield_now();
        }
        assert!(!s.saving(), "busy を残さない");
        assert!(!s.on_frame(Some(play(&g, &tut, &mut last)), &ui));
        assert!(!s.enabled(), "保存をやめる");
        s.request(SaveCause::Quit);
        assert!(!s.saving());
        s.finish(std::time::Duration::from_millis(10));
    }

    #[test]
    fn settings_are_written_only_when_changed() {
        let store = MemStore::new();
        store.set_settings(Some(
            encode_settings(Settings {
                music: 2,
                sound: 3,
                show_explanations: false,
            })
            .into_bytes(),
        ));
        let (mut s, loaded) = SaveState::open(&SavePolicy::Memory(store.clone()), &balance(), "v");
        let loaded = loaded.unwrap();
        assert_eq!(
            (loaded.music, loaded.sound, loaded.show_explanations),
            (2, 3, false)
        );
        let mut ui = Ui::new();
        ui.set_settings(loaded);
        s.write_settings_if_changed(&ui);
        wait(&s);
        assert_eq!(store.counts().settings_writes, 0, "読んだ値と同じ");
        ui.set_settings(Settings::default());
        s.write_settings_if_changed(&ui);
        s.write_settings_if_changed(&ui);
        wait(&s);
        assert_eq!(store.counts().settings_writes, 1);
        assert_eq!(
            decode_settings(&store.settings().unwrap()),
            Ok(Settings::default())
        );
    }

    #[test]
    fn off_and_unusable_dir_do_not_save() {
        let (s, settings) = SaveState::open(&SavePolicy::Off, &balance(), "v");
        assert!(!s.enabled() && !s.has_save() && !s.saving() && settings.is_none());
        let file = std::env::temp_dir().join(format!("tm-app-save-file-{}", std::process::id()));
        std::fs::write(&file, b"").unwrap();
        let (s, _) = SaveState::open(&SavePolicy::Dir(file.join("x")), &balance(), "v");
        assert!(!s.enabled());
        std::fs::remove_file(&file).unwrap();
    }
}
