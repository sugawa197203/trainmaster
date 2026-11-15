//! 演出: 収入のポップアップ、会社レベルの帯、トースト、紙ふぶき、光と星 [13 §3.12]。
//!
//! 時間はすべて `MonoTime` で数える(一時停止中も進む)。補間は整数だけで行う。

use std::collections::VecDeque;

use tm_assets::font::FontId;
use tm_assets::sound;
use tm_assets::sprite::{self, ANIM_FX_SPARKLE};
use tm_assets::text::TextArg;
use tm_assets::txt;
use tm_core::{GameView, LineId, StationId, TrainId, UpgradeScope};
use tm_input::InputDevice;
use tm_numeric::{BigNum, FormatStyle, JA_UNITS};
use tm_platform_api::{MonoTime, Rect, SoundId};

use crate::draw::palette::{self, with_alpha};
use crate::draw::{DrawBuilder, Shed, rect};
use crate::hud::decimal;
use crate::layout::{
    BANNER, BANNER_H3, Cam, GOAL_BAND, MAP_SCALE, MAP_X, SCREEN_H, SCREEN_W, TILE, TOAST,
    near_view, tile_center,
};
use crate::text::numbers::NumText;
use crate::text::{DynText, TextBlock, layout};

/// 収入のポップアップの同時の上限 [UI-D-22]。
pub(crate) const POPUP_MAX: usize = 12;
const POPUP_MS: u64 = 800;
const POPUP_FADE_MS: u64 = 300;
const POPUP_MERGE_MS: u64 = 300;
const POPUP_RISE: i32 = 12;
/// ポップアップの高さ(重なりを避けて 1 段ずらす量。Body12 の行の高さ)。
const POPUP_ROW: i32 = 16;
/// 帯・トーストの文の最大の幅(箱の左右に 8 ずつ)。
const NOTICE_W: u16 = BANNER.w as u16 - 16;
/// ゴールの文を描く倍率(Head16 の 2 倍。15 §4.10)。
const GOAL_SCALE: u8 = 2;
const BANNER_MS: u64 = 3000;
const BANNER_SLIDE_MS: u64 = 200;
const BANNER_QUEUE: usize = 16;
const TOAST_UNIT_MS: u64 = 3000;
const TOAST_PASS_MS: u64 = 2500;
const TOAST_REBIRTH_MS: u64 = 3500;
const TOAST_QUEUE: usize = 4;
/// 紙ふぶきの数の上限。
pub(crate) const CONFETTI_MAX: u32 = 60;
const GLOW_MS: u64 = 1000;
const SPARKLE_MS: u64 = 400;
/// 駅の出現の演出の長さ(絵の 4px → 14px を 2 倍で描く)[13 §3.12]。
pub(crate) const SPAWN_MS: u64 = 400;
/// 駅の絵の、出現の演出の始まりと終わりの大きさ(画面の px)。
const SPAWN_FROM_PX: u64 = 4 * MAP_SCALE as u64;
const SPAWN_TO_PX: u64 = 14 * MAP_SCALE as u64;
/// 星を置く、中心からのずれ。
const SPARKLE_AT: i32 = 9 * MAP_SCALE;
const LEVELUP_MS: u64 = 600;
const FX_MAX: usize = 32;
/// 対象の矢印の上限(駅の最大数。バランスの表と同じことをテストで確かめる)。表示域の中に描くものだけを数える。
pub(crate) const LEVELUP_TARGETS: usize = 27;

fn ms(us: u64) -> u64 {
    us / 1000
}

fn elapsed_ms(now: MonoTime, start: MonoTime) -> u64 {
    ms(now.saturating_since(start))
}

/// 音の長さ(ms)。サンプル数 ÷ 22050 [02 §4.6]。
pub(crate) fn sound_ms(id: SoundId) -> u64 {
    tm_assets::bundle()
        .ok()
        .and_then(|b| b.sounds.get(usize::from(id.0)))
        .map_or(0, |s| s.samples.len() as u64 * 1000 / 22_050)
}

/// 札の左端を、幅 `view_w` のマップの表示域(パネルを開いていれば 416)の左右に収める [13 §3.12]。
pub(crate) fn clamp_to_map(x: i32, w: i32, view_w: i32) -> i32 {
    x.clamp(MAP_X + 2, (MAP_X + view_w - 2 - w).max(MAP_X + 2))
}

