// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! M3-S12 Plasma progress display via `org.kde.JobViewServerV2` (`AC-M3-12-1`, `AC-M3-12-2`).

mod common;

use common::fakes::FakeJobViewHandle;
use common::m3::{
    LocalBackupFixture, M3Rig, ResticVariant, StdFakes, backup_summary, latest_variant,
    restic_variants, wait_for_state, wait_until,
};
use common::{CapturedKesticSignal, KesticSignalCollector, run_isolated_test};
use kestic_core::model::{Run, RunStatus};
use kestic_daemon::dbus_api::{KESTIC_INTERFACE_V1, KESTIC_OBJECT_PATH, RunProgressPhase};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;
use time::macros::datetime;

const JOB_VIEW_DEST: &str = "org.kde.JobViewServer";

fn start_rig(root: &Path, variant: &ResticVariant, base: &str) -> (M3Rig, LocalBackupFixture) {
    let rig = M3Rig::start(
        root,
        variant,
        base,
        datetime!(2026-12-01 10:59:30 UTC),
        |opts| opts.enable_job_view = true,
    );
    let fixture = rig.add_backup("Docs", |_| {});
    (rig, fixture)
}

/// AC-M3-12-1 (FR-53): a starting backup run is announced to the `JobViewServerV2` fake -> (a)
/// `requestView` with Kestic's desktop id, killable capability and `immediate` hint, (b) progress
/// in percent, (c) completion with status, (d) all calls of one view come from one D-Bus sender.
#[test]
fn ac_m3_12_1_run_is_announced_with_progress_and_completion_status() {
    run_isolated_test(
        "ac_m3_12_1_run_is_announced_with_progress_and_completion_status",
        "KESTIC_M3_12_1_INNER",
        |root| {
            let job_view = FakeJobViewHandle::start(JOB_VIEW_DEST);
            let _fakes = StdFakes::start();
            let rec = &job_view.recorder;
            for variant in restic_variants() {
                let requests_before = rec.request_count.load(Ordering::SeqCst);
                let terminations_before = rec.terminations.lock().expect("lock").len();
                rec.updates.lock().expect("lock").clear();
                let (rig, fx) = start_rig(root, &variant, "M3J1");
                let client = &rig.daemon.client;
                let bid = &fx.backup.id;
                rig.wrapper.hold("backup");

                client.start_backup(&fx.backup_id_str).expect("start run");
                // (a) requestView with the desktop id of Kestic.
                wait_until("requestView", || {
                    rec.request_count.load(Ordering::SeqCst) > requests_before
                });
                assert_eq!(
                    rec.request_count.load(Ordering::SeqCst) - requests_before,
                    1,
                    "{}: (a) one view per run",
                    variant.label
                );
                assert_eq!(
                    *rec.desktop_entry.lock().expect("lock"),
                    "net.daester.Kestic",
                    "{}: (a)",
                    variant.label
                );
                assert_eq!(
                    rec.capabilities.load(Ordering::SeqCst),
                    1,
                    "{}: (a) the view is announced as killable",
                    variant.label
                );
                assert!(
                    rec.immediate.load(Ordering::SeqCst),
                    "{}: (a) the view is requested with the `immediate` hint",
                    variant.label
                );
                // (b) progress in percent (the held run reports 25 %).
                wait_until("progress update", || {
                    rec.updates
                        .lock()
                        .expect("lock")
                        .iter()
                        .any(|(pct, _)| *pct == 25)
                });
                assert!(
                    rec.updates
                        .lock()
                        .expect("lock")
                        .iter()
                        .any(|(pct, msg)| *pct == 25 && msg.contains("25%")),
                    "{}: (b) percent and info message: {:?}",
                    variant.label,
                    rec.updates.lock().expect("lock")
                );
                assert!(
                    rec.sizes
                        .lock()
                        .expect("lock")
                        .iter()
                        .any(|(processed, total, _)| *processed == 1_048_576 && *total == 4_194_304),
                    "{}: (b) processed and total bytes are reported: {:?}",
                    variant.label,
                    rec.sizes.lock().expect("lock")
                );

                // (c) completion: ok -> code 0.
                rig.wrapper.release("backup");
                wait_for_state(client, Duration::from_secs(30), "run Ok", |s| {
                    backup_summary(s, bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Ok)
                });
                wait_until("terminate", || {
                    rec.terminations.lock().expect("lock").len() > terminations_before
                });
                {
                    let terms = rec.terminations.lock().expect("lock");
                    assert_eq!(
                        terms[terminations_before].0, 0,
                        "{}: (c) ok run completes with code 0",
                        variant.label
                    );
                }

                {
                    let request = rec.request_sender.lock().expect("lock").clone();
                    assert!(
                        !request.is_empty(),
                        "{}: (d) requestView has a sender",
                        variant.label
                    );
                    assert_eq!(
                        *rec.update_sender.lock().expect("lock"),
                        request,
                        "{}: (d) update comes from the connection that requested the view",
                        variant.label
                    );
                    assert_eq!(
                        *rec.terminate_sender.lock().expect("lock"),
                        request,
                        "{}: (d) terminate comes from the connection that requested the view",
                        variant.label
                    );
                }

                // (c) completion with a warning carries a non-zero status and a message.
                let bad = fx.src_dir.join("unreadable.secret");
                fs::write(&bad, b"x").expect("write");
                fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).expect("chmod");
                client.start_backup(&fx.backup_id_str).expect("second run");
                wait_for_state(client, Duration::from_secs(30), "run Warning", |s| {
                    backup_summary(s, bid)
                        .last_run
                        .as_ref()
                        .is_some_and(|r| r.status == RunStatus::Warning)
                });
                wait_until("second terminate", || {
                    rec.terminations.lock().expect("lock").len() > terminations_before + 1
                });
                let _ = fs::set_permissions(&bad, fs::Permissions::from_mode(0o644));
                {
                    let terms = rec.terminations.lock().expect("lock");
                    assert_ne!(
                        terms[terminations_before + 1].0,
                        0,
                        "{}: (c) warning completes with a non-zero status",
                        variant.label
                    );
                    assert!(
                        !terms[terminations_before + 1].1.is_empty(),
                        "{}: (c) completion carries the status message",
                        variant.label
                    );
                }
                rig.daemon.stop();
            }
        },
    );
}

