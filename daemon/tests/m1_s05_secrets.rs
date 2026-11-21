// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use common::{find_kestic_binary, run_isolated_test};
use kestic_core::secrets::{FileSecretStore, SecretError, SecretStore, SecretValue};
use kestic_daemon::secrets::SecretServiceStore;
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use zbus::{OwnedObjectPath, OwnedVariant, block_on, interface};

/// `kestic` command reading from the isolated test keyring over D-Bus.
fn kestic_cmd(args: &[&str]) -> Command {
    let mut cmd = Command::new(find_kestic_binary());
    cmd.args(args);
    cmd
}

/// AC-M1-05-2 (`kestic secret get` outputs the exact password with Exit 0 and fails with Exit 1
/// and empty stdout for an unknown id) and AC-M1-05-3 (real restic reads the repository password
/// through `RESTIC_PASSWORD_COMMAND`), against the headless keyring on the private session bus.
#[test]
fn ac_m1_05_1_05_2_05_3_05_4_kestic_secret_get_over_isolated_dbus_keyring_and_restic() {
    run_isolated_test(
        "ac_m1_05_1_05_2_05_3_05_4_kestic_secret_get_over_isolated_dbus_keyring_and_restic",
        "KESTIC_M1_S05_SECRET_GET_INNER",
        |root| {
            let restic_exe = common::restic_test_binaries()
                .pop()
                .expect("latest restic")
                .1;
            let repo_dir = root.join("repo");
            let bin = find_kestic_binary();

            let store = common::isolated_secret_store();
            let dest_id = "k7m2q9xa";
            let pw = SecretValue::from_str_secret("secret-from-isolated-dbus-kestic!");
            store
                .set_secret(dest_id, "repo", &pw)
                .expect("set secret in isolated keyring");

            // AC-M1-05-2: `kestic secret get <ziel-id>` outputs exact password + Exit 0.
            let ok_out = kestic_cmd(&["secret", "get", dest_id])
                .output()
                .expect("run kestic secret get");
            assert_eq!(ok_out.status.code(), Some(0));
            assert_eq!(
                String::from_utf8_lossy(&ok_out.stdout),
                pw.as_str().unwrap()
            );

            // AC-M1-05-2: Unknown `<ziel-id>` -> Exit 1, empty stdout.
            let miss_out = kestic_cmd(&["secret", "get", "zzzzzzzz"])
                .output()
                .expect("run kestic secret get missing");
            assert_eq!(miss_out.status.code(), Some(1));
            assert!(miss_out.stdout.is_empty());

            // AC-M1-05-3: Real restic init + snapshots via `RESTIC_PASSWORD_COMMAND`.
            let pw_cmd = format!("{} secret get {dest_id}", bin.display());
            let init_out = Command::new(&restic_exe)
                .args(["-r", &repo_dir.to_string_lossy(), "init"])
                .env_remove("RESTIC_PASSWORD")
                .env("RESTIC_PASSWORD_COMMAND", &pw_cmd)
                .output()
                .expect("restic init");
            assert!(
                init_out.status.success(),
                "restic init failed: {}",
                String::from_utf8_lossy(&init_out.stderr)
            );

            let snap_out = Command::new(&restic_exe)
                .args(["-r", &repo_dir.to_string_lossy(), "snapshots", "--json"])
                .env_remove("RESTIC_PASSWORD")
                .env("RESTIC_PASSWORD_COMMAND", &pw_cmd)
                .output()
                .expect("restic snapshots");
            assert!(snap_out.status.success());
        },
    );
}

