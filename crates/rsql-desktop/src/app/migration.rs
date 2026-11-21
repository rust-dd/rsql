mod dialog;
mod drafts;
use super::*;
use crate::desktop::{DesktopServices, Relaunch};
use gpui_kit::component::WindowExt;
use rsql_desktop::{migration::review::ReviewedOperation, update::install::Staged};
use std::{cell::Cell, path::PathBuf, pin::Pin};
use tokio::{runtime::Handle, sync::watch};

actions!(rsql, [ReviewMigration]);

type Selection = Pin<Box<dyn Future<Output = Option<PathBuf>>>>;
type Picker = Rc<dyn Fn(&Window, bool) -> Selection>;
type Outcome = Result<String, String>;
type Transition =
    Rc<dyn Fn(uuid::Uuid, Option<ReviewedOperation>, Option<Staged>, &mut Window, &mut App)>;
pub(super) type Delivery = watch::Receiver<Option<Outcome>>;

pub(super) struct Migration {
    runtime: Handle,
    profile: PathBuf,
    live_lease: RefCell<std::sync::Weak<rsql_core::migration::ProfileLease>>,
    picker: Picker,
    quit: fn(&mut App),
    restart: Result<Relaunch, String>,
    attached: Cell<bool>,
    copy_saved: Cell<bool>,
    #[cfg(target_os = "macos")]
    termination: RefCell<Option<gpui_macos::termination::TerminationGuard>>,
    startup_error: Option<Rc<RefCell<Option<String>>>>,
    pub(super) transition: RefCell<Option<Transition>>,
    reservation: Cell<Option<uuid::Uuid>>,
    close_requested: Cell<bool>,
    delivery: RefCell<Option<Delivery>>,
    dialog: RefCell<Option<WeakEntity<dialog::Dialog>>>,
}

pub(super) async fn receive(mut delivery: Delivery) -> Outcome {
    delivery
        .wait_for(Option::is_some)
        .await
        .map_err(|_| "Migration worker stopped".to_string())?
        .clone()
        .unwrap()
}

impl Migration {
    pub(super) fn new(
        runtime: Handle,
        profile: PathBuf,
        restart: Result<Relaunch, String>,
        startup_error: Option<Rc<RefCell<Option<String>>>>,
    ) -> Rc<Self> {
        Rc::new(Self {
            runtime,
            profile,
            live_lease: Default::default(),
            picker: Rc::new(DesktopServices::pick_migration_path),
            quit: |cx| cx.quit(),
            restart,
            attached: Cell::new(startup_error.is_none()),
            copy_saved: Cell::default(),
            #[cfg(target_os = "macos")]
            termination: RefCell::default(),
            startup_error,
            transition: Default::default(),
            reservation: Default::default(),
            close_requested: Cell::default(),
            delivery: RefCell::default(),
            dialog: RefCell::default(),
        })
    }

    pub(super) fn attach_lease(&self, lease: &std::sync::Arc<rsql_core::migration::ProfileLease>) {
        self.live_lease.replace(std::sync::Arc::downgrade(lease));
    }

    pub(super) fn bind(self: &Rc<Self>, window: &Window, cx: &mut App) {
        #[cfg(target_os = "macos")]
        self.bind_system_quit();
        let migration = self.clone();
        let handle = window.window_handle();
        cx.on_action(move |_: &ReviewMigration, cx| {
            let migration = migration.clone();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, w, cx| migration.open(w, cx));
            });
        });
    }

    #[cfg(target_os = "macos")]
    fn bind_system_quit(self: &Rc<Self>) {
        let main = objc2_foundation::MainThreadMarker::new();
        let main = main.expect("AppKit windows bind on the main thread");
        let owner = Rc::downgrade(self);
        self.termination
            .replace(Some(gpui_macos::termination::TerminationGuard::new(
                main,
                move || owner.upgrade().is_some_and(|gate| !gate.defer_close()),
            )));
    }

    fn open(self: &Rc<Self>, w: &mut Window, cx: &mut App) {
        if self.busy()
            || self.copy_saved.get()
            || self
                .dialog
                .borrow()
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some()
        {
            return;
        }
        let view = cx.new(|_| dialog::Dialog::new(self.clone()));
        self.dialog.replace(Some(view.downgrade()));
        w.open_dialog(cx, move |frame, _, _| {
            frame
                .title("Import and recover profile")
                .w(px(590.))
                .close_button(false)
                .keyboard(false)
                .overlay_closable(false)
                .child(view.clone())
        });
    }

    fn busy(&self) -> bool {
        self.reservation.get().is_some()
    }

    pub(super) fn reserve(&self) -> Option<uuid::Uuid> {
        if self.busy() {
            return None;
        }
        let id = uuid::Uuid::new_v4();
        self.reservation.set(Some(id));
        Some(id)
    }

    pub(super) fn defer_close(&self) -> bool {
        if self.busy() {
            self.close_requested.set(true);
            true
        } else {
            false
        }
    }

    pub(super) fn detached(&self) {
        self.attached.set(false);
        self.copy_saved.set(false);
    }

    fn submit<F, W>(&self, prepare: F) -> Delivery
    where
        F: FnOnce() -> Result<W, String> + Send + 'static,
        W: FnOnce() -> Outcome + Send + 'static,
    {
        let (tx, rx) = watch::channel(None);
        self.delivery.replace(Some(rx.clone()));
        self.runtime.spawn_blocking(move || {
            let result = prepare().and_then(|work| work());
            tx.send_replace(Some(result));
        });
        rx
    }

    fn start(&self, work: ReviewedOperation) -> Delivery {
        self.submit(move || Ok(move || work.run().map_err(|e| e.to_string())))
    }

    pub(super) fn retiring(&self, work: Option<ReviewedOperation>, session: &Session) -> Delivery {
        let transfer = session.retire_profile().map_err(|e| e.to_string());
        let relaunch_error = self.restart.as_ref().err().cloned();
        self.submit(move || {
            let lease = transfer?
                .blocking_recv()
                .map_err(|_| "Profile retirement interrupted".to_string())?;
            if let Some(error) = relaunch_error {
                return Err(error);
            }
            let reviewed = work
                .map(|operation| operation.bind_retired(&lease))
                .transpose()
                .map_err(|e| e.to_string())?;
            drop(lease);
            Ok(move || match reviewed {
                Some(operation) => operation.run().map_err(|e| e.to_string()),
                None => Ok(String::new()),
            })
        })
    }

    pub(super) fn finish(&self, id: uuid::Uuid, cx: &mut App) -> bool {
        if self.reservation.get() != Some(id) {
            return false;
        }
        self.reservation.set(None);
        self.delivery.take();
        if self.close_requested.replace(false) {
            (self.quit)(cx);
        }
        true
    }

    pub(super) fn join_admitted(&self) {
        if let Some(rx) = self.delivery.take()
            && let Err(error) = self.runtime.block_on(receive(rx))
        {
            eprintln!("Profile operation failed during shutdown: {error}");
        }
    }

    pub(super) fn closing(&self) -> bool {
        self.close_requested.get()
    }
}
