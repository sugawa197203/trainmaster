//! APP-T-26、27、31: 電源断の模擬、破損と版違い、書き込みの失敗と保存しない設定(16 §2.4、§1.20)。

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Rig, SEED_A, TITLE_READY_FRAME, config, newest, saving, wait_saved};
use log::LevelFilter;
use tm_app::save::{SaveFile, SaveHeader, WriteStage, body_fnv};
use tm_app::{App, AppConfig, FsStore, MemStore, SavePolicy, SaveStore, SeedPolicy, Slot};
use tm_assets::txt;
use tm_core::{
    BalanceTable, BuyMode, Command, Game, GameConfig, GameView, LineEnd, LineId, ParamKey,
    ParamOwner, StationId, Tick, UpgradeScope,
};
use tm_platform_api::{HeadlessClock, Key};
use tm_tutorial::Tutorial;
use tm_ui::testing::draw_text;

const GOLDEN: &str = "tests/golden/save_v1.tms";
/// M7〜M10(`RULES_VERSION` 2〜5)の版で書いた本物のセーブ。いまの版では読まない。
const GOLDEN_OLD: [(&str, u32); 4] = [
    ("tests/golden/save_rules2.tms", 2),
    ("tests/golden/save_rules3.tms", 3),
    ("tests/golden/save_rules4.tms", 4),
    ("tests/golden/save_rules5.tms", 5),
];

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tm-app-save-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

fn warned(what: &str) -> bool {
    tm_app::logging::recent(256)
        .iter()
        .any(|l| l.contains("WARN") && l.contains(what))
}

/// 始めて 1 分遊び、3 つの保存(初めから、せってい、60 秒)を書く。最後の保存の (tick, state_hash)。
fn three_saves(r: &mut Rig) -> (u64, u64) {
    r.start_game();
    wait_saved(&r.app);
    r.disable_explanations();
    wait_saved(&r.app);
    while r.tick() < 3_600 {
        r.frame();
    }
    wait_saved(&r.app);
    (r.tick(), r.game().state_hash())
}

fn reopen(cfg: AppConfig) -> Rig {
    let mut r = Rig::new(HeadlessClock::Exact60, cfg);
    r.frames(TITLE_READY_FRAME);
    r
}

#[test]
fn app_t_26_power_loss_in_mem_store() {
    for stage in [
        WriteStage::TmpPartial,
        WriteStage::BeforeFsync,
        WriteStage::BeforeRename,
        WriteStage::BeforeDirFsync,
    ] {
        let store = MemStore::new();
        let cfg = saving(SeedPolicy::Fixed(SEED_A), &store);
        let mut r = Rig::new(HeadlessClock::Exact60, cfg.clone());
        let (tick, hash) = three_saves(&mut r);
        let (slot, h, _) = newest(&store).unwrap();
        assert_eq!((slot, h.seq, h.tick), (Slot::A, 3, tick));
        r.frames(10);
        store.crash_next_write(stage);
        r.tap(Key::Escape);
        let (crash_tick, crash_hash) = (r.tick(), r.game().state_hash());
        wait_saved(&r.app);
        assert_eq!(r.app.stats().save_failures, 1);
        drop(r);

        let mut r = reopen(cfg);
        r.continue_game();
        if stage == WriteStage::BeforeDirFsync {
            assert_eq!(
                (r.tick(), r.game().state_hash()),
                (crash_tick, crash_hash),
                "rename の後は新しい方"
            );
        } else {
            assert_eq!((r.tick(), r.game().state_hash()), (tick, hash), "{stage:?}");
            assert!(store.tmp(Slot::B).is_some(), "書きかけ");
        }
        r.frames(2);
        r.tap(Key::Escape);
        wait_saved(&r.app);
        assert!(
            store.tmp(Slot::A).is_none() && store.tmp(Slot::B).is_none(),
            "次の保存で消える"
        );
        r.assert_audio_ok();
    }
}

