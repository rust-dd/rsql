use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};

use crate::ApplicationServices;

#[derive(serde::Serialize, Clone, Debug)]
pub struct SystemResourceUsage {
    pub app_cpu_percent: f32,
    pub app_memory_rss_mb: u64,
    pub app_process_count: usize,
    pub network_rx_mbps: f32,
    pub network_tx_mbps: f32,
    pub db_connections_in_use: usize,
    pub db_connections_open: usize,
    pub db_connections_max: usize,
    pub db_connections_waiting: usize,
}

/// Walk a parent-of map downward from `root`.
///
/// One pass over the pids to build children lists, then a breadth-first walk.
/// The previous version rescanned every process on the machine once per level
/// of the tree until no new child turned up.
fn collect_descendants(parent_of: &[(Pid, Option<Pid>)], root: Pid) -> HashSet<Pid> {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, parent) in parent_of {
        if let Some(parent) = parent {
            children.entry(*parent).or_default().push(*pid);
        }
    }

    let mut included = HashSet::new();
    let mut queue = VecDeque::new();
    included.insert(root);
    queue.push_back(root);

    while let Some(pid) = queue.pop_front() {
        for child in children.get(&pid).into_iter().flatten() {
            if included.insert(*child) {
                queue.push_back(*child);
            }
        }
    }

    included
}

/// How many samples may reuse the known process tree before it is rediscovered.
/// Enumerating every process on the machine is the expensive part of a sample,
/// and the tree only changes when the app spawns something such as a terminal.
const REDISCOVER_EVERY: u32 = 5;

pub struct ResourceMonitor {
    system: System,
    networks: Networks,
    last_sample_at: Instant,
    app_pid: Pid,
    tracked: Vec<Pid>,
    samples_since_rediscover: u32,
}

impl ResourceMonitor {
    pub fn new() -> Result<Self, sysinfo::Error> {
        let app_pid = get_current_pid().unwrap_or_else(|_| Pid::from_u32(std::process::id()));
        let mut system = System::new_all()?;
        system.refresh_processes(ProcessesToUpdate::All, true);
        let mut networks = Networks::new_with_refreshed_list()?;
        networks.refresh(true);

        Ok(Self {
            system,
            networks,
            last_sample_at: Instant::now(),
            app_pid,
            tracked: vec![app_pid],
            samples_since_rediscover: REDISCOVER_EVERY,
        })
    }

    pub fn sample(&mut self) -> SystemResourceUsage {
        let refresh_kind = ProcessRefreshKind::nothing().with_cpu().with_memory();

        // Enumerating everything is only needed to notice processes the app has
        // spawned since the last look; in between, refresh just the ones we
        // already know about.
        let rediscover = self.samples_since_rediscover >= REDISCOVER_EVERY;
        if rediscover {
            self.system
                .refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);
            self.samples_since_rediscover = 0;

            let parent_of: Vec<(Pid, Option<Pid>)> = self
                .system
                .processes()
                .iter()
                .map(|(pid, process)| (*pid, process.parent()))
                .collect();
            self.tracked = collect_descendants(&parent_of, self.app_pid)
                .into_iter()
                .collect();
        } else {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&self.tracked),
                true,
                refresh_kind,
            );
            self.samples_since_rediscover += 1;
        }

        self.networks.refresh(true);
        let dt = self.last_sample_at.elapsed().as_secs_f32().max(0.001);
        self.last_sample_at = Instant::now();

        let processes = self.system.processes();
        let included = &self.tracked;

        let mut total_cpu = 0.0f32;
        let mut total_rss = 0u64;
        for pid in included {
            if let Some(process) = processes.get(pid) {
                total_cpu += process.cpu_usage();
                total_rss = total_rss.saturating_add(process.memory());
            }
        }

        let mut rx_bytes: u64 = 0;
        let mut tx_bytes: u64 = 0;
        for (_iface, net) in &self.networks {
            rx_bytes = rx_bytes.saturating_add(net.received());
            tx_bytes = tx_bytes.saturating_add(net.transmitted());
        }
        let network_rx_mbps = (rx_bytes as f32 * 8.0) / dt / 1_000_000.0;
        let network_tx_mbps = (tx_bytes as f32 * 8.0) / dt / 1_000_000.0;

        SystemResourceUsage {
            app_cpu_percent: total_cpu,
            app_memory_rss_mb: total_rss / (1024 * 1024),
            app_process_count: included.len(),
            network_rx_mbps,
            network_tx_mbps,
            db_connections_in_use: 0,
            db_connections_open: 0,
            db_connections_max: 0,
            db_connections_waiting: 0,
        }
    }
}

pub async fn with_pool_usage(
    mut usage: SystemResourceUsage,
    app_state: &ApplicationServices,
) -> SystemResourceUsage {
    let (query_open, query_available, query_max, query_waiting) = {
        let clients = app_state.clients.lock().await;
        let mut open = 0usize;
        let mut available = 0usize;
        let mut max = 0usize;
        let mut waiting = 0usize;
        for pool in clients.values() {
            let status = pool.status();
            open = open.saturating_add(status.size);
            available = available.saturating_add(status.available);
            max = max.saturating_add(status.max_size);
            waiting = waiting.saturating_add(status.waiting);
        }
        (open, available, max, waiting)
    };

    let (meta_open, meta_available, meta_max, meta_waiting) = {
        let clients = app_state.meta_clients.lock().await;
        let mut open = 0usize;
        let mut available = 0usize;
        let mut max = 0usize;
        let mut waiting = 0usize;
        for pool in clients.values() {
            let status = pool.status();
            open = open.saturating_add(status.size);
            available = available.saturating_add(status.available);
            max = max.saturating_add(status.max_size);
            waiting = waiting.saturating_add(status.waiting);
        }
        (open, available, max, waiting)
    };

    let total_open = query_open.saturating_add(meta_open);
    let total_available = query_available.saturating_add(meta_available);

    usage.db_connections_open = total_open;
    usage.db_connections_max = query_max.saturating_add(meta_max);
    usage.db_connections_waiting = query_waiting.saturating_add(meta_waiting);
    usage.db_connections_in_use = total_open.saturating_sub(total_available);

    usage
}
