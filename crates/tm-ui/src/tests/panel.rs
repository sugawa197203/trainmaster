//! 強化パネル(13 §3.8、§10.2): 開き方、タブ、対象の並び、購入、表示、カメラ。

use tm_assets::sound::{SE_CANCEL, SE_CANNOT_BUY, SE_CONFIRM, SE_CURSOR, SE_ERROR};
use tm_assets::txt;
use tm_core::{
    BuyMode, Command, GridPos, LineId, ParamCategory, ParamKey, StationId, UpgradeScope,
};
use tm_input::Action;
use tm_tutorial::{MenuAnchor, UiAnchor, UiEvent};

use super::screens::{notice, play, ses, wait};
use super::{cheap_game, create_line, fill_at, harness_with_line, map_tiles, plan, sprite_at};
use crate::layout::{HUD, MAP_H, MAP_W, MAP_W_PANEL, MAP_Y, PANEL, TILE};
use crate::scene::{PanelState, PanelTab, PlayMode};
use crate::testing::UiHarness;

fn panel(h: &UiHarness) -> &PanelState {
    match &play(h).mode {
        PlayMode::Panel(p) => p,
        _ => panic!("パネルが開いていない"),
    }
}

fn is_panel(h: &UiHarness) -> bool {
    matches!(play(h).mode, PlayMode::Panel(_))
}

