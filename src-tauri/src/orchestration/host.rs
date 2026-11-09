//! Host metrics for admission control (docs/ROUTING.md): free memory, the
//! load average and the resident memory of every live Claude Code process
//! the roster knows, sampled with `sysinfo` every few seconds by the
//! scheduler's loop ([`super::scheduler::start`]).
//!
//! Only what is read is refreshed: the RAM figures (not swap) and, for the
//! pids the roster names, their memory — never a scan of every process on
//! the machine. A sample is a few system calls; it runs off the async
//! runtime all the same.
//!
//! The memory headroom of plan §9: `floor((free − reserve) / estimate)` more
//! role sessions fit, where the estimate is what live sessions measure (the
//! 75th percentile of their resident memory) and 600 MB until one was
//! measured. "Free" is what the system can hand a new process without
//! swapping (`sysinfo`'s available memory: free plus reclaimable caches).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

use serde::Serialize;

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
/// Memory kept free for everything else on the machine (plan §9: 2 GB).
pub const DEFAULT_RESERVE: u64 = 2 * GIB;
/// What a role session is assumed to need until sessions were measured
/// (plan §9: 600 MB; three VS Code sessions measured 190–450 MB here).
pub const DEFAULT_SESSION: u64 = 600 * MIB;
/// Bounds of a measured estimate: a session that just started is tiny, and
/// one runaway process must not stop everything.
const MIN_ESTIMATE: u64 = 100 * MIB;
const MAX_ESTIMATE: u64 = 8 * GIB;
/// The most role sessions memory ever admits at once (plan §9).
pub const HARDWARE_CAP_MAX: usize = 16;

/// One live Claude Code process and its resident memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMemory {
    pub session_id: String,
    pub pid: u32,
    pub rss_bytes: u64,
}

/// One sample of the machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSample {
    /// Epoch ms; 0 until the first sample.
    pub sampled_at: i64,
    pub total_memory: u64,
    /// What a new process can get without swapping (shown as "free").
    pub available_memory: u64,
    pub used_memory: u64,
    /// Logical CPUs.
    pub cpus: usize,
    /// 1-, 5- and 15-minute load averages; `None` where the system keeps
    /// none (Windows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<[f64; 3]>,
    /// Live sessions with a process we could measure, largest first.
    pub sessions: Vec<SessionMemory>,
}

impl HostSample {
    /// Whether this is a real sample (memory figures known).
    pub fn sampled(&self) -> bool {
        self.sampled_at > 0 && self.total_memory > 0
    }
}

/// What a role session is expected to need: the 75th percentile (nearest
/// rank) of the measured sessions, clamped to 100 MB – 8 GB, and whether it
/// was measured at all ([`DEFAULT_SESSION`] when not).
pub fn session_estimate(sessions: &[SessionMemory]) -> (u64, bool) {
    let mut sizes: Vec<u64> = sessions
        .iter()
        .map(|s| s.rss_bytes)
        .filter(|b| *b > 0)
        .collect();
    if sizes.is_empty() {
        return (DEFAULT_SESSION, false);
    }
    sizes.sort_unstable();
    // Nearest rank: ceil(0.75 · n), 1-based.
    let rank = (sizes.len() * 3).div_ceil(4).max(1);
    let p75 = sizes[rank - 1];
    (p75.clamp(MIN_ESTIMATE, MAX_ESTIMATE), true)
}

/// How many more role sessions fit: `floor((available − reserve) /
/// estimate)`, 0 when the reserve is not even there.
pub fn headroom(available: u64, reserve: u64, estimate: u64) -> usize {
    let spare = available.saturating_sub(reserve);
    usize::try_from(spare / estimate.max(1)).unwrap_or(usize::MAX)
}

/// The hardware cap on role sessions running at once: those running plus
/// the headroom, at least 1 (a lone session always starts) and at most
/// [`HARDWARE_CAP_MAX`].
pub fn hardware_cap(running: usize, headroom: usize) -> usize {
    running.saturating_add(headroom).clamp(1, HARDWARE_CAP_MAX)
}

/// The sampler and its last sample. Lives in [`super::scheduler::Scheduler`].
#[derive(Default)]
pub struct HostMonitor {
    /// Created on the first sample (cheap: nothing is read until asked).
    sys: Mutex<Option<sysinfo::System>>,
    /// Whether creating it failed (said once in the log).
    unavailable: AtomicBool,
    last: RwLock<HostSample>,
}

impl std::fmt::Debug for HostMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostMonitor")
            .field("last", &self.last())
            .finish_non_exhaustive()
    }
}

