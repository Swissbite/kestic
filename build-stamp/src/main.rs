// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! Prints the version a build of the current directory gets, the same one the daemon's build
//! script records; `just deb` passes it to `cargo deb`. With `--tag <tag>` it prints the version
//! of that tag instead, which the tests of the release workflow compare with their own mapping.

use kestic_build_stamp::version::{OVERRIDE_VAR, build_version, from_tag};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let version = match args.as_slice() {
        [] => {
            let override_value = env::var(OVERRIDE_VAR).ok();
            let workspace_root = env::current_dir().expect("current directory");
            build_version(&workspace_root, override_value.as_deref())
        }
        [flag, tag] if flag == "--tag" => {
            from_tag(tag).ok_or_else(|| format!("'{tag}' is no version tag"))
        }
        _ => Err("usage: kestic-build-stamp [--tag <tag>]".to_owned()),
    };
    match version {
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
