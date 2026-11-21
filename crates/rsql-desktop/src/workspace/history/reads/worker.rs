use super::*;
use rsql_core::storage::LocalConnection;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;

pub(in crate::workspace::history) struct Read {
    _cancel: oneshot::Sender<()>,
    _delivery: Task<()>,
}

impl Panel {
    pub(in crate::workspace::history) fn read<T, F>(
        &self,
        work: impl FnOnce(LocalConnection) -> F + Send + 'static,
        publish: impl FnOnce(&mut Panel, Result<T, String>, &mut Context<Panel>) + 'static,
        cx: &mut Context<Self>,
    ) -> Read
    where
        T: Send + 'static,
        F: Future<Output = Result<T, String>>,
    {
        let services = self.session.services.clone();
        let mut stop = self.session.preflight_stop.subscribe();
        let (cancel, cancelled) = oneshot::channel::<()>();
        let runtime = self.session.runtime.clone();
        let job = self.session.runtime.spawn(async move {
            let (opened, connection) = oneshot::channel();
            let stopping = Arc::new(AtomicBool::new(false));
            let worker_stopping = stopping.clone();
            let mut worker = tokio::task::spawn_blocking(move || {
                if worker_stopping.load(Ordering::Acquire) {
                    return None;
                }
                let db = match services.local_db.connect() {
                    Ok(db) => db,
                    Err(e) => return Some(Err(e.to_string())),
                };
                let _ = opened.send(db.clone());
                if worker_stopping.load(Ordering::Acquire) {
                    return None;
                }
                Some(runtime.block_on(work(db)))
            });
            tokio::select! {
                biased;
                _ = stop.wait_for(|stop| *stop) => {}
                _ = cancelled => {}
                result = &mut worker => return result.unwrap_or_else(|e| Some(Err(e.to_string()))),
            }
            stopping.store(true, Ordering::Release);
            worker.abort();
            if let Ok(db) = connection.await {
                // SQLite forgets interrupts delivered before the first step or between statements.
                loop {
                    let _ = db.interrupt();
                    tokio::select! {
                        _ = &mut worker => break,
                        _ = tokio::time::sleep(Duration::from_millis(2)) => {}
                    }
                }
            } else {
                let _ = worker.await;
            }
            // Shutdown waits on this receiver until the blocking worker has exited.
            drop(stop);
            None
        });
        let delivery = cx.spawn(async move |panel, cx| {
            let result = job.await.unwrap_or_else(|e| Some(Err(e.to_string())));
            if let Some(result) = result {
                let _ = panel.update(cx, |p, cx| publish(p, result, cx));
            }
        });
        Read {
            _cancel: cancel,
            _delivery: delivery,
        }
    }
}
