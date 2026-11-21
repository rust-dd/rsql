use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    runtime::Handle,
    sync::{Notify, oneshot, watch},
    task::JoinHandle,
};

pub const FLAG: &str = "--format-sql";

pub fn child_main() -> Result<(), String> {
    let mut sql = String::new();
    std::io::stdin()
        .read_to_string(&mut sql)
        .map_err(|e| e.to_string())?;
    let text = rsql_desktop::sql::format(&sql)?;
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())
}

type Reply = Result<String, String>;
type Queue = Arc<Mutex<VecDeque<Job>>>;

struct Job {
    sql: String,
    cancelled: oneshot::Receiver<()>,
    reply: oneshot::Sender<Reply>,
}

pub struct Formatter {
    queue: Queue,
    wake: Arc<Notify>,
    stop: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}

impl Formatter {
    pub fn new(runtime: &Handle, executable: PathBuf) -> Self {
        let queue = Queue::default();
        let wake = Arc::new(Notify::new());
        let (stop, mut stopped) = watch::channel(false);
        let pending = queue.clone();
        let ready = wake.clone();
        let worker = runtime.spawn(async move {
            loop {
                if *stopped.borrow() {
                    break;
                }
                let next = pending.lock().unwrap().pop_front();
                if let Some(mut job) = next {
                    if !matches!(
                        job.cancelled.try_recv(),
                        Err(oneshot::error::TryRecvError::Empty)
                    ) {
                        continue;
                    }
                    let cancelled = async {
                        tokio::select! { _ = &mut job.cancelled => (), _ = stopped.changed() => () }
                    };
                    let result = format_child(&executable, &job.sql, cancelled).await;
                    let _ = job.reply.send(result);
                } else {
                    tokio::select! { _ = ready.notified() => (), _ = stopped.changed() => () }
                }
            }
            pending.lock().unwrap().clear();
        });
        Self {
            queue,
            wake,
            stop,
            worker: Some(worker),
        }
    }

    pub fn submit(
        &self,
        sql: String,
    ) -> Result<(oneshot::Sender<()>, oneshot::Receiver<Reply>), String> {
        let (cancel, cancelled) = oneshot::channel();
        let (reply, result) = oneshot::channel();
        let mut queue = self.queue.lock().unwrap();
        queue.retain_mut(|job| {
            matches!(
                job.cancelled.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            )
        });
        if queue.len() >= 8 || *self.stop.borrow() {
            return Err("Formatter busy. Try again shortly.".into());
        }
        queue.push_back(Job {
            sql,
            cancelled,
            reply,
        });
        self.wake.notify_one();
        Ok((cancel, result))
    }

    pub fn shutdown(&mut self, runtime: &Handle) {
        self.stop.send_replace(true);
        if let Some(worker) = self.worker.take() {
            let _ = runtime.block_on(worker);
        }
    }
}

impl Drop for Formatter {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

async fn format_child(executable: &Path, sql: &str, cancelled: impl Future<Output = ()>) -> Reply {
    let mut command = Command::new(executable);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .arg(FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Start formatter: {e}"))?;
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let mut errors = child.stderr.take().unwrap();
    let mut formatted = String::new();
    let mut error = String::new();
    let exchange = async {
        let write = async {
            input.write_all(sql.as_bytes()).await?;
            drop(input);
            Ok::<_, std::io::Error>(())
        };
        tokio::try_join!(
            write,
            output.read_to_string(&mut formatted),
            errors.read_to_string(&mut error),
            child.wait()
        )
    };
    let response =
        tokio::select! { biased; _ = cancelled => None, result = exchange => Some(result) };
    match response {
        Some(Ok((_, _, _, status))) if status.success() => Ok(formatted),
        Some(Ok(_)) => Err(error.trim().to_owned()),
        failure => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(failure
                .and_then(Result::err)
                .map_or_else(|| "Formatting cancelled".into(), |e| e.to_string()))
        }
    }
}
