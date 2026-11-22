#[path = "form_content.rs"]
mod content;
#[path = "form_feedback.rs"]
mod feedback;
#[path = "form_fields.rs"]
mod fields;
use super::*;
use feedback::{Feedback, Outcome};
use fields::{Field, FocusRow, SslMode, SslState};
use gpui_kit::component::{
    IndexPath, WindowExt,
    button::Button,
    input::{InputEvent, InputState},
    select::{SearchableVec, SelectEvent, SelectState},
};
use rsql_desktop::{
    connection::{ConnectionProfile, ConnectionUrl},
    profile_service::Challenge,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    New,
    Edit,
    Reconnect,
}

impl Mode {
    fn primary(self) -> Operation {
        match self {
            Self::New | Self::Reconnect => Operation::Connect,
            Self::Edit => Operation::Save,
        }
    }
}

struct RouteJob {
    op: Operation,
    task: tokio::task::AbortHandle,
    cancelled: Arc<AtomicBool>,
}

impl Drop for RouteJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.task.abort();
    }
}

pub(super) struct Form {
    profile: ConnectionProfile,
    mode: Mode,
    window: AnyWindowHandle,
    fields: Vec<Entity<InputState>>,
    url: Entity<InputState>,
    ssl: Entity<SslState>,
    ssh_focus: FocusHandle,
    field_rows: Vec<FocusRow>,
    url_row: FocusRow,
    ssl_row: FocusRow,
    ssh_row: FocusRow,
    scroll: ScrollHandle,
    initial_focus: std::cell::Cell<bool>,
    auto_name: Option<String>,
    url_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
    challenge: Option<(Challenge, Operation, ConnectionProfile)>,
    job: Option<RouteJob>,
    saving: bool,
    connecting: Option<rsql_core::services::ProjectTicket>,
    feedback: Option<Feedback>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Operation {
    Test,
    Save,
    Connect,
}

impl Workspace {
    pub(super) fn button(
        &self,
        label: impl Into<SharedString>,
        cx: &Context<Self>,
        callback: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        let label = label.into();
        Button::new(label.clone())
            .label(label)
            .on_click(cx.listener(move |view, _, w, cx| callback(view, w, cx)))
    }

    pub(super) fn open_form(
        &mut self,
        profile: ConnectionProfile,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connection_form(Mode::New, profile, w, cx);
    }

    pub(super) fn open_edit_form(
        &mut self,
        profile: ConnectionProfile,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connection_form(Mode::Edit, profile, w, cx);
    }

    pub(super) fn open_reconnect_form(
        &mut self,
        profile: ConnectionProfile,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connection_form(Mode::Reconnect, profile, w, cx);
    }