/// 安い表で、すべてのパラメータを会社レベル 0 から使えるようにしたゲームに路線を 1 本作った画面。
fn unlocked_with_line() -> UiHarness {
    let mut spec = tm_core::BalanceTable::builtin().spec().clone();
    for p in &mut spec.params {
        p.base_yen = 1;
        p.cost_ratio = 1;
        p.unlock = 0;
    }
    spec.economy.initial_money_yen = 1_000_000_000_000;
    let table = tm_core::BalanceTable::from_spec(spec).expect("検証を通る表");
    let game = tm_core::Game::with_station_plan(
        tm_core::GameConfig {
            seed: 3,
            balance: std::sync::Arc::new(table),
        },
        plan(),
    );
    let mut h = UiHarness::from_game(game, false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    h
}

/// 安い表で最初の 3 駅の路線を作った画面。
fn cheap_with_line() -> UiHarness {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    h
}

fn last_levelup(h: &UiHarness) -> (ParamKey, UpgradeScope, BuyMode) {
    match h.commands.last() {
        Some(Command::LevelUp { param, scope, mode }) => (*param, *scope, *mode),
        c => panic!("LevelUp がない: {c:?}"),
    }
}

#[test]
fn confirm_on_a_connected_station_opens_the_station_tab() {
    let mut h = harness_with_line();
    h.move_cursor_to(plan()[1]);
    wait(&mut h, 400);
    let (mark, ev) = (h.audio.len(), h.ui_events.len());
    h.press(Action::Confirm);
    let p = panel(&h);
    assert_eq!(p.tab, PanelTab::Param(ParamCategory::Station));
    assert_eq!(p.target_station, None, "対象の初期値は ぜんぶ");
    assert_eq!(p.item_index(), 0);
    assert_eq!(ses(&h, mark), [SE_CONFIRM]);
    assert_eq!(
        h.ui_events[ev..],
        [
            UiEvent::MenuOpened(MenuAnchor::UpgradePanel),
            UiEvent::MenuOpened(MenuAnchor::UpgradeTab(ParamCategory::Station))
        ]
    );
    h.press(Action::MoveRight);
    assert_eq!(
        panel(&h).target_station,
        Some(StationId(1)),
        "ぜんぶ の次は開いた駅"
    );
}

#[test]
fn confirm_on_an_unconnected_station_refuses() {
    let mut h = UiHarness::with_game(plan(), false);
    h.frames(1);
    h.move_cursor_to(plan()[0]);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert!(matches!(play(&h).mode, PlayMode::Map));
    assert_eq!(ses(&h, mark), [SE_ERROR]);
    assert_eq!(notice(&h), Some(txt::ERR_NOT_CONNECTED));
    h.move_cursor_to(GridPos { x: 20, y: 15 });
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert!(matches!(play(&h).mode, PlayMode::Map), "何もないマス");
    assert!(ses(&h, mark).is_empty());
}

#[test]
fn confirm_on_track_opens_the_train_tab_for_that_line() {
    let mut h = harness_with_line();
    h.move_cursor_to(GridPos { x: 8, y: 4 });
    h.press(Action::Confirm);
    assert_eq!(panel(&h).tab, PanelTab::Param(ParamCategory::Train));
    h.press(Action::MoveRight);
    assert_eq!(panel(&h).target_line, Some(LineId(0)));
    assert_eq!(
        panel(&h).scope(),
        UpgradeScope::Line(LineId(0)),
        "電車のパラメータも路線ごと"
    );
}

#[test]
fn tab_opens_the_last_tab_and_tabs_wrap() {
    let mut h = harness_with_line();
    h.press(Action::TabNext);
    assert_eq!(
        panel(&h).tab,
        PanelTab::Param(ParamCategory::Station),
        "初回は えき"
    );
    wait(&mut h, 400);
    let (mark, ev) = (h.audio.len(), h.ui_events.len());
    h.press(Action::TabPrev);
    assert_eq!(panel(&h).tab, PanelTab::Rebirth, "左の端で折り返す");
    assert_eq!(ses(&h, mark), [SE_CURSOR]);
    assert_eq!(
        h.ui_events[ev..],
        [
            UiEvent::MenuClosed(MenuAnchor::UpgradeTab(ParamCategory::Station)),
            UiEvent::MenuOpened(MenuAnchor::UpgradeRebirth)
        ]
    );
    h.press(Action::TabNext);
    assert_eq!(
        panel(&h).tab,
        PanelTab::Param(ParamCategory::Station),
        "右の端で折り返す"
    );
    h.press(Action::TabNext);
    assert_eq!(panel(&h).tab, PanelTab::Param(ParamCategory::Train));
    wait(&mut h, 400);
    let (mark, ev) = (h.audio.len(), h.ui_events.len());
    h.press(Action::Cancel);
    assert!(matches!(play(&h).mode, PlayMode::Map));
    assert_eq!(ses(&h, mark), [SE_CANCEL]);
    assert_eq!(
        h.ui_events[ev..],
        [
            UiEvent::MenuClosed(MenuAnchor::UpgradeTab(ParamCategory::Train)),
            UiEvent::MenuClosed(MenuAnchor::UpgradePanel)
        ]
    );
    h.press(Action::TabPrev);
    assert_eq!(
        panel(&h).tab,
        PanelTab::Param(ParamCategory::Train),
        "前回のタブ"
    );
}

#[test]
fn items_stop_at_the_ends_and_follow_the_table() {
    let mut h = cheap_with_line();
    h.press(Action::TabNext);
    h.press(Action::TabNext);
    assert_eq!(panel(&h).tab, PanelTab::Param(ParamCategory::Train));
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::MoveUp);
    assert_eq!(panel(&h).item_index(), 0);
    assert!(ses(&h, mark).is_empty(), "端で止まる(折り返さない)");
    let mut got = vec![];
    for _ in 0..6 {
        got.push(panel(&h).param());
        h.press(Action::MoveDown);
    }
    assert_eq!(
        got,
        [
            ParamKey::T1Speed,
            ParamKey::T2Crowding,
            ParamKey::T3Boarding,
            ParamKey::T4CarCapacity,
            ParamKey::T5Cars,
            ParamKey::T5Cars
        ]
    );
    assert!(
        h.ui_events
            .contains(&UiEvent::CursorOn(UiAnchor::Menu(MenuAnchor::UpgradeItem(
                ParamKey::T5Cars
            ))))
    );
}