fn copy_newest_to_tmp(dir: &Path, cut: Option<usize>) -> (Slot, SaveHeader) {
    let slots: Vec<(Slot, Vec<u8>)> = Slot::ALL
        .into_iter()
        .filter_map(|s| fs::read(dir.join(s.file_name())).ok().map(|b| (s, b)))
        .collect();
    let (slot, bytes) = slots
        .iter()
        .max_by_key(|(_, b)| SaveFile::header(b).unwrap().seq)
        .unwrap();
    let h = SaveFile::header(bytes).unwrap();
    let mut b = bytes.clone();
    b = b.replacen_seq(h.seq, h.seq + 1);
    if let Some(n) = cut {
        b.truncate(b.len() * n / 100);
    }
    fs::write(dir.join(format!("{}.tmp", slot.other().file_name())), b).unwrap();
    (*slot, h)
}

trait ReplaceSeq {
    fn replacen_seq(self, from: u64, to: u64) -> Self;
}

impl ReplaceSeq for Vec<u8> {
    fn replacen_seq(self, from: u64, to: u64) -> Self {
        String::from_utf8(self)
            .unwrap()
            .replacen(&format!("seq: {from},"), &format!("seq: {to},"), 1)
            .into_bytes()
    }
}

#[test]
fn app_t_26_power_loss_in_fs_store() {
    let dir = temp_dir("fs");
    let cfg = AppConfig {
        save: SavePolicy::Dir(dir.join("trainmaster")),
        ..config(SeedPolicy::Fixed(SEED_A))
    };
    let mut r = Rig::new(HeadlessClock::Exact60, cfg.clone());
    let (tick, hash) = three_saves(&mut r);
    drop(r);
    let save = dir.join("trainmaster");
    let mut names: Vec<String> = fs::read_dir(&save)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["settings.ron", "slot-a.tms", "slot-b.tms"]);

    for cut in [Some(0), Some(50), Some(99), None] {
        let (slot, h) = copy_newest_to_tmp(&save, cut);
        assert_eq!((slot, h.tick), (Slot::A, tick));
        let mut r = reopen(cfg.clone());
        r.continue_game();
        assert_eq!((r.tick(), r.game().state_hash()), (tick, hash), "{cut:?}");
        drop(r);
    }
    fs::write(save.join("slot-b.tms"), b"").unwrap();
    let mut r = reopen(cfg.clone());
    r.continue_game();
    assert_eq!(r.tick(), tick, "空のスロットは使わない");
    r.frames(2);
    r.tap(Key::Escape);
    wait_saved(&r.app);
    assert!(!save.join("slot-b.tms.tmp").exists(), "次の保存で消える");
    let b = fs::read(save.join("slot-b.tms")).unwrap();
    assert_eq!(SaveFile::header(&b).unwrap().seq, h_seq(&save, Slot::A) + 1);
    drop(r);
    fs::remove_dir_all(&dir).unwrap();
}

fn h_seq(dir: &Path, slot: Slot) -> u64 {
    SaveFile::header(&fs::read(dir.join(slot.file_name())).unwrap())
        .unwrap()
        .seq
}

/// ヘッダの 1 か所を変える。
fn edit_header(b: &[u8], from: &str, to: &str) -> Vec<u8> {
    let s = String::from_utf8(b.to_vec()).unwrap();
    let (h, body) = s.split_once('\n').unwrap();
    assert!(h.contains(from), "{h}");
    format!("{}\n{body}", h.replacen(from, to, 1)).into_bytes()
}

/// 本体を変えて `body_fnv` を合わせる。
fn edit_body(b: &[u8], from: &str, to: &str) -> Vec<u8> {
    let s = String::from_utf8(b.to_vec()).unwrap();
    let (h, body) = s.split_once('\n').unwrap();
    assert!(body.contains(from));
    let body = body.replacen(from, to, 1);
    let old = SaveFile::header(b).unwrap().body_fnv;
    let h = h.replacen(
        &format!("body_fnv: {old}"),
        &format!("body_fnv: {}", body_fnv(body.as_bytes())),
        1,
    );
    format!("{h}\n{body}").into_bytes()
}