/// 2 つの矩形が重なるか。
pub(crate) fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w as i32
        && b.x < a.x + a.w as i32
        && a.y < b.y + b.h as i32
        && b.y < a.y + a.h as i32
}

/// 0..=255 を `num / den` 倍する。
fn scale_alpha(num: u64, den: u64) -> u8 {
    (255 * num.min(den) / den.max(1)) as u8
}

struct Popup {
    station: StationId,
    start: MonoTime,
    amount: BigNum,
    num: NumText,
}

/// 収入のポップアップ。配列と文字列のバッファは使い回す。
#[derive(Default)]
pub(crate) struct PopupPool {
    live: Vec<Popup>,
    spare: Vec<Popup>,
    plus: TextBlock,
}

impl PopupPool {
    /// 収入を足す。同じ駅の 300ms 以内はまとめ、12 個を超えたら古いものから消す。
    pub(crate) fn add(&mut self, now: MonoTime, station: StationId, income: BigNum) {
        if self.plus.glyphs.is_empty() {
            layout(
                "+",
                FontId::Body12,
                u16::MAX,
                &[],
                InputDevice::Keyboard,
                &mut self.plus,
            );
        }
        if let Some(p) = self
            .live
            .iter_mut()
            .rev()
            .find(|p| p.station == station && elapsed_ms(now, p.start) < POPUP_MERGE_MS)
        {
            p.amount = p.amount.saturating_add(income);
            p.num.set(p.amount, &FormatStyle::DEFAULT, FontId::Body12);
            return;
        }
        let mut p = if self.live.len() >= POPUP_MAX {
            self.live.remove(0)
        } else {
            self.spare.pop().unwrap_or_else(|| Popup {
                station,
                start: now,
                amount: BigNum::ZERO,
                num: NumText::default(),
            })
        };
        p.station = station;
        p.start = now;
        p.amount = income;
        p.num.set(income, &FormatStyle::DEFAULT, FontId::Body12);
        self.live.push(p);
    }

    fn prune(&mut self, now: MonoTime) {
        while let Some(i) = self
            .live
            .iter()
            .position(|p| elapsed_ms(now, p.start) >= POPUP_MS)
        {
            let p = self.live.remove(i);
            self.spare.push(p);
        }
    }

    /// 出ている数。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.live.len()
    }

    /// 描く矩形。古いものから置き、重なる新しいものは 1 段上に、それでも重なれば今は出さない(読めるように)。
    /// 左右は幅 `view_w` の表示域に収める。表示域の外の駅のものは出さない。
    pub(crate) fn layout(
        &self,
        view: GameView<'_>,
        now: MonoTime,
        cam: Cam,
        view_w: i32,
    ) -> [Option<Rect>; POPUP_MAX] {
        let mut out = [None; POPUP_MAX];
        for (i, p) in self.live.iter().enumerate().take(POPUP_MAX) {
            let Some(st) = view.station(p.station) else {
                continue;
            };
            let t = elapsed_ms(now, p.start).min(POPUP_MS);
            let rise = POPUP_RISE * t as i32 / POPUP_MS as i32;
            let w = i32::from(self.plus.width) + i32::from(p.num.block.width);
            let (cx, cy) = tile_center(st.pos, cam);
            if !near_view(cx, cy, view_w) {
                continue;
            }
            let x = clamp_to_map(cx - w / 2, w, view_w);
            let y = cy - TILE / 2 - POPUP_ROW - 2 - rise;
            let free = |r: Rect| out[..i].iter().flatten().all(|q| !overlaps(r, *q));
            out[i] = [0, -POPUP_ROW]
                .into_iter()
                .map(|dy| rect(x - 2, y + dy, (w + 4) as u32, POPUP_ROW as u32))
                .find(|&r| free(r));
        }
        out
    }

    /// `placed` は同じフレームの `layout` の結果。
    fn draw(&self, b: &mut DrawBuilder<'_>, now: MonoTime, placed: &[Option<Rect>]) {
        for (p, &r) in self.live.iter().zip(placed) {
            let Some(r) = r else { continue };
            let t = elapsed_ms(now, p.start).min(POPUP_MS);
            let left = POPUP_MS - t;
            let alpha = if left < POPUP_FADE_MS {
                scale_alpha(left, POPUP_FADE_MS)
            } else {
                255
            };
            let (x, y) = (r.x + 2, r.y);
            b.group(Shed::Popup, |b| {
                let bg = with_alpha(palette::PANEL, alpha);
                b.fill(r, bg);
                let c = with_alpha(palette::INCOME, alpha);
                b.text(&self.plus, x, y, c);
                b.text(&p.num.block, x + i32::from(self.plus.width), y, c);
            });
        }
    }
}

