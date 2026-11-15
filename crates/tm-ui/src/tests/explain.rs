//! 説明の箱、止めない案内、highlight、`CursorOn` の送り直し、`se_popup`、説明中の入力(13 §3.4、§3.13、§10.2)。

use tm_assets::sound::{SE_CONFIRM, SE_MENU_CLOSE, SE_MENU_OPEN, SE_POPUP};
use tm_assets::txt;
use tm_core::{Command, GridPos, LineId, ParamCategory, ParamKey, StationId};
use tm_input::Action;
use tm_platform_api::{MonoTime, Rect};
use tm_tutorial::{
    DirectiveKind, MenuAnchor, SettingsItem, TutorialDirective, TutorialStageId, UiAnchor, UiEvent,
};

use super::screens::{notice, play, play_mut, ses, wait};
use super::{cheap_game, create_line, fill_at, harness_with_line, plan, sprite_at};
use crate::anchor::{Mark, Toward};
use crate::layout::{
    BAND_LEFT, BAND_ROW2, BOX_BOTTOM, BOX_TOP_Y, Cam, EXPLAIN_X, HUD_ROW1, MAP_SCALE, SETTINGS,
    TILE, tile_screen,
};
use crate::scene::{PanelTab, PlayMode};
use crate::testing::UiHarness;

/// 説明つきのゲーム(駅は `plan()` の順)。#1 の説明が出ている。
fn tutorial_game() -> UiHarness {
    let mut h = UiHarness::with_game(plan(), true);
    h.frames(1);
    h
}

fn directive(h: &UiHarness) -> Option<TutorialDirective> {
    h.tutorial.as_ref().and_then(|t| t.directive().copied())
}

fn stage_page(h: &UiHarness) -> Option<(u8, DirectiveKind, u8)> {
    directive(h).map(|d| (d.stage.0, d.kind, d.page))
}

fn mark(h: &UiHarness) -> Option<Mark> {
    play(h).explain.shown.and_then(|s| s.mark)
}

fn frame_rect(h: &UiHarness) -> Rect {
    match mark(h) {
        Some(Mark::Frame(r) | Mark::Tile(r)) => r,
        m => panic!("枠でない: {m:?}"),
    }
}

/// ガードを待って決定で 1 ページ読む。
fn read_page(h: &mut UiHarness) {
    wait(h, 320);
    h.press(Action::Confirm);
}

/// #15 のページ数(p3 は「ゲームを終わる」の行を指す)。
fn stage_15_pages() -> u8 {
    tm_tutorial::stage_pages(TutorialStageId(15)).len() as u8
}

fn count(h: &UiHarness, from: usize, ev: UiEvent) -> usize {
    h.ui_events[from..].iter().filter(|e| **e == ev).count()
}

pub(super) fn fake(
    stage: u8,
    kind: DirectiveKind,
    highlight: Option<UiAnchor>,
) -> TutorialDirective {
    TutorialDirective {
        stage: TutorialStageId(stage),
        kind,
        message: txt::TUT_S03_G1,
        page: 0,
        page_count: 1,
        highlight,
        pauses_game: kind == DirectiveKind::Explanation,
    }
}

fn popups(h: &UiHarness, from: usize) -> usize {
    ses(h, from).iter().filter(|&&s| s == SE_POPUP).count()
}

const DIM: &str = "#00000080";

/// マス `pos` を(16px のマスの 3px の 2 倍の)6px 広げた highlight の枠(カメラ `cam`)。
fn ring(pos: GridPos, cam: Cam) -> Rect {
    let (x, y) = tile_screen(pos, cam);
    let r = 3 * MAP_SCALE;
    Rect {
        x: x - r,
        y: y - r,
        w: (TILE + 2 * r) as u32,
        h: (TILE + 2 * r) as u32,
    }
}

/// `from` を含む最初の行から最後まで(層 7・8 などのスナップショット用)。
fn tail_from(text: &str, from: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let i = lines
        .iter()
        .position(|l| l.contains(from))
        .expect("行がある");
    lines[i..].join("\n")
}

