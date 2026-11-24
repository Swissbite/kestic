// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

const BLOCKED: Duration = Duration::from_millis(300);
const DONE: Duration = Duration::from_secs(5);

/// AC-M5-06-8 (b): the replace lease comes only once no restic process holds a read lease.
#[test]
fn ac_m5_06_8_replace_waits_for_readers() {
    let gate: &'static ResticBinaryGate = Box::leak(Box::new(ResticBinaryGate::new()));
    let reader = gate.read();
    assert_eq!(gate.readers(), 1);
    let (sender, receiver) = mpsc::channel();
    let replacer = thread::spawn(move || {
        let lease = gate.replace_when_idle(&|| false);
        sender.send(lease.is_some()).expect("send");
    });
    assert!(
        receiver.recv_timeout(BLOCKED).is_err(),
        "the replace lease must wait while a restic process runs"
    );
    drop(reader);
    assert_eq!(receiver.recv_timeout(DONE), Ok(true));
    replacer.join().expect("replacer");
    assert_eq!(gate.readers(), 0);
}

/// AC-M5-06-8 (b): while an installation waits, other restic processes still start.
#[test]
fn ac_m5_06_8_readers_start_while_an_install_waits() {
    let gate: &'static ResticBinaryGate = Box::leak(Box::new(ResticBinaryGate::new()));
    let first = gate.read();
    let (replaced, replaced_rx) = mpsc::channel();
    let replacer = thread::spawn(move || {
        let lease = gate.replace_when_idle(&|| false);
        replaced.send(lease.is_some()).expect("send");
    });
    assert!(replaced_rx.recv_timeout(BLOCKED).is_err());
    let (started, started_rx) = mpsc::channel();
    let second = thread::spawn(move || {
        let lease = gate.read();
        started.send(()).expect("send");
        lease
    });
    assert_eq!(started_rx.recv_timeout(DONE), Ok(()));
    let second = second.join().expect("second reader");
    assert_eq!(gate.readers(), 2);
    drop(first);
    assert!(replaced_rx.recv_timeout(BLOCKED).is_err());
    drop(second);
    assert_eq!(replaced_rx.recv_timeout(DONE), Ok(true));
    replacer.join().expect("replacer");
}

/// AC-M5-06-8 (c): a restic process that wants to start during the replacement waits for it.
#[test]
fn ac_m5_06_8_reader_waits_during_replacement() {
    let gate: &'static ResticBinaryGate = Box::leak(Box::new(ResticBinaryGate::new()));
    let lease = gate.replace_when_idle(&|| false).expect("idle gate");
    let (started, started_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let lease = gate.read();
        started.send(()).expect("send");
        drop(lease);
    });
    assert!(
        started_rx.recv_timeout(BLOCKED).is_err(),
        "a restic process must wait while the binary is replaced"
    );
    drop(lease);
    assert_eq!(started_rx.recv_timeout(DONE), Ok(()));
    reader.join().expect("reader");
}

/// A cancelled installation stops waiting and leaves the gate open.
#[test]
fn cancelled_install_stops_waiting() {
    let gate: &'static ResticBinaryGate = Box::leak(Box::new(ResticBinaryGate::new()));
    let reader = gate.read();
    let cancel: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
    let (sender, receiver) = mpsc::channel();
    let replacer = thread::spawn(move || {
        let lease = gate.replace_when_idle(&|| cancel.load(Ordering::SeqCst));
        sender.send(lease.is_some()).expect("send");
    });
    assert!(receiver.recv_timeout(BLOCKED).is_err());
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(receiver.recv_timeout(DONE), Ok(false));
    replacer.join().expect("replacer");
    let second = gate.read();
    assert_eq!(gate.readers(), 2);
    drop((reader, second));
    assert!(gate.replace_when_idle(&|| false).is_some());
}