struct Banner {
    level: u8,
    unlocked: bool,
    lines: [DynText; 3],
}

impl Banner {
    fn height(&self) -> u32 {
        if self.unlocked { BANNER_H3 } else { BANNER.h }
    }
}

/// 会社レベルの帯。1 レベルずつ順に出す [01 §5.1]。
#[derive(Default)]
pub(crate) struct BannerQueue {
    items: VecDeque<Banner>,
    head_start: Option<MonoTime>,
}

impl BannerQueue {
    fn duration() -> u64 {
        BANNER_MS.max(sound_ms(sound::JINGLE_COMPANY_UP))
    }

    fn push(&mut self, now: MonoTime, level: u8) {
        if self.items.len() >= BANNER_QUEUE {
            return;
        }
        let mut banner = Banner {
            level,
            unlocked: false,
            lines: Default::default(),
        };
        let n = level.to_string();
        banner.lines[0].set(
            txt::NOTICE_COMPANY_UP,
            &[TextArg {
                name: "n",
                value: &n,
            }],
            FontId::Head16,
            NOTICE_W,
            InputDevice::Keyboard,
        );
        banner.lines[1].set(
            txt::NOTICE_COMPANY_UP2,
            &[],
            FontId::Body12,
            NOTICE_W,
            InputDevice::Keyboard,
        );
        self.items.push_back(banner);
        if self.head_start.is_none() {
            self.head_start = Some(now);
        }
    }

    fn mark_unlocked(&mut self, level: u8) {
        if let Some(b) = self.items.iter_mut().find(|b| b.level == level) {
            b.unlocked = true;
            b.lines[2].set(
                txt::NOTICE_UNLOCKED,
                &[],
                FontId::Body12,
                NOTICE_W,
                InputDevice::Keyboard,
            );
        }
    }

    fn prune(&mut self, now: MonoTime) {
        while let Some(start) = self.head_start {
            if elapsed_ms(now, start) < Self::duration() {
                break;
            }
            self.items.pop_front();
            self.head_start = (!self.items.is_empty()).then_some(now);
        }
    }

    /// 待っている帯の数(出ているものを含む)。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    fn draw(&self, b: &mut DrawBuilder<'_>, now: MonoTime) {
        let (Some(head), Some(start)) = (self.items.front(), self.head_start) else {
            return;
        };
        let t = elapsed_ms(now, start);
        let total = Self::duration();
        let h = head.height();
        let hidden = BANNER.y + h as i32;
        let off = if t < BANNER_SLIDE_MS {
            hidden * (BANNER_SLIDE_MS - t) as i32 / BANNER_SLIDE_MS as i32
        } else if t + BANNER_SLIDE_MS > total {
            hidden * (t + BANNER_SLIDE_MS - total).min(BANNER_SLIDE_MS) as i32
                / BANNER_SLIDE_MS as i32
        } else {
            0
        };
        let r = rect(BANNER.x, BANNER.y - off, BANNER.w, h);
        b.nine_slice(&sprite::NS_UI_WIN, r, palette::WHITE);
        let mut y = r.y + 8;
        for (i, line) in head.lines.iter().enumerate() {
            if i == 2 && !head.unlocked {
                break;
            }
            let x = r.x + (r.w as i32 - i32::from(line.block.width)) / 2;
            let c = if i == 0 {
                palette::MONEY
            } else {
                palette::TEXT
            };
            b.text(&line.block, x, y, c);
            y += i32::from(line.block.line_height);
        }
    }
}

struct Toast {
    ms: u64,
    text: DynText,
}

/// トースト。順に 1 つずつ出す。4 つを超えたら古いものを捨てる。
#[derive(Default)]
pub(crate) struct ToastQueue {
    items: VecDeque<Toast>,
    head_start: Option<MonoTime>,
}

