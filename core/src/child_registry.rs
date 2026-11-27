// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every child process Kestic starts (restic and the short helper programs), so the daemon can
//! stop them all when it ends: after [`ChildRegistry::close`] no child starts any more and each
//! running one gets `SIGTERM` once; [`ChildRegistry::kill_remaining`] ends the rest together with
//! every process they started.

use crate::sync::LockUnpoisoned;
use rustix::process::{Pid, Signal, getpid, kill_process};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The error of a start refused after [`ChildRegistry::close`].
pub const STOPPING_MESSAGE: &str = "Kestic is stopping; no new process starts";

/// The child processes of this process that have not been reaped yet.
#[derive(Debug, Default)]
pub struct ChildRegistry {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    closed: bool,
    /// Set by the `SIGTERM` handler: from that moment on nothing starts, even before the daemon
    /// calls [`ChildRegistry::close`].
    stop_flag: Option<Arc<AtomicBool>>,
    /// The processes below the live children when [`ChildRegistry::close`] ran, with their start
    /// time: a child that ends leaves them to another parent, but they may still hold its output.
    below: Vec<(u32, u64)>,
    /// Each live child and whether it got `SIGTERM` from Kestic already.
    live: HashMap<u32, bool>,
}

/// Keeps a child in the registry until it is dropped; drop it once the child is reaped.
#[derive(Debug)]
pub(crate) struct Registration<'a> {
    registry: &'a ChildRegistry,
    pid: u32,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.registry.inner.lock_unpoisoned().live.remove(&self.pid);
    }
}

impl ChildRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry of this process.
    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<ChildRegistry> = OnceLock::new();
        GLOBAL.get_or_init(Self::new)
    }

    /// Starts `cmd` and registers the child, or refuses once the registry is closed. The start
    /// happens under the registry's lock, so no child can start after [`Self::close`] has
    /// signalled the others.
    pub(crate) fn spawn(&self, cmd: &mut Command) -> io::Result<(Child, Registration<'_>)> {
        let mut inner = self.inner.lock_unpoisoned();
        if inner.closed
            || inner
                .stop_flag
                .as_ref()
                .is_some_and(|f| f.load(Ordering::SeqCst))
        {
            return Err(io::Error::other(STOPPING_MESSAGE));
        }
        let child = cmd.spawn()?;
        let pid = child.id();
        inner.live.insert(pid, false);
        Ok((
            child,
            Registration {
                registry: self,
                pid,
            },
        ))
    }

    /// Sends `SIGTERM` to the live child `pid` and its direct children, at most once per child;
    /// returns whether it sent one. A child that is no longer registered has been reaped, and its
    /// number may belong to another process by now, so it gets nothing.
    pub(crate) fn terminate(&self, pid: u32) -> bool {
        {
            let mut inner = self.inner.lock_unpoisoned();
            match inner.live.get_mut(&pid) {
                Some(sent) if !*sent => *sent = true,
                _ => return false,
            }
        }
        crate::restic::send_sigterm_to_tree(pid);
        true
    }

    /// Refuses every start once `flag` is set (the flag of the `SIGTERM` handler).
    pub fn stop_on(&self, flag: Arc<AtomicBool>) {
        self.inner.lock_unpoisoned().stop_flag = Some(flag);
    }

    /// Refuses every later start and sends `SIGTERM` to each live child that has not had one.
    pub fn close(&self) {
        let table = process_table();
        let pids: Vec<u32> = {
            let mut inner = self.inner.lock_unpoisoned();
            inner.closed = true;
            let below: Vec<(u32, u64)> = inner
                .live
                .keys()
                .flat_map(|root| descendants(*root, &table))
                .filter_map(|pid| start_time(pid).map(|start| (pid, start)))
                .collect();
            inner.below = below;
            inner
                .live
                .iter_mut()
                .filter(|(_, sent)| !**sent)
                .map(|(pid, sent)| {
                    *sent = true;
                    *pid
                })
                .collect()
        };
        for pid in pids {
            crate::restic::send_sigterm_to_tree(pid);
        }
    }

    /// Whether [`Self::close`] has run.
    pub fn is_closed(&self) -> bool {
        self.inner.lock_unpoisoned().closed
    }

    /// The number of children that have not been reaped yet.
    pub fn live_count(&self) -> usize {
        self.inner.lock_unpoisoned().live.len()
    }

    /// Sends `SIGKILL` to every live child that is still a child of this process, to all
    /// processes below it, and to the processes that were below the children at [`Self::close`]
    /// and still run (same start time, so not a reused number); returns how many got it. The
    /// tree is read before anything is killed, because the descendants of a killed process move
    /// to another parent.
    pub fn kill_remaining(&self) -> usize {
        let (roots, below): (Vec<u32>, Vec<(u32, u64)>) = {
            let inner = self.inner.lock_unpoisoned();
            (inner.live.keys().copied().collect(), inner.below.clone())
        };
        let table = process_table();
        let own = u32::try_from(getpid().as_raw_nonzero().get()).unwrap_or(0);
        let mut targets: Vec<u32> = below
            .into_iter()
            .filter(|(pid, start)| start_time(*pid) == Some(*start))
            .map(|(pid, _)| pid)
            .collect();
        for root in roots {
            if table.get(&root) == Some(&own) {
                targets.extend(descendants(root, &table));
                targets.push(root);
            }
        }
        targets.sort_unstable();
        targets.dedup();
        targets
            .into_iter()
            .filter(|pid| send(*pid, Signal::KILL))
            .count()
    }
}

/// Every process below `root` in `table` (pid → parent pid), deepest last.
pub(crate) fn descendants(root: u32, table: &HashMap<u32, u32>) -> Vec<u32> {
    let mut found = Vec::new();
    let mut seen = HashSet::from([root]);
    let mut queue = vec![root];
    while let Some(parent) = queue.pop() {
        for (pid, ppid) in table {
            if *ppid == parent && seen.insert(*pid) {
                found.push(*pid);
                queue.push(*pid);
            }
        }
    }
    found
}

/// The parent of every process in `/proc` (pid → parent pid).
pub(crate) fn process_table() -> HashMap<u32, u32> {
    let mut table = HashMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return table;
    };
    for entry in entries.map_while(Result::ok) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(ppid) = fs::read_to_string(entry.path().join("stat"))
            .ok()
            .and_then(|stat| parent_of_stat(&stat))
        {
            table.insert(pid, ppid);
        }
    }
    table
}

/// The parent pid in a `/proc/<pid>/stat` line. The program name in parentheses may hold spaces
/// and parentheses itself, so the fields count from the last `)`.
pub(crate) fn parent_of_stat(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// When process `pid` started (clock ticks since boot), from `/proc/<pid>/stat`.
fn start_time(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn send(pid: u32, signal: Signal) -> bool {
    i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .is_some_and(|pid| kill_process(pid, signal).is_ok())
}

#[cfg(test)]
#[path = "child_registry_tests.rs"]
mod tests;