#[test]
fn targets_go_all_origin_then_ids_and_wrap() {
    let mut h = harness_with_line();
    h.move_cursor_to(plan()[1]);
    h.press(Action::Confirm);
    let mut seen = vec![];
    for _ in 0..5 {
        h.press(Action::MoveRight);
        seen.push(panel(&h).target_station);
    }
    assert_eq!(
        seen,
        [
            Some(StationId(1)),
            Some(StationId(2)),
            Some(StationId(0)),
            None,
            Some(StationId(1))
        ]
    );
    h.press(Action::MoveLeft);
    h.press(Action::MoveLeft);
    assert_eq!(panel(&h).target_station, Some(StationId(0)), "左は逆の順");
    assert!(
        h.ui_events
            .contains(&UiEvent::CursorOn(UiAnchor::Menu(MenuAnchor::ScopeToggle)))
    );
}

#[test]
fn r2_is_company_wide_and_ignores_left_right() {
    let mut h = unlocked_with_line();
    h.press(Action::TabNext);
    h.press(Action::TabPrev);
    h.press(Action::TabPrev);
    assert_eq!(panel(&h).tab, PanelTab::Param(ParamCategory::Line));
    h.press(Action::MoveRight);
    assert_eq!(panel(&h).target_line, Some(LineId(0)));
    h.press(Action::MoveDown);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::MoveRight);
    assert!(ses(&h, mark).is_empty(), "R2 では左右が効かない");
    assert_eq!(panel(&h).target_line, Some(LineId(0)));
    h.press(Action::Confirm);
    assert_eq!(
        last_levelup(&h),
        (ParamKey::R2LineSlots, UpgradeScope::Company, BuyMode::One)
    );
    h.press(Action::MoveUp);
    h.press(Action::BuyMax);
    assert_eq!(
        last_levelup(&h),
        (
            ParamKey::R1MaxTrains,
            UpgradeScope::Line(LineId(0)),
            BuyMode::Max
        )
    );
}

#[test]
fn buying_sends_levelup_with_the_chosen_scope() {
    let mut h = cheap_with_line();
    h.move_cursor_to(plan()[2]);
    h.press(Action::Confirm);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert_eq!(
        last_levelup(&h),
        (ParamKey::S1Spawn, UpgradeScope::AllStations, BuyMode::One)
    );
    assert!(
        !ses(&h, mark).contains(&SE_CONFIRM),
        "購入の決定では se_confirm を鳴らさない"
    );
    h.press(Action::MoveRight);
    h.press(Action::MoveDown);
    h.press(Action::BuyMax);
    assert_eq!(
        last_levelup(&h),
        (
            ParamKey::S2Fare,
            UpgradeScope::Station(StationId(2)),
            BuyMode::Max
        )
    );
}

#[test]
fn holding_confirm_buys_every_200ms() {
    let mut h = cheap_with_line();
    h.press(Action::TabNext);
    h.hold(Action::Confirm, 190);
    assert_eq!(h.commands.len(), 1, "200ms までは 1 回");
    h.hold(Action::Confirm, 600);
    assert_eq!(h.commands.len(), 1 + 3, "押して 1 回、200ms ごとに 1 回");
    assert!(h.commands.iter().all(|c| matches!(
        c,
        Command::LevelUp {
            mode: BuyMode::One,
            ..
        }
    )));
}

#[test]
fn failed_purchase_stops_the_repeat_until_released() {
    let mut h = harness_with_line();
    h.press(Action::TabNext);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.hold(Action::Confirm, 1000);
    assert!(h.commands.is_empty());
    assert_eq!(ses(&h, mark), [SE_CANNOT_BUY], "失敗の音は 1 回だけ");
    assert_eq!(notice(&h), Some(txt::ERR_MONEY));
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert_eq!(ses(&h, mark), [SE_CANNOT_BUY], "離せばまた試せる");
}

#[test]
fn jump_recommend_opens_the_recommended_item() {
    let mut h = cheap_with_line();
    h.frames(40);
    let rec = h
        .game
        .as_ref()
        .unwrap()
        .view()
        .recommendation()
        .expect("おすすめ");
    h.press(Action::JumpRecommend);
    let p = panel(&h);
    assert_eq!(p.param(), rec.param);
    assert_eq!(p.scope(), rec.scope);
    let i = p
        .row_states()
        .iter()
        .position(|r| r.0 == rec.param)
        .unwrap();
    assert_eq!(p.row_states()[i].3, Some(rec.affordable), "星と札");
    h.press(Action::TabNext);
    h.press(Action::MoveRight);
    h.press(Action::JumpRecommend);
    assert_eq!(panel(&h).param(), rec.param, "パネルの中でも おすすめへ");
    assert_eq!(panel(&h).scope(), rec.scope, "対象は ぜんぶ に戻す");
}