impl ToastQueue {
    fn push(&mut self, now: MonoTime, toast: Toast) {
        if self.items.len() >= TOAST_QUEUE {
            self.items.pop_front();
            self.head_start = Some(now);
        }
        self.items.push_back(toast);
        if self.head_start.is_none() {
            self.head_start = Some(now);
        }
    }

    fn prune(&mut self, now: MonoTime) {
        while let (Some(head), Some(start)) = (self.items.front(), self.head_start) {
            if elapsed_ms(now, start) < head.ms {
                break;
            }
            self.items.pop_front();
            self.head_start = (!self.items.is_empty()).then_some(now);
        }
    }

    /// 待っているトーストの数(出ているものを含む)。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    fn draw(&self, b: &mut DrawBuilder<'_>) {
        let Some(head) = self.items.front() else {
            return;
        };
        b.group(Shed::Toast, |b| {
            b.nine_slice(&sprite::NS_UI_POPUP, TOAST, palette::WHITE);
            let blk = &head.text.block;
            let x = TOAST.x + (TOAST.w as i32 - i32::from(blk.width)) / 2;
            let y = TOAST.y + (TOAST.h as i32 - blk.height()) / 2;
            b.text(blk, x, y, palette::TEXT);
        });
    }
}

struct Goal {
    start: MonoTime,
    ms: u64,
    text: DynText,
}

#[derive(Clone, Copy)]
struct Sparkle {
    train: TrainId,
    start: MonoTime,
}

#[derive(Clone, Copy)]
struct LevelUp {
    scope: UpgradeScope,
    start: MonoTime,
}

/// プレイ中の演出のまとめ。
#[derive(Default)]
pub(crate) struct Anims {
    pub popups: PopupPool,
    pub banners: BannerQueue,
    pub toasts: ToastQueue,
    goal: Option<Goal>,
    glows: Vec<(LineId, MonoTime)>,
    sparkles: Vec<Sparkle>,
    levelups: Vec<LevelUp>,
    /// 出現の演出の駅と、演出を始める時刻(ジングルの終わり)。
    spawns: Vec<(StationId, MonoTime)>,
}

fn push_capped<T>(v: &mut Vec<T>, item: T) {
    if v.len() >= FX_MAX {
        v.remove(0);
    }
    v.push(item);
}

impl Anims {
    /// 出ているポップアップの数(テスト用)。
    #[cfg(test)]
    pub(crate) fn popups_len(&self) -> usize {
        self.popups.len()
    }

    /// 収入。
    pub(crate) fn income(&mut self, now: MonoTime, station: StationId, income: BigNum) {
        if !income.is_zero() {
            self.popups.add(now, station, income);
        }
    }

    /// 会社レベルが上がった。
    pub(crate) fn company_up(&mut self, now: MonoTime, level: u8) {
        self.banners.push(now, level);
    }

    /// 機能が解放された会社レベル(帯を 3 行にする)。
    pub(crate) fn unlocked(&mut self, level: u8) {
        self.banners.mark_unlocked(level);
    }

    /// 所持金が単位 k に届いた。
    pub(crate) fn unit_reached(&mut self, now: MonoTime, unit: u8) {
        let Some(&(kanji, yomi)) = JA_UNITS.get(usize::from(unit)) else {
            return;
        };
        let mut text = DynText::default();
        text.set(
            txt::NOTICE_UNIT,
            &[
                TextArg {
                    name: "unit",
                    value: kanji,
                },
                TextArg {
                    name: "yomi",
                    value: yomi,
                },
            ],
            FontId::Body12,
            NOTICE_W,
            InputDevice::Keyboard,
        );
        self.toasts.push(
            now,
            Toast {
                ms: TOAST_UNIT_MS,
                text,
            },
        );
    }

    /// 通過駅ができた。
    pub(crate) fn pass_through(&mut self, now: MonoTime) {
        let mut text = DynText::default();
        text.set(
            txt::NOTICE_PASS,
            &[],
            FontId::Body12,
            NOTICE_W,
            InputDevice::Keyboard,
        );
        self.toasts.push(
            now,
            Toast {
                ms: TOAST_PASS_MS,
                text,
            },
        );
    }

