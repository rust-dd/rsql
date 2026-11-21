use super::*;
use chrono::Local;
use tokio::sync::mpsc;

enum Delivery {
    Ready,
    Event(log::Received),
}

async fn run(
    services: Arc<ApplicationServices>,
    route: ProjectTicket,
    channel: String,
    mut lifetime: Lifetime,
    mut stop: watch::Receiver<bool>,
    output: mpsc::Sender<Delivery>,
) -> Result<(), String> {
    let cancelled = async {
        tokio::select! {
            _ = lifetime.ended() => (),
            _ = stop.wait_for(|v| *v) => (),
            _ = output.closed() => (),
        }
    };
    tokio::pin!(cancelled);
    let mut rx = services
        .listen_guarded(&route, &channel, &mut cancelled)
        .await
        .map_err(|e| e.to_string())?;
    let deliver = async {
        if output.send(Delivery::Ready).await.is_err() {
            return Ok(());
        }
        while let Some(event) = rx.recv().await {
            let received = log::Received {
                time: Local::now(),
                event,
            };
            if output.send(Delivery::Event(received)).await.is_err() {
                return Ok(());
            }
        }
        Err("Listener connection closed".to_owned())
    };
    let result = tokio::select! {
        biased;
        _ = &mut cancelled => Ok(()),
        result = deliver => result,
    };
    rx.close().await;
    result
}

impl Panel {
    pub(super) fn subscribe_manual(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let draft = self.channel.read(cx).value();
        self.subscribe(
            draft.trim(),
            Some((self.subscribe_revision, draft.clone())),
            w,
            cx,
        );
    }

    pub(super) fn subscribe(
        &mut self,
        name: &str,
        draft: Option<(u64, SharedString)>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected() || name.is_empty() || self.channels.contains_key(name) {
            return;
        }
        let channel = name.to_owned();
        let id = Uuid::new_v4();
        let (stop, cancelled) = watch::channel(false);
        self.channels.insert(
            channel.clone(),
            Channel {
                id,
                active: false,
                stop,
            },
        );
        self.subscribe_error = None;
        let (output, mut input) = mpsc::channel(1);
        let job = self.session.runtime.spawn(run(
            self.session.services.clone(),
            self.route.clone(),
            channel.clone(),
            self.lifetime(),
            cancelled,
            output,
        ));
        cx.spawn_in(w, async move |panel, cx| {
            while let Some(message) = input.recv().await {
                let accepted = panel.update_in(cx, |p, w, cx| {
                    if !p.attached() {
                        return false;
                    }
                    let Some(entry) = p
                        .channels
                        .get_mut(&channel)
                        .filter(|e| e.id == id && !*e.stop.borrow())
                    else {
                        return false;
                    };
                    match message {
                        Delivery::Ready => {
                            entry.active = true;
                            if let Some((revision, text)) = &draft
                                && *revision == p.subscribe_revision
                                && p.channel.read(cx).value() == *text
                            {
                                p.channel.update(cx, |s, cx| s.set_value("", w, cx));
                            }
                        }
                        Delivery::Event(event) => {
                            if let Some(removed) = p.log.push(event) {
                                if removed > 0 {
                                    p.list.splice(0..removed, 0);
                                }
                                let index = p.log.rows.len() - 1;
                                p.list.splice(index..index, 1);
                            }
                        }
                    }
                    cx.notify();
                    true
                });
                if !matches!(accepted, Ok(true)) {
                    break;
                }
            }
            drop(input);
            let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
            let _ = panel.update(cx, |p, cx| {
                if p.attached() && p.channels.get(&channel).is_some_and(|e| e.id == id) {
                    p.channels.remove(&channel);
                    if let Err(error) = result {
                        p.subscribe_error = Some(format!("{channel}: {error}"));
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn unsubscribe(&mut self, channel: &str, cx: &mut Context<Self>) {
        if self.selected()
            && let Some(entry) = self.channels.get_mut(channel)
        {
            entry.active = false;
            entry.stop.send_replace(true);
            cx.notify();
        }
    }
}
