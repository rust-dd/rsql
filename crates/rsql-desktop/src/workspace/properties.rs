mod editor;
mod maintenance;
mod overview;
mod transfer;
mod view;
use super::*;
use catalog::{Action, Capture};
use gpui_kit::component::{ActiveTheme, dialog::DialogEntrance};
use rsql_desktop::metadata::{
    ObjectRef,
    properties::{self, Content, Section},
};
use std::collections::BTreeMap;
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Structure,
    Columns,
    Indexes,
    ForeignKeys,
    Ddl,
    Actions,
}
impl Tab {
    fn sections(self, table: bool) -> &'static [Section] {
        match self {
            Self::Overview if table => &[
                Section::Overview,
                Section::Constraints,
                Section::Triggers,
                Section::Policies,
                Section::Rules,
            ],
            Self::Overview => &[Section::Overview],
            Self::Structure => &[Section::Structure, Section::Columns],
            Self::Columns => &[Section::Columns],
            Self::Indexes => &[Section::Indexes],
            Self::ForeignKeys => &[Section::Outgoing, Section::Incoming],
            Self::Ddl => &[Section::Ddl],
            Self::Actions => &[],
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Structure => "Structure",
            Self::Columns => "Columns",
            Self::Indexes => "Indexes",
            Self::ForeignKeys => "Foreign Keys",
            Self::Ddl => "DDL",
            Self::Actions => "Actions",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Copied {
    Name,
    Ddl,
    Code(&'static str),
}

enum LoadState<T> {
    Loading {
        revision: Uuid,
        _cancel: oneshot::Sender<()>,
    },
    Ready(T),
    Failed(String),
}

pub(super) struct Properties {
    id: Uuid,
    modal: tokio::sync::watch::Sender<()>,
    editor: Option<editor::Editor>,
    action: maintenance::Confirmation,
    busy: bool,
    status: String,
    copied: Option<(Uuid, Copied)>,
    recovery_summary: Option<String>,
    object: ObjectRef,
    pub(super) capture: Capture,
    tab: Tab,
    sections: BTreeMap<Section, LoadState<Content>>,
    transfer: Option<(Uuid, oneshot::Sender<()>)>,
}

impl Properties {
    fn status_text(&self) -> &str {
        if self.status.is_empty() {
            self.action
                .notice
                .as_ref()
                .map_or("", |(_, message)| message)
        } else {
            &self.status
        }
    }

    pub(super) fn preserve(&self) -> bool {
        self.busy
            || self.action.retired
            || self
                .editor
                .as_ref()
                .is_some_and(|e| e.draft.change_count() != 0 || e.recover)
    }
}
impl Workspace {
    pub(super) fn open_properties(
        &mut self,
        capture: Capture,
        object: ObjectRef,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !capture.current(self) || object.project != capture.project {
            return;
        }
        if self.properties.as_ref().is_some_and(|p| p.busy) {
            return;
        }
        if self.properties.is_some() {
            self.dismiss_properties(w, cx);
        }
        let id = Uuid::new_v4();
        let tab = Tab::Overview;
        let table = object.kind.is_table();
        let title = SharedString::from(object.name.clone());
        // Columns and Indexes load up front so the tab strip shows their counts, as the source does.
        let mut sections = tab.sections(table).to_vec();
        if table {
            sections.extend([Section::Columns, Section::Indexes]);
        }
        self.properties = Some(Properties {
            id,
            object,
            editor: None,
            action: Default::default(),
            busy: false,
            status: String::new(),
            copied: None,
            recovery_summary: None,
            modal: tokio::sync::watch::channel(()).0,
            capture,
            tab,
            sections: Default::default(),
            transfer: None,
        });
        let owner = cx.weak_entity();
        let panel = view::Panel::watch(&cx.entity(), id, cx);
        w.open_dialog(cx, move |d, w, cx| {
            let available = owner
                .upgrade()
                .is_some_and(|v| v.read(cx).properties.as_ref().is_some_and(|p| !p.busy));
            let owner = owner.clone();
            let panel = panel.clone();
            let viewport = w.viewport_size();
            let width = px(if viewport.width >= px(640.) {
                760.
            } else {
                448.
            });
            d.accessibility_label(title.clone())
                .w(width.min(viewport.width - px(32.)))
                .h(viewport.height * 0.85)
                .centered()
                .entrance(DialogEntrance::Fade)
                .p_0()
                .gap_0()
                .rounded(px(16.))
                .overflow_hidden()
                .bg(cx.theme().group_box)
                .border_color(cx.theme().border.opacity(0.4))
                .backdrop_color(black().opacity(0.6))
                .shadow(vec![
                    BoxShadow {
                        color: white().opacity(0.05),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(1.),
                        inset: false,
                    },
                    BoxShadow {
                        color: black().opacity(0.5),
                        offset: point(px(0.), px(25.)),
                        blur_radius: px(50.),
                        spread_radius: px(-12.),
                        inset: false,
                    },
                ])
                .close_button(false)
                .keyboard(available)
                .overlay_closable(available)
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |v, cx| {
                        if v.properties.as_ref().is_some_and(|p| p.id == id && !p.busy) {
                            v.properties = None;
                            cx.notify();
                        }
                    });
                })
                .content(move |body, _, _| {
                    body.gap_0().min_h_0().child(
                        clipboard_order::capture(div().size_full().min_h_0()).child(panel.clone()),
                    )
                })
        });
        for section in sections {
            self.load_property(section, w, cx);
        }
        cx.notify();
    }

