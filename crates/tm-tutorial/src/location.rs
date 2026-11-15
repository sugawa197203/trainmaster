//! UiEvent から追う UI の現在地(13 §6.2 `UiLocation`)。案内の highlight を決めるのに使う。

use tm_core::ParamCategory;

use crate::anchor::MenuAnchor;
use crate::ui_event::UiEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildKind {
    Create,
    Extend,
}

/// ろせんモードのどこにいるか。`Confirm` は「いいえ」で戻る先を持つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildLoc {
    Closed,
    Choose,
    Draft(BuildKind),
    Confirm(BuildKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UiLocation {
    pub(crate) panel_open: bool,
    pub(crate) tab: Option<ParamCategory>,
    pub(crate) build: BuildLoc,
    pub(crate) settings_open: bool,
}

impl UiLocation {
    pub(crate) const fn new() -> Self {
        UiLocation {
            panel_open: false,
            tab: None,
            build: BuildLoc::Closed,
            settings_open: false,
        }
    }

    pub(crate) fn observe(&mut self, ev: &UiEvent) {
        match *ev {
            UiEvent::MenuOpened(m) => self.opened(m),
            UiEvent::MenuClosed(m) => self.closed(m),
            UiEvent::CursorOn(_) | UiEvent::MessageAcknowledged | UiEvent::FanfareFinished => {}
        }
    }

    fn opened(&mut self, m: MenuAnchor) {
        match m {
            MenuAnchor::UpgradePanel => self.panel_open = true,
            MenuAnchor::UpgradeTab(c) => self.tab = Some(c),
            MenuAnchor::BuildMode => self.build = BuildLoc::Choose,
            MenuAnchor::BuildCreate => self.build = BuildLoc::Draft(BuildKind::Create),
            MenuAnchor::BuildExtend => self.build = BuildLoc::Draft(BuildKind::Extend),
            MenuAnchor::LineConfirm => {
                if let BuildLoc::Draft(k) = self.build {
                    self.build = BuildLoc::Confirm(k);
                }
            }
            MenuAnchor::SettingsMenu => self.settings_open = true,
            MenuAnchor::UpgradeItem(_)
            | MenuAnchor::UpgradeRebirth
            | MenuAnchor::ScopeToggle
            | MenuAnchor::BuyMaxHint
            | MenuAnchor::SettingsItem(_)
            | MenuAnchor::ResetConfirm { .. }
            | MenuAnchor::QuitConfirm { .. } => {}
        }
    }

    fn closed(&mut self, m: MenuAnchor) {
        match m {
            MenuAnchor::UpgradePanel => {
                self.panel_open = false;
                self.tab = None;
            }
            MenuAnchor::UpgradeTab(c) => {
                if self.tab == Some(c) {
                    self.tab = None;
                }
            }
            MenuAnchor::BuildMode => self.build = BuildLoc::Closed,
            MenuAnchor::BuildCreate | MenuAnchor::BuildExtend => {
                if self.build != BuildLoc::Closed {
                    self.build = BuildLoc::Choose;
                }
            }
            MenuAnchor::LineConfirm => {
                if let BuildLoc::Confirm(k) = self.build {
                    self.build = BuildLoc::Draft(k);
                }
            }
            MenuAnchor::SettingsMenu => self.settings_open = false,
            MenuAnchor::UpgradeItem(_)
            | MenuAnchor::UpgradeRebirth
            | MenuAnchor::ScopeToggle
            | MenuAnchor::BuyMaxHint
            | MenuAnchor::SettingsItem(_)
            | MenuAnchor::ResetConfirm { .. }
            | MenuAnchor::QuitConfirm { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_bridge_event_order_of_13_3_13() {
        let mut l = UiLocation::new();
        for ev in [
            UiEvent::MenuOpened(MenuAnchor::UpgradePanel),
            UiEvent::MenuOpened(MenuAnchor::UpgradeTab(ParamCategory::Station)),
        ] {
            l.observe(&ev);
        }
        assert!(l.panel_open);
        assert_eq!(l.tab, Some(ParamCategory::Station));
        l.observe(&UiEvent::MenuClosed(MenuAnchor::UpgradeTab(
            ParamCategory::Station,
        )));
        l.observe(&UiEvent::MenuOpened(MenuAnchor::UpgradeTab(
            ParamCategory::Line,
        )));
        assert_eq!(l.tab, Some(ParamCategory::Line));
        l.observe(&UiEvent::MenuClosed(MenuAnchor::UpgradeTab(
            ParamCategory::Line,
        )));
        l.observe(&UiEvent::MenuClosed(MenuAnchor::UpgradePanel));
        assert!(!l.panel_open);
        assert_eq!(l.tab, None);

        l.observe(&UiEvent::MenuOpened(MenuAnchor::BuildMode));
        assert_eq!(l.build, BuildLoc::Choose);
        l.observe(&UiEvent::MenuOpened(MenuAnchor::BuildExtend));
        assert_eq!(l.build, BuildLoc::Draft(BuildKind::Extend));
        l.observe(&UiEvent::MenuOpened(MenuAnchor::LineConfirm));
        assert_eq!(l.build, BuildLoc::Confirm(BuildKind::Extend));
        l.observe(&UiEvent::MenuClosed(MenuAnchor::LineConfirm));
        assert_eq!(l.build, BuildLoc::Draft(BuildKind::Extend));
        l.observe(&UiEvent::MenuClosed(MenuAnchor::BuildMode));
        assert_eq!(l.build, BuildLoc::Closed);

        l.observe(&UiEvent::MenuOpened(MenuAnchor::SettingsMenu));
        assert!(l.settings_open);
        let before = l;
        for step in [1, 2] {
            l.observe(&UiEvent::MenuOpened(MenuAnchor::QuitConfirm { step }));
            l.observe(&UiEvent::MenuClosed(MenuAnchor::QuitConfirm { step }));
            l.observe(&UiEvent::MenuOpened(MenuAnchor::ResetConfirm { step }));
            l.observe(&UiEvent::MenuClosed(MenuAnchor::ResetConfirm { step }));
        }
        assert_eq!(l, before, "確認の開閉は現在地を変えない");
        l.observe(&UiEvent::MenuClosed(MenuAnchor::SettingsMenu));
        assert!(!l.settings_open);
    }
}
