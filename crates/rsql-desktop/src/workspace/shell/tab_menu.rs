use super::*;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};

pub(super) fn commands(
    mut menu: PopupMenu,
    owner: WeakEntity<Workspace>,
    id: String,
    others: bool,
    cx: &App,
) -> PopupMenu {
    let can_copy = owner
        .read_with(cx, |v, cx| {
            v.capture_edits(cx);
            v.session
                .documents
                .borrow()
                .tabs
                .iter()
                .any(|t| t.id == id && t.kind == Kind::Query && !t.main.draft.text.is_empty())
        })
        .unwrap_or(false);
    for (label, icon, include) in [
        ("Close", IconName::X, true),
        ("Close Others", IconName::CircleX, others),
        ("Close All", IconName::Trash, true),
        ("Copy SQL", IconName::Copy, can_copy),
    ] {
        if !include {
            continue;
        }
        let id = id.clone();
        let owner = owner.clone();
        let action = PopupMenuItem::new(label)
            .icon(icon)
            .on_click(move |_, w, cx| {
                let _ = owner.update(cx, |v, cx| {
                    if label == "Copy SQL" {
                        v.capture_edits(cx);
                        if let Some(t) = v
                            .session
                            .documents
                            .borrow()
                            .tabs
                            .iter()
                            .find(|t| t.id == id)
                        {
                            clipboard_order::ClipboardWrite::begin(cx);
                            crate::desktop::DesktopServices::write_clipboard(
                                cx,
                                ClipboardItem::new_string(t.main.draft.text.clone()),
                            );
                        }
                    } else {
                        v.update_documents(w, cx, |d| {
                            if d.tabs.iter().any(|tab| tab.id == id) {
                                match label {
                                    "Close" => d.close(&id),
                                    "Close Others" => d.close_others(&id),
                                    _ => d.close_all(),
                                }
                            }
                        });
                    }
                });
            });
        if label == "Copy SQL" {
            menu = menu.separator();
        }
        menu = menu.item(action);
    }
    menu
}
