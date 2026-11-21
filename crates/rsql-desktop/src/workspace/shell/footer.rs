use super::*;
use crate::runtime::resources::Snapshot;
use crate::workspace::numbers::fixed;
use rsql_core::metrics::SystemResourceUsage;
use rsql_desktop::connection::ConnectionProfile;

fn label(id: impl Into<ElementId>, text: impl Into<SharedString>, color: Hsla) -> Label {
    Label::new(text)
        .id(id)
        .text_color(color)
        .text_size(px(11.))
        .line_height(px(16.))
}

fn connection(project: Option<&ConnectionProfile>, status: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let Some(p) = project else {
        return label(
            "footer-connection-name",
            "No connection",
            theme.muted_foreground,
        )
        .into_any_element();
    };
    let tone = match status {
        "Connected" => Some(theme.success),
        "Connecting" => Some(theme.warning),
        "Failed" => Some(theme.danger),
        _ => None,
    };
    let foreground = tone.unwrap_or(theme.muted_foreground);
    div()
        .id("footer-connection")
        .test_support()
        .flex()
        .min_w_0()
        .max_w(relative(0.25))
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(2.))
        .rounded_full()
        .when_some(tone, |d, c| d.bg(c.opacity(0.1)))
        .child(
            div()
                .id("footer-connection-dot")
                .test_support()
                .size(px(6.))
                .flex_shrink_0()
                .rounded_full()
                .bg(tone.unwrap_or(theme.muted)),
        )
        .child(
            label(
                "footer-connection-name",
                p.display_name().to_owned(),
                foreground,
            )
            .min_w_0()
            .truncate(),
        )
        .child(label(
            "footer-connection-bullet",
            "•",
            foreground.opacity(0.5),
        ))
        .child(
            label("footer-database", p.database.clone(), foreground)
                .min_w_0()
                .truncate(),
        )
        .into_any_element()
}

fn memory(mb: u64) -> String {
    if mb < 1000 {
        format!("{mb} MB")
    } else {
        format!("{} GB", fixed(mb as f64 / 1000., 2))
    }
}

fn source_number(value: f32) -> f64 {
    // The web footer parses the shortest JSON decimal of the sampled f32 as a Number.
    let decimal =
        serde_json::to_string(&value).expect("f32 serialization to a String is infallible");
    decimal.parse::<f64>().unwrap_or(f64::from(value))
}

fn rate(mbps: f32) -> String {
    let mbps = source_number(mbps);
    if mbps < 1000. {
        format!("{} Mbps", fixed(mbps, 2))
    } else {
        format!("{} Gbps", fixed(mbps / 1000., 2))
    }
}

fn values(usage: Option<&SystemResourceUsage>) -> [String; 5] {
    let Some(u) = usage else {
        return std::array::from_fn(|_| "--".into());
    };
    let mut connections = format!("{}/{}", u.db_connections_in_use, u.db_connections_open);
    if u.db_connections_waiting > 0 {
        connections.push_str(&format!(" ({} wait)", u.db_connections_waiting));
    }
    [
        format!("{}%", fixed(source_number(u.app_cpu_percent), 1)),
        memory(u.app_memory_rss_mb),
        u.app_process_count.to_string(),
        connections,
        format!(
            "↓ {} ↑ {}",
            rate(u.network_rx_mbps),
            rate(u.network_tx_mbps)
        ),
    ]
}

pub(super) fn render(
    project: Option<&ConnectionProfile>,
    status: &str,
    resources: Snapshot,
    history: String,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let fields = [
        ("footer-cpu", "APP CPU"),
        ("footer-rss", "RSS"),
        ("footer-processes", "PROC"),
        ("footer-pools", "CONN"),
        ("footer-network", "NET"),
    ];
    let mut readout = div()
        .id("footer-resources")
        .role(Role::Group)
        .aria_label("System resources")
        .test_support()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .overflow_x_scroll()
        .when_some(resources.error, |d, e| d.aria_description(e));
    for (index, ((id, name), value)) in fields
        .into_iter()
        .zip(values(resources.usage.as_ref()))
        .enumerate()
    {
        if index != 0 {
            readout = readout
                .child(label(("footer-separator", index), "•", muted.opacity(0.4)).flex_shrink_0());
        }
        readout = readout.child(label(id, format!("{name} {value}"), muted).flex_shrink_0());
    }
    div()
        .id("resource-footer")
        .test_support()
        .h(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .px(px(12.))
        .bg(theme.group_box.opacity(0.6))
        .whitespace_nowrap()
        .child(connection(project, status, cx))
        .child(readout)
        .child(label("footer-history-status", history, muted).flex_shrink_0())
}

pub(super) fn errors(save: Option<String>, history: Option<String>, cx: &App) -> Option<Div> {
    if save.is_none() && history.is_none() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_col()
            .px(px(12.))
            .bg(cx.theme().group_box)
            .children(save.map(|e| label("footer-save-error", e, cx.theme().danger)))
            .children(history.map(|e| label("footer-history-error", e, cx.theme().danger))),
    )
}
