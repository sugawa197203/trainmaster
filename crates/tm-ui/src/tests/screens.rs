//! 画面遷移(13 §3.3)、せってい とリセット(§3.4、01 §11.3)、入力の振り分け(§3.4)。

use tm_assets::sound::{
    SE_CANCEL, SE_COIN, SE_CONFIRM, SE_CURSOR, SE_ERROR, SE_MENU_CLOSE, SE_MENU_OPEN, SE_RESET,
};
use tm_assets::txt;
use tm_core::{Command, GridPos, ParamCategory};
use tm_input::Action;
use tm_platform_api::{AudioBus, AudioCmd, SoundId};
use tm_tutorial::{
    DirectiveKind, MenuAnchor, SettingsItem, TutorialDirective, TutorialStageId, UiAnchor, UiEvent,
};

use super::{cheap_game, harness_with_line, plan};
use crate::AppRequest;
use crate::scene::{PanelTab, PlayMode, PlayScene};
use crate::testing::UiHarness;

pub(super) fn play(h: &UiHarness) -> &PlayScene {
    h.ui.play().expect("ゲームの画面")
}

pub(super) fn play_mut(h: &mut UiHarness) -> &mut PlayScene {
    h.ui.play_mut().expect("ゲームの画面")
}

/// `from` 以降に鳴らした効果音。
pub(super) fn ses(h: &UiHarness, from: usize) -> Vec<SoundId> {
    h.audio[from..]
        .iter()
        .filter_map(|c| match c {
            AudioCmd::PlaySe(id) => Some(*id),
            _ => None,
        })
        .collect()
}

/// `ms` だけ進める。
pub(super) fn wait(h: &mut UiHarness, ms: u64) {
    h.frames((ms * 1000).div_ceil(crate::testing::FRAME_US) as u32);
}

pub(super) fn menu_page(h: &UiHarness) -> Option<crate::scene::settings_menu::Page> {
    play(h).settings_menu.as_ref().map(|m| m.page)
}

pub(super) fn notice(h: &UiHarness) -> Option<tm_assets::text::TextId> {
    play(h).band.notice_text()
}

pub(super) fn open_settings(h: &mut UiHarness) {
    h.press(Action::Menu);
    assert!(play(h).settings_open());
}

pub(super) fn select_item(h: &mut UiHarness, item: SettingsItem) {
    let want = crate::scene::settings_menu::ITEMS
        .iter()
        .position(|&i| i == item)
        .unwrap();
    for _ in 0..8 {
        let sel = play(h).settings_menu.as_ref().unwrap().sel;
        if sel == want {
            return;
        }
        h.press(if sel < want {
            Action::MoveDown
        } else {
            Action::MoveUp
        });
    }
    panic!("{item:?} に届かない");
}

fn explanation(page: u8) -> TutorialDirective {
    TutorialDirective {
        stage: TutorialStageId(1),
        kind: DirectiveKind::Explanation,
        message: txt::TUT_S01_P1,
        page,
        page_count: 2,
        highlight: None,
        pauses_game: true,
    }
}

#[test]
fn menu_opens_and_closes_settings_and_pauses_the_game() {
    let mut h = harness_with_line();
    wait(&mut h, 400);
    let mark = h.audio.len();
    open_settings(&mut h);
    assert_eq!(ses(&h, mark), [SE_MENU_OPEN]);
    assert!(h.ui.is_paused(h.tutorial.as_ref()), "せってい の間は止める");
    assert!(h.audio.contains(&AudioCmd::SetBgmDuck(true)));
    let tick = h.game.as_ref().unwrap().view().tick();
    h.frames(30);
    assert_eq!(h.game.as_ref().unwrap().view().tick(), tick);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Menu);
    assert!(!play(&h).settings_open());
    assert_eq!(ses(&h, mark), [SE_MENU_CLOSE], "閉じる音だけ");
    assert!(!h.ui.is_paused(h.tutorial.as_ref()));
    assert!(
        h.ui_events
            .ends_with(&[UiEvent::MenuClosed(MenuAnchor::SettingsMenu)])
    );
}

