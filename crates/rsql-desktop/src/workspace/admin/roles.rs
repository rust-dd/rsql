use super::*;
use gpui_kit::component::label::Label;
use theme::controls::role_superuser_color;
use view::{badge, heading};

fn identity(role: &PgRole, cx: &App) -> Icon {
    let (icon, color) = if role.superuser {
        (IconName::ShieldCheck, role_superuser_color())
    } else if role.login {
        (IconName::User, cx.theme().primary)
    } else {
        (IconName::Users, cx.theme().muted_foreground)
    };
    Icon::new(icon).size(px(14.)).text_color(color)
}

impl Panel {
    pub(super) fn roles(&self, cx: &Context<Self>) -> Div {
        let roles = match &self.rows {
            Some(Rows::Roles(rows)) => rows.as_slice(),
            _ => &[],
        };
        let selected = self.grants.selected.as_ref();
        let mut list = div()
            .id("role-list")
            .test_support()
            .w(px(240.))
            .flex_shrink_0()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1()
                    .child(heading(
                        "role-count",
                        &format!("Roles ({})", roles.len()),
                        cx,
                    ))
                    .child(self.refresh_button(cx)),
            )
            .children(self.status(Lane::Main, &self.main, cx));
        for role in roles {
            let name = role.name.clone();
            let active = selected == Some(&name);
            list = list.child(
                compact_button(Button::new(SharedString::from(format!("role-{name}"))))
                    .ghost()
                    .w_full()
                    .justify_start()
                    .accessibility_label(name.clone())
                    .when(active, |d| {
                        d.bg(cx.theme().primary.opacity(0.1))
                            .text_color(cx.theme().primary)
                    })
                    .child(identity(role, cx))
                    .child(Label::new(name.clone()).text_color(if active {
                        cx.theme().primary
                    } else {
                        cx.theme().foreground
                    }))
                    .when(role.superuser, |d| {
                        d.child(badge("superuser-badge", "SUPER", role_superuser_color()))
                    })
                    .on_click(cx.listener(move |p, _, _, cx| p.select_role(name.clone(), cx))),
            );
        }
        if roles.is_empty() && self.main.ready() {
            list = list.child(Label::new("No roles found"));
        }
        let detail = div()
            .id("role-detail")
            .min_w_0()
            .flex_1()
            .p_4()
            .overflow_y_scroll();
        let detail = match roles.iter().find(|r| Some(&r.name) == selected) {
            Some(r) => detail.child(self.role_detail(r, cx)),
            None => detail.flex().items_center().justify_center().child(
                Label::new("Select a role").text_color(cx.theme().muted_foreground.opacity(0.4)),
            ),
        };
        div().size_full().flex().child(list).child(detail)
    }

    fn role_detail(&self, r: &PgRole, cx: &Context<Self>) -> impl IntoElement {
        let mut kind = if r.login { "Login role" } else { "Group role" }.to_owned();
        if r.conn_limit >= 0 {
            kind.push_str(&format!(" (max {} connections)", r.conn_limit));
        }
        let attributes = [
            ("SUPERUSER", r.superuser),
            ("CREATEDB", r.create_db),
            ("CREATEROLE", r.create_role),
            ("LOGIN", r.login),
            ("REPLICATION", r.replication),
            ("BYPASSRLS", r.bypass_rls),
        ];
        let mut detail = div()
            .id("role-content")
            .test_support()
            .max_w(px(800.))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(40.))
                            .rounded_xl()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(if r.superuser {
                                role_superuser_color()
                            } else {
                                cx.theme().primary
                            }
                            .opacity(0.1))
                            .child(
                                Icon::new(if r.superuser {
                                    IconName::ShieldCheck
                                } else {
                                    IconName::Shield
                                })
                                .size(px(20.))
                                .text_color(if r.superuser {
                                    role_superuser_color()
                                } else {
                                    cx.theme().primary
                                }),
                            ),
                    )
                    .child(
                        div()
                            .child(
                                Label::new(r.name.clone())
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(Label::new(kind).text_color(cx.theme().muted_foreground)),
                    ),
            )
            .child(heading("attributes-heading", "Attributes", cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(attributes.map(|(name, on)| {
                        let color = if on {
                            cx.theme().primary
                        } else {
                            cx.theme().muted_foreground.opacity(0.4)
                        };
                        div()
                            .rounded_full()
                            .px_2()
                            .py(px(2.))
                            .bg(if on {
                                cx.theme().primary.opacity(0.1)
                            } else {
                                cx.theme().muted.opacity(0.3)
                            })
                            .border_1()
                            .border_color(
                                if on {
                                    cx.theme().primary
                                } else {
                                    cx.theme().border
                                }
                                .opacity(0.2),
                            )
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Icon::new(if on {
                                    IconName::ShieldCheck
                                } else {
                                    IconName::ShieldX
                                })
                                .size(px(10.))
                                .text_color(color),
                            )
                            .child(
                                Label::new(name)
                                    .id(name)
                                    .text_size(px(10.))
                                    .text_color(color),
                            )
                    })),
            );
        if !r.member_of.is_empty() {
            detail = detail
                .child(heading("memberships-heading", "Member of", cx))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(r.member_of.iter().map(|name| {
                            let name = name.clone();
                            compact_button(Button::new(SharedString::from(format!(
                                "membership-{name}"
                            ))))
                            .ghost()
                            .h(px(22.))
                            .rounded_full()
                            .bg(cx.theme().accent.opacity(0.5))
                            .icon(Icon::new(IconName::Users).size(px(10.)))
                            .label(name.clone())
                            .on_click(
                                cx.listener(move |p, _, _, cx| p.select_role(name.clone(), cx)),
                            )
                        })),
                );
        }
        detail = detail.child(self.grant_sections(cx));
        if !r.valid_until.is_empty() {
            detail = detail.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::Key).size(px(12.)))
                    .child(
                        Label::new(format!("Password valid until: {}", r.valid_until))
                            .text_color(cx.theme().muted_foreground),
                    ),
            );
        }
        detail
    }
}
