// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Randomness and hex helpers for generated test secrets and repository IDs.

use std::fs::File;
use std::io::Read;

/// Fresh random bytes from the kernel.
pub(crate) fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(len);
    File::open("/dev/urandom")
        .and_then(|f| f.take(len as u64).read_to_end(&mut buf))
        .expect("read /dev/urandom");
    assert_eq!(buf.len(), len, "short read from /dev/urandom");
    buf
}

/// Random index below `bound`.
pub(crate) fn random_index(bound: usize) -> usize {
    let raw: [u8; 8] = random_bytes(8).try_into().expect("8 random bytes");
    (u64::from_le_bytes(raw) % bound as u64) as usize
}

/// Lowercase hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