#[test]
fn app_t_27_corrupt_and_other_versions() {
    tm_app::logging::init(LevelFilter::Info);
    let store = MemStore::new();
    let cfg = saving(SeedPolicy::Fixed(SEED_A), &store);
    let mut r = Rig::new(HeadlessClock::Exact60, cfg.clone());
    let (tick, _) = three_saves(&mut r);
    r.frames(30);
    r.tap(Key::Escape);
    wait_saved(&r.app);
    drop(r);
    let (new_slot, h, good) = newest(&store).unwrap();
    assert_eq!((new_slot, h.seq), (Slot::B, 4));
    let old_slot = new_slot.other();
    let old = store.slot(old_slot).unwrap();
    let money = {
        let s = String::from_utf8(good.clone()).unwrap();
        let i = s.find("money_bits: ").unwrap();
        s[i..i + s[i..].find(',').unwrap()].to_owned()
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("magic", edit_header(&good, "TMSAVE", "TMSAVF")),
        (
            "save_format",
            edit_header(&good, "save_format: 1", "save_format: 7"),
        ),
        (
            "rules_version",
            edit_header(&good, "rules_version: 6", "rules_version: 4"),
        ),
        (
            "config_hash",
            edit_header(
                &good,
                &format!("config_hash: {}", h.config_hash),
                "config_hash: 1",
            ),
        ),
        ("body_fnv", {
            let mut b = good.clone();
            let n = b.len() - 20;
            b[n] ^= 0x01;
            b
        }),
        (
            "本体が読めない",
            edit_body(&good, "SaveBody(", "SaveBody((("),
        ),
        (
            "HashMismatch",
            edit_body(&good, &money, "money_bits: 4611686018427387904"),
        ),
        ("ヘッダが読めない", b"\x00\x01garbage".to_vec()),
    ];
    for (what, bytes) in cases {
        store.set_slot(new_slot, Some(bytes.clone()));
        let mut r = reopen(cfg.clone());
        assert!(r.app.app_status().has_save, "{what}");
        assert!(warned(what), "{what}: {:?}", tm_app::logging::recent(4));
        r.continue_game();
        assert_eq!(r.tick(), tick, "{what}: 古い方を読む");
        drop(r);
        store.set_slot(old_slot, Some(bytes));
        let r = reopen(cfg.clone());
        assert!(
            !r.app.app_status().has_save,
            "{what}: 両方だめなら「初めから」だけ"
        );
        drop(r);
        store.set_slot(old_slot, Some(old.clone()));
    }

    store.set_slot(
        new_slot,
        Some(edit_header(
            &good,
            &format!("game_version: \"{}\"", h.game_version),
            "game_version: \"v0.0.0-other\"",
        )),
    );
    let mut r = reopen(cfg.clone());
    r.continue_game();
    assert_eq!(r.tick(), h.tick, "game_version だけの違いは読む");
    assert!(
        tm_app::logging::recent(256)
            .iter()
            .any(|l| l.contains("v0.0.0-other")),
        "ログに出す"
    );
}

