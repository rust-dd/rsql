use super::{PopupMenu, PopupMenuItem};
use crate::ActiveTheme;
use crate::menu::menu_item::MenuItemColors;
use gpui::{App, BoxShadow, SharedString, Styled, black, px};
use std::collections::HashMap;

/// Optional presentation details for an action in a context menu.
#[derive(Default)]
pub struct ContextMenuItemStyle {
    pub destructive: bool,
    pub shortcut: Option<SharedString>,
}

#[derive(Default)]
pub(super) struct ContextAppearance {
    items: HashMap<usize, ContextMenuItemStyle>,
}

impl ContextAppearance {
    pub(super) fn get(&self, index: usize) -> Option<&ContextMenuItemStyle> {
        self.items.get(&index)
    }

    pub(super) fn clear(&mut self) {
        self.items.clear();
    }
}

impl PopupMenu {
    /// Use compact context-menu chrome while retaining the popup's normal interaction.
    pub fn context_menu_appearance(mut self) -> Self {
        self.context_appearance.get_or_insert_with(Default::default);
        self
    }

    /// Add an item with context-menu presentation, keeping its activation handler.
    pub fn context_item(mut self, item: PopupMenuItem, style: ContextMenuItemStyle) -> Self {
        self.context_appearance
            .get_or_insert_with(Default::default)
            .items
            .insert(self.menu_items.len(), style);
        self.item(item)
    }
}

pub(super) fn surface<T: Styled>(element: T, cx: &App) -> T {
    let theme = cx.theme();
    let shadow = black().opacity(0.2);
    element
        // Opaque: the legacy 95% surface relied on a backdrop blur GPUI cannot draw.
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border.opacity(0.8))
        .rounded(px(12.))
        .shadow(vec![
            BoxShadow::new(px(0.), px(20.), shadow)
                .blur_radius(px(25.))
                .spread_radius(px(-5.)),
            BoxShadow::new(px(0.), px(8.), shadow)
                .blur_radius(px(10.))
                .spread_radius(px(-6.)),
        ])
}

pub(super) fn colors(
    item: &PopupMenuItem,
    style: Option<&ContextMenuItemStyle>,
    cx: &App,
) -> MenuItemColors {
    let theme = cx.theme();
    let destructive = style.is_some_and(|item| item.destructive);
    let foreground = if destructive {
        theme.danger
    } else {
        theme.popover_foreground
    };
    MenuItemColors {
        foreground,
        selected_foreground: foreground,
        selected_background: if destructive {
            theme.danger.opacity(0.1)
        } else {
            theme.accent.opacity(0.8)
        }
        .into(),
        disabled_foreground: theme.muted_foreground.opacity(
            if matches!(item, PopupMenuItem::Label(_)) {
                0.6
            } else {
                0.4
            },
        ),
    }
}
