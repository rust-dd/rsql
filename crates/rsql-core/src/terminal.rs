use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::spawn_blocking,
};

#[cfg(unix)]
#[path = "terminal_unix.rs"]
mod unix;

#[path = "terminal_completion.rs"]
mod completion;
pub use completion::{OutputAfterFailure, TerminalCompletion};

pub struct OwnedTerminal {
    pub output: mpsc::Receiver<Vec<u8>>,
    pub completion: TerminalCompletion,
}
type Output = mpsc::Receiver<Vec<u8>>;
struct Handles {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}
type SessionIo = Arc<Handles>;

struct Session {
    io: std::sync::Weak<Handles>,
    stop: watch::Sender<bool>,
    task: tokio::task::Id,
    worker: TerminalCompletion,
}

#[derive(Default)]
pub struct Terminals {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    lifecycle: tokio::sync::Mutex<()>,
    closed: AtomicBool,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}
fn message(e: impl ToString) -> String {
    e.to_string()
}

struct Opened {
    child: Box<dyn Child + Send + Sync>,
    reader: Box<dyn Read + Send>,
    io: SessionIo,
    #[cfg(unix)]
    read_fd: std::os::fd::OwnedFd,
}
impl Opened {
    fn new(cmd: CommandBuilder, cols: u16, rows: u16) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(message)?;
        let reader = pair.master.try_clone_reader().map_err(message)?;
        let writer = pair.master.take_writer().map_err(message)?;
        #[cfg(unix)]
        let read_fd = {
            use std::os::fd::BorrowedFd;
            let fd = pair
                .master
                .as_raw_fd()
                .ok_or("Terminal descriptor unavailable")?;
            // The master owns this descriptor until after the duplicate is created.
            unsafe { BorrowedFd::borrow_raw(fd) }
                .try_clone_to_owned()
                .map_err(message)?
        };
        let child = pair.slave.spawn_command(cmd).map_err(message)?;
        Ok(Self {
            child,
            reader,
            io: Arc::new(Handles {
                master: Arc::new(Mutex::new(pair.master)),
                writer: Arc::new(Mutex::new(writer)),
            }),
            #[cfg(unix)]
            read_fd,
        })
    }
}

impl Terminals {
    pub async fn spawn(&self, id: &str, cols: u16, rows: u16) -> Result<Output, String> {
        Ok(self.spawn_owned(id, cols, rows).await?.output)
    }

    pub async fn spawn_owned(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<OwnedTerminal, String> {
        let mut cmd = CommandBuilder::new_default_prog();
        cmd.env("TERM", "xterm-256color");
        self.spawn_command_owned(id, cols, rows, cmd).await
    }

    pub async fn spawn_command(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        cmd: CommandBuilder,
    ) -> Result<Output, String> {
        Ok(self.spawn_command_owned(id, cols, rows, cmd).await?.output)
    }

    pub async fn spawn_command_owned(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        cmd: CommandBuilder,
    ) -> Result<OwnedTerminal, String> {
        let _guard = self.lifecycle.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err("Terminals closed".into());
        }
        self.remove(id).await?;
        let (output, rx) = mpsc::channel(16);
        let (stop, mut stopping) = watch::channel(false);
        let (ready, opened) = oneshot::channel();
        let (request, failed_control) = oneshot::channel();
        let (registered, start) = oneshot::channel();
        let registry = Arc::downgrade(&self.sessions);
        let name = id.to_owned();
        let worker = tokio::spawn(async move {
            let opened = spawn_blocking(move || Opened::new(cmd, cols, rows))
                .await
                .map_err(message)?;
            let opened = match opened {
                Ok(o) => o,
                Err(e) => {
                    let _ = ready.send(Err(e.clone()));
                    return Err(e);
                }
            };
            let _ = ready.send(Ok(opened.io.clone()));
            let _ = start.await;
            let result = run_terminal(opened, output, &mut stopping, failed_control).await;
            if let Err(error) = &result {
                tracing::error!(terminal_id = %name, %error, "Terminal session cleanup failed");
            }
            if let Some(registry) = registry.upgrade() {
                let mut sessions = registry.lock().unwrap();
                if sessions
                    .get(&name)
                    .is_some_and(|s| s.task == tokio::task::id())
                {
                    sessions.remove(&name);
                }
            }
            result
        });
        let task = worker.id();
        let completion = TerminalCompletion::new(worker, request);
        let io = match opened.await.map_err(message).and_then(|result| result) {
            Ok(io) => io,
            Err(error) => {
                return completion.wait().await.and(Err(error));
            }
        };
        self.sessions.lock().unwrap().insert(
            id.into(),
            Session {
                io: Arc::downgrade(&io),
                stop,
                task,
                worker: completion.clone(),
            },
        );
        let _ = registered.send(());
        Ok(OwnedTerminal {
            output: rx,
            completion,
        })
    }

    async fn remove(&self, id: &str) -> Result<(), String> {
        let session = self.sessions.lock().unwrap().remove(id);
        if let Some(session) = session {
            session.stop.send_replace(true);
            session.worker.wait().await?;
        }
        Ok(())
    }