#[test]
fn app_t_27_golden_save_v1() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);
    if std::env::var_os("TM_UPDATE_GOLDEN").is_some() {
        let store = MemStore::new();
        let mut r = Rig::new(
            HeadlessClock::Exact60,
            saving(SeedPolicy::Fixed(SEED_A), &store),
        );
        r.build_extend_upgrade();
        let t = r.tick();
        r.frames(36_000 - t);
        r.tap(Key::Escape);
        wait_saved(&r.app);
        fs::write(&path, newest(&store).unwrap().2).unwrap();
    }
    let bytes = fs::read(&path).expect("tests/golden/save_v1.tms");
    let balance = Arc::new(BalanceTable::builtin());
    let l = SaveFile::load(&bytes, &balance).expect("今の版で読める");
    let f = SaveFile::decode(&bytes, &balance).unwrap();
    assert_eq!(f.header.save_format, 1);
    assert_eq!((l.seed, l.game.now()), (SEED_A, Tick(36_000)));
    assert_eq!(l.game.state_hash(), f.header.state_hash);
    assert_eq!(l.game.view().lines().len(), 1);

    let store = MemStore::new();
    store.set_slot(Slot::A, Some(bytes));
    let mut r = reopen(saving(SeedPolicy::Fixed(SEED_A), &store));
    r.continue_game();
    assert_eq!(r.tick(), 36_000);
}

/// 前の版のセーブは読まずに「初めから」だけにする。panic しない。
#[test]
fn app_t_27_previous_rules_save_is_refused() {
    tm_app::logging::init(LevelFilter::Info);
    for (path, rules) in GOLDEN_OLD {
        let bytes = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).expect(path);
        let balance = Arc::new(BalanceTable::builtin());
        assert!(
            matches!(
                SaveFile::load(&bytes, &balance),
                Err(tm_app::save::SaveError::RulesVersion { found }) if found == rules
            ),
            "{path}"
        );
        let store = MemStore::new();
        store.set_slot(Slot::A, Some(bytes));
        let r = reopen(saving(SeedPolicy::Fixed(SEED_A), &store));
        assert!(!r.app.app_status().has_save, "{path}");
        assert!(warned("rules_version"), "{path}");
    }
}

#[test]
fn app_t_31_write_failures_stop_saving() {
    tm_app::logging::init(LevelFilter::Info);
    let store = MemStore::new();
    store.fail_next_writes(u32::MAX);
    let mut r = Rig::new(
        HeadlessClock::Exact60,
        saving(SeedPolicy::Fixed(SEED_A), &store),
    );
    r.start_game();
    wait_saved(&r.app);
    assert!(r.app.app_status().save_enabled);
    r.disable_explanations();
    wait_saved(&r.app);
    assert_eq!(r.app.stats().save_failures, 2);
    assert!(warned("を書けない"));
    while r.tick() < 3_600 + 5 {
        r.frame();
    }
    wait_saved(&r.app);
    r.frame();
    let st = r.app.app_status();
    assert!(!st.save_enabled && !st.saving, "3 回続けて失敗したらやめる");
    assert_eq!(r.app.stats().save_failures, 3);
    assert!(warned("続けて失敗"));
    let t = r.tick();
    store.hold();
    r.tap(Key::Escape);
    assert!(!r.app.app_status().saving, "保存中の印も出さない");
    assert!(
        !store.wait_held(Duration::from_millis(200)),
        "やめた後は書かない"
    );
    store.release();
    r.tap(Key::Escape);
    r.frames(60);
    assert!(r.tick() > t, "遊びは続く");
    assert_eq!(store.counts().failed_writes, 3);
    assert_eq!(store.counts().slot_writes, 0);
}

#[test]
fn app_t_31_a_panicking_writer_stops_saving() {
    let store = MemStore::new();
    store.panic_next_write();
    let mut r = Rig::new(
        HeadlessClock::Exact60,
        saving(SeedPolicy::Fixed(SEED_A), &store),
    );
    r.start_game();
    let t = Instant::now();
    while r.app.app_status().save_enabled {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "スレッドの終わりに気づく"
        );
        r.frame();
    }
    assert!(!r.app.app_status().saving, "保存中の印を残さない");
    r.disable_explanations();
    let tick = r.tick();
    r.frames(60);
    assert_eq!(r.tick(), tick + 60, "遊びは続く");
    assert!(!r.app.app_status().saving);
    assert_eq!(store.counts().slot_writes, 0);
    let t = Instant::now();
    drop(r);
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "終わるときに待ち続けない"
    );
}