#[test]
fn rows_show_validate_results() {
    let mut h = harness_with_line();
    h.press(Action::TabNext);
    h.frames(7);
    let p = panel(&h);
    assert_eq!(
        p.row_note(2),
        Some(txt::PANEL_LOCKED),
        "S3 は会社レベル 1 から"
    );
    assert_eq!(p.foot_text(), Some(txt::PANEL_NEED_MORE), "お金が足りない");
    assert!(p.row_states()[0].1, "灰色");
    h.press(Action::MoveDown);
    h.press(Action::MoveDown);
    assert_eq!(
        panel(&h).foot_text(),
        Some(txt::PARAM_S3_DESC),
        "買えない理由がお金でなければ説明"
    );
    let mut h = UiHarness::with_game(plan(), false);
    h.press(Action::TabNext);
    assert_eq!(panel(&h).row_note(0), Some(txt::PANEL_NO_STATION));
    h.press(Action::TabNext);
    assert_eq!(panel(&h).row_note(0), Some(txt::PANEL_NO_LINE));
}

#[test]
fn level_is_a_range_for_all_and_single_for_one() {
    let mut h = cheap_with_line();
    let g = h.game.as_mut().unwrap();
    g.enqueue(Command::LevelUp {
        param: ParamKey::S2Fare,
        scope: UpgradeScope::Station(StationId(0)),
        mode: BuyMode::One,
    });
    h.frames(2);
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    h.frames(7);
    assert_eq!(panel(&h).row_level(1), Some(txt::PANEL_LEVEL_RANGE));
    h.press(Action::MoveRight);
    assert_eq!(panel(&h).target_station, Some(StationId(0)));
    assert_eq!(panel(&h).row_level(1), Some(txt::PANEL_LEVEL));
}

#[test]
fn build_mode_closes_the_panel_and_starts_build() {
    let mut h = harness_with_line();
    h.press(Action::TabNext);
    let ev = h.ui_events.len();
    h.press(Action::BuildMode);
    assert!(matches!(play(&h).mode, PlayMode::Build(_)));
    assert_eq!(
        h.ui_events[ev..],
        [
            UiEvent::MenuClosed(MenuAnchor::UpgradeTab(ParamCategory::Station)),
            UiEvent::MenuClosed(MenuAnchor::UpgradePanel),
            UiEvent::MenuOpened(MenuAnchor::BuildMode)
        ]
    );
}

