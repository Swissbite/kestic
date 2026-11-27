// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! Prints the version a build of the current directory gets, the same one the daemon's build
//! script records; `just deb` passes it to `cargo deb`. With `--tag <tag>` it prints the version
//! of that tag instead, which the tests of the release workflow compare with their own mapping.
//! `metainfo <template> <output>` writes the AppStream metainfo with the release entry of that
//! version, for `just deb`.

use kestic_build_stamp::metainfo::{SOURCE_DATE_VAR, release_date, with_release};
use kestic_build_stamp::version::{OVERRIDE_VAR, build_version, from_tag};
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: kestic-build-stamp [--tag <tag> | metainfo <template> <output>]";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.as_slice() {
        [] => current_version().map(|version| println!("{version}")),
        [flag, tag] if flag == "--tag" => from_tag(tag)
            .map(|version| println!("{version}"))
            .ok_or_else(|| format!("'{tag}' is no version tag")),
        [command, template, output] if command == "metainfo" => {
            write_metainfo(Path::new(template), Path::new(output))
        }
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("kestic-build-stamp: {message}");
            ExitCode::FAILURE
        }
    }
}

fn current_version() -> Result<String, String> {
    let override_value = env::var(OVERRIDE_VAR).ok();
    let workspace_root = env::current_dir().expect("current directory");
    build_version(&workspace_root, override_value.as_deref())
}

fn write_metainfo(template: &Path, output: &Path) -> Result<(), String> {
    let metainfo = fs::read_to_string(template)
        .map_err(|e| format!("cannot read {}: {e}", template.display()))?;
    let workspace_root = env::current_dir().expect("current directory");
    let date = release_date(&workspace_root, env::var(SOURCE_DATE_VAR).ok().as_deref())?;
    let with_release = with_release(&metainfo, &current_version()?, &date)?;
    fs::write(output, with_release).map_err(|e| format!("cannot write {}: {e}", output.display()))
}