    pub async fn close(&self, id: &str) -> Result<(), String> {
        let _guard = self.lifecycle.lock().await;
        self.remove(id).await
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        let _guard = self.lifecycle.lock().await;
        self.closed.store(true, Ordering::Release);
        let sessions = std::mem::take(&mut *self.sessions.lock().unwrap());
        for s in sessions.values() {
            s.stop.send_replace(true);
        }
        let mut result = Ok(());
        for s in sessions.into_values() {
            result = result.and(s.worker.wait().await);
        }
        result
    }

    /// Whether PTY I/O handles can still be acquired. They can outlive child
    /// exit and be released before the forwarder delivers its final output.
    pub fn has_live_io(&self, id: &str) -> bool {
        self.io(id).is_ok()
    }

    fn io(&self, id: &str) -> Result<SessionIo, String> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .and_then(|s| s.io.upgrade())
            .ok_or_else(|| "Terminal not found".into())
    }

    pub async fn write(&self, id: &str, data: Vec<u8>) -> Result<(), String> {
        let writer = self.io(id)?.writer.clone();
        spawn_blocking(move || {
            let mut writer = writer.lock().unwrap();
            writer
                .write_all(&data)
                .and_then(|_| writer.flush())
                .map_err(message)
        })
        .await
        .map_err(message)?
    }
    pub async fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let master = self.io(id)?.master.clone();
        spawn_blocking(move || {
            master
                .lock()
                .unwrap()
                .resize(size(cols, rows))
                .map_err(message)
        })
        .await
        .map_err(message)?
    }
}

fn child_exited(child: &mut dyn Child) -> Result<bool, String> {
    #[cfg(unix)]
    {
        unix::exited(child.process_id().ok_or("Missing terminal PID")?).map_err(message)
    }
    #[cfg(windows)]
    {
        child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(message)
    }
}

async fn run_terminal(
    opened: Opened,
    output: mpsc::Sender<Vec<u8>>,
    stop: &mut watch::Receiver<bool>,
    mut failed_control: oneshot::Receiver<completion::FailureReply>,
) -> Result<(), String> {
    let Opened {
        mut child,
        mut reader,
        io,
        #[cfg(unix)]
        read_fd,
    } = opened;
    let (tx, mut chunks) = mpsc::channel(1);
    #[cfg(unix)]
    let stopping = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    let finished = stopping.clone();
    let read = spawn_blocking(move || {
        let mut buf = [0; 4096];
        loop {
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                if finished.load(Ordering::Acquire) {
                    break;
                }
                let mut descriptor = libc::pollfd {
                    fd: read_fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // read_fd remains owned by this worker for the duration of poll.
                let n = unsafe { libc::poll(&mut descriptor, 1, 100) };
                if n == 0
                    || (n < 0
                        && std::io::Error::last_os_error().kind()
                            == std::io::ErrorKind::Interrupted)
                {
                    continue;
                }
                if n < 0 {
                    break;
                }
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // ConPTY can flush output during ClosePseudoConsole, even after cancellation.
                    if !tx.is_closed() {
                        let _ = tx.blocking_send(buf[..n].to_vec());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
    let stop_forwarding = tokio::sync::Notify::new();
    let mut forward_stop = stop.clone();
    let forward = async {
        let drain = async {
            while let Some(bytes) = chunks.recv().await {
                if output.send(bytes).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            _ = stop_forwarding.notified() => {}
            _ = forward_stop.changed() => {}
            _ = output.closed() => {}
            _ = drain => {}
        }
        #[cfg(unix)]
        stopping.store(true, Ordering::Release);
        drop(chunks);
    };
    let supervise = async {
        let exit = async {
            loop {
                if child_exited(child.as_mut())? {
                    return Ok::<_, String>(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        };
        let completion = tokio::select! {
            _ = stop.changed() => Ok(()),
            _ = output.closed() => Ok(()),
            result = exit => result,
            Ok(reply) = &mut failed_control => {
                let decision = child_exited(child.as_mut()).map(|exited| {
                    if exited {
                        OutputAfterFailure::Drain
                    } else {
                        stop_forwarding.notify_one();
                        OutputAfterFailure::CloseReceiver
                    }
                });
                let _ = reply.send(decision.clone());
                decision.map(|_| ())
            }
        };
        drop(failed_control);
        spawn_blocking(move || {
            // The Unix leader remains unreaped until its jobs have been signalled.
            #[cfg(unix)]
            let jobs =
                unix::terminate_jobs(child.process_id().ok_or("Missing terminal PID")? as i32)
                    .map_err(message);
            #[cfg(windows)]
            let jobs = Ok(());
            if child.try_wait().map_err(message)?.is_none() {
                child.kill().map_err(message)?;
            }
            // ClosePseudoConsole may flush output; the reader remains active here.
            drop(io);
            child.wait().map_err(message)?;
            jobs.and(completion)
        })
        .await
        .map_err(message)?
    };
    let finish = async {
        let result = supervise.await;
        if result.is_err() {
            stop_forwarding.notify_one();
        }
        result
    };
    let ((), result) = tokio::join!(forward, finish);
    read.await.map_err(message)?;
    result
}