#[test]
fn cancel_at_top_and_continue_close_settings() {
    let mut h = harness_with_line();
    open_settings(&mut h);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Cancel);
    assert!(!play(&h).settings_open());
    assert_eq!(ses(&h, mark), [SE_MENU_CLOSE], "Cancel でも閉じる音だけ");
    open_settings(&mut h);
    assert_eq!(
        play(&h).settings_menu.as_ref().unwrap().sel,
        0,
        "初期は つづける"
    );
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert!(!play(&h).settings_open());
    assert_eq!(ses(&h, mark), [SE_MENU_CLOSE]);
}

#[test]
fn settings_items_are_in_order_and_stop_at_the_ends() {
    let mut h = harness_with_line();
    open_settings(&mut h);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::MoveUp);
    assert_eq!(ses(&h, mark), [], "いちばん上で止まる(音なし)");
    let mut seen = vec![];
    for _ in 0..8 {
        let m = play(&h).settings_menu.as_ref().unwrap();
        seen.push(crate::scene::settings_menu::ITEMS[m.sel]);
        h.press(Action::MoveDown);
    }
    assert_eq!(
        seen,
        [
            SettingsItem::Continue,
            SettingsItem::MusicVolume,
            SettingsItem::SoundVolume,
            SettingsItem::ReplayExplanations,
            SettingsItem::ShowExplanations,
            SettingsItem::Quit,
            SettingsItem::Reset,
            SettingsItem::Reset
        ]
    );
    assert!(h.ui_events.contains(&UiEvent::CursorOn(UiAnchor::Menu(
        MenuAnchor::SettingsItem(SettingsItem::Reset)
    ))));
}

#[test]
fn volumes_change_by_one_and_play_one_sound() {
    let mut h = harness_with_line();
    open_settings(&mut h);
    select_item(&mut h, SettingsItem::MusicVolume);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::MoveRight);
    assert_eq!(h.ui.settings().music, 7);
    assert_eq!(
        h.audio[mark..],
        [
            AudioCmd::SetVolume {
                bus: AudioBus::Bgm,
                level: 7
            },
            AudioCmd::PlaySe(SE_CURSOR)
        ]
    );
    for _ in 0..12 {
        h.press(Action::MoveLeft);
    }
    assert_eq!(h.ui.settings().music, 0, "0 で止まる");
    select_item(&mut h, SettingsItem::SoundVolume);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::MoveRight);
    assert_eq!(h.ui.settings().sound, 9);
    assert_eq!(
        h.audio[mark..],
        [
            AudioCmd::SetVolume {
                bus: AudioBus::Se,
                level: 9
            },
            AudioCmd::PlaySe(SE_COIN)
        ],
        "音の大きさは試しの se_coin だけ"
    );
    h.press(Action::MoveRight);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::MoveRight);
    assert_eq!(h.ui.settings().sound, 10);
    assert!(h.audio[mark..].is_empty(), "10 で止まる");
}

#[test]
fn show_explanations_toggles_and_survives_reset() {
    let mut h = harness_with_line();
    open_settings(&mut h);
    select_item(&mut h, SettingsItem::ShowExplanations);
    h.press(Action::MoveRight);
    assert!(!h.ui.show_explanations(), "右で しない");
    h.press(Action::MoveLeft);
    assert!(h.ui.show_explanations(), "左で する");
    h.press(Action::Confirm);
    assert!(!h.ui.show_explanations(), "決定でも切り替わる");
    select_item(&mut h, SettingsItem::MusicVolume);
    h.press(Action::MoveLeft);
    reset_twice(&mut h);
    assert!(!h.ui.show_explanations(), "リセットしても保つ");
    assert_eq!(h.ui.settings().music, 5);
}

