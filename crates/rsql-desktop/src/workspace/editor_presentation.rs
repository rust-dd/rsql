use gpui_kit::{Task, component::input::Rope};
use std::cell::{Cell, RefCell};
use tokio::sync::watch;

struct Active {
    text: Rope,
    caret: usize,
    stop: watch::Sender<bool>,
}

#[derive(Default)]
pub(super) struct Presentation {
    generation: Cell<u64>,
    active: RefCell<Option<Active>>,
    references: RefCell<Option<Task<()>>>,
}

pub(super) struct Request {
    pub generation: u64,
    stop: watch::Sender<bool>,
}

impl Request {
    pub fn signal(&self) -> watch::Receiver<bool> {
        self.stop.subscribe()
    }

    pub async fn while_current<T>(&self, work: impl Future<Output = T>) -> Option<T> {
        let mut signal = self.signal();
        tokio::select! { biased;
            _ = signal.wait_for(|stop| *stop) => None,
            result = work => Some(result),
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

impl Presentation {
    pub fn start(&self, text: &Rope, caret: usize) -> Request {
        self.dismiss();
        let stop = watch::channel(false).0;
        self.active.replace(Some(Active {
            text: text.clone(),
            caret,
            stop: stop.clone(),
        }));
        Request {
            generation: self.generation.get(),
            stop,
        }
    }
    pub fn own_references(&self, task: Task<()>) {
        self.references.replace(Some(task));
    }
    pub fn matches(&self, generation: u64) -> bool {
        self.generation.get() == generation
    }
    pub fn changed(&self, text: &Rope, caret: usize) {
        if self
            .active
            .borrow()
            .as_ref()
            .is_some_and(|a| a.caret != caret || a.text != *text)
        {
            self.dismiss();
        }
    }
    pub fn pending(&self) -> bool {
        self.active
            .borrow()
            .as_ref()
            .is_some_and(|a| !*a.stop.borrow())
    }
    pub fn dismiss(&self) {
        self.generation.set(self.generation.get() + 1);
        if let Some(active) = self.active.take() {
            active.stop.send_replace(true);
        }
        self.references.take();
    }
}