/// AC-M3-12-2 (FR-53, FR-21): cancelling in the Plasma progress display ends the run `abgebrochen`.
#[test]
fn ac_m3_12_2_cancel_in_plasma_progress_cancels_the_run() {
    run_isolated_test(
        "ac_m3_12_2_cancel_in_plasma_progress_cancels_the_run",
        "KESTIC_M3_12_2_INNER",
        |root| {
            let job_view = FakeJobViewHandle::start(JOB_VIEW_DEST);
            let _fakes = StdFakes::start();
            let variant = latest_variant();
            let (rig, fx) = start_rig(root, &variant, "M3J2");
            let client = &rig.daemon.client;
            rig.wrapper.hold("backup");
            let signals = KesticSignalCollector::start(
                &variant.bus_name("M3J2"),
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            let run_id = client.start_backup(&fx.backup_id_str).expect("start run");
            wait_until("progress shown in Plasma", || {
                !job_view.recorder.updates.lock().expect("lock").is_empty()
            });

            job_view.emit_cancel_requested();
            signals.wait_until(Duration::from_secs(15), |sigs| {
                sigs.iter().any(|s| {
                    if let CapturedKesticSignal::RunUpdated(json) = s
                        && let Ok(run) = serde_json::from_str::<Run>(json)
                    {
                        return run.id.as_str() == run_id && run.status == RunStatus::Cancelled;
                    }
                    false
                })
            });
            let state = client.get_state().expect("GetState");
            assert_eq!(
                backup_summary(&state, &fx.backup.id)
                    .last_run
                    .as_ref()
                    .map(|r| r.status),
                Some(RunStatus::Cancelled),
                "(a) the run ends `abgebrochen`"
            );
            assert_eq!(
                rig.wrapper.of('T', "backup").len(),
                1,
                "(a) restic was terminated"
            );
            drop(signals);
            rig.wrapper.release("backup");
            rig.daemon.stop();
        },
    );
}

/// While restic still finds files (no `seconds_remaining`), the run shows no percentage
/// (`docs/spec/ui/tray-und-benachrichtigungen.md` §10.3, restic-integration «Fortschritt beim
/// Sichern»): (a) the Plasma progress view gets percent 0, (b) no amounts and (c) no speed, (d) its
/// text says that files are looked through, (e) the `RunProgress` signal carries phase `scanning`
/// with the amount looked through.
#[test]
fn progress_view_and_signal_show_no_percentage_while_restic_still_finds_files() {
    run_isolated_test(
        "progress_view_and_signal_show_no_percentage_while_restic_still_finds_files",
        "KESTIC_M3_12_SCANNING_INNER",
        |root| {
            let job_view = FakeJobViewHandle::start(JOB_VIEW_DEST);
            let _fakes = StdFakes::start();
            let rec = &job_view.recorder;
            let variant = latest_variant();
            let (rig, fx) = start_rig(root, &variant, "M3JS");
            let client = &rig.daemon.client;
            let signals = KesticSignalCollector::start(
                &variant.bus_name("M3JS"),
                KESTIC_OBJECT_PATH,
                KESTIC_INTERFACE_V1,
            );
            rig.wrapper.hold_with_output(
                "backup",
                concat!(
                    r#"{"message_type":"status","percent_done":0.99,"total_files":4000,"#,
                    r#""files_done":3500,"total_bytes":5000000000,"bytes_done":4950000000}"#,
                    "\n",
                ),
            );

            let run_id = client.start_backup(&fx.backup_id_str).expect("start run");
            wait_until("progress update while scanning", || {
                rec.updates
                    .lock()
                    .expect("lock")
                    .iter()
                    .any(|(_, msg)| msg.contains("looking through files"))
            });
            let (index, pct) = {
                let updates = rec.updates.lock().expect("lock");
                let index = updates
                    .iter()
                    .position(|(_, msg)| msg.contains("looking through files"))
                    .expect("scanning update");
                (index, updates[index].0)
            };
            let (processed, total, speed) = rec.sizes.lock().expect("lock")[index];
            assert_eq!(pct, 0, "(a) percent while scanning");
            assert_eq!(processed, 0, "(b) processedBytes while scanning");
            assert_eq!(total, 0, "(b) totalBytes while scanning");
            assert_eq!(speed, 0, "(c) speed while scanning");
            assert_eq!(
                rec.updates.lock().expect("lock")[index].1,
                "Docs: backing up · looking through files",
                "(d) progress view text"
            );

            signals.wait_until(Duration::from_secs(10), |sigs| {
                sigs.iter().any(
                    |s| matches!(s, CapturedKesticSignal::RunProgress(p) if p.run_id == run_id),
                )
            });
            let progress = signals
                .snapshot()
                .into_iter()
                .find_map(|s| match s {
                    CapturedKesticSignal::RunProgress(p) if p.run_id == run_id => Some(p),
                    _ => None,
                })
                .expect("RunProgress signal");
            assert_eq!(
                progress.phase,
                RunProgressPhase::Scanning,
                "(e) signal phase"
            );
            assert_eq!(progress.percent_done, 0.0, "(e) signal percent");
            assert_eq!(
                progress.bytes_done, 4_950_000_000,
                "(e) amount looked through"
            );
            assert_eq!(progress.files_done, 3500, "(e) files looked through");

            drop(signals);
            rig.wrapper.release("backup");
            rig.daemon.stop();
        },
    );
}