fn reset_twice(h: &mut UiHarness) {
    select_item(h, SettingsItem::Reset);
    h.press(Action::Confirm);
    wait(h, 300);
    h.press(Action::MoveRight);
    h.press(Action::Confirm);
    wait(h, 300);
    h.press(Action::MoveRight);
    h.press(Action::Confirm);
}

#[test]
fn reset_needs_two_yes_and_returns_to_title() {
    use crate::scene::settings_menu::Page;
    let mut h = harness_with_line();
    open_settings(&mut h);
    select_item(&mut h, SettingsItem::Reset);
    h.press(Action::Confirm);
    assert!(matches!(
        menu_page(&h),
        Some(Page::Reset {
            step: 1,
            yes: false,
            ..
        })
    ));
    wait(&mut h, 300);
    h.press(Action::MoveRight);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert!(
        matches!(
            menu_page(&h),
            Some(Page::Reset {
                step: 2,
                yes: false,
                ..
            })
        ),
        "2 回目も「いいえ」から"
    );
    assert_eq!(ses(&h, mark), [SE_CONFIRM]);
    assert!(h.requests.is_empty(), "1 回目の「はい」では戻らない");
    wait(&mut h, 300);
    h.press(Action::MoveRight);
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert_eq!(h.requests, [AppRequest::ResetToTitle]);
    assert_eq!(ses(&h, mark), [SE_RESET], "se_confirm は鳴らさない");
    assert!(
        h.game.is_none() && h.ui.map_cursor().is_none(),
        "タイトルへ"
    );
    assert!(
        h.audio[mark..]
            .iter()
            .all(|c| !matches!(c, AudioCmd::PlaySe(id) if *id == SE_CONFIRM))
    );
}

#[test]
fn reset_confirm_starts_on_no_and_ignores_quick_confirm() {
    use crate::scene::settings_menu::Page;
    let mut h = harness_with_line();
    open_settings(&mut h);
    select_item(&mut h, SettingsItem::Reset);
    h.press(Action::Confirm);
    h.press(Action::MoveRight);
    h.press(Action::Confirm);
    assert!(
        matches!(menu_page(&h), Some(Page::Reset { step: 1, .. })),
        "出てから 300ms は決定を受けない"
    );
    wait(&mut h, 300);
    h.press(Action::MoveLeft);
    h.press(Action::Confirm);
    assert_eq!(menu_page(&h), Some(Page::Top), "「いいえ」でメニューへ");
    assert!(h.requests.is_empty());
    assert_eq!(
        crate::scene::settings_menu::ITEMS[play(&h).settings_menu.as_ref().unwrap().sel],
        SettingsItem::Reset,
        "選択は 最初からやり直す のまま"
    );
}

#[test]
fn every_way_out_of_reset_confirm_returns_to_menu() {
    use crate::scene::settings_menu::Page;
    for step in [1u8, 2] {
        for exit in [Action::Cancel, Action::Menu, Action::Confirm] {
            let mut h = harness_with_line();
            open_settings(&mut h);
            select_item(&mut h, SettingsItem::Reset);
            h.press(Action::Confirm);
            if step == 2 {
                wait(&mut h, 300);
                h.press(Action::MoveRight);
                h.press(Action::Confirm);
            }
            wait(&mut h, 300);
            wait(&mut h, 400);
            let mark = h.audio.len();
            h.press(exit);
            assert_eq!(menu_page(&h), Some(Page::Top), "段 {step} で {exit:?}");
            assert!(play(&h).settings_open(), "メニューは閉じない");
            assert!(h.requests.is_empty());
            let want = if exit == Action::Confirm {
                SE_CONFIRM
            } else {
                SE_CANCEL
            };
            assert_eq!(ses(&h, mark), [want]);
            assert!(
                h.ui_events
                    .ends_with(&[UiEvent::MenuClosed(MenuAnchor::ResetConfirm { step })])
            );
        }
    }
}