    /// 転生した。
    pub(crate) fn rebirth(&mut self, now: MonoTime, points: u64) {
        let mut buf = [0u8; 8];
        let mut text = DynText::default();
        text.set(
            txt::NOTICE_REBIRTH,
            &[TextArg {
                name: "n",
                value: decimal(points, &mut buf),
            }],
            FontId::Body12,
            NOTICE_W,
            InputDevice::Keyboard,
        );
        self.toasts.push(
            now,
            Toast {
                ms: TOAST_REBIRTH_MS,
                text,
            },
        );
    }

    /// ゴール。
    pub(crate) fn goal(&mut self, now: MonoTime) {
        let mut text = DynText::default();
        text.set(
            txt::NOTICE_GOAL,
            &[],
            FontId::Head16,
            GOAL_BAND.w as u16 / u16::from(GOAL_SCALE),
            InputDevice::Keyboard,
        );
        text.block.scale(GOAL_SCALE);
        let ms = sound_ms(sound::JINGLE_GOAL).max(BANNER_MS);
        self.goal = Some(Goal {
            start: now,
            ms,
            text,
        });
    }

    /// 路線ができた・延びた。
    pub(crate) fn line_done(&mut self, now: MonoTime, line: LineId) {
        push_capped(&mut self.glows, (line, now));
    }

    /// 駅が出た。始まりの時刻は `defer_spawns` で遅らせた `se_station_new` の時刻に合わせる。
    pub(crate) fn station_spawned(&mut self, now: MonoTime, id: StationId) {
        self.spawns.push((id, now));
    }

    /// まだ始まっていない出現の演出を `until`(最後のジングルの終わり)まで遅らせる [13 §3.12、§5.3]。
    pub(crate) fn defer_spawns(&mut self, now: MonoTime, until: MonoTime) {
        for (_, start) in &mut self.spawns {
            if *start >= now {
                *start = (*start).max(until);
            }
        }
    }