/// AC-M1-05-1 (FR-08): against the headless keyring a secret can be (a) written and read back,
/// (b) replaced by a new value and (c) deleted, after which it reads as absent.
#[test]
fn ac_m1_05_1_secret_service_set_replace_and_delete_on_headless_keyring() {
    run_isolated_test(
        "ac_m1_05_1_secret_service_set_replace_and_delete_on_headless_keyring",
        "KESTIC_M1_S05_KEYRING_INNER",
        |_root| {
            let store: &dyn SecretStore = &common::isolated_secret_store();
            let initial = SecretValue::from_str_secret("kestic-repo-pass-01-äöü");
            let updated = SecretValue::from_str_secret("kestic-repo-pass-02-replaced");

            store
                .set_secret("nas-prod", "repo-password", &initial)
                .expect("write secret to headless keyring");
            assert_eq!(
                store
                    .get_secret("nas-prod", "repo-password")
                    .expect("read secret"),
                Some(initial),
                "(a) the written secret reads back identically"
            );

            store
                .set_secret("nas-prod", "repo-password", &updated)
                .expect("replace secret");
            assert_eq!(
                store
                    .get_secret("nas-prod", "repo-password")
                    .expect("read replaced secret"),
                Some(updated),
                "(b) a second write replaces the value"
            );

            store
                .delete_secret("nas-prod", "repo-password")
                .expect("delete secret");
            assert_eq!(
                store
                    .get_secret("nas-prod", "repo-password")
                    .expect("read deleted secret"),
                None,
                "(c) a deleted secret reads as absent"
            );
        },
    );
}

/// AC-M1-05-1 (FR-08, Q5): the file backend (a) stores a secret that reads back identically,
/// (b) keeps the secret file at mode 0600 and (c) reads a deleted secret as absent.
#[test]
fn ac_m1_05_1_file_backend_roundtrips_with_mode_0600() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store_path = temp.path().join("secrets.hex");
    let store = FileSecretStore::new(&store_path);
    let secret = SecretValue::from_str_secret("fallback-secret-token");

    store
        .set_secret("s3-archive", "backend-secret-key", &secret)
        .expect("write fallback secret");

    assert_eq!(
        store
            .get_secret("s3-archive", "backend-secret-key")
            .expect("read fallback secret"),
        Some(secret),
        "(a) the written secret reads back identically"
    );
    let mode = fs::metadata(&store_path)
        .expect("stat secret file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "(b) the secret file must be 0600");

    store
        .delete_secret("s3-archive", "backend-secret-key")
        .expect("delete fallback secret");
    assert_eq!(
        store
            .get_secret("s3-archive", "backend-secret-key")
            .expect("read deleted fallback secret"),
        None,
        "(c) a deleted secret reads as absent"
    );
}

struct FakeLockedSecretService;

#[interface(name = "org.freedesktop.Secret.Service")]
impl FakeLockedSecretService {
    #[zbus(name = "OpenSession")]
    fn open_session(
        &self,
        _algorithm: &str,
        _input: OwnedVariant,
    ) -> (OwnedVariant, OwnedObjectPath) {
        (
            OwnedVariant::from(zbus::Str::from("")),
            OwnedObjectPath::try_from("/org/freedesktop/secrets/session/s1").expect("path"),
        )
    }

    #[zbus(name = "ReadAlias")]
    fn read_alias(&self, _name: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/org/freedesktop/secrets/collection/locked").expect("path")
    }

    #[zbus(name = "Unlock")]
    fn unlock(&self, _objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
        (
            Vec::new(),
            OwnedObjectPath::try_from("/org/freedesktop/secrets/prompt/p1").expect("prompt"),
        )
    }

    #[zbus(name = "SearchItems")]
    fn search_items(
        &self,
        _attributes: HashMap<String, String>,
    ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        (Vec::new(), Vec::new())
    }
}

struct FakeLockedCollection {
    locked: Arc<AtomicBool>,
}

#[interface(name = "org.freedesktop.Secret.Collection")]
impl FakeLockedCollection {
    #[zbus(property, name = "Locked")]
    fn locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }
}

struct FakeSecretSession;

#[interface(name = "org.freedesktop.Secret.Session")]
impl FakeSecretSession {
    #[zbus(name = "Close")]
    fn close(&self) {}
}

