//! Terminate jobs belonging to a PTY whose session leader is still our unreaped child.
use std::io;

/// Observe child exit without reaping the session leader until terminal teardown.
pub(super) fn exited(pid: u32) -> io::Result<bool> {
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(unsafe { info.assume_init().si_pid() } != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

pub(super) fn terminate_jobs(leader: i32) -> io::Result<()> {
    if leader <= 1 {
        return Ok(());
    }
    // The unreaped leader pins this group ID. Stop the shell from launching further jobs.
    unsafe { libc::kill(-leader, libc::SIGKILL) };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut system = sysinfo::System::new().map_err(io::Error::other)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                true,
                sysinfo::ProcessRefreshKind::nothing(),
            );
            let mut remaining = false;
            let mut first_error = None;
            for (pid, process) in system.processes() {
                let pid = pid.as_u32() as i32;
                if pid <= 1
                    || matches!(
                        process.status(),
                        sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
                    )
                    || unsafe { libc::getsid(pid) } != leader
                    || unsafe { libc::getpgid(pid) } == leader
                {
                    continue;
                }
                remaining = true;
                if let Err(error) = signal_member(pid, leader) {
                    first_error.get_or_insert(error);
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
            if !remaining {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Terminal jobs did not exit",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn signal_member(pid: i32, leader: i32) -> io::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return gone_or_error();
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    if unsafe { libc::getsid(pid) } == leader
        && unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                fd.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } < 0
    {
        return gone_or_error();
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn signal_member(pid: i32, leader: i32) -> io::Result<()> {
    use mach2::{
        mach_port::mach_port_deallocate,
        port::mach_port_t as Port,
        task_info::{TASK_AUDIT_TOKEN, TASK_AUDIT_TOKEN_COUNT},
        traps::mach_task_self,
    };
    unsafe extern "C" {
        fn task_name_for_pid(task: Port, pid: i32, name: *mut Port) -> i32;
    }
    // Old macOS releases lack this function; keep the legacy app loadable there.
    let symbol =
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
    if symbol.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "This macOS version cannot safely signal terminal jobs in separate process groups",
        ));
    }
    type SignalJob = unsafe extern "C" fn(*mut [u32; 8], i32) -> i32;
    let signal_job = unsafe { std::mem::transmute::<*mut libc::c_void, SignalJob>(symbol) };
    let mut token = [0u32; 8];
    let mut length = token.len() as u32;
    let mut name = 0;
    // TASK_AUDIT_TOKEN gives us a generation-qualified process identity.
    let task = unsafe { mach_task_self() };
    let result = unsafe { task_name_for_pid(task, pid, &mut name) };
    if result != 0 {
        if unsafe { libc::getsid(pid) } != leader {
            return Ok(());
        }
        return Err(io::Error::other(format!(
            "Cannot identify terminal job {pid} (Mach error {result})"
        )));
    }
    let result = unsafe {
        libc::task_info(
            name,
            TASK_AUDIT_TOKEN,
            token.as_mut_ptr().cast(),
            &mut length,
        )
    };
    unsafe { mach_port_deallocate(task, name) };
    if unsafe { libc::getsid(pid) } != leader {
        return Ok(());
    }
    if result != 0 || length != TASK_AUDIT_TOKEN_COUNT {
        return Err(io::Error::other(format!(
            "Cannot read terminal job identity (Mach error {result})"
        )));
    }
    match unsafe { signal_job(&mut token, libc::SIGKILL) } {
        0 | libc::ESRCH => Ok(()),
        errno => Err(io::Error::from_raw_os_error(errno)),
    }
}

#[cfg(target_os = "linux")]
fn gone_or_error() -> io::Result<()> {
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}
