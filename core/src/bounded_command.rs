// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs a short helper program (`ssh -G`, `ssh-keygen`, `ssh-keyscan`, `gpgv`, `restic version`)
//! with a time limit and at most [`OUTPUT_LIMIT`] bytes of each output.

use crate::child_registry::ChildRegistry;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// The most bytes kept of stdout and of stderr.
pub(crate) const OUTPUT_LIMIT: u64 = 1 << 20;

/// Runs `cmd` with stdout and stderr captured and kills it after `limit`.
pub(crate) fn output_within(cmd: Command, limit: Duration) -> std::io::Result<Output> {
    output_with_stdin(cmd, limit, None)
}

/// [`output_within`] that writes `stdin` to the program's stdin (empty without it). The program
/// gets a process group of its own, and the whole group ends at the time limit: commands it
/// started (ssh's `Match exec`, a wrapper's background process) do not outlive it.
pub(crate) fn output_with_stdin(
    mut cmd: Command,
    limit: Duration,
    stdin: Option<&[u8]>,
) -> std::io::Result<Output> {
    cmd.process_group(0);
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    // Registered until it is reaped below, so the daemon can stop it when it ends.
    let (mut child, registration) = ChildRegistry::global().spawn(&mut cmd)?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let text = text.to_vec();
        // On its own thread, so a program that does not read stdin cannot hold up the deadline.
        // A program that ends before reading all of it says itself what went wrong.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&text);
        });
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
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                stop(&mut child);
                return Err(timed_out("no answer"));
            }
            Err(err) => {
                stop(&mut child);
                return Err(err);
            }
        }
    };
    drop(registration);
    // The child is reaped: its PID may belong to another process by now. Its process group
    // number stays reserved while a member lives, so only the group may still be signalled.
    let collected = |receiver: Receiver<Vec<u8>>| {
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| {
                kill_group(child.id());
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

/// Ends the child that has not been reaped yet, with its process group.
fn stop(child: &mut Child) {
    kill_group(child.id());
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

    /// The background process that holds the pipes ends with the program's process group.
    #[test]
    fn the_background_process_ends_with_the_group() {
        let dir = tempfile::tempdir().expect("dir");
        let marker = dir.path().join("still-running");
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!(
            "(sleep 1; touch '{}') & echo started",
            marker.display()
        ));
        let result = output_within(cmd, Duration::from_millis(300));
        assert!(
            matches!(&result, Err(err) if err.kind() == std::io::ErrorKind::TimedOut),
            "{result:?}"
        );
        // quiet window: the background process would touch the marker after 1 s
        std::thread::sleep(Duration::from_millis(1500));
        assert!(
            !marker.exists(),
            "the background process outlived its group"
        );
    }

    /// Stdin reaches the program; output beyond the limit is cut.
    #[test]
    fn stdin_is_passed_and_output_is_capped() {
        let out = output_with_stdin(
            Command::new("cat"),
            Duration::from_secs(5),
            Some(b"host key\n"),
        )
        .expect("cat");
        assert_eq!(out.stdout, b"host key\n");
        let mut cmd = Command::new("head");
        cmd.args(["-c", "2000000", "/dev/zero"]);
        let out = output_within(cmd, Duration::from_secs(5)).expect("head");
        assert_eq!(u64::try_from(out.stdout.len()).ok(), Some(OUTPUT_LIMIT));
    }

    /// A program that ends without reading its stdin gives its own result, not a write error.
    #[test]
    fn a_program_that_ignores_stdin_gives_its_own_result() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo refused >&2; exit 3"]);
        let input = vec![b'x'; 1 << 20];
        let out = output_with_stdin(cmd, Duration::from_secs(5), Some(&input)).expect("sh");
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.stderr, b"refused\n");
    }
}