#[test]
fn replay_list_is_a_sub_screen() {
    use crate::scene::settings_menu::Page;
    let mut h = harness_with_line();
    open_settings(&mut h);
    select_item(&mut h, SettingsItem::ReplayExplanations);
    h.press(Action::Confirm);
    assert!(matches!(menu_page(&h), Some(Page::Replay { .. })));
    h.press(Action::Confirm);
    assert!(
        matches!(menu_page(&h), Some(Page::Replay { .. })),
        "まだ ないよ のときは何もしない"
    );
    h.press(Action::Menu);
    assert_eq!(
        menu_page(&h),
        Some(Page::Top),
        "サブ画面の Menu はいちばん上へ"
    );
    h.press(Action::Confirm);
    h.press(Action::Cancel);
    assert_eq!(menu_page(&h), Some(Page::Top));
    h.press(Action::Cancel);
    assert!(!play(&h).settings_open());
}

#[test]
fn replay_reads_pages_of_seen_stages() {
    use crate::scene::settings_menu::Page;
    let mut h = harness_with_line();
    open_settings(&mut h);
    select_item(&mut h, SettingsItem::ReplayExplanations);
    h.press(Action::Confirm);
    let stages: Vec<TutorialStageId> = [1, 3, 2, 4, 5, 6, 7, 8, 9, 10]
        .into_iter()
        .map(TutorialStageId)
        .collect();
    play_mut(&mut h)
        .settings_menu
        .as_mut()
        .unwrap()
        .set_stages(&stages);
    for _ in 0..9 {
        h.press(Action::MoveDown);
    }
    let last_first = stages.len() - crate::layout::SETTINGS_ROWS as usize;
    assert_eq!(
        menu_page(&h),
        Some(Page::Replay {
            sel: 9,
            first: last_first
        })
    );
    h.press(Action::MoveUp);
    h.press(Action::MoveUp);
    h.press(Action::MoveUp);
    assert_eq!(
        menu_page(&h),
        Some(Page::Replay {
            sel: 6,
            first: last_first
        })
    );
    for _ in 0..6 {
        h.press(Action::MoveUp);
    }
    h.press(Action::MoveDown);
    h.press(Action::Confirm);
    assert!(matches!(
        menu_page(&h),
        Some(Page::Read {
            stage: TutorialStageId(3),
            page: 0,
            ..
        })
    ));
    h.press(Action::Confirm);
    h.press(Action::Confirm);
    assert!(matches!(menu_page(&h), Some(Page::Read { page: 2, .. })));
    h.press(Action::Confirm);
    assert_eq!(
        menu_page(&h),
        Some(Page::Replay { sel: 1, first: 0 }),
        "最後のページで決定すると一覧へ"
    );
    assert!(
        !h.ui_events.contains(&UiEvent::MessageAcknowledged),
        "読み直しでは MessageAcknowledged を出さない"
    );
}

#[test]
fn menu_opens_from_every_play_state_and_keeps_it() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.frames(2);
    h.move_cursor_to(plan()[0]);
    h.press(Action::BuildMode);
    h.press(Action::Confirm);
    h.press(Action::Confirm);
    h.move_cursor_to(plan()[1]);
    h.press(Action::Confirm);
    h.press(Action::Confirm);
    assert!(matches!(&play(&h).mode, PlayMode::Build(b) if b.confirm.is_some()));
    open_settings(&mut h);
    h.press(Action::MoveDown);
    h.press(Action::Menu);
    match &play(&h).mode {
        PlayMode::Build(b) => {
            let c = b.confirm.as_ref().expect("確認はそのまま");
            assert!(c.yes, "せってい の入力は下の画面に渡らない");
        }
        _ => panic!("ろせんモードのまま"),
    }
    h.press(Action::Cancel);
    h.press(Action::Cancel);
    h.press(Action::Cancel);
    h.press(Action::Cancel);
    assert!(matches!(play(&h).mode, PlayMode::Map));
    h.press(Action::TabNext);
    h.press(Action::TabNext);
    open_settings(&mut h);
    h.press(Action::Cancel);
    assert!(
        matches!(&play(&h).mode, PlayMode::Panel(p) if p.tab == PanelTab::Param(ParamCategory::Train))
    );
}

