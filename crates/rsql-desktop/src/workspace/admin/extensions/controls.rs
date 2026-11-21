use super::*;
use gpui_kit::base::{Button as ActionButton, TestTextExt};
use gpui_kit::component::tooltip::Tooltip;

impl Panel {
    pub(in crate::workspace::admin) fn extension_button(
        &self,
        row: &Extension,
        action: ExtensionAction,
        cx: &Context<Self>,
    ) -> ActionButton {
        let (id, icon, label) = match action {
            ExtensionAction::Update => (
                "extension-update",
                IconName::CircleArrowUp,
                format!("Update to {}", row.update_version().unwrap_or_default()),
            ),
            ExtensionAction::Drop => (
                "extension-drop",
                IconName::Trash,
                format!("Drop {}", row.name),
            ),
            ExtensionAction::Install => ("extension-install", IconName::Download, "Install".into()),
        };
        let name = row.name.clone();
        let ready = if action == ExtensionAction::Install {
            self.extensions.available_section.ready()
        } else {
            self.main.ready()
        };
        let (foreground, hover_foreground, hover_background) =
            theme::controls::extension_action_colors(action, cx);
        let ring = cx.theme().ring;
        ActionButton::new(id)
            .h(px(24.))
            .px_2()
            .gap_1()
            .rounded(px(8.))
            .text_size(px(10.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(foreground)
            .hover(|s| s.bg(hover_background).text_color(hover_foreground))
            .focus_visible(|s| {
                s.shadow(vec![
                    BoxShadow::new(px(0.), px(0.), ring).spread_radius(px(2.)),
                ])
            })
            .when(action == ExtensionAction::Install, |b| {
                b.border_1().border_color(cx.theme().border)
            })
            .when(action == ExtensionAction::Drop, |b| {
                let tooltip = label.clone();
                b.w(px(24.))
                    .p_0()
                    .tooltip(move |w, cx| Tooltip::new(tooltip.clone()).build(w, cx))
            })
            .disabled(
                !ready || self.extensions.running.is_some() || self.extensions.review.is_some(),
            )
            .styles(|s| s.disabled(|s| s.opacity(0.5)))
            .accessibility_label(label.clone())
            .child(Icon::new(icon).size(px(12.)))
            .when(action != ExtensionAction::Drop, |b| {
                b.child(div().child(label.clone()).test_text(label))
            })
            .on_click(cx.listener(move |p, _, w, cx| p.choose_extension(action, &name, w, cx)))
    }
}