/// 終わる確認の 1 回目を開いた画面に描かれた文字(並びのまま)。
fn quit_confirm_text(cfg: AppConfig) -> String {
    let mut r = Rig::new(HeadlessClock::Exact60, cfg);
    r.start_game();
    r.disable_explanations();
    r.tap(Key::Escape);
    for _ in 0..6 {
        r.tap(Key::Down);
    }
    r.tap(Key::Up);
    r.tap(Key::Z);
    draw_text(r.app.draw_list())
        .lines()
        .filter_map(|l| l.strip_prefix("Sprite '")?.chars().next())
        .collect()
}

fn shows(drawn: &str, id: tm_assets::text::TextId) -> bool {
    let want: String = tm_assets::text::text(id)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    drawn.contains(&want)
}

#[test]
fn app_t_31_save_off() {
    let mut r = Rig::new(HeadlessClock::Exact60, config(SeedPolicy::Fixed(SEED_A)));
    r.frame();
    let st = r.app.app_status();
    assert!(!st.save_enabled && !st.has_save && !st.saving);
    r.start_game();
    r.disable_explanations();
    r.frames(10);
    assert_eq!(r.app.stats().saves_written, 0);
    drop(r);
    let mut r = Rig::new(HeadlessClock::Exact60, config(SeedPolicy::Fixed(SEED_A)));
    r.frame();
    assert!(r.app.ui().show_explanations(), "設定も残らない");

    let file = temp_dir("not-a-dir");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"").unwrap();
    let cfg = AppConfig {
        save: SavePolicy::Dir(file.join("trainmaster")),
        ..config(SeedPolicy::Fixed(SEED_A))
    };
    let mut p = common::platform(HeadlessClock::Exact60);
    let app = App::new(&mut p, cfg.clone()).expect("作れない TM_SAVE_DIR でも起動する");
    assert!(
        !app.app_status().save_enabled,
        "作れない TM_SAVE_DIR は off"
    );
    drop(app);

    let off = quit_confirm_text(cfg);
    let on = quit_confirm_text(saving(SeedPolicy::Fixed(SEED_A), &MemStore::new()));
    assert!(
        shows(&off, txt::QUIT_CONFIRM1_NOSAVE) && !shows(&off, txt::QUIT_CONFIRM1),
        "保存しないときは終わる確認の文が変わる: {off}"
    );
    assert!(
        shows(&on, txt::QUIT_CONFIRM1) && !shows(&on, txt::QUIT_CONFIRM1_NOSAVE),
        "保存するときの終わる確認の文: {on}"
    );
    fs::remove_file(&file).unwrap();
}

fn cheap() -> Arc<BalanceTable> {
    let mut spec = BalanceTable::builtin().spec().clone();
    for p in &mut spec.params {
        p.base_yen = 1;
        p.cost_ratio = 1;
        p.cost_steps.clear();
    }
    spec.economy.initial_money_yen = u64::MAX;
    spec.economy.track.c1_yen = 1;
    spec.economy.track.level_ratio = 1;
    spec.economy.track.tile_ratio = (1, 1);
    Arc::new(BalanceTable::from_spec(spec).expect("検証を通る表"))
}

fn create(v: &GameView<'_>, mut ids: Vec<StationId>, k: usize) -> Command {
    let n = ids.len();
    ids.rotate_left((k * 3) % n);
    if k % 2 == 1 {
        ids.reverse();
    }
    let bends = ids.windows(2).map(|w| v.default_bend(w[0], w[1])).collect();
    Command::CreateLine {
        stations: ids,
        bends,
    }
}