    /// 出現の演出が終わっていない駅(出た順)。静的な層に入れない。
    pub(crate) fn hidden_stations(&self, now: MonoTime) -> impl Iterator<Item = StationId> + '_ {
        self.spawns
            .iter()
            .filter(move |&&(_, s)| now.0 < s.0 + SPAWN_MS * 1000)
            .map(|&(id, _)| id)
    }

    /// 列車が増えた。
    pub(crate) fn train_added(&mut self, now: MonoTime, id: TrainId) {
        push_capped(
            &mut self.sparkles,
            Sparkle {
                train: id,
                start: now,
            },
        );
    }

    /// 強化を買えた。
    pub(crate) fn level_up(&mut self, now: MonoTime, scope: UpgradeScope) {
        push_capped(&mut self.levelups, LevelUp { scope, start: now });
    }

    /// 終わった演出を外す。
    pub(crate) fn prune(&mut self, now: MonoTime) {
        self.popups.prune(now);
        self.banners.prune(now);
        self.toasts.prune(now);
        if self
            .goal
            .as_ref()
            .is_some_and(|g| elapsed_ms(now, g.start) >= g.ms)
        {
            self.goal = None;
        }
        self.glows.retain(|&(_, s)| elapsed_ms(now, s) < GLOW_MS);
        self.sparkles
            .retain(|s| elapsed_ms(now, s.start) < SPARKLE_MS);
        self.levelups
            .retain(|l| elapsed_ms(now, l.start) < LEVELUP_MS);
        self.spawns.retain(|&(_, s)| now.0 < s.0 + SPAWN_MS * 1000);
    }

    /// 完成した路線の光(層 3 の列車より下)。表示域の外の区間は描かない。
    pub(crate) fn draw_glow(
        &self,
        b: &mut DrawBuilder<'_>,
        paths: &[Vec<tm_core::GridPos>],
        now: MonoTime,
        cam: Cam,
        view_w: i32,
    ) {
        for &(line, start) in &self.glows {
            let Some(path) = paths.get(line.index()) else {
                continue;
            };
            let a = scale_alpha(GLOW_MS - elapsed_ms(now, start).min(GLOW_MS), GLOW_MS);
            let base = palette::line_color(line);
            let lift = |v: u8| v + (255 - v) / 2;
            let c = tm_platform_api::Color {
                r: lift(base.r),
                g: lift(base.g),
                b: lift(base.b),
                a,
            };
            for w in path.windows(2) {
                let (x0, y0) = tile_center(w[0], cam);
                let (x1, y1) = tile_center(w[1], cam);
                if !near_view(x0, y0, view_w) && !near_view(x1, y1, view_w) {
                    continue;
                }
                if x0 == x1 || y0 == y1 {
                    let half = crate::draw::map::RAIL_W / 2;
                    let (l, t) = (x0.min(x1) - half, y0.min(y1) - half);
                    b.fill(
                        rect(
                            l,
                            t,
                            (x0 - x1).unsigned_abs() + 2 * half as u32,
                            (y0 - y1).unsigned_abs() + 2 * half as u32,
                        ),
                        c,
                    );
                } else {
                    let w = crate::draw::map::RAIL_W * 3 / 4;
                    let (lo, hi) = (-w / 2, (w + 1) / 2);
                    b.slanted_band((x0, y0), (x1, y1), lo, hi, c);
                }
            }
        }
    }

    /// マップの上の演出(層 3 の人の点より上)。星、矢印、ポップアップ(`popups` は `PopupPool::layout` の結果)。
    /// 表示域の外のものは描かない。
    pub(crate) fn draw_fx(
        &self,
        b: &mut DrawBuilder<'_>,
        view: GameView<'_>,
        now: MonoTime,
        cam: Cam,
        view_w: i32,
        popups: &[Option<Rect>],
    ) {
        for &(id, start) in &self.spawns {
            if now < start {
                continue;
            }
            let Some(st) = view.station(id) else { continue };
            let (x, y) = tile_center(st.pos, cam);
            if !near_view(x, y, view_w) {
                continue;
            }
            let t = elapsed_ms(now, start).min(SPAWN_MS);
            let px = (SPAWN_FROM_PX + (SPAWN_TO_PX - SPAWN_FROM_PX) * t / SPAWN_MS) as i32;
            let body = if st.connected {
                sprite::STN_BODY
            } else {
                sprite::STN_BODY_OFF
            };
            b.push(tm_platform_api::DrawCmd::Sprite {
                id: body,
                dst: rect(x - px / 2, y - px / 2, px as u32, px as u32),
                tint: palette::WHITE,
            });
            let frame = ANIM_FX_SPARKLE.frame_at(elapsed_ms(now, start));
            sparkle(b, frame, x, y);
        }
        for s in &self.sparkles {
            let Some((x, y)) = view
                .train(s.train)
                .map(|t| crate::draw::map::train_screen(t.point, cam))
                .filter(|&(x, y)| near_view(x, y, view_w))
            else {
                continue;
            };
            let frame = ANIM_FX_SPARKLE.frame_at(elapsed_ms(now, s.start));
            sparkle(b, frame, x, y);
        }
        for l in &self.levelups {
            let t = elapsed_ms(now, l.start).min(LEVELUP_MS);
            let rise = 6 * MAP_SCALE * t as i32 / LEVELUP_MS as i32;
            let mut n = 0;
            for_each_target(view, l.scope, |pos| {
                let (x, y) = tile_center(pos, cam);
                if n < LEVELUP_TARGETS && near_view(x, y, view_w) {
                    n += 1;
                    b.map_sprite(
                        sprite::ICO_UP,
                        x - 5 * MAP_SCALE,
                        y - TILE - rise,
                        palette::WHITE,
                    );
                }
            });
        }
        self.popups.draw(b, now, popups);
    }

    /// 画面の上に重ねる演出(層 6)。帯、トースト、紙ふぶき、ゴールの帯。
    pub(crate) fn draw_overlay(&self, b: &mut DrawBuilder<'_>, now: MonoTime) {
        self.banners.draw(b, now);
        self.toasts.draw(b);
        if let Some(g) = &self.goal {
            let t = elapsed_ms(now, g.start);
            for i in 0..CONFETTI_MAX {
                b.group(Shed::Confetti, |b| {
                    let (x, y, c) = confetti_piece(i, t);
                    b.fill(rect(x, y, 4, 4), c);
                });
            }
            let blk = &g.text.block;
            let r = GOAL_BAND;
            b.nine_slice(&sprite::NS_UI_WIN, r, palette::WHITE);
            b.text(
                blk,
                r.x + (r.w as i32 - i32::from(blk.width)) / 2,
                r.y + (r.h as i32 - blk.height()) / 2,
                palette::MONEY,
            );
        }
    }
}

