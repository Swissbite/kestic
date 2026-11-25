// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs a short helper program (`ssh -G`, `ssh-keygen`, `ssh-keyscan`, `gpgv`, `restic version`)
//! with a time limit and at most [`OUTPUT_LIMIT`] bytes of each output.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// The most bytes kept of stdout and of stderr.
pub(crate) const OUTPUT_LIMIT: u64 = 1 << 20;

/// How the program runs.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Bounds<'a> {
    /// Text written to the program's stdin; without it stdin is empty.
    pub stdin: Option<&'a [u8]>,
    /// The program gets a process group of its own, and the whole group ends at the time limit:
    /// commands it started (ssh's `Match exec`) do not outlive it.
    pub own_group: bool,
}

/// Runs `cmd` with stdout and stderr captured and kills it after `limit`.
pub(crate) fn output_within(cmd: Command, limit: Duration) -> std::io::Result<Output> {
    output_bounded(cmd, limit, Bounds::default())
}

/// [`output_within`] with stdin and process group as `bounds` says.
pub(crate) fn output_bounded(
    mut cmd: Command,
    limit: Duration,
    bounds: Bounds<'_>,
) -> std::io::Result<Output> {
    if bounds.own_group {
        cmd.process_group(0);
    }
    let mut child = cmd
        .stdin(if bounds.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(text), Some(mut pipe)) = (bounds.stdin, child.stdin.take()) {
        pipe.write_all(text)?;
    }
    // Each pipe is read on its own thread. A process the child left in the background may keep
    // a pipe open; the reader then never finishes, so the result is awaited only until the
    // deadline.
    let stdout = read_on_thread(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = read_on_thread(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + limit;
    let timed_out = |what: &str| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("{what} within {} s", limit.as_secs_f32()),
        )
    };
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            stop(&mut child, bounds.own_group);
            return Err(timed_out("no answer"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // The child is reaped: its PID may belong to another process by now. Its process group
    // number stays reserved while a member lives, so only the group may still be signalled.
    let collected = |receiver: Receiver<Vec<u8>>| {
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| {
                if bounds.own_group {
                    kill_group(child.id());
                }
                timed_out("output not closed")
            })
    };
    Ok(Output {
        status,
        stdout: collected(stdout)?,
        stderr: collected(stderr)?,
    })
}

fn read_on_thread(pipe: Option<Box<dyn Read + Send>>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = pipe {
            // Whatever arrived before an error is all there is.
            let _ = pipe.take(OUTPUT_LIMIT).read_to_end(&mut buf);
        }
        // The receiver is gone once the deadline has passed.
        let _ = sender.send(buf);
    });
    receiver
}

/// Ends the child that has not been reaped yet, with its group if it has one of its own.
fn stop(child: &mut Child, own_group: bool) {
    if own_group {
        kill_group(child.id());
    }
    // Already gone or never reachable: `wait` below reaps it either way.
    let _ = child.kill();
    let _ = child.wait();
}

fn kill_group(pgid: u32) {
    if let Some(pgid) = i32::try_from(pgid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        // A group without members is fine.
        let _ = rustix::process::kill_process_group(pgid, rustix::process::Signal::KILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A helper that leaves a process holding its output pipes still ends at the time limit.
    #[test]
    fn output_within_ends_while_a_background_process_holds_the_pipes() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 4 & echo done"]);
        let started = Instant::now();
        let result = output_within(cmd, Duration::from_millis(300));
        assert!(
            matches!(&result, Err(err) if err.kind() == std::io::ErrorKind::TimedOut),
            "{result:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    /// In a group of its own, the background process that holds the pipes ends with the group.
    #[test]
    fn own_group_ends_the_background_process_too() {
        let dir = tempfile::tempdir().expect("dir");
        let marker = dir.path().join("still-running");
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!(
            "(sleep 1; touch '{}') & echo started",
            marker.display()
        ));
        let result = output_bounded(
            cmd,
            Duration::from_millis(300),
            Bounds {
                stdin: None,
                own_group: true,
            },
        );
        assert!(
            matches!(&result, Err(err) if err.kind() == std::io::ErrorKind::TimedOut),
            "{result:?}"
        );
        std::thread::sleep(Duration::from_millis(1500));
        assert!(
            !marker.exists(),
            "the background process outlived its group"
        );
    }

    /// Stdin reaches the program; output beyond the limit is cut.
    #[test]
    fn stdin_is_passed_and_output_is_capped() {
        let out = output_bounded(
            Command::new("cat"),
            Duration::from_secs(5),
            Bounds {
                stdin: Some(b"host key\n"),
                own_group: false,
            },
        )
        .expect("cat");
        assert_eq!(out.stdout, b"host key\n");
        let mut cmd = Command::new("head");
        cmd.args(["-c", "2000000", "/dev/zero"]);
        let out = output_within(cmd, Duration::from_secs(5)).expect("head");
        assert_eq!(u64::try_from(out.stdout.len()).ok(), Some(OUTPUT_LIMIT));
    }
}
