use rsql_core::services::ProjectTicket;
use rsql_desktop::{
    geometry::{self, Bounds, Geometry, GeometryBudget, GeometryError, Resource},
    results::diff::RowSnapshot,
};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
use tokio::sync::oneshot;

#[derive(Default)]
pub struct Shapes {
    pub geometries: Box<[Arc<Geometry>]>,
    pub bounds: Bounds,
    pub omitted: [usize; 6],
}

pub struct Output {
    pub column: Option<usize>,
    pub shapes: Option<Shapes>,
}

struct Job {
    rows: RowSnapshot,
    route: ProjectTicket,
    parse: bool,
    stop: Arc<AtomicBool>,
    send: oneshot::Sender<Output>,
}

impl Job {
    fn run(self) {
        let stopped = || self.stop.load(Ordering::Acquire) || !self.route.is_current();
        if stopped() {
            return;
        }
        let rows = &self.rows;
        let sample =
            (0..rows.len()).filter_map(|i| rows.page.get(rows.source_index(i)?).map(Vec::as_slice));
        let column = geometry::detect_sample(&rows.columns, sample);
        let mut shapes = None;
        if self.parse {
            let mut result = Shapes::default();
            let mut geometries = Vec::new();
            let budget = GeometryBudget::default();
            if let Some(column) = column {
                for i in 0..rows.len() {
                    let Some(row) = rows.source_index(i) else {
                        continue;
                    };
                    let Some(value) = rows.page.get(row).and_then(|r| r.get(column)) else {
                        continue;
                    };
                    match geometry::decode(value, row, &budget, stopped) {
                        Ok(None) => (),
                        Ok(Some(geometry)) => {
                            geometries.push(Arc::new(geometry));
                        }
                        Err(GeometryError::Cancelled) => return,
                        Err(reason) => {
                            let index = match reason {
                                GeometryError::Unsupported => 0,
                                GeometryError::Malformed => 1,
                                GeometryError::Resource(Resource::InputBytes) => 2,
                                GeometryError::Resource(Resource::Coordinates) => 3,
                                GeometryError::Resource(Resource::RetainedBytes) => 4,
                                GeometryError::Resource(Resource::ParserBytes) => 5,
                                GeometryError::Cancelled => unreachable!(),
                            };
                            result.omitted[index] += 1;
                        }
                    }
                }
            }
            let Some(bounds) = Bounds::for_geometries(geometries.iter().map(Arc::as_ref), stopped)
            else {
                return;
            };
            result.bounds = bounds;
            result.geometries = geometries.into_boxed_slice();
            shapes = Some(result);
        }
        if !stopped() {
            let _ = self.send.send(Output { column, shapes });
        }
    }
}

#[derive(Default)]
struct Queue {
    pending: Option<Job>,
    running: Option<Arc<AtomicBool>>,
    stopped: bool,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

pub struct Ticket {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let mut queue = self.shared.queue.lock().unwrap();
        if queue
            .pending
            .as_ref()
            .is_some_and(|job| Arc::ptr_eq(&job.stop, &self.stop))
        {
            queue.pending.take();
        }
    }
}

#[derive(Default)]
pub struct Worker {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Worker {
    pub fn submit(
        &self,
        rows: RowSnapshot,
        route: ProjectTicket,
        parse: bool,
    ) -> (Ticket, oneshot::Receiver<Output>) {
        let (send, receive) = oneshot::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let ticket = Ticket {
            shared: self.shared.clone(),
            stop: stop.clone(),
        };
        let mut queue = self.shared.queue.lock().unwrap();
        if !queue.stopped {
            if let Some(active) = &queue.running {
                active.store(true, Ordering::Release);
            }
            queue.pending = Some(Job {
                rows,
                route,
                parse,
                stop,
                send,
            });
            self.thread.lock().unwrap().get_or_insert_with(|| {
                let shared = self.shared.clone();
                std::thread::spawn(move || Self::run(&shared))
            });
            self.shared.ready.notify_one();
        }
        (ticket, receive)
    }

    fn run(shared: &Shared) {
        loop {
            let mut queue = shared
                .ready
                .wait_while(shared.queue.lock().unwrap(), |q| {
                    !q.stopped && q.pending.is_none()
                })
                .unwrap();
            if queue.stopped {
                return;
            }
            let job = queue.pending.take().unwrap();
            queue.running = Some(job.stop.clone());
            drop(queue);
            job.run();
            shared.queue.lock().unwrap().running.take();
        }
    }

    pub fn shutdown(&self) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.stopped = true;
        queue.pending.take();
        if let Some(active) = &queue.running {
            active.store(true, Ordering::Release)
        }
        self.shared.ready.notify_one();
        drop(queue);
        if let Some(thread) = self.thread.lock().unwrap().take() {
            thread.join().unwrap()
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown()
    }
}