struct FakeSecretPrompt {
    locked: Arc<AtomicBool>,
    dismiss: Arc<AtomicBool>,
}

#[interface(name = "org.freedesktop.Secret.Prompt")]
impl FakeSecretPrompt {
    #[zbus(name = "Prompt")]
    async fn prompt(
        &self,
        _window_id: &str,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) {
        let dismissed = self.dismiss.load(Ordering::SeqCst);
        if !dismissed {
            self.locked.store(false, Ordering::SeqCst);
        }
        let result: OwnedVariant = zbus::Variant::from(vec![
            OwnedObjectPath::try_from("/org/freedesktop/secrets/collection/locked").expect("path"),
        ])
        .try_into()
        .expect("owned variant");
        let _ = Self::completed(&emitter, dismissed, result).await;
    }

    #[zbus(signal, name = "Completed")]
    async fn completed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        dismissed: bool,
        result: OwnedVariant,
    ) -> zbus::Result<()>;
}

/// AC-M1-05-1 (FR-08, ADR-0004): a locked Secret Service collection that needs a prompt (a) fails
/// `check_availability` and (b) fails `set_secret` with `LockedOrPromptRequired` instead of
/// hanging; (c) a dismissed unlock prompt keeps it locked; (d) an accepted prompt unlocks it.
#[test]
fn ac_m1_05_1_locked_collection_reports_prompt_required_until_the_prompt_is_accepted() {
    run_isolated_test(
        "ac_m1_05_1_locked_collection_reports_prompt_required_until_the_prompt_is_accepted",
        "KESTIC_M1_S05_LOCKED_INNER",
        |_root| {
            let locked = Arc::new(AtomicBool::new(true));
            let dismiss = Arc::new(AtomicBool::new(true));
            let _conn = block_on(async {
                zbus::connection::Builder::session()
                    .name("net.daester.Kestic.FakeLockedSecrets")
                    .serve_at("/org/freedesktop/secrets", FakeLockedSecretService)
                    .serve_at(
                        "/org/freedesktop/secrets/collection/locked",
                        FakeLockedCollection {
                            locked: Arc::clone(&locked),
                        },
                    )
                    .serve_at("/org/freedesktop/secrets/session/s1", FakeSecretSession)
                    .serve_at(
                        "/org/freedesktop/secrets/prompt/p1",
                        FakeSecretPrompt {
                            locked: Arc::clone(&locked),
                            dismiss: Arc::clone(&dismiss),
                        },
                    )
                    .build()
                    .await
                    .expect("build fake locked secret service")
            });
            let store = SecretServiceStore::new("net.daester.Kestic.FakeLockedSecrets", "default");

            let avail_err = store
                .check_availability()
                .expect_err("locked collection must fail check_availability");
            assert!(
                matches!(avail_err, SecretError::LockedOrPromptRequired(_)),
                "(a) expected LockedOrPromptRequired from check_availability, got {avail_err:?}"
            );

            let set_err = store
                .set_secret("nas", "repo-password", &SecretValue::from_str_secret("pw"))
                .expect_err("locked collection must reject the write");
            assert!(
                matches!(set_err, SecretError::LockedOrPromptRequired(_)),
                "(b) expected LockedOrPromptRequired from set_secret, got {set_err:?}"
            );

            let cancel_err = store
                .unlock_interactive()
                .expect_err("dismissed prompt must stay locked");
            assert!(
                matches!(cancel_err, SecretError::LockedOrPromptRequired(_)),
                "(c) expected LockedOrPromptRequired after dismissing, got {cancel_err:?}"
            );

            dismiss.store(false, Ordering::SeqCst);
            store
                .unlock_interactive()
                .expect("(d) accepted prompt must unlock the collection");
            store
                .check_availability()
                .expect("(d) check_availability must succeed after the unlock");
        },
    );
}
