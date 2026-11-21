// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The run history («Befehle» tab) splits a recorded low-priority command
//! (`ionice -c 3 nice -n 10 ionice -r <repo> --json 10 forget …`) back into the restic
//! invocation exactly (`FR-34`).

use kestic_core::model::{Destination, DestinationKind};
use kestic_core::restic::{RecordedInvocation, ResticCommandSpec, parse_recorded_invocation};
use std::path::PathBuf;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

const RESTIC: &str = "/usr/local/bin/restic";
const FORGET: [&str; 10] = [
    "forget",
    "--tag",
    "kestic:rxorlvyx",
    "--group-by",
    "tags",
    "--keep-daily",
    "7",
    "--keep-weekly",
    "4",
    "--keep-monthly",
];

fn forget_args() -> Vec<String> {
    let mut v = args(&FORGET);
    v.push("12".to_string());
    v
}

fn recorded(prefix: &[&str]) -> Vec<String> {
    let mut v = args(prefix);
    v.extend(args(&[RESTIC, "-r", "/home/u/repo", "--json"]));
    v.extend(forget_args());
    v
}

#[test]
fn m3_acceptance_recorded_invocation_variants_split_back_exactly() {
    let expected = |low_priority| RecordedInvocation {
        restic_binary: RESTIC.to_string(),
        low_priority,
        json_output: true,
        subcommand_args: forget_args(),
    };

    // (a) ionice + nice, as recorded on the reference system.
    assert_eq!(
        parse_recorded_invocation("ionice", &recorded(&["-c", "3", "nice", "-n", "10"])),
        expected(true)
    );
    // (b) nice only.
    assert_eq!(
        parse_recorded_invocation("nice", &recorded(&["-n", "10"])),
        expected(true)
    );
    // (c) ionice only.
    assert_eq!(
        parse_recorded_invocation("ionice", &recorded(&["-c", "3"])),
        expected(true)
    );
    // (d) restic directly, e.g. `unlock`: not low priority.
    let mut direct = args(&["-r", "/home/u/repo", "--json"]);
    direct.extend(forget_args());
    assert_eq!(parse_recorded_invocation(RESTIC, &direct), expected(false));
}

#[test]
fn m3_acceptance_recorded_forget_is_shown_as_the_command_that_ran() {
    let dest = Destination::new(
        "M3-Test",
        DestinationKind::Local {
            path: PathBuf::from("/home/u/repo"),
        },
    )
    .expect("dest");
    let inv = parse_recorded_invocation("ionice", &recorded(&["-c", "3", "nice", "-n", "10"]));
    let mut spec = ResticCommandSpec::for_destination(
        &inv.restic_binary,
        "/usr/bin/kestic",
        &dest,
        inv.subcommand_args,
    );
    spec.low_priority = inv.low_priority;
    spec.json_output = inv.json_output;

    let shown = spec.to_transparent_shell_command();

    assert!(
        shown.ends_with(&format!(
            "ionice -c 3 nice -n 10 {RESTIC} -r /home/u/repo --json forget --tag kestic:rxorlvyx \
             --group-by tags --keep-daily 7 --keep-weekly 4 --keep-monthly 12"
        )),
        "unexpected command: {shown}"
    );
}
