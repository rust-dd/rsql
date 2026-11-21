use super::*;
use super::{
    editor_overlay::{Menu, Rows},
    editor_presentation::{Presentation, Request},
    editor_services::{Response, Stamp, resolve},
};
use gpui_kit::component::input::{CompletionProvider, DefinitionProvider, Rope, RopeExt};
use lsp_types::{
    CompletionContext, CompletionList, CompletionResponse, CompletionTriggerKind, LocationLink,
};
use rsql_desktop::completion::{Analysis, suggestions};
use std::{cell::Cell, ops::Range};

fn completion_prefix(raw: &str) -> String {
    raw.strip_prefix('"').map_or_else(
        || raw.to_lowercase(),
        |inner| inner.replace("\"\"", "\"").to_lowercase(),
    )
}

#[derive(Clone)]
pub(super) struct Provider {
    workspace: WeakEntity<Workspace>,
    owner: Owner,
    editor: EntityId,
    epoch: Rc<Cell<u64>>,
    presentation: Rc<Presentation>,
}

impl Provider {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        owner: Owner,
        editor: EntityId,
        epoch: Rc<Cell<u64>>,
        presentation: Rc<Presentation>,
    ) -> Self {
        Self {
            workspace,
            owner,
            editor,
            epoch,
            presentation,
        }
    }

    fn prepare(
        &self,
        text: &Rope,
        epoch: u64,
        cx: &mut AsyncWindowContext,
    ) -> Option<(Stamp, Response)> {
        self.workspace
            .update_in(cx, |v, w, cx| {
                let live = v.editors.get(&self.owner)?;
                if live.state.entity_id() != self.editor
                    || live.state.read(cx).text() != text
                    || self.epoch.get() != epoch
                    || live
                        .state
                        .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
                {
                    return None;
                }
                v.analysis_for(&self.owner, w, cx)
            })
            .ok()
            .flatten()
    }

    async fn suggestions(
        &self,
        text: Rope,
        offset: usize,
        explicit: bool,
        epoch: u64,
        job: &Request,
        cx: &mut AsyncWindowContext,
    ) -> Option<bool> {
        let (stamp, response) = self.prepare(&text, epoch, cx)?;
        if stamp.caret != offset {
            return None;
        }
        let parsed = super::editor_services::reply(response).await?;
        let (snapshot, incomplete) = match &parsed {
            Ok(parsed) => self.catalog(&stamp, &parsed.completion, cx).await?,
            Err(()) => (Default::default(), false),
        };
        let signal = job.signal();
        let (items, prefix) = cx
            .background_executor()
            .spawn(async move {
                let fallback;
                let context = match &parsed {
                    Ok(parsed) => &parsed.completion,
                    Err(()) => {
                        if text.len_utf16() > rsql_desktop::completion::MAX_COMPLETION_UTF16 {
                            return None;
                        }
                        fallback = rsql_desktop::completion::fallback_context(
                            &text.to_string(),
                            offset,
                            || *signal.borrow(),
                        )?;
                        &fallback
                    }
                };
                let range = &context.range;
                let prefix = text
                    .slice(range.start..offset.clamp(range.start, range.end))
                    .to_string();
                let prefix = completion_prefix(&prefix);
                let items = suggestions(context, &snapshot, &prefix, || *signal.borrow())?;
                Some((items, prefix))
            })
            .await?;
        self.workspace
            .update_in(cx, |v, w, cx| {
                if !v.is_current(&stamp, w, cx) || !self.presentation.matches(job.generation) {
                    return None;
                }
                let language = &mut v.editors.get_mut(&self.owner)?.language;
                if language.snippet.is_some() {
                    return None;
                }
                language.menu = (!items.is_empty()).then(|| {
                    Menu::new(
                        stamp,
                        job.generation,
                        Rows::Suggestions {
                            items,
                            explicit,
                            prefix,
                        },
                    )
                });
                cx.notify();
                Some(incomplete)
            })
            .ok()?
    }
    async fn catalog(
        &self,
        stamp: &Stamp,
        context: &rsql_desktop::completion::CompletionContext,
        cx: &mut AsyncWindowContext,
    ) -> Option<(rsql_desktop::completion::CatalogSnapshot, bool)> {
        let mut catalog = self
            .workspace
            .update_in(cx, |v, w, cx| {
                if !v.is_current(stamp, w, cx) || v.editors[&self.owner].language.snippet.is_some()
                {
                    return None;
                }
                Some(v.editor_catalog(stamp.project.as_deref(), context, cx))
            })
            .ok()??;
        // Schema discovery and the selected index share one typing deadline.
        let timer = cx.background_executor().timer(Duration::from_secs(3));
        tokio::pin!(timer);
        while let Some(mut wait) = catalog.wait.take() {
            let published =
                tokio::select! { r = wait.changed() => r.is_ok(), _ = &mut timer => false };
            if catalog.discovering && published {
                catalog = self
                    .workspace
                    .update_in(cx, |v, w, cx| {
                        v.is_current(stamp, w, cx)
                            .then(|| v.editor_catalog(stamp.project.as_deref(), context, cx))
                    })
                    .ok()??;
            } else {
                (catalog.snapshot, catalog.incomplete) = self
                    .workspace
                    .update_in(cx, |v, w, cx| {
                        v.is_current(stamp, w, cx)
                            .then(|| v.read_editor_catalog(stamp.project.as_deref(), context))
                    })
                    .ok()??;
                break;
            }
        }
        Some((catalog.snapshot, catalog.incomplete))
    }
}

