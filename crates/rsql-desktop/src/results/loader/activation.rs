use super::{Arc, Loader, PageLoader, Weak, pump};

pub struct FirstPageRead {
    loader: Weak<Loader>,
    generation: u64,
}

impl FirstPageRead {
    pub fn is_current(&self) -> bool {
        self.loader
            .upgrade()
            .is_some_and(|loader| self.generation == *loader.generation.borrow())
    }
}

impl Drop for FirstPageRead {
    fn drop(&mut self) {
        if let Some(loader) = self.loader.upgrade() {
            loader.stop(false, Some(self.generation));
        }
    }
}

impl PageLoader {
    pub fn resume(&self) {
        if self.0.schedule.lock().unwrap().grid_viewport.is_some() {
            self.suspend();
        }
        self.activate(false);
    }

    pub fn read_first_page(&self) -> FirstPageRead {
        self.suspend();
        self.activate(true);
        FirstPageRead {
            loader: Arc::downgrade(&self.0),
            generation: self.generation(),
        }
    }

    pub(crate) fn suspend_grid(&self) {
        if self.0.schedule.lock().unwrap().grid_viewport.is_none() {
            self.suspend();
        }
    }

    fn activate(&self, first_page: bool) {
        let mut lane = self.0.lane.0.lock().unwrap();
        let mut s = self.0.schedule.lock().unwrap();
        if s.disposed || s.active {
            return;
        }
        s.active = true;
        let mut cache = self.0.cache.0.state.lock().unwrap();
        cache.active = true;
        if first_page {
            s.grid_viewport = Some(std::mem::replace(&mut cache.viewport, 0..1));
        }
        drop(cache);
        self.0.queue(&mut s, lane.active);
        drop(s);
        pump(&mut lane);
    }
}