#[test]
fn first_explanation_draws_box_dim_and_pages() {
    let mut h = tutorial_game();
    assert_eq!(stage_page(&h), Some((1, DirectiveKind::Explanation, 0)));
    assert!(h.ui.is_paused(h.tutorial.as_ref()));
    let text = h.draw_text();
    let a = play(&h).explain.shown.unwrap().area;
    assert_eq!((a.x, a.y + a.h as i32), (EXPLAIN_X, BOX_BOTTOM), "下に出す");
    assert!(text.contains(&sprite_at("ui_popup_tl", a.x, a.y)));
    assert!(
        text.contains(&format!("Fill 0,0 640x360 {DIM}")),
        "highlight がなければ全体に 1 まい"
    );
    assert_eq!(text.matches(DIM).count(), 1);
    assert!(text.contains("'社'16"), "本文は Head16");
    assert!(text.contains("'1'12") && text.contains("'2'12"), "1/2");
    assert!(
        text.contains("'へ'12")
            && text
                .lines()
                .any(|l| l.starts_with("Sprite 'へ'12") && l.ends_with("#A6AEC0FF")),
        "ガードの間は「つぎへ」を灰色にする"
    );
    assert!(
        text.lines()
            .any(|l| l.starts_with("Sprite btn_key_z") && l.ends_with("#A6AEC0FF")),
        "ボタンの絵も灰色"
    );
    assert_eq!(popups(&h, 0), 1, "見せ始めに se_popup");
    insta::assert_snapshot!("explain_box", tail_from(&text, DIM));
    let mark = h.ui_events.len();
    h.press(Action::Confirm);
    assert_eq!(
        count(&h, mark, UiEvent::MessageAcknowledged),
        0,
        "300ms の間は読まない"
    );
    wait(&mut h, 300);
    assert!(
        h.draw_text()
            .lines()
            .any(|l| l.starts_with("Sprite 'へ'12") && l.ends_with("#FFFFFFFF")),
        "読めるようになったら白"
    );
    assert!(
        h.draw_text()
            .lines()
            .any(|l| l.starts_with("Sprite btn_key_z") && l.ends_with("#FFFFFFFF"))
    );
    h.press(Action::Confirm);
    assert_eq!(count(&h, mark, UiEvent::MessageAcknowledged), 1);
    assert_eq!(stage_page(&h), Some((1, DirectiveKind::Explanation, 1)));
    let a = h.audio.len();
    read_page(&mut h);
    assert_eq!(stage_page(&h), Some((2, DirectiveKind::Explanation, 0)));
    assert_eq!(
        ses(&h, a),
        [SE_CONFIRM],
        "決定と同じフレームでは se_popup を鳴らさない"
    );
    assert!(
        matches!(
            play(&h).explain.shown.and_then(|s| s.mark),
            Some(Mark::Tile(_))
        ),
        "始めのカメラが始めの 3 駅を見せているので、最初の駅をすぐ枠で指す"
    );
    h.frames(30);
    assert_eq!(
        frame_rect(&h),
        ring(plan()[0], play(&h).camera.pos),
        "#2 は最初の駅"
    );
    let (cx, cy) = tile_screen(h.ui.map_cursor().unwrap(), play(&h).camera.pos);
    assert!(
        cx >= 0 && cx + TILE <= 640 && cy >= 32 && cy + TILE <= 320,
        "カーソルも見えたまま"
    );
    let text = h.draw_text();
    let dim = |kind: &str| {
        text.lines()
            .filter(|l| l.starts_with(kind) && l.contains(DIM))
            .count()
    };
    assert_eq!(dim("Fill "), 4, "highlight の場所は重ねない");
    assert_eq!(dim("Sprite ui_hole_"), 4, "枠の丸い角の外も暗くする");
    assert_eq!(text.matches(DIM).count(), 8);
    let r = frame_rect(&h);
    let arrow = if r.y - 24 < crate::layout::MAP_Y {
        "Sprite fx_arrow_up "
    } else {
        "Sprite fx_arrow_down "
    };
    assert!(text.contains(arrow), "表示域の上に余地がなければ下から指す");
    assert!(
        text.lines()
            .any(|l| l.starts_with(arrow) && l.contains(" 22x16 ")),
        "マップの印の矢印は 2 倍"
    );
}

#[test]
fn cursor_already_on_a_station_is_resent_and_finishes_stage_2() {
    let mut h = tutorial_game();
    read_page(&mut h);
    read_page(&mut h);
    assert_eq!(stage_page(&h), Some((2, DirectiveKind::Explanation, 0)));
    let mark = h.ui_events.len();
    h.move_cursor_to(plan()[0]);
    assert_eq!(
        count(&h, mark, UiEvent::CursorOn(UiAnchor::Station(StationId(0)))),
        1,
        "止める説明の間も Move でカーソルが動き CursorOn が出る"
    );
    assert_eq!(stage_page(&h), Some((2, DirectiveKind::Explanation, 0)));
    read_page(&mut h);
    assert_eq!(
        count(&h, mark, UiEvent::CursorOn(UiAnchor::Station(StationId(0)))),
        3,
        "directive が変わるたび(案内へ、#3 の説明へ)に送り直す"
    );
    assert_eq!(
        stage_page(&h),
        Some((3, DirectiveKind::Explanation, 0)),
        "動かさなくても #2 が終わる"
    );
}

#[test]
fn map_cursor_reports_station_line_and_nothing() {
    let mut h = harness_with_line();
    let path = play(&h).map.as_ref().unwrap().paths[0].clone();
    let track: Vec<GridPos> = path
        .iter()
        .copied()
        .filter(|&t| h.game.as_ref().unwrap().view().station_at(t).is_none())
        .collect();
    h.move_cursor_to(plan()[0]);
    let mark = h.ui_events.len() - 1;
    h.move_cursor_to(track[0]);
    h.move_cursor_to(track[1]);
    assert_eq!(
        h.ui_events[mark..]
            .iter()
            .filter(|e| matches!(e, UiEvent::CursorOn(_)))
            .copied()
            .collect::<Vec<_>>(),
        [
            UiEvent::CursorOn(UiAnchor::Station(StationId(0))),
            UiEvent::CursorOn(UiAnchor::Line(LineId(0))),
        ],
        "同じ路線の上を動いても出さない"
    );
    let mark = h.ui_events.len();
    h.move_cursor_to(GridPos {
        x: track[1].x,
        y: 0,
    });
    assert_eq!(h.ui_events.len(), mark, "何もないマスでは出さない");
    h.move_cursor_to(track[0]);
    assert_eq!(
        h.ui_events[mark..],
        [UiEvent::CursorOn(UiAnchor::Line(LineId(0)))]
    );
}

#[test]
fn panel_item_is_resent_when_the_directive_changes() {
    let mut h = harness_with_line();
    h.press(Action::TabNext);
    h.press(Action::MoveDown);
    let mark = h.ui_events.len();
    h.frames(5);
    assert_eq!(h.ui_events.len(), mark, "変わらなければ送らない");
    play_mut(&mut h).fake_directive = Some(fake(6, DirectiveKind::Guide, None));
    h.frames(1);
    let mut d = fake(6, DirectiveKind::Guide, None);
    d.page = 0;
    d.kind = DirectiveKind::Explanation;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    let item = UiEvent::CursorOn(UiAnchor::Menu(MenuAnchor::UpgradeItem(ParamKey::S2Fare)));
    assert_eq!(
        h.ui_events[mark..],
        [item, item],
        "kind が変わっても送り直す"
    );
    d.page = 1;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    play_mut(&mut h).fake_directive = None;
    h.frames(1);
    assert_eq!(count(&h, mark, item), 4, "page と、なくなったときも");
}