impl CompletionProvider for Provider {
    fn is_completion_trigger(&self, _: usize, inserted: &str, _: &mut App) -> bool {
        inserted
            .chars()
            .last()
            .is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '.' | '"'))
    }
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        context: CompletionContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<CompletionResponse>> {
        let provider = self.clone();
        let epoch = self.epoch.get();
        let job = self.presentation.start(text, offset);
        let text = text.clone();
        window.spawn(cx, async move |cx| {
            let explicit = context.trigger_kind == CompletionTriggerKind::INVOKED;
            let incomplete = job
                .while_current(provider.suggestions(text, offset, explicit, epoch, &job, cx))
                .await
                .flatten()
                .unwrap_or(false);
            // Kit owns input and IME. The application menu owns snippet acceptance.
            Ok(CompletionResponse::List(CompletionList {
                is_incomplete: incomplete,
                items: Vec::new(),
            }))
        })
    }
}

fn symbol(
    analysis: &Analysis,
    at: usize,
    cancelled: &impl Fn() -> bool,
) -> Option<(Range<usize>, Range<usize>)> {
    let mut boundary = None;
    for (origin, target) in analysis
        .references
        .iter()
        .map(|r| (&r.range, &r.definition))
        .chain(analysis.declarations.iter().map(|d| (&d.range, &d.range)))
    {
        if cancelled() {
            return None;
        }
        if origin.contains(&at) {
            return Some((origin.clone(), target.clone()));
        }
        if origin.end == at && boundary.is_none() {
            boundary = Some((origin.clone(), target.clone()));
        }
    }
    boundary
}

fn reference_rows(
    parsed: &Analysis,
    text: &Rope,
    caret: usize,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<(String, Range<usize>)>> {
    let (_, target) = symbol(parsed, caret, &cancelled)?;
    let mut rows = Vec::new();
    for r in &parsed.references {
        if cancelled() {
            return None;
        }
        if r.definition != target {
            continue;
        }
        let pos = text.offset_to_position(r.range.start);
        let line: String = text
            .slice_line(pos.line as usize)
            .chars()
            .take(120)
            .collect();
        rows.push((
            format!("{}:{}  {}", pos.line + 1, pos.character + 1, line.trim()),
            r.range.clone(),
        ));
    }
    Some(rows)
}
impl DefinitionProvider for Provider {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<gpui_kit::Result<Vec<LocationLink>>> {
        let provider = self.clone();
        let epoch = self.epoch.get();
        let text = text.clone();
        window.spawn(cx, async move |cx| {
            let Some((stamp, response)) = provider.prepare(&text, epoch, cx) else {
                return Ok(vec![]);
            };
            let Some(analysis) = resolve(response).await else {
                return Ok(vec![]);
            };
            let Some((origin, target)) = symbol(&analysis, offset, &|| false) else {
                return Ok(vec![]);
            };
            if !provider
                .workspace
                .update_in(cx, |v, w, cx| v.is_current(&stamp, w, cx))
                .unwrap_or(false)
            {
                return Ok(vec![]);
            }
            let position = |r: Range<usize>| {
                lsp_types::Range::new(
                    text.offset_to_position(r.start),
                    text.offset_to_position(r.end),
                )
            };
            let target = position(target);
            Ok(vec![LocationLink {
                origin_selection_range: Some(position(origin)),
                target_uri: "rsql:document".parse()?,
                target_range: target,
                target_selection_range: target,
            }])
        })
    }
}

impl Workspace {
    pub(super) fn find_references(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.focused_owner(w, cx) else {
            return;
        };
        let Some((stamp, response)) = self.analysis_for(&owner, w, cx) else {
            return;
        };
        let live = &self.editors[&owner];
        let presentation = live.language.presentation.clone();
        let text = live.state.read(cx).text().clone();
        let request = presentation.start(&text, stamp.caret);
        cx.notify();
        let generation = request.generation;
        let task = cx.spawn_in(w, async move |view, cx| {
            let signal = request.signal();
            let work = async {
                let parsed = resolve(response).await?;
                cx.background_executor()
                    .spawn(async move {
                        reference_rows(&parsed, &text, stamp.caret, || *signal.borrow())
                    })
                    .await
            };
            let Some(Some(rows)) = request.while_current(work).await else {
                return;
            };
            let _ = view.update_in(cx, |v, w, cx| {
                if !v.is_current(&stamp, w, cx) {
                    return;
                }
                let live = v.editors.get_mut(&owner).unwrap();
                if !live.language.presentation.matches(generation) {
                    return;
                }
                match rows.as_slice() {
                    [] => live.language.menu = None,
                    [(_, range)] => {
                        live.language.menu = None;
                        live.state
                            .update(cx, |s, cx| s.set_selected_range(range.clone(), cx));
                    }
                    _ => {
                        live.language.menu =
                            Some(Menu::new(stamp, generation, Rows::References(rows)))
                    }
                }
                cx.notify();
            });
        });
        presentation.own_references(task);
    }
}
