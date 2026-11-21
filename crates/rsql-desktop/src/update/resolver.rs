use reqwest::dns::{Name, Resolve, Resolving};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::PathBuf,
    process::{Child, Command, ExitCode, Stdio},
    time::Duration,
};

const FLAG: &str = "--resolve-update-host";
const ADDRESS_LIMIT: usize = 8;
const OUTPUT_LIMIT: usize = 320;

pub(super) struct RejectLookup;
impl Resolve for RejectLookup {
    fn resolve(&self, _: Name) -> Resolving {
        Box::pin(async { Err("update address was not pre-resolved".into()) })
    }
}

fn valid_host(host: &str) -> bool {
    let labels = host.strip_suffix('.').unwrap_or(host);
    labels.len() <= 253
        && labels.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
}

/// Dispatch before desktop initialization. This mode only prints addresses for one hostname.
pub fn dispatch(arguments: &[OsString]) -> Option<ExitCode> {
    if arguments.first().map(OsString::as_os_str) != Some(OsStr::new(FLAG)) {
        return None;
    }
    let result = match arguments {
        [_, name] => name
            .to_str()
            .filter(|name| valid_host(name))
            .and_then(|name| lookup(name, &mut std::io::stdout().lock()).ok()),
        _ => None,
    };
    Some(if result.is_some() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn lookup(host: &str, out: &mut impl Write) -> std::io::Result<()> {
    let mut unique = Vec::new();
    for addr in (host, 0).to_socket_addrs()? {
        if !unique.contains(&addr.ip()) {
            unique.push(addr.ip());
            writeln!(out, "{}", addr.ip())?;
            if unique.len() == ADDRESS_LIMIT {
                break;
            }
        }
    }
    if unique.is_empty() {
        return Err(std::io::Error::other("no addresses"));
    }
    Ok(())
}

struct OwnedChild {
    process: Child,
    reaped: bool,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }
}

pub(super) struct Resolver {
    executable: PathBuf,
}

impl Resolver {
    pub(super) fn new() -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(Self { executable })
    }

    pub(super) async fn resolve(&self, host: &str) -> Result<Vec<SocketAddr>, String> {
        if !valid_host(host) {
            return Err("Invalid update hostname.".into());
        }
        let mut command = Command::new(&self.executable);
        command.args([FLAG, host]);
        let mut child = OwnedChild {
            process: command
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "Cannot start update hostname lookup.")?,
            reaped: false,
        };
        loop {
            if let Some(status) = child
                .process
                .try_wait()
                .map_err(|_| "Cannot wait for update hostname lookup.")?
            {
                child.reaped = true;
                if !status.success() {
                    return Err("Update hostname lookup failed.".into());
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut output = Vec::new();
        child
            .process
            .stdout
            .take()
            .ok_or("Missing resolver output")?
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut output)
            .map_err(|_| "Cannot read update hostname lookup.")?;
        decode(&output)
    }
}

fn decode(output: &[u8]) -> Result<Vec<SocketAddr>, String> {
    if output.len() > OUTPUT_LIMIT || !output.ends_with(b"\n") {
        return Err("Invalid resolver response.".into());
    }
    let text = std::str::from_utf8(output).map_err(|_| "Invalid resolver output")?;
    let mut addrs = Vec::new();
    for line in text.lines() {
        let ip: IpAddr = line.parse().map_err(|_| "Invalid resolver address")?;
        let addr = SocketAddr::new(ip, 0);
        if addrs.len() >= ADDRESS_LIMIT || addrs.contains(&addr) || ip.to_string() != line {
            return Err("Invalid resolver address list.".into());
        }
        addrs.push(addr);
    }
    if addrs.is_empty() {
        return Err("Empty resolver response.".into());
    }
    Ok(addrs)
}
