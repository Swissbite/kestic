// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of the D-Bus server's helpers.

use super::*;

/// Issue #180, item 1: a panicking blocking task answers with an internal error instead of
/// leaving the D-Bus call pending until the client's timeout.
#[test]
fn run_blocking_task_answers_a_panic_with_an_internal_error() {
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let res: Result<u32, KesticDbusError> =
            zbus::block_on(run_blocking_task(|| panic!("task broke")));
        done_tx.send(res).ok();
    });
    let res = done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("a panicking task must end the wait");
    match res {
        Err(KesticDbusError::Internal(msg)) => {
            assert!(msg.contains("task broke"), "the panic message is kept: {msg}");
        }
        other => panic!("expected an internal error, got {other:?}"),
    }
}

/// A task that returns normally still hands its value back.
#[test]
fn run_blocking_task_returns_the_value_of_the_task() {
    let res = zbus::block_on(run_blocking_task(|| 42_u32));
    assert_eq!(res.ok(), Some(42));
}