#[test]
fn settings_take_all_input_while_open() {
    let mut h = harness_with_line();
    let c0 = h.ui.map_cursor();
    open_settings(&mut h);
    for a in [
        Action::MoveLeft,
        Action::BuildMode,
        Action::TabNext,
        Action::JumpRecommend,
        Action::BuyMax,
        Action::ToggleBend,
    ] {
        h.press(a);
    }
    assert_eq!(h.ui.map_cursor(), c0);
    assert!(matches!(play(&h).mode, PlayMode::Map));
    assert!(h.commands.is_empty());
}

#[test]
fn map_cancel_does_nothing_and_max_outside_only_shows_a_hint() {
    let mut h = harness_with_line();
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Cancel);
    h.press(Action::ToggleBend);
    assert!(ses(&h, mark).is_empty(), "マップの Cancel は音も出さない");
    h.press(Action::BuyMax);
    assert!(ses(&h, mark).is_empty(), "案内だけ(音なし)");
    assert_eq!(notice(&h), Some(txt::ERR_MAX_OUTSIDE));
    assert!(h.commands.is_empty());
}

#[test]
fn explanation_takes_confirm_and_passes_moves_down() {
    let mut h = harness_with_line();
    play_mut(&mut h).fake_directive = Some(explanation(0));
    h.frames(1);
    let c0 = h.ui.map_cursor().unwrap();
    wait(&mut h, 400);
    let mark = h.audio.len();
    for a in [
        Action::Cancel,
        Action::BuyMax,
        Action::TabNext,
        Action::TabPrev,
        Action::BuildMode,
        Action::ToggleBend,
        Action::JumpRecommend,
    ] {
        h.press(a);
    }
    assert!(
        matches!(play(&h).mode, PlayMode::Map),
        "ほかの Action は無視"
    );
    assert!(ses(&h, mark).is_empty(), "音も出さない");
    h.press(Action::MoveLeft);
    assert_eq!(
        h.ui.map_cursor(),
        Some(GridPos {
            x: c0.x - 1,
            y: c0.y
        }),
        "Move は下の画面へ"
    );
    assert!(!h.ui_events.contains(&UiEvent::MessageAcknowledged));
    wait(&mut h, 300);
    h.press(Action::Confirm);
    assert_eq!(
        h.ui_events
            .iter()
            .filter(|e| **e == UiEvent::MessageAcknowledged)
            .count(),
        1
    );
    assert!(matches!(play(&h).mode, PlayMode::Map), "パネルは開かない");
    play_mut(&mut h).fake_directive = Some(explanation(1));
    h.frames(1);
    h.press(Action::Confirm);
    assert_eq!(
        h.ui_events
            .iter()
            .filter(|e| **e == UiEvent::MessageAcknowledged)
            .count(),
        1,
        "ページごとに 300ms のガード"
    );
    h.press(Action::Menu);
    assert!(play(&h).settings_open(), "Menu で せってい は開く");
    assert!(h.commands.is_empty());
}

#[test]
fn explanation_never_lets_commands_out() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game
        .as_mut()
        .unwrap()
        .enqueue(super::create_line(&[0, 1, 2]));
    h.frames(2);
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    assert!(matches!(play(&h).mode, PlayMode::Panel(_)));
    play_mut(&mut h).fake_directive = Some(explanation(0));
    wait(&mut h, 400);
    h.hold(Action::Confirm, 1000);
    h.press(Action::BuyMax);
    assert!(h.commands.is_empty(), "止める説明の間はコマンドを出さない");
    h.press(Action::MoveDown);
    assert_eq!(
        match &play(&h).mode {
            PlayMode::Panel(p) => p.item_index(),
            _ => 99,
        },
        1,
        "パネルの項目は動かせる"
    );
}