#[test]
fn explanation_takes_only_confirm_moves_and_menu() {
    let mut h = tutorial_game();
    wait(&mut h, 400);
    let (ev, a) = (h.ui_events.len(), h.audio.len());
    for act in [
        Action::Cancel,
        Action::BuyMax,
        Action::TabNext,
        Action::TabPrev,
        Action::BuildMode,
        Action::ToggleBend,
        Action::JumpRecommend,
    ] {
        h.press(act);
    }
    assert!(matches!(play(&h).mode, PlayMode::Map));
    assert!(ses(&h, a).is_empty(), "音も出さない");
    assert_eq!(h.ui_events.len(), ev);
    assert_eq!(stage_page(&h), Some((1, DirectiveKind::Explanation, 0)));
    let tick = h.game.as_ref().unwrap().view().tick();
    h.move_cursor_to(plan()[1]);
    assert_eq!(h.ui.map_cursor(), Some(plan()[1]));
    assert!(h.ui_events[ev..].contains(&UiEvent::CursorOn(UiAnchor::Station(StationId(1)))));
    assert_eq!(h.game.as_ref().unwrap().view().tick(), tick, "止まっている");
    let a = h.audio.len();
    h.press(Action::Menu);
    assert!(play(&h).settings_open());
    assert_eq!(ses(&h, a), [SE_MENU_OPEN], "#15 は開いた音だけ");
    assert_eq!(stage_page(&h), Some((15, DirectiveKind::Explanation, 0)));
    assert!(h.commands.is_empty());
}

#[test]
fn settings_hide_the_main_explanation_and_show_stage_15_inside() {
    let mut h = tutorial_game();
    h.press(Action::Menu);
    let s = play(&h).explain.shown.expect("#15");
    assert!(s.boxed && !s.directive.pauses_game);
    let item = play(&h)
        .settings_menu
        .as_ref()
        .unwrap()
        .part_rect(MenuAnchor::SettingsItem(SettingsItem::SoundVolume));
    assert_eq!(mark(&h), item.map(Mark::Side), "音の 大きさ の行");
    let text = h.draw_text();
    assert_eq!(text.matches(DIM).count(), 1, "重なっても 1 まい");
    let menu_at = text.find("Sprite ui_win_tl").expect("せってい");
    let box_at = text.find("Sprite ui_popup_tl").expect("#15 の箱");
    assert!(menu_at < box_at, "せってい の後に描く");
    for page in 0..stage_15_pages() {
        let s = play(&h).explain.shown.unwrap();
        assert_eq!(s.directive.page, page);
        let bounds = play(&h).settings_menu.as_ref().unwrap().bounds();
        assert!(
            s.under_menu && !crate::anim::overlaps(s.area, bounds),
            "#15 の箱は せってい に重ねない: {:?}",
            s.area
        );
        assert_eq!(
            (s.area.x, s.area.y + s.area.h as i32),
            (EXPLAIN_X, BOX_BOTTOM),
            "せってい の下に固定"
        );
        read_page(&mut h);
    }
    assert!(
        play(&h).settings_open(),
        "読み終えても せってい は開いたまま"
    );
    let s = play(&h).explain.shown.expect("本線の説明");
    assert!(
        !s.boxed && s.mark.is_none(),
        "本線の説明は せってい の下に隠れる"
    );
    assert!(!h.draw_text().contains("ui_popup_tl"));
    let a = h.audio.len();
    h.press(Action::Menu);
    assert_eq!(ses(&h, a), [SE_MENU_CLOSE], "見えていた説明に戻るだけ");
    assert!(h.draw_text().contains("ui_popup_tl"));
}

#[test]
fn explanation_started_under_settings_plays_only_the_close_sound() {
    let mut h = tutorial_game();
    h.press(Action::Menu);
    for _ in 0..stage_15_pages() {
        read_page(&mut h);
    }
    let t = h.tutorial.as_mut().unwrap();
    t.observe_ui(&UiEvent::MessageAcknowledged);
    t.observe_ui(&UiEvent::MessageAcknowledged);
    assert_eq!(stage_page(&h), Some((2, DirectiveKind::Explanation, 0)));
    let a = h.audio.len();
    h.frames(5);
    assert_eq!(popups(&h, a), 0, "隠れている間は鳴らさない");
    h.press(Action::Menu);
    assert_eq!(ses(&h, a), [SE_MENU_CLOSE]);
    assert!(h.draw_text().contains("ui_popup_tl"));
}

#[test]
fn popup_rings_for_a_new_explanation_without_a_ui_sound() {
    let mut h = UiHarness::with_game(plan(), false);
    let mut d = fake(4, DirectiveKind::Guide, None);
    play_mut(&mut h).fake_directive = Some(d);
    wait(&mut h, 1500);
    let a = h.audio.len();
    d.kind = DirectiveKind::Explanation;
    d.page_count = 2;
    play_mut(&mut h).fake_directive = Some(d);
    wait(&mut h, 1500);
    assert_eq!(popups(&h, a), 1, "案内から説明へ");
    let a = h.audio.len();
    d.page = 1;
    play_mut(&mut h).fake_directive = Some(d);
    wait(&mut h, 1500);
    assert_eq!(popups(&h, a), 0, "次のページでは鳴らさない");
    d.page = 0;
    play_mut(&mut h).fake_directive = Some(d);
    wait(&mut h, 1500);
    assert_eq!(popups(&h, a), 1, "同じ段階でも 1 ページ目に戻ったら鳴らす");
    let a = h.audio.len();
    d.stage = TutorialStageId(5);
    play_mut(&mut h).fake_directive = Some(d);
    wait(&mut h, 1500);
    assert_eq!(popups(&h, a), 1, "段階が変わったら鳴らす");
    let a = h.audio.len();
    play_mut(&mut h).fake_directive = Some(fake(5, DirectiveKind::Guide, None));
    wait(&mut h, 1500);
    assert_eq!(popups(&h, a), 0, "説明から案内では鳴らさない");
}

