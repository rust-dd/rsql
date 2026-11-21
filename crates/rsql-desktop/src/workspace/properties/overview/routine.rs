use super::super::view::layout::code_text;
use super::*;
use rsql_desktop::metadata::ObjectKind;

impl Panel {
    pub(in crate::workspace::properties) fn object_overview(
        &self,
        p: &Properties,
        fields: &Fields,
        cx: &App,
    ) -> Div {
        let kind = p.object.kind;
        let summary: &[(&str, &str, IconName)] = match kind {
            ObjectKind::View => &[
                ("is updatable", "Updatable", IconName::Eye),
                ("check option", "Check Option", IconName::Shield),
            ],
            ObjectKind::MaterializedView => &[
                ("row estimate", "Rows (est.)", IconName::Database),
                ("total size", "Total Size", IconName::HardDrive),
                ("is populated", "Populated", IconName::Check),
            ],
            _ => &[
                ("language", "Language", IconName::FileCode),
                ("volatility", "Volatility", IconName::RefreshCw),
                ("return type", "Returns", IconName::ArrowRight),
            ],
        };
        let mut result = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(cards(summary, fields, cx));
        if !matches!(kind, ObjectKind::View | ObjectKind::MaterializedView) {
            let security = value(fields, "security definer");
            let strict = value(fields, "is strict");
            result = result.child(info_grid(
                [
                    (
                        "Security",
                        match security.as_ref() {
                            "true" => "SECURITY DEFINER".into(),
                            "false" => "SECURITY INVOKER".into(),
                            _ => security,
                        },
                    ),
                    (
                        "Strict",
                        match strict.as_ref() {
                            "true" => "YES (RETURNS NULL ON NULL INPUT)".into(),
                            "false" => "NO".into(),
                            _ => strict,
                        },
                    ),
                    ("Est. Cost", value(fields, "estimated cost")),
                    ("Est. Rows", value(fields, "estimated rows")),
                ],
                cx,
            ));
        }
        if let Some(arguments) = field(fields, "arguments").filter(|a| !a.full.is_empty()) {
            let surface = code_surface(cx)
                .px(px(12.))
                .py(px(8.))
                .child(code_text(arguments, cx));
            let body = self.code_box(p, "arguments", arguments, None, surface, cx);
            result = result.child(section("Arguments", IconName::Columns3, body, cx));
        }
        let definition = if kind == ObjectKind::View {
            "View Definition"
        } else {
            "Definition"
        };
        for (key, label, copy, height) in [
            ("source", "Source Code", "Copy source", 250.),
            ("definition", definition, "Copy definition", 200.),
        ] {
            if let Some(text) = field(fields, key) {
                let surface = code_surface(cx)
                    .id(key)
                    .max_h(px(height))
                    .overflow_y_scroll()
                    .p(px(16.))
                    .child(code_text(text, cx));
                let body = self.code_box(p, key, text, Some(copy), surface, cx);
                result = result.child(section(label, IconName::FileCode, body, cx));
            }
        }
        result
    }

    /// The source overlays its copy control on the code; abbreviated text gets one too.
    fn code_box(
        &self,
        p: &Properties,
        key: &'static str,
        text: &PropertyText,
        copy: Option<&'static str>,
        surface: impl IntoElement,
        cx: &App,
    ) -> Div {
        let label = copy.or_else(|| text.abbreviated().then_some("Copy full text"));
        let copied = p.copied.is_some_and(|(_, c)| c == Copied::Code(key));
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        div()
            .relative()
            .child(surface)
            .when_some(label, |d, label| {
                let full = text.full.clone();
                d.child(
                    self.press(label, move |v, w, cx| {
                        v.copy_property_code(full.clone(), key, w, cx)
                    })
                    .accessibility_label(label)
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .text_color(muted)
                    .hover(move |s| s.text_color(foreground))
                    .child(if copied {
                        icon(IconName::Check, 14., cx.theme().success)
                    } else {
                        gpui_kit::component::Icon::new(IconName::Copy).size(px(14.))
                    }),
                )
            })
    }
}

fn code_surface(cx: &App) -> Div {
    div()
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.3))
        .bg(cx.theme().background)
}