#[test]
fn closing_an_explanation_does_not_carry_the_held_confirm() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game
        .as_mut()
        .unwrap()
        .enqueue(super::create_line(&[0, 1, 2]));
    h.frames(2);
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    play_mut(&mut h).fake_directive = Some(explanation(0));
    wait(&mut h, 400);
    h.key_down(Action::Confirm);
    h.frames(1);
    play_mut(&mut h).fake_directive = None;
    h.frames(60);
    h.key_up(Action::Confirm);
    h.frames(1);
    assert!(h.commands.is_empty(), "押しっぱなしで購入しない");
    h.press(Action::Confirm);
    assert_eq!(h.commands.len(), 1, "押し直せば買える");
}

#[test]
fn explanation_inside_settings_takes_confirm() {
    let mut h = harness_with_line();
    open_settings(&mut h);
    let mut d = explanation(0);
    d.stage = TutorialStageId(15);
    d.pauses_game = false;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    wait(&mut h, 300);
    h.press(Action::Confirm);
    assert!(play(&h).settings_open(), "つづける を選んでいても閉じない");
    assert!(h.ui_events.contains(&UiEvent::MessageAcknowledged));
    h.press(Action::MoveDown);
    assert_eq!(play(&h).settings_menu.as_ref().unwrap().sel, 1);
}

#[test]
fn permits_false_refuses_with_error_and_hint() {
    let mut h = UiHarness::from_game(cheap_game(), false);
    h.game
        .as_mut()
        .unwrap()
        .enqueue(super::create_line(&[0, 1, 2]));
    h.frames(2);
    play_mut(&mut h).forbid = Some(|c: &Command| !matches!(c, Command::LevelUp { .. }));
    h.move_cursor_to(plan()[0]);
    h.press(Action::Confirm);
    h.frames(10);
    match &play(&h).mode {
        PlayMode::Panel(p) => assert!(p.row_states().iter().all(|r| r.2), "行を暗くする"),
        _ => panic!("パネル"),
    }
    wait(&mut h, 400);
    let mark = h.audio.len();
    h.press(Action::Confirm);
    assert!(h.commands.is_empty());
    assert_eq!(ses(&h, mark), [SE_ERROR]);
    assert_eq!(notice(&h), Some(txt::ERR_TUTORIAL));
}

/// #15 の説明は せってい のいちばん上でだけ決定を受ける。下位の画面では決定は確認の「いいえ」に渡る。
#[test]
fn stage_15_takes_confirm_only_on_the_top_page() {
    use crate::scene::settings_menu::Page;
    let mut h = harness_with_line();
    open_settings(&mut h);
    select_item(&mut h, SettingsItem::Reset);
    h.press(Action::Confirm);
    assert!(matches!(menu_page(&h), Some(Page::Reset { step: 1, .. })));
    let mut d = explanation(0);
    d.stage = TutorialStageId(15);
    d.pauses_game = false;
    play_mut(&mut h).fake_directive = Some(d);
    h.frames(1);
    assert!(
        !play(&h).explain.shown.unwrap().boxed,
        "確認の間は #15 を出さない"
    );
    wait(&mut h, 400);
    let ev = h.ui_events.len();
    h.press(Action::Confirm);
    assert_eq!(menu_page(&h), Some(Page::Top), "確認の「いいえ」");
    assert!(!h.ui_events[ev..].contains(&UiEvent::MessageAcknowledged));
    h.frames(1);
    assert!(
        play(&h).explain.shown.unwrap().boxed,
        "いちばん上に戻ると出る"
    );
}
