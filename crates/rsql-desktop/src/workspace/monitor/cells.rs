use super::*;
use gpui_kit::{
    component::{ActiveTheme, label::Label},
    prelude::FluentBuilder,
};
use view::color;

use crate::workspace::numbers::fixed;
use gpui_kit::component::theme::oklch;

#[derive(Debug, PartialEq)]
enum Style {
    Plain,
    Secondary,
    Badge(Tone),
    Bloat { tone: Tone, percent: Option<f64> },
}

pub(super) struct Cell {
    text: String,
    style: Style,
}

fn bloat_colors(tone: Tone, cx: &App) -> (Hsla, Hsla) {
    let dark = cx.theme().is_dark();
    match tone {
        Tone::Danger => (
            oklch(0.637, 0.237, 25.331),
            if dark {
                oklch(0.704, 0.191, 22.216)
            } else {
                oklch(0.577, 0.245, 27.325)
            },
        ),
        Tone::Warning => (
            oklch(0.795, 0.184, 86.047),
            if dark {
                oklch(0.852, 0.199, 91.936)
            } else {
                oklch(0.681, 0.162, 75.834)
            },
        ),
        Tone::Good => (oklch(0.723, 0.219, 149.579), cx.theme().muted_foreground),
        Tone::Neutral => (cx.theme().muted, cx.theme().muted_foreground),
    }
}

impl Cell {
    fn plain(text: &str) -> Self {
        Self {
            text: text.into(),
            style: Style::Plain,
        }
    }

    fn secondary(text: &str) -> Self {
        Self {
            text: text.into(),
            style: Style::Secondary,
        }
    }

    fn status(text: &str, tone: Tone) -> Self {
        Self {
            text: text.into(),
            style: Style::Badge(tone),
        }
    }

    pub(super) fn element(self, column: usize, width: f32, cx: &App) -> Stateful<Div> {
        let mut cell = div()
            .id(("monitor-cell", column))
            .w(px(width))
            .flex_shrink_0()
            .px_2()
            .flex()
            .items_center()
            .overflow_hidden();
        let mut wrapper = div().min_w_0().truncate();
        let mut label = Label::new(self.text).truncate();
        match self.style {
            Style::Plain => {}
            Style::Secondary => {
                label = label
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
            }
            Style::Badge(tone) => {
                let color = color(tone, cx);
                label = label.text_color(color);
                wrapper = wrapper.bg(color.opacity(0.1)).rounded_sm().px_1();
            }
            Style::Bloat { tone, percent } => {
                let (fill, foreground) = bloat_colors(tone, cx);
                label = label
                    .text_size(px(10.))
                    .text_color(foreground)
                    .when(tone == Tone::Danger, |l| l.font_weight(FontWeight::MEDIUM));
                if let Some(pct) = percent {
                    cell = cell.gap_2().child(
                        div()
                            .w(px(64.))
                            .h(px(8.))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .rounded_full()
                            .bg(cx.theme().muted)
                            .child(
                                div()
                                    .w(relative((pct / 100.) as f32))
                                    .h_full()
                                    .rounded_full()
                                    .bg(fill),
                            ),
                    );
                }
            }
        }
        cell.child(wrapper.child(label))
    }
}

fn date(text: &str) -> String {
    chrono::DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f%#z").map_or_else(
        |_| text.to_owned(),
        |dt| dt.with_timezone(&chrono::Local).format("%x").to_string(),
    )
}

fn fallback(text: &str) -> &str {
    if text.is_empty() { "-" } else { text }
}

