//! UI の通し(13 §10.3): 本物の `Game` と `Tutorial` で、ボタン操作だけで #1〜#6 を終える。
//! 子どもは画面に見えるもの(説明の箱、案内の highlight、「作る」の丸、表示域の縁の矢印)だけを見て操作する。
//! 案内の対象が表示域の外なら、縁の矢印のほうへカーソルを動かして(カメラが追う)対象を見つける。

use tm_assets::sound::SE_ERROR;
use tm_core::{Command, GameView, GridPos, LineId, ParamKey, StationId};
use tm_input::Action;
use tm_platform_api::{AudioCmd, MonoTime};
use tm_tutorial::{DirectiveKind, MenuAnchor, TutorialStageId, UiAnchor};

use super::camera::drawn_edge;
use super::screens::play;
use crate::anchor::{Mark, Toward};
use crate::layout::{MAP_SCALE, MAP_X, MAP_Y, TILE};
use crate::scene::PlayMode;
use crate::scene::build::Step;
use crate::testing::{FRAME_US, UiHarness};

/// 説明を 1 ページ読む時間(13 §10.3)。
const READ_MS: u64 = 5_000;
/// ゲームの時間でこれより長くかかったら失敗。
const LIMIT_US: u64 = 15 * 60 * 1_000_000;

struct Kid {
    h: UiHarness,
    paused_frames: u64,
    pages: u32,
}

/// 通しの結果。
#[derive(Debug, PartialEq)]
struct Report {
    commands: Vec<Command>,
    paused_frames: u64,
    pages: u32,
    end_us: u64,
}

