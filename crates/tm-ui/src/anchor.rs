//! `UiAnchor` → 画面の矩形。見えていないものは、そこへ行く入口に置き換える [13 §3.13]。

use tm_core::{GameView, GridPos, LineId};
use tm_platform_api::Rect;
use tm_tutorial::{MenuAnchor, UiAnchor};

use crate::draw::map::train_screen;
use crate::draw::rect;
use crate::hud::{Band, Hud};
use crate::layout::{
    Cam, MAP_H, MAP_SCALE, MAP_X, MAP_Y, TILE, tile_in_view, tile_ring, tile_screen,
};
use crate::scene::{BuildState, PanelState, SettingsMenu};

/// 表示域の外の対象を指す矢印の向き。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toward {
    /// 左。
    Left,
    /// 右。
    Right,
    /// 上。
    Up,
    /// 下。
    Down,
}

/// 縁の矢印を、表示域の角からこれだけ離す。
const EDGE_IN: i32 = TILE / 2;
/// 列車の枠の半分の大きさ。
const TRAIN_HALF: i32 = 10 * MAP_SCALE;

/// highlight の置き方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// 黄色い枠と、上下にはねる矢印。
    Frame(Rect),
    /// マップの上の駅や列車。`Frame` と同じ印を、マップの絵と同じ `MAP_SCALE` 倍の太さで描く。
    Tile(Rect),
    /// 黄色い枠と、横にはねる矢印(上に余地のない HUD と、行が詰まった せってい)。
    Side(Rect),
    /// 路線全体を光らせ、真ん中のマス(`mid`)に矢印。
    Line { line: LineId, mid: Rect },
    /// 表示域の外の駅や列車。表示域の縁の `(x, y)` に、対象のほうを向く矢印を出す。
    Edge { x: i32, y: i32, toward: Toward },
}

impl Mark {
    /// うすい黒の穴にする枠の矩形と、枠(`ui_sel`)を描く倍率。
    pub(crate) fn hole(self) -> Option<(Rect, i32)> {
        match self {
            Mark::Frame(r) | Mark::Side(r) => Some((r, 1)),
            Mark::Tile(r) => Some((r, crate::layout::MAP_SCALE)),
            Mark::Line { .. } | Mark::Edge { .. } => None,
        }
    }

    /// 縦の中心(矢印を含む)。説明の箱を上下のどちらに出すかに使う。
    pub(crate) fn center_y(self) -> i32 {
        match self {
            Mark::Frame(r) | Mark::Tile(r) | Mark::Side(r) | Mark::Line { mid: r, .. } => {
                r.y + r.h as i32 / 2
            }
            Mark::Edge { y, .. } => y,
        }
    }
}

/// 画面の矩形 `r`(表示域から出ている対象)へ向かう、表示域の縁の矢印。はみ出しの大きい軸の向きにする。
pub(crate) fn edge_toward(r: Rect, map_w: i32) -> Mark {
    let over = |lo: i32, hi: i32, vlo: i32, vhi: i32| {
        if lo < vlo {
            lo - vlo
        } else if hi > vhi {
            hi - vhi
        } else {
            0
        }
    };
    let (left, right, top, bottom) = (MAP_X, MAP_X + map_w, MAP_Y, MAP_Y + MAP_H);
    let dx = over(r.x, r.x + r.w as i32, left, right);
    let dy = over(r.y, r.y + r.h as i32, top, bottom);
    let (cx, cy) = (r.x + r.w as i32 / 2, r.y + r.h as i32 / 2);
    if dx.abs() >= dy.abs() {
        Mark::Edge {
            x: if dx < 0 { left } else { right },
            y: cy.clamp(top + EDGE_IN, bottom - EDGE_IN),
            toward: if dx < 0 { Toward::Left } else { Toward::Right },
        }
    } else {
        Mark::Edge {
            x: cx.clamp(left + EDGE_IN, right - EDGE_IN),
            y: if dy < 0 { top } else { bottom },
            toward: if dy < 0 { Toward::Up } else { Toward::Down },
        }
    }
}

/// マスの画面の矩形。
pub(crate) fn tile_rect(pos: GridPos, cam: Cam) -> Rect {
    let (x, y) = tile_screen(pos, cam);
    rect(x, y, TILE as u32, TILE as u32)
}