#[test]
fn guide_sits_between_error_and_mode_text() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(1);
    h.press(Action::BuildMode);
    h.press(Action::Confirm);
    play_mut(&mut h).fake_directive = Some(fake(3, DirectiveKind::Guide, None));
    h.frames(1);
    let band = &play(&h).band;
    assert_eq!(band.guide_text(), Some(txt::TUT_S03_G1));
    insta::assert_snapshot!("guide_band", tail_from(&h.draw_text(), &fill_at(BAND_ROW2)));
    assert_eq!(band.mode_text(), Some(txt::BUILD_CREATE_FIRST));
    let text = h.draw_text();
    let guide_mark = sprite_at("ico_guide", BAND_LEFT.x, BAND_ROW2.y + 6);
    assert!(
        text.contains(&format!("{guide_mark}6x12 #FFD400FF")),
        "案内の印"
    );
    h.move_cursor_to(GridPos { x: 30, y: 16 });
    h.press(Action::Confirm);
    assert_eq!(notice(&h), Some(txt::ERR_NOT_STATION));
    let text = h.draw_text();
    assert!(text.contains("Sprite ico_cross"), "エラーがいちばん上");
    assert!(!text.contains(&guide_mark));
    wait(&mut h, 2100);
    assert!(h.draw_text().contains(&guide_mark), "2 秒で案内に戻る");
    play_mut(&mut h).fake_directive = None;
    h.frames(1);
    assert_eq!(play(&h).band.guide_text(), None);
}

#[test]
fn guide_in_create_marks_the_stations_that_can_be_picked() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(1);
    play_mut(&mut h).forbid = Some(
        |c: &Command| !matches!(c, Command::CreateLine { stations, .. } if stations.len() > 2),
    );
    play_mut(&mut h).fake_directive = Some(fake(3, DirectiveKind::Guide, None));
    h.press(Action::BuildMode);
    h.press(Action::Confirm);
    let first3 = [plan()[0], plan()[1], plan()[2]];
    assert_eq!(play(&h).explain.picks(), first3);
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    assert_eq!(
        play(&h).explain.picks(),
        [plan()[1], plan()[2]],
        "同じ駅は選べない"
    );
    h.move_cursor_to(plan()[1]);
    h.press(Action::Confirm);
    assert_eq!(
        play(&h).explain.picks(),
        [plan()[1]],
        "3 駅目は許されないので、仕上げの最後の駅だけ"
    );
    let mut d = fake(
        3,
        DirectiveKind::Guide,
        Some(UiAnchor::Station(StationId(2))),
    );
    d.message = txt::TUT_S03_G1;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    assert!(
        play(&h).explain.picks().is_empty(),
        "highlight があれば付けない"
    );
}

fn on_map(h: &UiHarness) -> &crate::hud::Band {
    &play(h).band
}

#[test]
fn anchors_fall_back_to_the_entrance_when_hidden() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    let set = |h: &mut UiHarness, a: UiAnchor| {
        play_mut(h).fake_directive = Some(fake(6, DirectiveKind::Guide, Some(a)));
        h.frames(1);
    };
    let item = UiAnchor::Menu(MenuAnchor::UpgradeItem(ParamKey::T1Speed));
    set(&mut h, item);
    let chip_panel = on_map(&h).chip_rect(false).unwrap();
    assert_eq!(frame_rect(&h), chip_panel, "閉じていればチップ");
    set(&mut h, UiAnchor::Menu(MenuAnchor::BuildMode));
    assert_eq!(frame_rect(&h), on_map(&h).chip_rect(true).unwrap());
    assert!(
        frame_rect(&h).x < chip_panel.x,
        "ろせん は レベルアップ の左"
    );
    set(&mut h, UiAnchor::MoneyDisplay);
    assert!(
        matches!(mark(&h), Some(Mark::Side(r)) if r.y == HUD_ROW1.y),
        "HUD は横に矢印"
    );
    set(&mut h, item);
    h.press(Action::TabNext);
    let panel = match &play(&h).mode {
        PlayMode::Panel(p) => p,
        _ => panic!("パネル"),
    };
    assert_eq!(panel.tab, PanelTab::Param(ParamCategory::Station));
    assert_eq!(
        frame_rect(&h),
        panel
            .part_rect(MenuAnchor::UpgradeTab(ParamCategory::Train))
            .unwrap(),
        "タブが違えばそのタブ"
    );
    h.press(Action::TabNext);
    let panel = match &play(&h).mode {
        PlayMode::Panel(p) => p,
        _ => panic!("パネル"),
    };
    let row = panel
        .part_rect(MenuAnchor::UpgradeItem(ParamKey::T1Speed))
        .unwrap();
    let scope = panel.part_rect(MenuAnchor::ScopeToggle).unwrap();
    let last = panel
        .part_rect(MenuAnchor::UpgradeItem(ParamKey::T5Cars))
        .unwrap();
    assert_eq!(frame_rect(&h), row);
    assert_eq!(
        (row.x, row.y),
        (scope.x, scope.y + scope.h as i32),
        "1 行目"
    );
    set(&mut h, UiAnchor::Menu(MenuAnchor::BuyMaxHint));
    let r = frame_rect(&h);
    assert!(
        r.y > last.y + last.h as i32 + 16
            && r.y + r.h as i32 <= crate::layout::PANEL.y + crate::layout::PANEL.h as i32
            && r.w > 16
            && r.w < 80,
        "下の欄の 2 行目の (Y)MAX だけ: {r:?}"
    );
    set(&mut h, UiAnchor::Station(StationId(1)));
    assert!(
        matches!(mark(&h), Some(Mark::Tile(_)) | Some(Mark::Edge { .. })),
        "パネルの下の駅"
    );
    h.frames(60);
    assert!(
        matches!(mark(&h), Some(Mark::Tile(_))),
        "カメラが動いて見える"
    );
    h.press(Action::Cancel);
    set(&mut h, UiAnchor::Menu(MenuAnchor::LineConfirm));
    assert_eq!(mark(&h), None, "確認が出ていなければ置かない");
    h.press(Action::BuildMode);
    set(&mut h, UiAnchor::Menu(MenuAnchor::BuildExtend));
    let r = frame_rect(&h);
    set(&mut h, UiAnchor::Menu(MenuAnchor::BuildCreate));
    let first = frame_rect(&h);
    assert_eq!(first.y + first.h as i32, r.y, "選択肢の 2 行目");
    set(&mut h, UiAnchor::Line(LineId(0)));
    assert!(matches!(
        mark(&h),
        Some(Mark::Line {
            line: LineId(0),
            ..
        })
    ));
}