/// 4 つの星を `(x, y)` のまわりに置く。
fn sparkle(b: &mut DrawBuilder<'_>, frame: tm_platform_api::SpriteId, x: i32, y: i32) {
    let d = SPARKLE_AT;
    for (dx, dy) in [(-d, -d), (d, -d), (-d, d), (d, d)] {
        b.map_sprite(frame, x + dx, y + dy, palette::HIGHLIGHT);
    }
}

fn for_each_target(view: GameView<'_>, scope: UpgradeScope, mut f: impl FnMut(tm_core::GridPos)) {
    match scope {
        UpgradeScope::Station(id) => {
            if let Some(st) = view.station(id) {
                f(st.pos);
            }
        }
        UpgradeScope::AllStations => view
            .stations()
            .filter(|s| s.connected)
            .for_each(|s| f(s.pos)),
        UpgradeScope::Line(l) => {
            if let Some(line) = view.line(l) {
                line.stations
                    .iter()
                    .filter_map(|&s| view.station(s))
                    .for_each(|s| f(s.pos));
            }
        }
        UpgradeScope::AllLines => view
            .stations()
            .filter(|s| !s.lines.is_empty())
            .for_each(|s| f(s.pos)),
        UpgradeScope::Company => {}
    }
}

/// 紙ふぶきの 1 片の位置と色。添字と経過時間だけで決まる(乱数を使わない)。
fn confetti_piece(i: u32, t_ms: u64) -> (i32, i32, tm_platform_api::Color) {
    let x0 = (i * 97 + 13) % SCREEN_W as u32;
    let speed = 40 + (i * 37) % 60;
    let y0 = (i * 53) % SCREEN_H as u32;
    let y = (u64::from(y0) + t_ms * u64::from(speed) / 1000) % SCREEN_H as u64;
    let sway = ((t_ms / 100 + u64::from(i)) % 8) as i32 - 4;
    let c = palette::LINES[(i as usize) % palette::LINES.len()];
    (x0 as i32 + sway, y as i32, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> MonoTime {
        MonoTime(ms * 1000)
    }

    #[test]
    fn popups_merge_and_cap() {
        let mut p = PopupPool::default();
        p.add(t(0), StationId(0), BigNum::from_u64(5));
        p.add(t(299), StationId(0), BigNum::from_u64(7));
        assert_eq!(p.len(), 1);
        assert_eq!(p.live[0].num.text, "12円");
        p.add(t(300), StationId(0), BigNum::from_u64(1));
        assert_eq!(p.len(), 2);
        for i in 0..20 {
            p.add(t(400), StationId(i + 1), BigNum::ONE);
        }
        assert_eq!(p.len(), POPUP_MAX);
        assert_eq!(p.live[0].station, StationId(9), "古いものから消す");
        p.prune(t(1199));
        assert_eq!(p.len(), POPUP_MAX);
        p.prune(t(1200));
        assert_eq!(p.len(), 0);
        assert_eq!(p.spare.len(), POPUP_MAX);
        p.add(t(1300), StationId(3), BigNum::ONE);
        assert_eq!(p.spare.len(), POPUP_MAX - 1, "使い回す");
    }

    #[test]
    fn banners_run_one_after_another() {
        let mut q = BannerQueue::default();
        q.push(t(0), 1);
        q.push(t(10), 2);
        q.mark_unlocked(2);
        let d = BannerQueue::duration();
        assert!(d >= BANNER_MS);
        q.prune(t(d - 1));
        assert_eq!(q.len(), 2);
        q.prune(t(d));
        assert_eq!(q.len(), 1);
        assert!(q.items[0].unlocked);
        assert_eq!(q.items[0].height(), BANNER_H3);
        q.prune(t(2 * d - 1));
        assert_eq!(q.len(), 1);
        q.prune(t(2 * d));
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn toasts_keep_the_newest_four() {
        let mut a = Anims::default();
        for k in 1..=6 {
            a.unit_reached(t(0), k);
        }
        assert_eq!(a.toasts.len(), TOAST_QUEUE);
        a.prune(t(TOAST_UNIT_MS));
        assert_eq!(a.toasts.len(), 3);
        a.pass_through(t(TOAST_UNIT_MS));
        assert_eq!(a.toasts.len(), 4);
    }

    #[test]
    fn goal_text_is_head16_twice_and_fits_the_band() {
        let mut a = Anims::default();
        a.goal(t(0));
        let blk = &a.goal.as_ref().unwrap().text.block;
        let mut one = TextBlock::default();
        layout(
            tm_assets::text::text(txt::NOTICE_GOAL),
            FontId::Head16,
            u16::MAX,
            &[],
            InputDevice::Keyboard,
            &mut one,
        );
        assert_eq!(blk.lines, 1);
        assert_eq!(blk.width, 2 * one.width);
        assert_eq!(blk.line_height, 2 * one.line_height);
        assert!(i32::from(blk.width) <= GOAL_BAND.w as i32 - 16);
        assert!(blk.height() <= GOAL_BAND.h as i32 - 8);
    }

    /// 帯の 1 行目は Head16、2・3 行目は Body12。行はそれぞれの行送りで下へ並べる [13 §3.5]。
    #[test]
    fn banner_lines_use_head16_then_body12() {
        use tm_assets::font::face;
        let mut q = BannerQueue::default();
        q.push(t(0), 3);
        q.mark_unlocked(3);
        let mut out = Vec::new();
        let mut b = DrawBuilder::emitting(&mut out, [0; 4]);
        q.draw(&mut b, t(BANNER_SLIDE_MS));
        let banner = &q.items[0];
        let (head, body) = (face(FontId::Head16), face(FontId::Body12));
        let heights: Vec<u8> = banner.lines.iter().map(|l| l.block.line_height).collect();
        assert_eq!(
            heights,
            [head.line_height, body.line_height, body.line_height]
        );
        let first = tm_assets::text::text(txt::NOTICE_COMPANY_UP)
            .chars()
            .find(|c| !c.is_ascii())
            .unwrap();
        let sprite = |f| {
            tm_assets::font::glyph(f, first)
                .unwrap()
                .bitmap
                .unwrap()
                .sprite
        };
        assert_eq!(
            banner.lines[0].block.glyphs[0].sprite,
            sprite(FontId::Head16)
        );
        let glyphs: Vec<Rect> = out
            .iter()
            .skip(9)
            .filter_map(|c| match *c {
                tm_platform_api::DrawCmd::Sprite { dst, .. } => Some(dst),
                _ => None,
            })
            .collect();
        let mut want = Vec::new();
        let mut top = BANNER.y + 8;
        for line in &banner.lines {
            let x = BANNER.x + (BANNER.w as i32 - i32::from(line.block.width)) / 2;
            for g in &line.block.glyphs {
                want.push(rect(
                    x + i32::from(g.x),
                    top + i32::from(g.y),
                    u32::from(g.w),
                    u32::from(g.h),
                ));
            }
            top += i32::from(line.block.line_height);
        }
        assert_eq!(glyphs, want);
        assert_eq!(top + 8, BANNER.y + BANNER_H3 as i32, "3 行で帯の高さ 68");
    }

    #[test]
    fn popups_stay_inside_the_view() {
        use crate::layout::{MAP_W, MAP_W_PANEL};
        for view_w in [MAP_W, MAP_W_PANEL] {
            for w in [6, 60, 150] {
                for x in [-100, 0, view_w / 2, view_w - w, view_w + 100] {
                    let left = clamp_to_map(x, w, view_w);
                    assert!(left >= MAP_X + 2, "{view_w} {w} {x}");
                    assert!(left + w + 2 <= MAP_X + view_w, "{view_w} {w} {x}");
                }
            }
            assert_eq!(clamp_to_map(view_w, 60, view_w), MAP_X + view_w - 2 - 60);
        }
    }

    #[test]
    fn banner_lines_fit_the_band_height() {
        let head = i32::from(tm_assets::font::face(FontId::Head16).line_height);
        let body = i32::from(tm_assets::font::face(FontId::Body12).line_height);
        assert_eq!(BANNER.h as i32, 8 + head + body + 8);
        assert_eq!(BANNER_H3 as i32, 8 + head + 2 * body + 8);
        assert_eq!(TOAST.h as i32, 4 + body + 4);
    }

    #[test]
    fn confetti_stays_on_screen() {
        for i in 0..CONFETTI_MAX {
            for ms in [0, 333, 5000, 123_456] {
                let (x, y, _) = confetti_piece(i, ms);
                assert!((-4..SCREEN_W).contains(&x) && (0..SCREEN_H).contains(&y));
            }
        }
    }
}