impl HostMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// The last sample (a default, unsampled one before the first).
    pub fn last(&self) -> HostSample {
        self.last.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Replace the last sample (the loop, and tests with synthetic signals).
    pub fn set(&self, sample: HostSample) {
        *self.last.write().unwrap_or_else(|p| p.into_inner()) = sample;
    }

    /// Sample the machine now: RAM, load and the resident memory of
    /// `procs` (`(session id, pid)`). Blocking but cheap; the loop runs it
    /// on a blocking thread.
    pub fn sample(&self, procs: &[(String, u32)], now: i64) -> HostSample {
        use sysinfo::{MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

        let mut guard = self.sys.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            match System::new() {
                Ok(sys) => *guard = Some(sys),
                // Tried again at the next sample; said once.
                Err(e) if !self.unavailable.swap(true, Ordering::Relaxed) => {
                    tracing::warn!(error = %e, "host memory cannot be read");
                }
                Err(_) => {}
            }
        }
        // Without a reader the sample carries no memory figures, so it is
        // not `sampled()` and admission goes without them.
        let mut sample = HostSample {
            sampled_at: now,
            cpus: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            load: load_average(),
            ..HostSample::default()
        };
        if let Some(sys) = guard.as_mut() {
            sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
            let pids: Vec<Pid> = procs.iter().map(|(_, pid)| Pid::from_u32(*pid)).collect();
            if !pids.is_empty() {
                sys.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&pids),
                    true,
                    ProcessRefreshKind::nothing().with_memory(),
                );
            }
            let mut sessions: Vec<SessionMemory> = procs
                .iter()
                .filter_map(|(id, pid)| {
                    let rss = sys.process(Pid::from_u32(*pid))?.memory();
                    (rss > 0).then(|| SessionMemory {
                        session_id: id.clone(),
                        pid: *pid,
                        rss_bytes: rss,
                    })
                })
                .collect();
            sessions.sort_by_key(|s| std::cmp::Reverse(s.rss_bytes));
            sample.total_memory = sys.total_memory();
            sample.available_memory = sys.available_memory();
            sample.used_memory = sys.used_memory();
            sample.sessions = sessions;
        }
        drop(guard);
        self.set(sample.clone());
        sample
    }
}

#[cfg(not(windows))]
fn load_average() -> Option<[f64; 3]> {
    let l = sysinfo::System::load_average().ok()?;
    let load = [l.one, l.five, l.fifteen];
    load.iter()
        .all(|v| v.is_finite() && *v >= 0.0)
        .then_some(load)
}

/// Windows keeps no load average (`sysinfo` answers zeros).
#[cfg(windows)]
fn load_average() -> Option<[f64; 3]> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: &str, mb: u64) -> SessionMemory {
        SessionMemory {
            session_id: id.into(),
            pid: 1,
            rss_bytes: mb * MIB,
        }
    }

    #[test]
    fn the_estimate_is_measured_once_sessions_run() {
        assert_eq!(session_estimate(&[]), (DEFAULT_SESSION, false));
        // The three sessions measured for the plan: p75 of 187/390/445 MB.
        let (est, measured) = session_estimate(&[mem("a", 187), mem("b", 390), mem("c", 445)]);
        assert!(measured);
        assert_eq!(est, 445 * MIB);
        // Four: rank ceil(3) = 3rd smallest.
        let (est, _) =
            session_estimate(&[mem("a", 200), mem("b", 300), mem("c", 400), mem("d", 900)]);
        assert_eq!(est, 400 * MIB);
        // Clamped both ways; a zero reading does not count.
        assert_eq!(session_estimate(&[mem("a", 10)]).0, MIN_ESTIMATE);
        assert_eq!(session_estimate(&[mem("a", 20_000)]).0, MAX_ESTIMATE);
        assert_eq!(session_estimate(&[mem("a", 0)]), (DEFAULT_SESSION, false));
    }

    #[test]
    fn headroom_and_the_hardware_cap_follow_plan_9() {
        // 5 GB available, 2 GB reserved, 600 MB each: 5 more.
        assert_eq!(headroom(5 * GIB, DEFAULT_RESERVE, DEFAULT_SESSION), 5);
        // Below the reserve: none.
        assert_eq!(headroom(GIB, DEFAULT_RESERVE, DEFAULT_SESSION), 0);
        assert_eq!(headroom(10 * GIB, 0, 0), usize::try_from(10 * GIB).unwrap());
        // A lone session always starts; never more than 16 at once.
        assert_eq!(hardware_cap(0, 0), 1);
        assert_eq!(hardware_cap(3, 0), 3);
        assert_eq!(hardware_cap(3, 2), 5);
        assert_eq!(hardware_cap(10, 40), HARDWARE_CAP_MAX);
    }

    #[test]
    fn a_real_sample_reads_this_machine() {
        let monitor = HostMonitor::new();
        assert!(!monitor.last().sampled());
        let me = std::process::id();
        let s = monitor.sample(&[("self".into(), me), ("gone".into(), u32::MAX - 7)], 42);
        assert!(s.sampled());
        assert!(s.total_memory > 0 && s.available_memory <= s.total_memory);
        assert!(s.cpus >= 1);
        // This test process is measured; a pid that does not exist is not.
        assert_eq!(s.sessions.len(), 1);
        assert_eq!(s.sessions[0].pid, me);
        assert_eq!(monitor.last(), s);
        #[cfg(not(windows))]
        assert!(s.load.is_some());
    }
}