#[test]
fn explanation_box_goes_up_when_the_highlight_is_in_the_lower_half() {
    let mut h = harness_with_line();
    let mut d = fake(
        5,
        DirectiveKind::Explanation,
        Some(UiAnchor::Station(StationId(2))),
    );
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    assert!(
        (play(&h).explain.shown.unwrap().area.y == BOX_TOP_Y),
        "y = 10 のマスは下半分"
    );
    assert!(
        h.draw_text()
            .contains(&sprite_at("ui_popup_tl", EXPLAIN_X, BOX_TOP_Y))
    );
    d.highlight = Some(UiAnchor::Station(StationId(0)));
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    assert!(play(&h).explain.shown.unwrap().area.y != BOX_TOP_Y);
    d.highlight = Some(UiAnchor::Menu(MenuAnchor::BuildMode));
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    assert!(
        (play(&h).explain.shown.unwrap().area.y == BOX_TOP_Y),
        "下の帯のチップ"
    );
}

#[test]
fn refused_command_makes_the_highlight_twice_as_bright() {
    use crate::scene::explain::mark_color;
    for ms in (0..1000).step_by(25) {
        let t = MonoTime(ms * 1000);
        let (n, l) = (mark_color(t, false).a, mark_color(t, true).a);
        assert_eq!(u32::from(l), (2 * u32::from(n)).min(255));
    }
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    play_mut(&mut h).forbid = Some(|c: &Command| !matches!(c, Command::LevelUp { .. }));
    play_mut(&mut h).fake_directive = Some(fake(
        6,
        DirectiveKind::Guide,
        Some(UiAnchor::Menu(MenuAnchor::UpgradePanel)),
    ));
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    assert!(!play(&h).explain.shown.unwrap().loud);
    wait(&mut h, 400);
    h.press(Action::Confirm);
    assert_eq!(notice(&h), Some(txt::ERR_TUTORIAL));
    assert!(play(&h).explain.shown.unwrap().loud);
    wait(&mut h, 2100);
    assert!(!play(&h).explain.shown.unwrap().loud, "2 秒で戻る");
}