/// 01 の上限(駅 27、路線 6、列車 48、全パラメータ最大)まで作り、10 分走らせた終盤のゲーム。
fn full_game() -> Game {
    let bal = cheap();
    let mut g = Game::new(GameConfig {
        seed: SEED_A,
        balance: Arc::clone(&bal),
    });
    let mut ev = Vec::new();
    let full = |v: &GameView<'_>| {
        v.stations().len() == bal.max_stations() as usize
            && v.lines().len() == bal.max_lines() as usize
            && v.trains().len() == 48
    };
    while !full(&g.view()) {
        assert!(g.now().0 < 20_000, "上限の規模に届かない");
        let v = g.view();
        let ids: Vec<StationId> = v.stations().map(|s| s.id).collect();
        let mut cmds = Vec::new();
        if v.lines().len() == 0 {
            cmds.push(create(&v, ids.clone(), 0));
        } else if let Some(s) = v.stations().find(|s| !s.connected).map(|s| s.id) {
            cmds.push(Command::ExtendLine {
                line: LineId(0),
                end: LineEnd::Tail,
                station: s,
                bend: v.default_bend(v.line_ends(LineId(0))[1].1, s),
            });
        } else if ids.len() == bal.max_stations() as usize {
            let (used, slots) = v.line_slots();
            if used < slots {
                cmds.push(create(&v, ids.clone(), used as usize));
            }
        }
        for p in ParamKey::ALL {
            let scope = match p.owner() {
                ParamOwner::Station => UpgradeScope::AllStations,
                ParamOwner::Line => UpgradeScope::AllLines,
                ParamOwner::Company => UpgradeScope::Company,
            };
            let c = Command::LevelUp {
                param: p,
                scope,
                mode: BuyMode::Max,
            };
            if v.validate(&c).is_ok() {
                cmds.push(c);
            }
        }
        for c in cmds {
            g.enqueue(c);
        }
        g.step(&mut ev);
        ev.clear();
    }
    for _ in 0..36_000 {
        g.step(&mut ev);
        ev.clear();
    }
    g
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

#[test]
#[ignore = "APP-V-08 の開発機の記録(時間を出す)。test:long で回す"]
fn app_v_08_save_size_and_times() {
    let g = full_game();
    let tut = Tutorial::new(false);
    let bal = Arc::clone(&g.config().balance);
    let mut enc = Vec::new();
    let mut text = String::new();
    for i in 0..50 {
        let t = Instant::now();
        text = SaveFile::encode(i, &g.snapshot(), &tut.save(), "bench");
        enc.push(t.elapsed());
    }
    let bytes = text.into_bytes();
    assert!(bytes.len() < 256 * 1024, "{} B", bytes.len());
    let mut load = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        let other = SaveFile::header(&bytes).unwrap();
        let l = SaveFile::load(&bytes, &bal).unwrap();
        load.push(t.elapsed());
        assert_eq!((l.game.state_hash(), other.seq), (g.state_hash(), 49));
    }
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("app-v-08-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut fs_store = FsStore::open(&dir).unwrap();
    let mut write = Vec::new();
    for i in 0..20 {
        let slot = if i % 2 == 0 { Slot::A } else { Slot::B };
        let t = Instant::now();
        fs_store.write_slot(slot, &bytes).unwrap();
        write.push(t.elapsed());
    }
    fs::remove_dir_all(&dir).unwrap();
    let stat = |v: &[Duration]| {
        let avg = v.iter().map(|d| us(*d)).sum::<f64>() / v.len() as f64;
        let max = v.iter().map(|d| us(*d)).fold(0.0, f64::max);
        format!("平均 {avg:.0}µs 最大 {max:.0}µs")
    };
    let s = g.snapshot();
    println!(
        "APP-V-08(開発機): 駅 {}、路線 {}、列車 {}、セーブ {} B。保存 1 回のメインスレッド(snapshot + RON + FNV) {}。\
         起動時の検査(スロット 2 つのヘッダ + 新しい方の load) {}。書き込みスレッド(FsStore、fsync 2 回) {}",
        s.stations.len(),
        s.lines.len(),
        s.trains.len(),
        bytes.len(),
        stat(&enc),
        stat(&load),
        stat(&write)
    );
}
