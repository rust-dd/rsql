use super::*;
use rsql_core::postgres::roles_schema_objects::discover_notify_channels;

impl Panel {
    pub(super) fn discover(&mut self, cx: &mut Context<Self>) {
        if self.discovering.is_some() {
            return;
        }
        let id = Uuid::new_v4();
        self.discovering = Some(id);
        self.discovery_error = None;
        let mut lifetime = self.lifetime();
        let route = self.route.clone();
        let project = self.project.clone();
        let services = self.session.services.clone();
        let job = self.session.runtime.spawn(async move {
            services
                .run_metadata(&project, lifetime.ended(), async |client| {
                    if !route.is_current() {
                        return Err(rsql_core::Error::ClientNotConnected(project.clone()));
                    }
                    discover_notify_channels(client).await
                })
                .await
                .map_err(|e| e.to_string())
        });
        cx.spawn(async move |panel, cx| {
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = panel.update(cx, |p, cx| {
                if p.attached() && p.discovering == Some(id) {
                    p.discovering = None;
                    match result {
                        Ok(hints) => p.known = hints,
                        Err(e) => p.discovery_error = Some(e),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn send(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let target = self.target.read(cx).value();
        if !self.selected() || target.trim().is_empty() || self.sending.is_some() {
            return;
        }
        let id = Uuid::new_v4();
        let payload = self.payload.read(cx).value();
        let revision = self.send_revision;
        self.sending = Some(id);
        self.send_error = None;
        let mut lifetime = self.lifetime();
        let route = self.route.clone();
        let services = self.session.services.clone();
        let channel = target.trim().to_owned();
        let value = payload.clone();
        let job = self.session.runtime.spawn(async move {
            services
                .notify_guarded(&route, &channel, &value, lifetime.ended())
                .await
                .map_err(|e| e.to_string())
        });
        cx.spawn_in(w, async move |panel, cx| {
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = panel.update_in(cx, |p, w, cx| {
                if !p.attached() || p.sending != Some(id) {
                    return;
                }
                p.sending = None;
                match result {
                    Ok(())
                        if p.send_revision == revision
                            && p.target.read(cx).value() == target
                            && p.payload.read(cx).value() == payload =>
                    {
                        p.payload.update(cx, |s, cx| s.set_value("", w, cx));
                    }
                    Ok(()) => (),
                    Err(e) => p.send_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn clear(&mut self, cx: &mut Context<Self>) {
        if self.selected() {
            self.log.clear();
            self.list.reset(0);
            cx.notify();
        }
    }
}
