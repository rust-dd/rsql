use super::*;

impl Workspace {
    pub(in crate::workspace::properties) fn revalidate_structure(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.properties.as_ref().filter(|p| !p.busy) else {
            return;
        };
        let Some(e) = p.editor.as_ref().filter(|e| !e.recover) else {
            return;
        };
        let Some(capture) = Capture::new(self, &p.object.project)
            .filter(|c| c.route.same_generation(&p.capture.route))
        else {
            return;
        };
        let draft = e.draft.clone();
        let id = p.id;
        let object = p.object.clone();
        let Some(owner) = StructureOwner::capture(self, &object, p.modal.subscribe()) else {
            return;
        };
        let route = capture.route.clone();
        let mut generation = capture.generation.clone();
        let mut stop = self.session.preflight_stop.subscribe();
        let svc = self.session.services.clone();
        let (cancel, cancelled) = oneshot::channel();
        let revision = Uuid::new_v4();
        let p = self.properties.as_mut().unwrap();
        p.transfer = Some((revision, cancel));
        p.status = "Checking the draft against current metadata…".into();
        let job = self.session.runtime.spawn(async move {
            let closed = async {
                tokio::select! {
                    _ = cancelled => (),
                    _ = generation.changed() => (),
                    _ = stop.wait_for(|s| *s) => (),
                }
            };
            let mut content = properties::read(&svc,&object,Section::Structure,&route,closed).await?;
            if let Content::Structure(fresh) = &mut content
                && fresh.take().is_some_and(|fresh| fresh.snapshot().metadata() == draft.snapshot().metadata()) {
                Ok(content)
            } else {
                Err(rsql_core::Error::QueryFailed("The table structure changed. Discard the old draft to load the new definition.".into()))
            }
        });
        cx.spawn(async move |view, cx| {
            let result = job
                .await
                .map_err(diagnostic::bounded_message)
                .and_then(|r| r.map_err(diagnostic::bounded_message));
            let _ =
                view.update(cx, |v, cx| {
                    let current = capture.current(v);
                    let Some(p) = v.properties.as_mut().filter(|p| {
                        p.id == id && p.transfer.as_ref().is_some_and(|(rev, _)| *rev == revision)
                    }) else {
                        return;
                    };
                    p.transfer = None;
                    match result {
                        Ok(content) if current => {
                            let Some(e) = p.editor.as_mut() else { return };
                            e.owner = owner;
                            e.invalidate();
                            e.targets
                                .retain(|_, target| matches!(target, LoadState::Ready(_)));
                            p.capture = capture;
                            p.sections.clear();
                            p.sections
                                .insert(Section::Structure, LoadState::Ready(content));
                            p.tab = Tab::Structure;
                            p.status =
                                "Draft matches current metadata. Review the SQL before applying."
                                    .into();
                        }
                        Ok(_) => p.status =
                            "The catalog changed again. Check the draft against current metadata."
                                .into(),
                        Err(error) => p.status = error,
                    }
                    cx.notify();
                });
        })
        .detach();
        cx.notify();
    }
}
