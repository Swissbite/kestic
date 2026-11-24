// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M4-S05: the speed of a running restore, from the growth of `bytes_restored`.

use kestic_core::transfer_speed::TransferSpeed;
use time::Duration;
use time::macros::datetime;

const MIB: u64 = 1024 * 1024;

/// AC-M4-05-30 (unit part): (a) the first status line gives no speed; (b) a second line 1 s
/// later with 4 MiB more gives 4 MiB/s; (c) a single slow line does not drop the speed to its
/// value, while (d) a lasting change shows within a few seconds; (e) a line at the same moment
/// changes nothing.
#[test]
fn ac_m4_05_30_restore_speed_follows_the_bytes_restored() {
    let start = datetime!(2026-11-24 20:00:00 UTC);
    let mut speed = TransferSpeed::new();
    assert_eq!(speed.update(start, 10 * MIB), None, "(a) first line");
    let at = |s: f64| start + Duration::seconds_f64(s);
    assert_eq!(
        speed.update(at(1.0), 14 * MIB),
        Some(4 * MIB),
        "(b) 4 MiB in 1 s"
    );
    let slow = speed.update(at(1.1), 14 * MIB).expect("speed");
    assert!(
        slow > 3 * MIB && slow < 4 * MIB,
        "(c) one line without progress: {slow}"
    );
    assert_eq!(
        speed.update(at(1.1), 20 * MIB),
        Some(slow),
        "(e) same moment"
    );
    let mut bytes = 14 * MIB;
    let mut now = 1.1;
    for _ in 0..100 {
        now += 0.1;
        bytes += MIB / 10;
        speed.update(at(now), bytes);
    }
    let settled = speed
        .update(at(now + 0.1), bytes + MIB / 10)
        .expect("speed");
    assert!(
        settled.abs_diff(MIB) < MIB / 10,
        "(d) after 10 s at 1 MiB/s: {settled}"
    );
}
