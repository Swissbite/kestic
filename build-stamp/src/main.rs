// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! Prints the version a build of the current directory gets, the same one the daemon's build
//! script records. `just deb` passes it to `cargo deb`.

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let override_value = env::var(kestic_build_stamp::version::OVERRIDE_VAR).ok();
    let workspace_root = env::current_dir().expect("current directory");
    match kestic_build_stamp::version::build_version(&workspace_root, override_value.as_deref()) {
        Ok(version) => {
            println!("{version}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("kestic-build-stamp: {message}");
            ExitCode::FAILURE
        }
    }
}