#[test]
#[ignore = "目で確かめるための画像を書く。TM_UI_DUMP_DIR がなければ何もしない"]
fn dump_explain_screens() {
    use super::raster::{rasterize, write_ppm};
    let Ok(dir) = std::env::var("TM_UI_DUMP_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    let mut shots: Vec<(&str, UiHarness)> = Vec::new();
    let mut h = tutorial_game();
    h.frames(30);
    shots.push(("explain_s01", h));
    let mut h = tutorial_game();
    read_page(&mut h);
    read_page(&mut h);
    h.frames(10);
    shots.push(("explain_s02_station", h));
    let mut h = tutorial_game();
    h.press(Action::Menu);
    h.frames(20);
    shots.push(("explain_s15_settings", h));
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(1);
    play_mut(&mut h).fake_directive = Some({
        let mut d = fake(
            3,
            DirectiveKind::Guide,
            Some(UiAnchor::Menu(MenuAnchor::BuildMode)),
        );
        d.message = txt::TUT_S03_G1;
        d
    });
    h.frames(10);
    shots.push(("guide_build_chip", h));
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(1);
    play_mut(&mut h).fake_directive = Some(fake(3, DirectiveKind::Guide, None));
    h.press(Action::BuildMode);
    h.press(Action::Confirm);
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    h.frames(10);
    shots.push(("guide_create_picks", h));
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    h.press(Action::TabNext);
    let mut d = fake(
        6,
        DirectiveKind::Explanation,
        Some(UiAnchor::Menu(MenuAnchor::UpgradeItem(ParamKey::S2Fare))),
    );
    d.message = txt::TUT_S06_P1;
    d.page_count = 2;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(30);
    shots.push(("explain_s06_panel", h));
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    let mut d = fake(4, DirectiveKind::Explanation, Some(UiAnchor::MoneyDisplay));
    d.message = txt::TUT_S04_P1;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(30);
    shots.push(("explain_s04_money", h));
    let mut h = harness_with_line();
    let mut d = fake(8, DirectiveKind::Explanation, Some(UiAnchor::CompanyGauge));
    d.message = txt::TUT_S08_P1;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(30);
    shots.push(("explain_s08_gauge", h));
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(1);
    let mut d = fake(14, DirectiveKind::Explanation, None);
    d.message = txt::TUT_S14_P1;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(30);
    shots.push(("explain_s14_longest", h));
    let mut h = harness_with_line();
    h.press(Action::BuildMode);
    h.press(Action::MoveDown);
    h.press(Action::Confirm);
    play_mut(&mut h).fake_directive = Some(fake(
        9,
        DirectiveKind::Guide,
        Some(UiAnchor::Menu(MenuAnchor::UpgradePanel)),
    ));
    h.frames(10);
    shots.push(("guide_pick_end_exit", h));
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    wait(&mut h, 400);
    h.press(Action::Confirm);
    h.frames(6);
    shots.push(("panel_levelup", h));
    let mut h = tutorial_game();
    h.press(Action::Menu);
    read_page(&mut h);
    h.frames(20);
    shots.push(("explain_s15_p2", h));
    let mut h = harness_with_line();
    h.frames(60);
    let now = h.now;
    h.ui.observe(
        now,
        &tm_core::GameEvent::GoalReached {
            total_earned: tm_core::Money::ONE,
        },
        &mut Vec::new(),
    );
    h.frames(90);
    shots.push(("goal", h));
    for (name, h) in &shots {
        write_ppm(&dir.join(format!("{name}.ppm")), &rasterize(h.draw_list()));
    }
}

fn company_level(h: &UiHarness) -> u8 {
    h.game.as_ref().unwrap().view().company().level.0
}

/// 買えるものを 1 段ずつ積む(UI を通さない)。
fn buy_round(h: &mut UiHarness) {
    let g = h.game.as_mut().unwrap();
    for p in ParamKey::ALL {
        let scope = match p.category() {
            ParamCategory::Station => tm_core::UpgradeScope::AllStations,
            _ if p == ParamKey::R2LineSlots => tm_core::UpgradeScope::Company,
            _ => tm_core::UpgradeScope::AllLines,
        };
        let cmd = Command::LevelUp {
            param: p,
            scope,
            mode: tm_core::BuyMode::One,
        };
        if g.validate(&cmd).is_ok() {
            g.enqueue(cmd);
        }
    }
}

/// 「しない」で会社 Lv1 まで進めてから「する」に戻し、パネルを開いて次の会社レベルを待つ画面。
fn waiting_for_next_level() -> UiHarness {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.move_cursor_to(plan()[0]);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    while company_level(&h) < 1 {
        buy_round(&mut h);
        h.frames(2);
    }
    while h.game.as_ref().unwrap().view().total_earned().is_zero() {
        h.frames(30);
    }
    wait(&mut h, 10_000);
    h.tutorial.as_mut().unwrap().set_show_explanations(true);
    h.frames(2);
    assert_eq!(directive(&h), None, "会社レベルの段階を待っている");
    h.press(Action::TabNext);
    assert!(matches!(play(&h).mode, PlayMode::Panel(_)));
    h
}

/// 次の会社レベルまで買い、`FanfareFinished` のフレームまで進める。`press_at` のフレームでは決定を押し、
/// 止める説明をすべて読んでから、そのフレームより後にコアへ渡ったコマンドの数を数える。
/// 戻り値: (`FanfareFinished` か `press_at` のフレームの番号、コマンドの数、そのフレームの directive、止まっているか)
fn until_fanfare(press_at: Option<u32>) -> (u32, usize, Option<TutorialDirective>, bool) {
    let mut h = waiting_for_next_level();
    let start = company_level(&h);
    for n in 0..3000 {
        if company_level(&h) == start && n % 2 == 0 {
            buy_round(&mut h);
        }
        let pressing = press_at == Some(n);
        if pressing {
            h.key_down(Action::Confirm);
        }
        let (c, e) = (h.commands.len(), h.ui_events.len());
        h.frames(1);
        let fanfare = h.ui_events[e..].contains(&UiEvent::FanfareFinished);
        if pressing || (press_at.is_none() && fanfare) {
            let (d, paused) = (directive(&h), h.ui.is_paused(h.tutorial.as_ref()));
            h.key_up(Action::Confirm);
            h.frames(1);
            for _ in 0..10 {
                if !h.ui.is_paused(h.tutorial.as_ref()) {
                    break;
                }
                read_page(&mut h);
            }
            assert!(!h.ui.is_paused(h.tutorial.as_ref()), "読み終えると再開する");
            h.frames(3);
            return (n, h.commands.len() - c, d, paused);
        }
    }
    panic!("FanfareFinished が来ない");
}

#[test]
fn fanfare_explanation_wins_over_the_same_frame_confirm() {
    let (f, _, d, paused) = until_fanfare(None);
    let d = d.expect("ファンファーレの後の説明");
    assert_eq!(d.kind, DirectiveKind::Explanation);
    assert!(d.pauses_game && d.stage.0 >= 9 && paused);
    let (_, before, _, _) = until_fanfare(Some(f - 1));
    assert_eq!(
        before, 1,
        "前のフレームの決定は買える(パネルの項目は買える状態)"
    );
    let (g, cmds, d2, _) = until_fanfare(Some(f));
    assert_eq!(g, f);
    assert_eq!(
        cmds, 0,
        "時刻で始まる止める説明が、同じフレームの入力より先に効く。止めている間にコマンドがたまらない(13 §1 の 4.)"
    );
    assert_eq!(d2, Some(d));
}

#[test]
fn camera_moves_to_a_highlighted_station_under_the_panel() {
    let mut p = plan();
    p[2] = GridPos { x: 28, y: 10 };
    let mut h = UiHarness::with_game(p.clone(), false);
    h.frames(1);
    h.press(Action::TabNext);
    play_mut(&mut h).fake_directive = Some(fake(
        9,
        DirectiveKind::Guide,
        Some(UiAnchor::Station(StationId(2))),
    ));
    h.frames(1);
    assert!(
        matches!(
            mark(&h),
            Some(Mark::Edge {
                toward: Toward::Right,
                ..
            })
        ),
        "パネルの下に隠れている間は表示域の右の端に矢印"
    );
    h.frames(60);
    assert_eq!(
        mark(&h),
        Some(Mark::Tile(ring(p[2], play(&h).camera.pos))),
        "カメラが動いて見える"
    );
    let (cx, _) = tile_screen(h.ui.map_cursor().unwrap(), play(&h).camera.pos);
    assert!(
        cx >= 0 && cx + TILE <= crate::layout::MAP_W_PANEL,
        "カーソルも見えたまま"
    );
}

#[test]
fn settings_hide_the_guide_highlight_but_keep_the_band() {
    let mut h = harness_with_line();
    play_mut(&mut h).fake_directive = Some(fake(
        5,
        DirectiveKind::Guide,
        Some(UiAnchor::Station(StationId(0))),
    ));
    h.frames(1);
    assert!(mark(&h).is_some());
    h.press(Action::Menu);
    assert_eq!(mark(&h), None, "せってい の上に highlight を出さない");
    assert_eq!(play(&h).band.guide_text(), Some(txt::TUT_S03_G1));
    let text = h.draw_text();
    let menu_at = text
        .find(&sprite_at("ui_win_tl", SETTINGS.x, SETTINGS.y))
        .expect("せってい");
    assert!(!text[menu_at..].contains("fx_arrow_down"));
}

fn lines_with(h: &UiHarness, prefix: &str) -> Vec<String> {
    h.draw_text()
        .lines()
        .filter(|l| l.starts_with(prefix))
        .map(str::to_string)
        .collect()
}

/// `Sprite <名前> x,y wxh #色` の x と y。
fn sprite_xy(line: &str) -> (i32, i32) {
    let pos = line.split(' ').nth(2).expect("位置");
    let (x, y) = pos.split_once(',').expect("x,y");
    (x.parse().unwrap(), y.parse().unwrap())
}

#[test]
fn edge_arrow_decides_where_the_box_goes() {
    for (y, top) in [(14, true), (2, false)] {
        let mut p = plan();
        p[2] = GridPos { x: 28, y };
        let mut h = UiHarness::with_game(p, false);
        h.frames(1);
        h.press(Action::TabNext);
        play_mut(&mut h).fake_directive = Some(fake(
            9,
            DirectiveKind::Explanation,
            Some(UiAnchor::Station(StationId(2))),
        ));
        h.frames(1);
        let s = play(&h).explain.shown.unwrap();
        assert!(matches!(s.mark, Some(Mark::Edge { .. })), "y = {y}");
        assert_eq!(
            s.area.y == BOX_TOP_Y,
            top,
            "端の矢印の高さで決める(y = {y})"
        );
    }
}

#[test]
fn single_page_explanations_hide_the_page_number() {
    let mut h = tutorial_game();
    let in_box = |h: &UiHarness| tail_from(&h.draw_text(), "ui_popup_tl").contains("'/'12");
    assert!(in_box(&h), "2 ページなら 1/2");
    read_page(&mut h);
    read_page(&mut h);
    assert_eq!(stage_page(&h), Some((2, DirectiveKind::Explanation, 0)));
    assert!(!in_box(&h), "1 ページだけなら出さない");

    use crate::scene::settings_menu::Page;
    let mut h = harness_with_line();
    h.press(Action::Menu);
    for (stage, shown) in [(2, false), (1, true)] {
        let menu = play_mut(&mut h).settings_menu.as_mut().unwrap();
        menu.set_stages(&[TutorialStageId(stage)]);
        menu.page = Page::Read {
            sel: 0,
            first: 0,
            stage: TutorialStageId(stage),
            page: 0,
        };
        h.frames(1);
        assert_eq!(in_box(&h), shown, "読み直しも同じ(#{stage})");
    }
}

#[test]
fn guard_counts_from_the_frame_the_page_appears() {
    let mut h = tutorial_game();
    wait(&mut h, 320);
    h.key_down(Action::Confirm);
    h.frames(1);
    assert_eq!(stage_page(&h), Some((1, DirectiveKind::Explanation, 1)));
    assert!(
        lines_with(&h, "Sprite btn_key_z")
            .iter()
            .all(|l| l.ends_with("#A6AEC0FF")),
        "次のページが出たフレームから灰色(ガードが始まっている)"
    );
    h.key_up(Action::Confirm);
    wait(&mut h, 200);
    h.press(Action::Confirm);
    assert_eq!(
        stage_page(&h),
        Some((1, DirectiveKind::Explanation, 1)),
        "出てから 300ms 経つまでは読まない"
    );
}

#[test]
fn only_the_tutorial_refusal_makes_the_highlight_bright() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(1);
    play_mut(&mut h).fake_directive = Some(fake(
        3,
        DirectiveKind::Guide,
        Some(UiAnchor::Menu(MenuAnchor::BuildMode)),
    ));
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    assert_eq!(notice(&h), Some(txt::ERR_NOT_CONNECTED));
    assert!(
        !play(&h).explain.shown.unwrap().loud,
        "ほかのエラーでは強くしない"
    );
    h.press(Action::BuyMax);
    assert_eq!(notice(&h), Some(txt::ERR_MAX_OUTSIDE));
    assert!(!play(&h).explain.shown.unwrap().loud);
}