    pub(super) fn dismiss_properties(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.properties.as_ref().is_some_and(|p| p.busy) {
            return;
        }
        self.properties.take();
        w.close_dialog(cx);
        cx.notify();
    }

    fn property_current(&self, id: Uuid) -> bool {
        self.properties
            .as_ref()
            .is_some_and(|p| p.id == id && p.capture.current(self))
    }

    fn change_property_tab(&mut self, tab: Tab, w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.properties.as_mut() else {
            return;
        };
        if p.busy {
            return;
        }
        if !p.action.retired {
            let notice = p.action.notice.take();
            p.action = Default::default();
            p.action.notice = notice;
        }
        p.tab = tab;
        p.transfer = None;
        let missing = tab
            .sections(p.object.kind.is_table())
            .iter()
            .copied()
            .filter(|s| !p.sections.contains_key(s))
            .collect::<Vec<_>>();
        for section in missing {
            self.load_property(section, w, cx);
        }
        cx.notify();
    }

    fn load_property(&mut self, section: Section, w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.properties.as_mut() else {
            return;
        };
        let (id, object) = (p.id, p.object.clone());
        let svc = self.session.services.clone();
        let route = p.capture.route.clone();
        let mut generation = p.capture.generation.clone();
        let mut stop = self.session.preflight_stop.subscribe();
        let (cancel, cancelled) = oneshot::channel();
        let revision = Uuid::new_v4();
        p.sections.insert(
            section,
            LoadState::Loading {
                revision,
                _cancel: cancel,
            },
        );
        let job = self.session.runtime.spawn(async move {
            svc.run_metadata(&object.project, async {
                tokio::select! { _ = cancelled => (), _ = stop.wait_for(|s| *s) => (), _ = generation.changed() => () }
            }, async |client| {
                properties::read_on(client, &svc, &object, section, &route).await
            }).await
        });
        cx.spawn_in(w, async move |view, cx| {
            let result = job
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = view.update(cx, |v, cx| {
                v.publish_property(id, section, revision, result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn publish_property(
        &mut self,
        id: Uuid,
        section: Section,
        revision: Uuid,
        result: Result<Content, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.property_current(id) {
            return;
        }
        let p = self.properties.as_mut().unwrap();
        if !matches!(p.sections.get(&section),Some(LoadState::Loading {revision: r,..}) if *r == revision)
        {
            return;
        }
        self.install_property(section, result, cx);
    }

    fn install_property(
        &mut self,
        section: Section,
        mut result: Result<Content, String>,
        cx: &mut Context<Self>,
    ) {
        let p = self.properties.as_mut().unwrap();
        if let Ok(Content::Structure(draft)) = &mut result
            && p.editor.is_none()
            && let Some(draft) = draft.take()
        {
            let receiver = p.modal.subscribe();
            let object = p.object.clone();
            let owner = structure_apply::StructureOwner::capture(self, &object, receiver);
            let p = self.properties.as_mut().unwrap();
            p.editor = owner.map(|o| editor::Editor::new(draft, o));
            if p.editor.is_some()
                && let Some(summary) = p.recovery_summary.take()
            {
                p.status = format!("{summary} Metadata is current.");
            }
        }
        self.properties.as_mut().unwrap().sections.insert(
            section,
            match result {
                Ok(data) => LoadState::Ready(data),
                Err(e) => LoadState::Failed(e),
            },
        );
        cx.notify();
    }

    fn property_copied(&mut self, kind: Copied, cx: &mut Context<Self>) {
        let Some(properties) = &mut self.properties else {
            return;
        };
        let revision = Uuid::new_v4();
        properties.copied = Some((revision, kind));
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_secs(2));
        cx.spawn(async move |view, cx| {
            timer.await;
            let _ = view.update(cx, |v, cx| {
                if let Some(properties) = &mut v.properties
                    && properties
                        .copied
                        .is_some_and(|(current, _)| current == revision)
                {
                    properties.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}