    fn open_connection_form(
        &mut self,
        mode: Mode,
        profile: ConnectionProfile,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.form.as_ref().is_some_and(|f| f.saving) {
            return;
        }
        self.close_form(cx);
        let values = profile.details();
        let scroll = ScrollHandle::new();
        let mut subscriptions = Vec::new();
        let (fields, field_rows) = Field::ALL
            .into_iter()
            .map(|field| {
                let input = cx.new(|cx| {
                    InputState::new(w, cx)
                        .placeholder(field.placeholder())
                        .masked(field.masked())
                        .default_value(values[field.slot()].clone())
                });
                let row = FocusRow::new(input.focus_handle(cx), &scroll);
                subscriptions.push(cx.subscribe(&input, move |view, _, event, cx| {
                    if matches!(event, InputEvent::Change) {
                        if field == Field::Name
                            && let Some(form) = &mut view.navigation.form
                        {
                            form.auto_name = None;
                        }
                        view.invalidate_form(cx);
                    } else if matches!(
                        event,
                        InputEvent::PressEnter {
                            secondary: false,
                            shift: false
                        }
                    ) {
                        view.submit_primary(cx);
                    }
                }));
                (input, row)
            })
            .unzip();
        // The URL can contain a password, so the retained input is always masked.
        let url = cx.new(|cx| {
            InputState::new(w, cx)
                .placeholder("postgresql://user:password@host:5432/database")
                .masked(true)
        });
        let url_row = FocusRow::new(url.focus_handle(cx), &scroll);
        subscriptions.push(
            cx.subscribe_in(&url, w, |view, _, event, w, cx| match event {
                InputEvent::Change => view.import_url(w, cx),
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => view.submit_primary(cx),
                _ => {}
            }),
        );

        let ssl_mode = SslMode::stored(&profile.ssl);
        let ssl = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(SslMode::ALL.to_vec()),
                Some(IndexPath::new(ssl_mode as usize)),
                w,
                cx,
            )
        });
        let ssl_row = FocusRow::new(ssl.focus_handle(cx), &scroll);
        subscriptions.push(cx.subscribe(&ssl, |view, _, event, cx| {
            if let SelectEvent::Confirm(Some(mode)) = event
                && let Some(form) = view.navigation.form.as_mut().filter(|f| !f.saving)
                && SslMode::stored(&form.profile.ssl) != *mode
            {
                form.profile.ssl = mode.value().into();
                view.invalidate_form(cx);
            }
        }));

        let ssh_focus = cx.focus_handle();
        let ssh_row = FocusRow::new(ssh_focus.clone(), &scroll);
        self.navigation.form = Some(Form {
            profile,
            mode,
            window: w.window_handle(),
            fields,
            url,
            ssl,
            ssh_focus,
            field_rows,
            url_row,
            ssl_row,
            ssh_row,
            scroll,
            initial_focus: true.into(),
            auto_name: None,
            url_error: None,
            _subscriptions: subscriptions,
            challenge: None,
            job: None,
            saving: false,
            connecting: None,
            feedback: None,
        });
        // A sidebar reconnect runs without the dialog, like the legacy app, unless it needs input.
        if mode != Mode::Reconnect && !self.navigation.modal_open {
            self.navigation.modal_open = true;
            content::open(&cx.entity(), w, cx);
        }
        cx.notify();
    }

    fn reveal_form(&mut self, cx: &mut Context<Self>) {
        let Some(window) = self.navigation.form.as_ref().map(|f| f.window) else {
            return;
        };
        if self.navigation.modal_open {
            return;
        }
        self.navigation.modal_open = true;
        let owner = cx.entity();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, w, cx| content::open(&owner, w, cx));
        });
    }

    fn invalidate_form(&mut self, cx: &mut Context<Self>) {
        let mut release = None;
        if let Some(f) = self.navigation.form.as_mut() {
            if f.saving {
                return;
            }
            self.navigation.serial += 1;
            f.job = None;
            if f.connecting
                .take()
                .is_some_and(|ticket| ticket.is_current())
            {
                self.navigation.status.remove(&f.profile.id);
                release = Some(f.profile.id.clone());
            }
            f.challenge = None;
            f.feedback = None;
        }
        if let Some(id) = release {
            self.disconnect(id, cx);
        }
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        if self.navigation.form.as_ref().is_some_and(|f| f.saving) {
            return;
        }
        self.invalidate_form(cx);
        self.navigation.form = None;
    }

    fn draft(&self, cx: &App) -> Option<ConnectionProfile> {
        let f = self.navigation.form.as_ref()?;
        let mut details = f.profile.details();
        for (field, input) in Field::ALL.into_iter().zip(&f.fields) {
            details[field.slot()] = input.read(cx).value().to_string();
        }
        Some(ConnectionProfile::from_row(f.profile.id.clone(), &details))
    }

    fn import_url(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(f) = self
            .navigation
            .form
            .as_ref()
            .filter(|f| !f.saving && matches!(f.mode, Mode::New))
        else {
            return;
        };
        let value = f.url.read(cx).value();
        self.invalidate_form(cx);
        let f = self.navigation.form.as_mut().unwrap();
        f.url_error = None;
        if value.trim().is_empty() {
            return;
        }
        let url = match ConnectionUrl::parse(&value) {
            Ok(url) => url,
            Err(error) => {
                f.url_error = Some(fields::safe_url_error(&value, error).into());
                return;
            }
        };
        let name = f.fields[Field::Name as usize].read(cx).value();
        if name.is_empty() || f.auto_name.as_deref() == Some(name.as_ref()) {
            f.fields[Field::Name as usize]
                .update(cx, |input, cx| input.set_value(url.database.clone(), w, cx));
            f.auto_name = Some(url.database.clone());
        }
        for (field, value) in [
            (Field::Host, url.host),
            (Field::Port, url.port),
            (Field::Database, url.database),
            (Field::Username, url.username),
            (Field::Password, url.password),
        ] {
            let input = &f.fields[field as usize];
            if input.read(cx).value().as_ref() != value {
                input.update(cx, |input, cx| input.set_value(value, w, cx));
            }
        }
        f.ssl.update(cx, |state, cx| {
            state.set_selected_value(&SslMode::stored(&url.ssl), w, cx);
        });
        f.profile.ssl = url.ssl;
    }

    fn submit_primary(&mut self, cx: &mut Context<Self>) {
        if let Some(f) = self.navigation.form.as_ref()
            && f.challenge.is_none()
        {
            self.submit(f.mode.primary(), cx);
        }
    }

    pub(super) fn submit(&mut self, op: Operation, cx: &mut Context<Self>) {
        let Some(f) = self.navigation.form.as_ref() else {
            return;
        };
        if self.navigation.busy || f.saving || f.job.is_some() {
            return;
        }
        let draft = self.draft(cx).unwrap();
        let p = match if op == Operation::Test {
            draft.check_route().map(|()| draft)
        } else {
            draft.validated()
        } {
            Ok(p) => p,
            Err(e) => {
                let f = self.navigation.form.as_mut().unwrap();
                f.feedback = Some(Feedback::error(e));
                f.scroll.scroll_to_bottom();
                cx.notify();
                return;
            }
        };
        self.invalidate_form(cx);
        if op == Operation::Test {
            self.run_route(p, op, None, cx);
            return;
        }
        let serial = self.navigation.serial;
        let svc = self.session.services.clone();
        let saved = p.clone();
        let work = self.session.spawn_profile_write(async move {
            saved
                .save(&svc)
                .await
                .map_err(|error| Feedback::error(saved.safe_error(error)))?;
            svc.disconnect(&saved.id).await;
            Ok(())
        });
        let f = self.navigation.form.as_mut().unwrap();
        f.saving = true;
        cx.spawn(async move |view, cx| {
            let result = work
                .await
                .map_err(|_| Feedback::error("Could not save the connection. Try again."))
                .and_then(|result| result);
            let _ = view.update(cx, |v, cx| {
                if v.navigation.serial != serial {
                    return;
                }
                let f = v.navigation.form.as_mut().unwrap();
                f.saving = false;
                match result {
                    Err(feedback) => {
                        f.feedback = Some(feedback);
                        f.scroll.scroll_to_bottom();
                    }
                    Ok(()) => {
                        v.session.connections.borrow_mut().put(p.clone());
                        v.retire_disconnected_route(&p.id, cx);
                        if op == Operation::Save {
                            v.close_form(cx);
                        } else {
                            v.run_route(p, op, None, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn run_route(
        &mut self,
        p: ConnectionProfile,
        op: Operation,
        approval: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.navigation.serial += 1;
        let serial = self.navigation.serial;
        let retry = approval.is_some();
        let routes = self.session.routes.clone();
        let target = p.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let work = self.session.runtime.spawn(async move {
            let trust = match approval {
                Some(id) => routes.trust(&id).await,
                None => Ok(()),
            };
            let result = match trust {
                Ok(()) => routes.request(&target, op == Operation::Test).await,
                Err(error) => Err(error),
            };
            tokio::task::spawn_blocking(move || {
                if worker_cancelled.load(Ordering::Acquire) {
                    return None;
                }
                let result = Outcome::prepare(result, &target, op, retry);
                (!worker_cancelled.load(Ordering::Acquire)).then_some(result)
            })
            .await
        });
        let f = self.navigation.form.as_mut().unwrap();
        f.job = Some(RouteJob {
            op,
            task: work.abort_handle(),
            cancelled,
        });
        if op == Operation::Connect {
            f.connecting = self.session.services.capture_project_ticket(&p.id).ok();
            self.navigation
                .status
                .insert(p.id.clone(), "Connecting".into());
        }
        cx.spawn(async move |view, cx| {
            let result = match work.await {
                Ok(Ok(Some(result))) => result,
                Ok(Ok(None)) => return,
                Err(error) if error.is_cancelled() => return,
                Ok(Err(_)) | Err(_) => {
                    Outcome::error("The connection operation failed. Try again.")
                }
            };
            let _ = view.update(cx, |v, cx| {
                v.finish_route(serial, p, op, result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_route(
        &mut self,
        serial: u64,
        p: ConnectionProfile,
        op: Operation,
        result: Outcome,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.serial != serial {
            return;
        }
        let f = self.navigation.form.as_mut().unwrap();
        if op == Operation::Connect
            && !f
                .connecting
                .as_ref()
                .is_some_and(|ticket| ticket.is_current())
        {
            self.close_form(cx);
            self.retire_disconnected_route(&p.id, cx);
            cx.notify();
            return;
        }
        f.job = None;
        f.challenge = None;
        match result {
            Outcome::Connected => {
                let ticket = f.connecting.take();
                self.close_form(cx);
                if let Some(ticket) = ticket {
                    self.publish_connection(&p.id, &ticket, cx);
                }
            }
            Outcome::Trust(challenge) => {
                f.challenge = Some((challenge, op, p));
                f.feedback = None;
                self.reveal_form(cx);
            }
            Outcome::Feedback(feedback) => {
                if op == Operation::Connect {
                    self.toasts.show(feedback.connection_failed(&p), cx);
                }
                f.feedback = Some(feedback);
                f.scroll.scroll_to_bottom();
                if op == Operation::Connect {
                    f.connecting = None;
                    self.navigation.status.insert(p.id, "Failed".into());
                    if !self.navigation.modal_open {
                        self.close_form(cx);
                    }
                }
            }
        }
        cx.notify();
    }

    fn current_form_result(&self, result: &Arc<String>) -> bool {
        self.navigation
            .form
            .as_ref()
            .and_then(|form| form.feedback.as_ref())
            .is_some_and(|feedback| Arc::ptr_eq(&feedback.message().full, result))
    }

    fn copy_form_result(&self, result: Arc<String>, cx: &mut Context<Self>) {
        if !self.current_form_result(&result) {
            return;
        }
        let order = clipboard_order::ClipboardWrite::begin(cx);
        let source = result.clone();
        let text = cx
            .background_executor()
            .spawn(async move { source.as_ref().clone() });
        cx.spawn(async move |view, cx| {
            let text = text.await;
            let _ = view.update(cx, |view, cx| {
                if view.current_form_result(&result) && order.current() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            });
        })
        .detach();
    }

    fn approve(&mut self, serial: u64, id: &str, cx: &mut Context<Self>) {
        if self.navigation.serial != serial {
            return;
        }
        let Some(f) = self.navigation.form.as_mut() else {
            return;
        };
        if !f.challenge.as_ref().is_some_and(|(c, _, _)| c.id == id) {
            return;
        }
        let (c, op, p) = f.challenge.take().unwrap();
        let draft = self.draft(cx).and_then(|p| {
            if op == Operation::Test {
                Some(p)
            } else {
                p.validated().ok()
            }
        });
        if draft.as_ref() == Some(&p) {
            self.run_route(p, op, Some(c.id), cx);
        }
    }
}