impl Rows {
    pub(super) fn headers(&self) -> &'static [&'static str] {
        match self {
            Self::Activity(_) => &[
                "PID", "User", "State", "Duration", "Wait", "Backend", "Client", "Query",
            ],
            Self::Tables(_) => &[
                "Schema",
                "Table",
                "Seq Scan",
                "Idx Scan",
                "Live Tuples",
                "Dead Tuples",
                "Inserts",
                "Updates",
                "Deletes",
                "Last Vacuum",
                "Last Analyze",
            ],
            Self::Locks(_) => &[
                "PID",
                "User",
                "Mode",
                "Lock Type",
                "Status",
                "Relation",
                "Duration",
                "Query",
            ],
            Self::Indexes(_) => &[
                "Schema",
                "Table",
                "Index",
                "Size",
                "Scans",
                "Status",
                "Definition",
            ],
            Self::Bloat(_) => &[
                "Schema",
                "Table",
                "Live Tuples",
                "Dead Tuples",
                "Bloat %",
                "Total Size",
                "Last Vacuum",
                "Last Analyze",
            ],
            Self::Stats(_) => &[],
        }
    }

    pub(super) fn count_and_empty(&self) -> (usize, &'static str) {
        match self {
            Self::Activity(r) => (r.len(), "No active connections"),
            Self::Tables(r) => (r.len(), "No table stats available"),
            Self::Locks(r) => (r.rows.len(), "No active locks"),
            Self::Indexes(r) => (r.rows.len(), "No non-primary indexes found"),
            Self::Bloat(r) => (r.rows.len(), "No table bloat data available"),
            Self::Stats(r) => (r.len(), "No stats available"),
        }
    }

    pub(super) fn note(&self) -> Option<(&SharedString, Tone)> {
        static TABLES: SharedString = SharedString::new_static(
            "Cumulative stats since server start or last pg_stat_reset(). Source: pg_stat_user_tables",
        );
        let note = match self {
            Self::Tables(_) => return Some((&TABLES, Tone::Neutral)),
            Self::Locks(r) => &r.note,
            Self::Indexes(r) => &r.note,
            Self::Bloat(r) => &r.note,
            _ => return None,
        };
        note.as_ref().map(|text| (text, Tone::Warning))
    }

    pub(super) fn details(&self, i: usize) -> Option<Vec<(&'static str, &str)>> {
        Some(match self {
            Self::Activity(r) => r.get(i)?.details(),
            Self::Tables(r) => r.get(i)?.details(),
            Self::Locks(r) => r.rows.get(i)?.details(),
            Self::Indexes(r) => r.rows.get(i)?.details(),
            Self::Bloat(r) => r.rows.get(i)?.details(),
            Self::Stats(_) => return None,
        })
    }

    pub(super) fn cells(&self, i: usize) -> Option<Vec<Cell>> {
        let c = Cell::plain;
        let status = Cell::status;
        Some(match self {
            Self::Activity(r) => {
                let r = r.get(i)?;
                let tone = match r.state.as_str() {
                    "active" => Tone::Good,
                    "idle in transaction" => Tone::Warning,
                    _ => Tone::Neutral,
                };
                vec![
                    c(&r.pid),
                    c(&r.user),
                    status(&r.state, tone),
                    c(&seconds(&r.seconds)),
                    Cell::secondary(fallback(&r.wait_event)),
                    Cell::secondary(&r.backend),
                    Cell::secondary(&r.client),
                    Cell::secondary(&r.query),
                ]
            }
            Self::Tables(r) => {
                let r = r.get(i)?;
                let dead = if r.dead_warning() {
                    let pct = r.dead.parse::<f64>().ok()? / r.live.parse::<f64>().ok()? * 100.;
                    status(&format!("{} ({}%)", r.dead, fixed(pct, 0)), Tone::Danger)
                } else {
                    c(&r.dead)
                };
                vec![
                    c(&r.schema),
                    c(&r.table),
                    c(&r.seq_scan),
                    c(&r.idx_scan),
                    c(&r.live),
                    dead,
                    c(&r.inserts),
                    c(&r.updates),
                    c(&r.deletes),
                    c(&date(&r.vacuum)),
                    c(&date(&r.analyze)),
                ]
            }
            Self::Locks(r) => {
                let r = r.rows.get(i)?;
                let tone = match r.status.as_str() {
                    "waiting" => Tone::Warning,
                    "granted" => Tone::Good,
                    _ => Tone::Neutral,
                };
                vec![
                    c(&r.pid),
                    c(&r.user),
                    c(&r.mode),
                    c(&r.lock_type),
                    status(&r.status, tone),
                    c(fallback(&r.relation)),
                    c(&seconds(&r.seconds)),
                    c(&r.query),
                ]
            }
            Self::Indexes(r) => {
                let r = r.rows.get(i)?;
                let (label, tone) = match r.status.as_str() {
                    "unused" => ("unused", Tone::Danger),
                    "rarely_used" => ("rarely used", Tone::Warning),
                    "active" => ("active", Tone::Good),
                    other => (other, Tone::Neutral),
                };
                vec![
                    c(&r.schema),
                    c(&r.table),
                    c(&r.index),
                    c(&r.size),
                    c(&r.scans),
                    status(label, tone),
                    c(&r.definition),
                ]
            }
            Self::Bloat(r) => {
                let r = r.rows.get(i)?;
                let n = number(&r.pct);
                let pct = Cell {
                    text: n.map_or_else(|| r.pct.clone(), |n| format!("{}%", fixed(n, 1))),
                    style: Style::Bloat {
                        tone: bloat_tone(n),
                        percent: n.map(|value| value.clamp(0., 100.)),
                    },
                };
                vec![
                    c(&r.schema),
                    c(&r.table),
                    c(&r.live),
                    c(&r.dead),
                    pct,
                    c(&r.size),
                    c(&date(&r.vacuum)),
                    c(&date(&r.analyze)),
                ]
            }
            Self::Stats(_) => return None,
        })
    }
}
