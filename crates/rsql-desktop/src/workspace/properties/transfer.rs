use super::*;
use rsql_desktop::structure::{EditId, StoredObject, StructureDraft};
use std::sync::Arc;

enum Source {
    Text(Arc<str>),
    Stored(Arc<StructureDraft>, EditId),
}

impl Source {
    fn read(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Stored(draft, id) => draft
                .snapshot()
                .stored_objects()
                .find(|object| object.id() == *id)
                .map(|object| stored_definition(&object)),
        }
    }
}

pub(super) fn stored_definition<'a>(stored: &StoredObject<'a>) -> &'a str {
    match stored {
        StoredObject::ForeignKey(c) | StoredObject::Unique(c) => &c.definition,
        StoredObject::Index(i) => &i.definition,
    }
}

impl Workspace {
    pub(super) fn transfer_ddl(&mut self, copy: bool, w: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.properties.as_ref() else {
            return;
        };
        let Some(LoadState::Ready(Content::Ddl(text))) = p.sections.get(&Section::Ddl) else {
            return;
        };
        self.transfer_property_source(
            Source::Text(text.full.clone()),
            copy,
            Some(Copied::Ddl),
            w,
            cx,
        );
    }

    pub(super) fn transfer_property_text(
        &mut self,
        text: Arc<str>,
        copy: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfer_property_source(Source::Text(text), copy, None, w, cx);
    }

    pub(super) fn copy_property_code(
        &mut self,
        text: Arc<str>,
        key: &'static str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfer_property_source(Source::Text(text), true, Some(Copied::Code(key)), w, cx);
    }

    pub(super) fn transfer_stored_definition(
        &mut self,
        id: EditId,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(draft) = self
            .properties
            .as_ref()
            .and_then(|p| p.editor.as_ref())
            .map(|e| e.draft.clone())
        {
            self.transfer_property_source(Source::Stored(draft, id), true, None, w, cx);
        }
    }

    fn transfer_property_source(
        &mut self,
        source: Source,
        copy: bool,
        feedback: Option<Copied>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.properties.as_mut() else {
            return;
        };
        let id = p.id;
        let revision = Uuid::new_v4();
        let order = copy.then(|| clipboard_order::ClipboardWrite::begin(cx));
        let (cancel, mut cancelled) = oneshot::channel();
        p.transfer = Some((revision, cancel));
        let stop = self.session.preflight_stop.subscribe();
        let job = self.session.runtime.spawn_blocking(move || {
            let mut active = || {
                !*stop.borrow() && cancelled.try_recv() == Err(oneshot::error::TryRecvError::Empty)
            };
            if !active() {
                return None;
            }
            let mut rest = source.read()?;
            let mut out = String::with_capacity(rest.len());
            while !rest.is_empty() {
                if !active() {
                    return None;
                }
                let n = rest.floor_char_boundary(rest.len().min(65536));
                out.push_str(&rest[..n]);
                rest = &rest[n..];
            }
            let editor = match (copy, source) {
                (false, Source::Text(text)) => Some(PreparedEditorText {
                    editor: SharedString::from(&text),
                    snapshot: text,
                }),
                _ => None,
            };
            Some((out, editor))
        });
        cx.spawn_in(w, async move |view, cx| {
            let Ok(Some((sql, editor_text))) = job.await else {
                return;
            };
            let _ = view.update_in(cx, |v, w, cx| {
                if !v.property_current(id) {
                    return;
                }
                let p = v.properties.as_mut().unwrap();
                if !p.transfer.as_ref().is_some_and(|(r, _)| *r == revision) {
                    return;
                }
                p.transfer.take();
                if let Some(order) = order {
                    if order.current() {
                        crate::desktop::DesktopServices::write_clipboard(
                            cx,
                            ClipboardItem::new_string(sql),
                        );
                        if let Some(feedback) = feedback {
                            v.property_copied(feedback, cx);
                        }
                    }
                } else {
                    let project = p.object.project.clone();
                    v.dismiss_properties(w, cx);
                    v.update_documents_with_initial(w, cx, editor_text, |docs| {
                        docs.open(Kind::Query, Some(project)).main.draft.text = sql;
                    });
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
