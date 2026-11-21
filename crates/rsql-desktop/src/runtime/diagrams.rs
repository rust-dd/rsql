use super::Session;
use rsql_desktop::erd::DiagramOwner;
use std::{cell::RefCell, rc::Rc};
use tokio::{runtime::Handle, sync::watch};

pub(crate) struct LiveDiagram {
    pub(crate) owner: RefCell<Option<DiagramOwner>>,
    pub(crate) export: RefCell<Option<(uuid::Uuid, watch::Sender<()>)>>,
    runtime: Handle,
    shutdown: watch::Sender<bool>,
}

impl Session {
    pub(crate) fn register_diagram(&self, owner: DiagramOwner) -> Rc<LiveDiagram> {
        let live = Rc::new(LiveDiagram {
            owner: RefCell::new(Some(owner)),
            export: Default::default(),
            runtime: self.runtime.clone(),
            shutdown: self.preflight_stop.clone(),
        });
        let mut diagrams = self.diagrams.borrow_mut();
        diagrams.retain(|diagram| diagram.strong_count() != 0);
        diagrams.push(Rc::downgrade(&live));
        live
    }
}

impl LiveDiagram {
    pub(crate) fn cancel_export(&self) {
        if let Some((_, cancel)) = self.export.take() {
            cancel.send_replace(());
        }
    }

    pub(crate) fn retire(&self) {
        self.cancel_export();
        if let Some(owner) = self.owner.take() {
            let settled = owner.retire();
            let joined = self.shutdown.subscribe();
            self.runtime.spawn(async move {
                settled.await;
                drop(joined);
            });
        }
    }
}

impl Drop for LiveDiagram {
    fn drop(&mut self) {
        self.retire();
    }
}
