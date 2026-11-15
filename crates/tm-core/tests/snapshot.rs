//! セーブの写しのゴールデン(11 §11.3): 決まった状態の `GameSnapshot` の RON。形が意図せず変わったら検出する。
//! `SNAPSHOT_FORMAT_VERSION` かルールを変えたら `TM_UPDATE_GOLDEN=1 cargo test -p tm-core --test snapshot` で作り直す。

#[allow(dead_code, reason = "テストのファイルごとに使う補助が違う")]
mod common;

use std::path::PathBuf;

use tm_core::{
    Bend, BuyMode, Command, Game, GameSnapshot, LineEnd, LineId, ParamKey, SNAPSHOT_FORMAT_VERSION,
    SnapPhase, StationId,
};

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/snapshot_mixed.ron")
}

fn to_text(snap: &GameSnapshot) -> String {
    let pretty = ron::ser::PrettyConfig::new().depth_limit(2);
    ron::ser::to_string_pretty(snap, pretty).expect("RON に書ける") + "\n"
}

fn step(g: &mut Game, cmds: Vec<Command>, ticks: u64) {
    cmds.into_iter().for_each(|c| g.enqueue(c));
    let mut ev = Vec::new();
    for _ in 0..ticks {
        g.step(&mut ev);
    }
}

fn buy_all() -> Vec<Command> {
    ParamKey::ALL
        .iter()
        .map(|&p| Command::LevelUp {
            param: p,
            scope: common::all_scope(p),
            mode: BuyMode::One,
        })
        .collect()
}

/// 2 本の路線(`DiagonalFirst`、先頭への延伸、通過駅)、走行中と停車中の列車、強化のある状態(所持金の多い表)。
fn mixed() -> Game {
    let bal = common::rich();
    let mut g = common::new_game(2, &bal);
    let first = Command::CreateLine {
        stations: vec![StationId(0), StationId(1), StationId(2)],
        bends: vec![Bend::DiagonalFirst, Bend::StraightFirst],
    };
    step(&mut g, vec![first], 1);
    for _ in 0..12 {
        step(&mut g, buy_all(), 30);
    }
    let free: Vec<StationId> = g
        .view()
        .stations()
        .filter(|s| !s.connected)
        .map(|s| s.id)
        .collect();
    let head = Command::ExtendLine {
        line: LineId(0),
        end: LineEnd::Head,
        station: free[0],
        bend: Bend::StraightFirst,
    };
    let second = common::create_line(&g.view(), vec![free[1], StationId(1)]);
    step(&mut g, vec![head, second], 2200);
    g
}

#[test]
fn snapshot_ron_golden() {
    let bal = common::rich();
    let g = mixed();
    let snap = g.snapshot();
    assert_eq!(snap.format, SNAPSHOT_FORMAT_VERSION);
    assert!(snap.lines.len() >= 2 && snap.waiting.data.iter().any(|&n| n > 0));
    assert!(
        snap.lines
            .iter()
            .any(|l| l.bends.contains(&Bend::DiagonalFirst))
    );
    assert!(snap.trains.iter().any(|t| t.phase == SnapPhase::Running));
    assert!(
        snap.trains
            .iter()
            .any(|t| matches!(t.phase, SnapPhase::Dwelling { .. }))
    );
    assert!(g.view().lines().any(|l| l.stops.contains(&false)), "通過駅");
    let text = to_text(&snap);
    let path = golden_path();
    if std::env::var_os("TM_UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        std::fs::write(&path, &text).expect("ゴールデンを書く");
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{path:?} がない。TM_UPDATE_GOLDEN=1 で作る"));
    assert_eq!(text, want);
    let read: GameSnapshot = ron::from_str(&want).expect("ゴールデンを読める");
    let r = Game::restore(bal, &read).expect("ゴールデンから作り直せる");
    assert_eq!(r.state_hash(), g.state_hash());
}

/// 前の版の写しは断る。panic しない。M7(`SNAPSHOT_FORMAT_VERSION` 1)は `born_level` がなく RON として読めない。
/// M8〜M10(形式 2、`RULES_VERSION` 3・4)は読めるが `Format` で断る。
#[test]
fn previous_format_snapshot_is_refused() {
    let read = |name: &str| {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name);
        std::fs::read_to_string(path).expect("前の版のゴールデン")
    };
    let err = ron::from_str::<GameSnapshot>(&read("snapshot_rules2.ron")).expect_err("形式 1");
    assert!(err.to_string().contains("born_level"), "{err}");
    for name in ["snapshot_rules3.ron", "snapshot_rules4.ron"] {
        let s: GameSnapshot = ron::from_str(&read(name)).expect("形式 2 は読める");
        let r = Game::restore(std::sync::Arc::new(tm_core::BalanceTable::builtin()), &s);
        assert_eq!(
            r.map(|_| ()),
            Err(tm_core::SnapshotError::Format { found: 2 }),
            "{name}"
        );
    }
}