impl Kid {
    fn view(&self) -> GameView<'_> {
        self.h.game.as_ref().expect("ゲーム").view()
    }

    fn pos(&self, id: StationId) -> GridPos {
        self.view().station(id).expect("駅").pos
    }

    fn step(&mut self) {
        self.paused_frames += u64::from(self.h.ui.is_paused(self.h.tutorial.as_ref()));
        self.h.frames(1);
        let p = play(&self.h);
        if let Some(s) = p.explain.shown.filter(|s| s.boxed && !s.under_menu) {
            let c = crate::layout::tile_ring(p.cursor, p.camera.pos);
            assert!(
                !crate::anim::overlaps(s.area, c),
                "説明の箱がカーソルを隠す: #{} {} ページ目、箱 {:?}、カーソル {c:?}",
                s.directive.stage.0,
                s.directive.page + 1,
                s.area
            );
        }
    }

    fn wait(&mut self, ms: u64) {
        for _ in 0..(ms * 1000).div_ceil(FRAME_US) {
            self.step();
        }
    }

    fn press(&mut self, a: Action) {
        self.h.key_down(a);
        self.step();
        self.h.key_up(a);
        self.step();
    }

    fn go(&mut self, pos: GridPos) {
        self.h.move_cursor_to(pos);
    }

    fn go_and_confirm(&mut self, pos: GridPos) {
        self.go(pos);
        self.press(Action::Confirm);
    }

    /// 画面に描かれた表示域の縁の矢印を読み、そのほうへカーソルを 1 マス動かす。矢印が描かれていなければ失敗。
    fn follow_edge_arrow(&mut self, what: &str) {
        let (t, _) =
            drawn_edge(&self.h).unwrap_or_else(|| panic!("{what}: 縁の矢印が描かれていない"));
        self.toward(t);
    }

    /// 画面に描かれた「作る」の丸(駅の上か下)が指す駅。
    fn drawn_picks(&self) -> Vec<GridPos> {
        let p = play(&self.h);
        let cam = p.camera.pos;
        let picks = p.explain.picks();
        let d = TILE / 2 + 4 * MAP_SCALE;
        let mut out = Vec::new();
        for c in &self.h.draw_list().cmds {
            let tm_platform_api::DrawCmd::Sprite { id, dst, .. } = *c else {
                continue;
            };
            if id != tm_assets::sprite::FX_DOT {
                continue;
            }
            let (x, y) = (dst.x + dst.w as i32 / 2, dst.y + dst.h as i32 / 2);
            for sy in [y + d, y - d] {
                let t = GridPos {
                    x: (x - MAP_X + cam.x).div_euclid(TILE),
                    y: (sy - MAP_Y + cam.y).div_euclid(TILE),
                };
                if picks.contains(&t) && !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// カーソルを 1 マス動かす。
    fn toward(&mut self, t: Toward) {
        self.press(match t {
            Toward::Left => Action::MoveLeft,
            Toward::Right => Action::MoveRight,
            Toward::Up => Action::MoveUp,
            Toward::Down => Action::MoveDown,
        });
    }

    fn choose(&mut self, row: usize) {
        if let PlayMode::Build(b) = &play(&self.h).mode
            && let Step::Choose { sel } = b.step
            && sel != row
        {
            self.press(if row == 0 {
                Action::MoveUp
            } else {
                Action::MoveDown
            });
            return;
        }
        self.press(Action::Confirm);
    }

    /// のばす: highlight の駅に近い方の はし を選ぶ。
    fn pick_end(&mut self, target: StationId) {
        let to = self.pos(target);
        let ends = self.view().line_ends(LineId(0));
        let (_, near) = ends
            .into_iter()
            .min_by_key(|&(_, s)| self.pos(s).chebyshev(to))
            .expect("はし");
        self.go_and_confirm(self.pos(near));
    }

    fn buy(&mut self, p: ParamKey) {
        let PlayMode::Panel(panel) = &play(&self.h).mode else {
            self.press(Action::TabNext);
            return;
        };
        let Some(c) = panel.tab.category() else {
            panic!("チュートリアルはパラメータのタブ");
        };
        let items = crate::scene::panel::items(c);
        let want = items.iter().position(|&x| x == p).expect("タブの項目");
        let now = panel.item_index();
        if now == want {
            self.wait(300);
            self.press(Action::Confirm);
        } else {
            self.press(if now < want {
                Action::MoveDown
            } else {
                Action::MoveUp
            });
        }
    }

    /// 案内の highlight と画面を見て 1 つ操作する。
    fn follow(&mut self, highlight: Option<UiAnchor>) {
        let step = match &play(&self.h).mode {
            PlayMode::Build(b) if b.confirm.is_none() => Some(b.step.clone()),
            _ => None,
        };
        match (highlight, step) {
            (Some(UiAnchor::Menu(MenuAnchor::BuildMode)), _) => self.press(Action::BuildMode),
            (Some(UiAnchor::Menu(MenuAnchor::BuildCreate)), _) => self.choose(0),
            (Some(UiAnchor::Menu(MenuAnchor::BuildExtend)), _) => self.choose(1),
            (Some(UiAnchor::Menu(MenuAnchor::LineConfirm)), _) => {
                self.wait(300);
                self.press(Action::Confirm);
            }
            (Some(UiAnchor::Menu(MenuAnchor::UpgradePanel | MenuAnchor::UpgradeTab(_))), _) => {
                self.press(Action::TabNext)
            }
            (Some(UiAnchor::Menu(MenuAnchor::UpgradeItem(p))), _) => self.buy(p),
            (Some(UiAnchor::Station(id)), Some(Step::PickEnd)) => self.pick_end(id),
            (Some(UiAnchor::Station(id)), Some(Step::PickNew(_))) => {
                self.go_and_confirm(self.pos(id))
            }
            (Some(UiAnchor::Station(id)), None) => {
                self.go(self.pos(id));
                self.wait(100);
            }
            (None, Some(Step::Create(_))) => {
                assert!(!play(&self.h).explain.picks().is_empty(), "選べる駅がある");
                match self.drawn_picks().first() {
                    Some(&next) => self.go_and_confirm(next),
                    None => self.follow_edge_arrow("見える丸がない"),
                }
            }
            (Some(UiAnchor::Train(_) | UiAnchor::MoneyDisplay), _) => self.wait(500),
            other => panic!("知らない案内: {other:?}"),
        }
    }
}

fn run(seed: u64, gamepad: bool) -> Report {
    let mut kid = Kid {
        h: UiHarness::with_seed(seed, true, MonoTime(0)),
        paused_frames: 0,
        pages: 0,
    };
    if gamepad {
        kid.h.use_gamepad();
    }
    kid.step();
    loop {
        assert!(
            kid.h.now.0 < LIMIT_US,
            "シード {seed}: 時間内に #6 が終わらない"
        );
        let t = kid.h.tutorial.as_ref().expect("チュートリアル");
        let shown = t.shown_stages().to_vec();
        let d = t.directive().copied();
        if shown.contains(&TutorialStageId(6)) && d.is_none() {
            break;
        }
        match d {
            Some(d) if d.kind == DirectiveKind::Explanation => {
                let s = play(&kid.h).explain.shown.expect("見えている");
                assert!(s.boxed && s.directive == d, "説明の箱を出している");
                if let Some(a) = d.highlight {
                    assert!(s.mark.is_some(), "シード {seed}: highlight {a:?} を置ける");
                }
                kid.wait(READ_MS);
                kid.press(Action::Confirm);
                kid.pages += 1;
            }
            Some(d) => {
                let s = play(&kid.h).explain.shown.expect("見えている");
                assert_eq!(
                    play(&kid.h).band.guide_text(),
                    Some(d.message),
                    "案内を下の帯に出す"
                );
                if let Some(a) = d.highlight {
                    match s.mark {
                        Some(Mark::Edge { .. }) => {
                            kid.follow_edge_arrow(&format!("シード {seed}: {a:?}"));
                            continue;
                        }
                        m => assert!(
                            matches!(
                                m,
                                Some(
                                    Mark::Frame(_)
                                        | Mark::Tile(_)
                                        | Mark::Side(_)
                                        | Mark::Line { .. }
                                )
                            ),
                            "シード {seed}: highlight {a:?} が見えるか、縁の矢印で指す"
                        ),
                    }
                }
                kid.follow(d.highlight);
            }
            None => kid.wait(500),
        }
    }
    let h = &kid.h;
    assert_eq!(
        h.tutorial.as_ref().unwrap().shown_stages(),
        (1..=6).map(TutorialStageId).collect::<Vec<_>>()
    );
    assert!(
        !h.audio.contains(&AudioCmd::PlaySe(SE_ERROR)),
        "シード {seed}: 案内どおりの操作でエラーにならない"
    );
    Report {
        commands: h.commands.clone(),
        paused_frames: kid.paused_frames,
        pages: kid.pages,
        end_us: h.now.0,
    }
}

fn check(seed: u64, gamepad: bool) -> Report {
    let r = run(seed, gamepad);
    assert!(
        matches!(&r.commands[..], [
            Command::CreateLine { stations, .. },
            Command::ExtendLine { .. },
            Command::LevelUp { param: ParamKey::S2Fare, .. },
        ] if stations.len() == 2),
        "シード {seed}: {:?}",
        r.commands
    );
    assert_eq!(
        r.pages, 11,
        "#1 2 + #2 1 + #3 3 + #4 1 + #5 2 + #6 2 ページ"
    );
    let paused_s = r.paused_frames * FRAME_US / 1_000_000;
    println!(
        "UI-V-12 シード {seed}: 止まった時間 {paused_s} 秒(11 ページ)、#6 の終わり {} 秒",
        r.end_us / 1_000_000
    );
    assert!(paused_s <= 200, "止まる時間の合計は 200 秒以下(UI-V-12)");
    r
}

#[test]
fn keyboard_only_reaches_stage_7_for_three_seeds() {
    for seed in [1, 2, 3] {
        check(seed, false);
    }
}

#[test]
fn gamepad_only_reaches_stage_7() {
    check(4, true);
}

#[test]
fn the_run_is_deterministic() {
    assert_eq!(run(5, false), run(5, false));
}