#[test]
fn hud_and_settings_arrows_stay_beside_the_frame() {
    let mut h = harness_with_line();
    let set = |h: &mut UiHarness, a: UiAnchor| {
        play_mut(h).fake_directive = Some(fake(8, DirectiveKind::Guide, Some(a)));
        h.frames(1);
    };
    set(&mut h, UiAnchor::MoneyDisplay);
    let Some(Mark::Side(r)) = mark(&h) else {
        panic!("横")
    };
    let arrows = lines_with(&h, "Sprite ico_tri_left");
    assert_eq!(arrows.len(), 1);
    let (x, y) = sprite_xy(&arrows[0]);
    assert!(x >= r.x + r.w as i32 + 10 && y >= r.y && y + 8 <= r.y + r.h as i32);
    assert!(
        lines_with(&h, "Sprite fx_arrow_down").is_empty(),
        "2 段目に重ねない"
    );
    set(&mut h, UiAnchor::CompanyGauge);
    let Some(Mark::Side(r)) = mark(&h) else {
        panic!("横")
    };
    let gauge = crate::layout::GAUGE;
    assert!(
        r.x < gauge.x && r.x + r.w as i32 > gauge.x + gauge.w as i32,
        "ゲージと会社レベルを囲む"
    );
    let (x, _) = sprite_xy(&lines_with(&h, "Sprite ico_tri_right")[0]);
    assert!(x + 8 <= r.x, "右の端なので左から指す");

    let mut h = tutorial_game();
    h.press(Action::Menu);
    let Some(Mark::Side(r)) = mark(&h) else {
        panic!("#15 の行は横")
    };
    let (x, _) = sprite_xy(lines_with(&h, "Sprite ico_tri_left").last().unwrap());
    let s = crate::layout::SETTINGS;
    assert!(
        x >= s.x + s.w as i32 && x > r.x + r.w as i32,
        "音量の目盛りと箱の縁にかからない"
    );
    assert!(lines_with(&h, "Sprite fx_arrow_down").is_empty());
}

