use super::{UpdateController, UpdateState, notes};
use gpui_kit::{
    component::{ActiveTheme, button::Button, label::Label},
    *,
};
use std::rc::Rc;

actions!(rsql, [InstallUpdate]);

fn format_bytes(bytes: u64) -> String {
    let units = [(1 << 30, "GB"), (1 << 20, "MB"), (1 << 10, "KB")];
    match units.iter().find(|(size, _)| bytes >= *size) {
        Some((size, unit)) => format!("{:.2} {unit}", bytes as f64 / *size as f64),
        None => format!("{bytes} B"),
    }
}

pub fn panel(
    controller: &Rc<UpdateController>,
    compact_button: fn(Button) -> Button,
    cx: &App,
) -> Option<Div> {
    let state = controller.observe_state().borrow().clone();
    let message = match &state {
        UpdateState::Checking {
            manual: true,
            already,
        } => if *already {
            "An update check is already in progress."
        } else {
            "Checking for updates…"
        }
        .into(),
        UpdateState::Available(c) => format!(
            "Version {} is available.\n{}",
            c.version(),
            notes(c.notes())
        ),
        UpdateState::Downloading {
            candidate,
            bytes,
            total,
        } => match total {
            Some(n) => format!(
                "Downloading {} · {bytes} of {n} bytes ({}%)",
                candidate.version(),
                bytes * 100 / n
            ),
            None => format!("Downloading {} · {bytes} bytes", candidate.version()),
        },
        UpdateState::Verifying(c) => format!("Verifying version {}…", c.version()),
        UpdateState::Cancelling => "Cancelling download…".into(),
        UpdateState::Ready(c) => format!(
            "Version {} is verified and ready to install. Downloaded {}.\nRestart now to finish applying the update?",
            c.version(),
            format_bytes(c.artifact().bytes)
        ),
        UpdateState::Installing(c) => {
            format!("Preparing version {} for installation…", c.version())
        }
        UpdateState::Failed {
            message,
            visible: true,
        }
        | UpdateState::NoUpdate {
            message,
            visible: true,
        } => message.clone(),
        _ => return None,
    };
    let mut panel = div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(div().flex_1().min_w_0().child(Label::new(message)));
    if let UpdateState::Ready(_) = &state {
        panel = panel.child(
            compact_button(Button::new("update-restart").label("Restart"))
                .on_click(|_, w, cx| w.dispatch_action(Box::new(InstallUpdate), cx)),
        );
    }
    if let UpdateState::Available(candidate) = &state {
        let weak = Rc::downgrade(controller);
        let selected = candidate.clone();
        panel = panel.child(
            compact_button(Button::new("update-install").label("Install")).on_click(
                move |_, _, _| {
                    if let Some(c) = weak.upgrade() {
                        c.accept(&selected);
                    }
                },
            ),
        );
    }
    match &state {
        UpdateState::Downloading { candidate, .. } | UpdateState::Verifying(candidate) => {
            let weak = Rc::downgrade(controller);
            let selected = candidate.clone();
            panel = panel.child(
                compact_button(Button::new("update-cancel").label("Cancel")).on_click(
                    move |_, _, _| {
                        if let Some(c) = weak.upgrade() {
                            c.cancel_download(&selected);
                        }
                    },
                ),
            );
        }
        UpdateState::Available(_)
        | UpdateState::Ready(_)
        | UpdateState::Failed { .. }
        | UpdateState::NoUpdate { .. } => {
            let weak = Rc::downgrade(controller);
            let label = if matches!(state, UpdateState::Available(_) | UpdateState::Ready(_)) {
                "Later"
            } else {
                "Dismiss"
            };
            panel = panel.child(
                compact_button(Button::new("update-dismiss").label(label)).on_click(
                    move |_, _, _| {
                        if let Some(c) = weak.upgrade() {
                            c.dismiss(&state);
                        }
                    },
                ),
            );
        }
        _ => {}
    }
    Some(panel)
}