#[test]
fn camera_keeps_the_focus_in_view_with_and_without_the_panel() {
    use crate::layout::{LAYER_W, WORLD_H, WORLD_W};
    let mut h = harness_with_line();
    let col = map_tiles(&h).0 - 1;
    h.move_cursor_to(GridPos { x: col, y: 8 });
    h.press(Action::TabNext);
    wait(&mut h, 500);
    let cam = play(&h).camera.pos;
    assert_eq!(
        cam.x,
        ((col + 1) * TILE + 2 * TILE - MAP_W_PANEL).clamp(0, WORLD_W - MAP_W_PANEL),
        "右端のマスを左右 2 マスの内側に入れる(端なら端で止まる)"
    );
    let left = col * TILE - cam.x;
    let top = 8 * TILE - cam.y;
    assert!(
        left >= 0 && left + TILE <= MAP_W_PANEL && top >= 0 && top + TILE <= MAP_H,
        "パネルを開いてもカーソルが表示域に入る"
    );
    let text = h.draw_text();
    let at = format!(" {},{} {LAYER_W}x{WORLD_H}", LAYER_W - cam.x, MAP_Y - cam.y);
    assert!(
        text.lines()
            .any(|l| l.starts_with("BeginLayer 1 ") && l.ends_with(&at)),
        "右の層は (640 − cam.x, 32 − cam.y) に置く"
    );
    assert!(
        !text.contains("BeginLayer 0 "),
        "表示域にかからない左の層は出さない"
    );
    h.press(Action::TabNext);
    h.press(Action::MoveRight);
    assert_eq!(panel(&h).target_line, Some(LineId(0)));
    wait(&mut h, 500);
    let cam = play(&h).camera.pos;
    let view = h.game.as_ref().unwrap().view();
    let f = panel(&h)
        .focus(view, &play(&h).map.as_ref().unwrap().paths)
        .unwrap();
    let (cx, cy) = (f.x * TILE + TILE / 2, f.y * TILE + TILE / 2);
    assert_eq!(
        (cam.x, cam.y),
        (
            (cx - MAP_W_PANEL / 2).clamp(0, WORLD_W - MAP_W_PANEL),
            (cy - MAP_H / 2).clamp(0, WORLD_H - MAP_H)
        ),
        "対象の路線を中央へ(端なら端で止まる)"
    );
    assert!((0..MAP_W_PANEL).contains(&(cx - cam.x)));
    h.press(Action::Cancel);
    assert!(!is_panel(&h));
    assert!(
        play(&h).camera.pos.x <= WORLD_W - MAP_W,
        "閉じて表示域が広がっても、マップの外を見せない"
    );
    wait(&mut h, 500);
    let cam = play(&h).camera.pos;
    let left = col * TILE - cam.x;
    assert!(
        left >= 0 && left + TILE <= MAP_W,
        "閉じるとカーソルに戻る: {cam:?}"
    );
}

#[test]
fn rejected_purchase_shows_the_reason() {
    let mut h = harness_with_line();
    let now = h.now;
    let one = tm_numeric::BigNum::from_u64(1);
    h.ui.observe(
        now,
        &tm_core::GameEvent::CommandRejected {
            kind: tm_core::CommandKind::LevelUp {
                param: ParamKey::S1Spawn,
                mode: BuyMode::One,
            },
            reason: tm_core::CommandError::InsufficientFunds {
                required: one,
                available: tm_numeric::BigNum::ZERO,
            },
        },
        &mut Vec::new(),
    );
    assert_eq!(notice(&h), Some(txt::ERR_MONEY));
}

fn levelup_lines(h: &UiHarness) -> Vec<String> {
    h.draw_text()
        .lines()
        .filter(|l| l.starts_with("Sprite 'レ'12") && l.ends_with("#FFE066FF"))
        .map(str::to_string)
        .collect()
}

#[test]
fn purchase_lights_the_row_with_levelup_for_600ms() {
    let mut h = cheap_with_line();
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    wait(&mut h, 400);
    assert!(levelup_lines(&h).is_empty());
    h.key_down(Action::Confirm);
    h.frames(1);
    h.key_up(Action::Confirm);
    h.frames(1);
    let lines = levelup_lines(&h);
    assert_eq!(lines.len(), 1, "買った行に「レベルアップ!」");
    let y: i32 = lines[0]
        .split(' ')
        .nth(2)
        .unwrap()
        .split(',')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let row0 = panel(&h)
        .part_rect(MenuAnchor::UpgradeItem(ParamKey::S1Spawn))
        .unwrap();
    assert!(
        (row0.y + row0.h as i32 / 2..row0.y + row0.h as i32).contains(&y),
        "1 行目(おきゃくさんの 数)の 2 段目: {y}"
    );
    let text = h.draw_text();
    assert!(
        text.lines()
            .any(|l| l.starts_with(&format!("{}#FFD400", fill_at(row0)))),
        "行を光らせる"
    );
    let panel_part: String = text
        .lines()
        .skip_while(|l| !l.starts_with(&sprite_at("ui_win_tl", PANEL.x, PANEL.y)))
        .take_while(|l| !l.starts_with(&fill_at(HUD)))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("panel_levelup", panel_part);
    wait(&mut h, 560);
    assert_eq!(levelup_lines(&h).len(), 1, "600ms の間は出す");
    wait(&mut h, 40);
    assert!(levelup_lines(&h).is_empty(), "600ms で消える");
}