#[test]
fn build_mode_highlights_point_to_the_way_out() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    let set = |h: &mut UiHarness, a: UiAnchor| {
        play_mut(h).fake_directive = Some(fake(9, DirectiveKind::Guide, Some(a)));
        h.frames(1);
    };
    let item = UiAnchor::Menu(MenuAnchor::UpgradeItem(ParamKey::R2LineSlots));
    h.press(Action::BuildMode);
    set(&mut h, item);
    let choose = match &play(&h).mode {
        PlayMode::Build(b) => b.part_rect(MenuAnchor::BuildMode).unwrap(),
        _ => panic!("ろせんモード"),
    };
    assert_eq!(frame_rect(&h), choose, "選ぶ間は選択肢の箱");
    h.press(Action::MoveDown);
    h.press(Action::Confirm);
    assert!(
        matches!(&play(&h).mode, PlayMode::Build(b) if b.step == crate::scene::build::Step::PickEnd)
    );
    set(&mut h, item);
    assert_eq!(
        Some(frame_rect(&h)),
        play(&h).band.hint_rect(),
        "はしを選ぶ間も「もどる」を指す"
    );
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    assert!(
        matches!(&play(&h).mode, PlayMode::Build(b) if matches!(b.step, crate::scene::build::Step::PickNew(_)))
    );
    for a in [
        item,
        UiAnchor::Menu(MenuAnchor::BuildMode),
        UiAnchor::Menu(MenuAnchor::UpgradePanel),
    ] {
        set(&mut h, a);
        assert_eq!(
            Some(frame_rect(&h)),
            play(&h).band.hint_rect(),
            "のばす間は「もどる」の操作({a:?})"
        );
    }
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(1);
    h.press(Action::BuildMode);
    h.press(Action::Confirm);
    assert!(
        matches!(&play(&h).mode, PlayMode::Build(b) if matches!(b.step, crate::scene::build::Step::Create(_)))
    );
    set(&mut h, UiAnchor::Menu(MenuAnchor::BuildMode));
    assert_eq!(Some(frame_rect(&h)), play(&h).band.hint_rect(), "作る間も");
}

#[test]
fn build_hints_fit_the_right_of_the_band() {
    use tm_input::InputDevice;
    for id in [
        txt::BUILD_HINT,
        txt::BUILD_CHOOSE_HINT,
        txt::BUILD_END_HINT,
        txt::BUILD_WHICH_HINT,
    ] {
        for dev in [InputDevice::Keyboard, InputDevice::Gamepad] {
            let mut band = crate::hud::Band::default();
            band.set_mode(None, crate::hud::BandChips::Hint(id), dev);
            let r = band.hint_rect().expect("ヒント");
            let right = crate::layout::BAND_RIGHT;
            assert!(
                r.x + 2 >= right.x && r.w - 4 <= right.w,
                "{id:?} {dev:?}: {r:?} は帯の右 {}px に収まる",
                right.w
            );
        }
    }
}

#[test]
fn every_build_step_shows_its_hint() {
    use crate::scene::build::Step;
    let mut h = harness_with_line();
    let hint = |h: &UiHarness| match &play(h).mode {
        PlayMode::Build(b) => b.band_mode().1,
        _ => panic!("ろせんモード"),
    };
    let chips = |id| crate::hud::BandChips::Hint(id);
    h.press(Action::BuildMode);
    assert_eq!(hint(&h), chips(txt::BUILD_CHOOSE_HINT));
    h.press(Action::MoveDown);
    h.press(Action::Confirm);
    assert_eq!(hint(&h), chips(txt::BUILD_END_HINT));
    match &mut play_mut(&mut h).mode {
        PlayMode::Build(b) => {
            b.step = Step::Which {
                station: StationId(0),
                cands: vec![(LineId(0), tm_core::LineEnd::Head)],
                idx: 0,
            }
        }
        _ => panic!("ろせんモード"),
    }
    h.frames(1);
    assert_eq!(hint(&h), chips(txt::BUILD_WHICH_HINT));
    assert!(play(&h).band.hint_rect().is_some(), "帯の右に出す");
}

#[test]
fn rows_are_not_darkened_while_an_explanation_pauses() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game.as_mut().unwrap().enqueue(create_line(&[0, 1, 2]));
    h.frames(2);
    play_mut(&mut h).forbid = Some(|c: &Command| !matches!(c, Command::LevelUp { .. }));
    h.press(Action::TabNext);
    let row_dim = "#00000070";
    let dims = |h: &UiHarness| match &play(h).mode {
        PlayMode::Panel(p) => p.row_states().iter().filter(|r| r.2).count(),
        _ => panic!("パネル"),
    };
    let mut d = fake(
        6,
        DirectiveKind::Explanation,
        Some(UiAnchor::Menu(MenuAnchor::UpgradeItem(ParamKey::S2Fare))),
    );
    d.message = txt::TUT_S06_P1;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(2);
    assert_eq!(dims(&h), 0, "止める説明の間は暗くしない(UI-D-31)");
    assert!(!h.draw_text().contains(row_dim));
    play_mut(&mut h).fake_directive = Some(fake(
        6,
        DirectiveKind::Guide,
        Some(UiAnchor::Menu(MenuAnchor::UpgradeItem(ParamKey::S2Fare))),
    ));
    h.frames(2);
    assert_eq!(dims(&h), 4, "案内の間は許さない行を暗くする");
    assert!(h.draw_text().contains(row_dim));
}

#[test]
fn last_page_button_says_close() {
    let in_box = |h: &UiHarness| tail_from(&h.draw_text(), "ui_popup_tl");
    let mut h = tutorial_game();
    assert!(
        in_box(&h).contains("'へ'12") && !in_box(&h).contains("'じ'12"),
        "1/2 は つぎへ"
    );
    read_page(&mut h);
    assert_eq!(stage_page(&h), Some((1, DirectiveKind::Explanation, 1)));
    let last = in_box(&h);
    assert!(
        last.contains("'じ'12") && !last.contains("'へ'12"),
        "最後のページは とじる"
    );
    insta::assert_snapshot!("explain_box_last_page", last);
    read_page(&mut h);
    assert_eq!(stage_page(&h), Some((2, DirectiveKind::Explanation, 0)));
    assert!(in_box(&h).contains("'じ'12"), "1 ページだけの説明も とじる");

    use crate::scene::settings_menu::Page;
    let mut h = harness_with_line();
    h.press(Action::Menu);
    for (page, close) in [(0, false), (1, true)] {
        let menu = play_mut(&mut h).settings_menu.as_mut().unwrap();
        menu.set_stages(&[TutorialStageId(1)]);
        menu.page = Page::Read {
            sel: 0,
            first: 0,
            stage: TutorialStageId(1),
            page,
        };
        h.frames(1);
        assert_eq!(
            in_box(&h).contains("'じ'12"),
            close,
            "読み直しも同じ({page})"
        );
    }
}