/// 解決に使う画面の部品。
pub(crate) struct Places<'a> {
    pub view: GameView<'a>,
    pub cam: Cam,
    /// マップの表示域の幅(パネルを開くと狭くなる)。
    pub map_w: i32,
    /// 路線ごとの通るマス(`MapCache::paths`)。
    pub paths: &'a [Vec<GridPos>],
    pub panel: Option<&'a PanelState>,
    pub build: Option<&'a BuildState>,
    pub menu: Option<&'a SettingsMenu>,
    pub band: &'a Band,
    pub hud: &'a Hud,
}

impl Places<'_> {
    fn at(&self, pos: GridPos) -> Mark {
        if tile_in_view(pos, self.cam, self.map_w) {
            Mark::Tile(tile_ring(pos, self.cam))
        } else {
            edge_toward(tile_rect(pos, self.cam), self.map_w)
        }
    }

    /// 下の帯のチップ。ろせんモードでチップがなければ、モードを出る入口(選択肢の箱か「もどる」)。
    fn chip(&self, build: bool) -> Option<Mark> {
        self.band
            .chip_rect(build)
            .or_else(|| match self.build {
                Some(b) if b.confirm.is_some() => None,
                _ => self
                    .build
                    .and_then(|b| b.part_rect(MenuAnchor::BuildMode))
                    .or_else(|| self.band.hint_rect()),
            })
            .map(Mark::Frame)
    }

    fn panel_part(&self, m: MenuAnchor) -> Option<Mark> {
        match self.panel {
            Some(p) => p.part_rect(m).map(Mark::Frame),
            None => self.chip(false),
        }
    }

    fn menu(&self, m: MenuAnchor) -> Option<Mark> {
        match m {
            MenuAnchor::UpgradePanel
            | MenuAnchor::UpgradeTab(_)
            | MenuAnchor::UpgradeRebirth
            | MenuAnchor::ScopeToggle
            | MenuAnchor::BuyMaxHint => self.panel_part(m),
            MenuAnchor::UpgradeItem(p) => match self.panel {
                Some(panel) => panel
                    .part_rect(m)
                    .or_else(|| panel.part_rect(MenuAnchor::UpgradeTab(p.category())))
                    .map(Mark::Frame),
                None => self.chip(false),
            },
            MenuAnchor::BuildMode | MenuAnchor::BuildCreate | MenuAnchor::BuildExtend => {
                match self.build.and_then(|b| b.part_rect(m)) {
                    Some(r) => Some(Mark::Frame(r)),
                    None => self.chip(true),
                }
            }
            MenuAnchor::LineConfirm => self.build?.part_rect(m).map(Mark::Frame),
            MenuAnchor::SettingsMenu
            | MenuAnchor::SettingsItem(_)
            | MenuAnchor::ResetConfirm { .. }
            | MenuAnchor::QuitConfirm { .. } => self.menu?.part_rect(m).map(Mark::Side),
        }
    }

    /// `a` をいまの画面の上の印にする。置けなければ `None`。
    pub(crate) fn resolve(&self, a: UiAnchor) -> Option<Mark> {
        match a {
            UiAnchor::Station(id) => self.view.station(id).map(|s| self.at(s.pos)),
            UiAnchor::Train(id) => {
                let t = self.view.train(id)?;
                let (x, y) = train_screen(t.point, self.cam);
                let r = rect(
                    x - TRAIN_HALF,
                    y - TRAIN_HALF,
                    2 * TRAIN_HALF as u32,
                    2 * TRAIN_HALF as u32,
                );
                let inside = r.x >= MAP_X
                    && r.x + r.w as i32 <= MAP_X + self.map_w
                    && r.y >= MAP_Y
                    && r.y + r.h as i32 <= MAP_Y + MAP_H;
                Some(if inside {
                    Mark::Tile(r)
                } else {
                    edge_toward(r, self.map_w)
                })
            }
            UiAnchor::Line(id) => {
                let path = self.paths.get(id.index()).filter(|p| !p.is_empty())?;
                Some(match self.at(path[path.len() / 2]) {
                    Mark::Tile(mid) => Mark::Line { line: id, mid },
                    edge => edge,
                })
            }
            UiAnchor::Menu(m) => self.menu(m),
            UiAnchor::MoneyDisplay
            | UiAnchor::IncomeDisplay
            | UiAnchor::CompanyLevel
            | UiAnchor::CompanyGauge => self.hud.part_rect(a).map(Mark::Side),
            UiAnchor::RecommendMark => match self.panel {
                Some(p) => p.rec_rect().map(Mark::Frame),
                None => self.chip(false),
            },
        }
    }
}
