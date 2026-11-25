// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Language-independent status and error codes shared by daemon and GUI over D-Bus
//! (`docs/spec/dbus-api.md` §Sprachunabhängige Codes).
//!
//! Every code serialises as a `snake_case` string. The texts for these codes live in
//! [`crate::messages`]. `kestic-core` converts its domain types into these codes.

/// Declares a code enum: variants with their wire code, plus `ALL`, `as_str` and `parse`.
/// Serde uses the same string, so `as_str` and the JSON name cannot drift apart.
macro_rules! code_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $code:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, ::serde::Serialize, ::serde::Deserialize,
        )]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $code)] $variant ),+
        }

        impl $name {
            pub const ALL: [Self; [$($code),+].len()] = [$(Self::$variant),+];

            /// Code string, equal to the serde name.
            pub fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $code ),+
                }
            }

            /// Parses a code string; `None` for unknown codes.
            pub fn parse(code: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|v| v.as_str() == code)
            }
        }
    };
}
pub(crate) use code_enum;

code_enum! {
    /// Action offered to the user for recoverable errors (`error_action`).
    RecoveryAction {
        UpdateStoredPassword => "update_stored_password",
        AddToExcludes => "add_to_excludes",
        CheckResticSource => "check_restic_source",
        UnlockRepository => "unlock_repository",
    }
}

code_enum! {
    /// Cause of a failed run, a run with warning, or a skipped backup (`error_kind`).
    ErrorKind {
        WrongPassword => "wrong_password",
        RepoMissing => "repo_missing",
        Locked => "locked",
        /// Destination unreachable, including DNS failures.
        DestinationUnreachable => "destination_unreachable",
        SourceUnavailable => "source_unavailable",
        AuthFailed => "auth_failed",
        IncompleteSnapshot => "incomplete_snapshot",
        /// Backed up, but the retention (`forget`) did not run.
        RetentionPending => "retention_pending",
        Cancelled => "cancelled",
        /// Only for skipped backups (`SkippedBackupDto`): the password store is locked.
        SecretStoreLocked => "secret_store_locked",
        /// Only for skipped backups (`SkippedBackupDto`): restic is missing or too old.
        ResticUnavailable => "restic_unavailable",
        /// SFTP: no `known_hosts` file knows the server's host key.
        HostKeyUnknown => "host_key_unknown",
        /// SFTP: a `known_hosts` file has another host key of the same type for the server.
        HostKeyChanged => "host_key_changed",
        /// A folder chosen for a restore does not exist in the snapshot.
        PathNotInSnapshot => "path_not_in_snapshot",
        /// A restore preview refused its target (`error_detail` is the folder concerned): restic
        /// would fail there or write into another folder. A restore run ends with it when its
        /// target is missing, another folder or needs a preview right before restic writes.
        InvalidTarget => "invalid_target",
        /// Anything not recognised (never a transient code).
        Unknown => "unknown",
    }
}

code_enum! {
    /// Why a due backup or due maintenance waits (`pending_reason_codes`,
    /// `maintenance_pending_reason`) and which run condition is not met (`unmet_run_conditions`).
    PendingReasonCode {
        SourceUnavailable => "source_unavailable",
        DestinationUnreachable => "destination_unreachable",
        OnBattery => "on_battery",
        NoNetwork => "no_network",
        Metered => "metered",
        WrongNetwork => "wrong_network",
        /// Another run on the same destination is active.
        Queue => "queue",
        /// Retry cooldown after a transient error; see `pending_retry_at_unix`.
        Backoff => "backoff",
    }
}

code_enum! {
    /// Phase of a running backup (`ActiveRunProgressDto.phase`, signal `RunProgress`). restic
    /// finds files and backs them up at the same time; only after its search the percentage
    /// is valid.
    RunProgressPhase {
        /// restic is still finding files; the total grows, so there is no percentage.
        Scanning => "scanning",
        /// restic knows the total; the percentage is valid and never falls.
        ScanFinished => "scan_finished",
    }
}

impl Default for RunProgressPhase {
    /// A run starts out scanning.
    fn default() -> Self {
        Self::Scanning
    }
}

code_enum! {
    /// Result of the source check when the source is unavailable (`source_status`).
    SourceStatus {
        PathMissing => "path_missing",
        DriveNotMounted => "drive_not_mounted",
        DriveMismatch => "drive_mismatch",
        DriveEmpty => "drive_empty",
    }
}

code_enum! {
    /// Why a backup cannot start right now (`blocked_reason`, signal `ManualRunBlocked`).
    BlockedReason {
        SourceUnavailable => "source_unavailable",
        DestinationUnreachable => "destination_unreachable",
        ConditionNotMet => "condition_not_met",
    }
}

code_enum! {
    /// Why «Back Up All Now» skipped a backup (`SkippedBackupDto.reason`).
    SkipReason {
        PermanentError => "permanent_error",
        SourceUnavailable => "source_unavailable",
        DestinationUnreachable => "destination_unreachable",
        ConditionNotMet => "condition_not_met",
        /// Start failed; the cause is in `SkippedBackupDto.error_kind`.
        Error => "error",
    }
}

impl From<BlockedReason> for SkipReason {
    fn from(reason: BlockedReason) -> Self {
        match reason {
            BlockedReason::SourceUnavailable => Self::SourceUnavailable,
            BlockedReason::DestinationUnreachable => Self::DestinationUnreachable,
            BlockedReason::ConditionNotMet => Self::ConditionNotMet,
        }
    }
}

code_enum! {
    /// Why the configured restic is unusable (`ResticStatusDto.error_kind`).
    ResticStatusCode {
        NotFound => "not_found",
        TooOld => "too_old",
        InvalidCustomPath => "invalid_custom_path",
        UnreadableVersion => "unreadable_version",
    }
}

code_enum! {
    /// Why a restic path cannot be used (`ResticStatusDto.path_problem`, with
    /// `invalid_custom_path`).
    ResticPathProblem {
        NotFound => "not_found",
        PermissionDenied => "permission_denied",
        NotExecutable => "not_executable",
        NotAFile => "not_a_file",
        Other => "other",
    }
}

code_enum! {
    /// The configured restic source (`ResticStatusDto.source`).
    ResticSourceCode {
        Automatic => "automatic",
        System => "system",
        AppManaged => "app_managed",
        Custom => "custom",
    }
}

code_enum! {
    /// Blocking state of the whole app (`BlockingStateDto.kind`).
    BlockingKind {
        SecretStoreLocked => "secret_store_locked",
        ResticUnavailable => "restic_unavailable",
    }
}

code_enum! {
    /// Result code of `TestDestinationConnection` (`ConnectionTestResultDto.error_kind`).
    ConnectionTestCode {
        InvalidInput => "invalid_input",
        /// `detail` = name of the destination that already uses the location.
        DuplicateLocation => "duplicate_location",
        DestinationUnreachable => "destination_unreachable",
        RepositoryExists => "repository_exists",
        NotWritable => "not_writable",
        MissingPassword => "missing_password",
        SecretStoreLocked => "secret_store_locked",
        Internal => "internal",
        ResticUnavailable => "restic_unavailable",
        WrongPassword => "wrong_password",
        AuthFailed => "auth_failed",
        RepositoryMissing => "repository_missing",
        CommandFailed => "command_failed",
        /// Set by the GUI when the daemon cannot be reached.
        DaemonUnreachable => "daemon_unreachable",
        /// A network address of the file dialog (KIO, e.g. `smb://…`) instead of a local path;
        /// `detail` = the address.
        NotALocalPath => "not_a_local_path",
        /// restic got no answer from the server within the time limit of the connection test.
        NotResponding => "not_responding",
        /// The repository URL of the generic form has no known scheme.
        UnsupportedScheme => "unsupported_scheme",
        /// A variable of the generic form would override Kestic's own environment or is no valid
        /// name; `detail` = the name.
        ReservedVariable => "reserved_variable",
        /// The backend needs a program that is not installed (`rclone`); `detail` = the program.
        ProgramMissing => "program_missing",
        /// SFTP: no `known_hosts` file knows the server; the result carries the key to confirm.
        UnknownHostKey => "unknown_host_key",
        /// SFTP: the server shows another key than a `known_hosts` file has; `detail` = that
        /// file.
        HostKeyChanged => "host_key_changed",
        /// SFTP: the server rejected the SSH key.
        SshKeyRejected => "ssh_key_rejected",
        /// SFTP: ssh reaches the server through a jump host (`ProxyJump`, `ProxyCommand`).
        JumpHostUnsupported => "jump_host_unsupported",
        /// Edited connection: the repository there has another ID than the destination's.
        OtherRepository => "other_repository",
        /// Edited connection: a credential is needed that is neither entered nor saved for this
        /// server; `detail` = the variable restic reads it from.
        CredentialsRequired => "credentials_required",
        /// An `rclone:` URL with a secret parameter; `detail` = the parameter name.
        SecretInUrl => "secret_in_url",
    }
}

code_enum! {
    /// Result of `UpdateDestination` (`DestinationUpdateResultDto.outcome`).
    DestinationUpdateOutcome {
        /// Saved: the same repository ID, or the connection did not change.
        Saved => "saved",
        /// Another repository ID at the edited connection: nothing saved.
        OtherRepository => "other_repository",
    }
}

code_enum! {
    /// Note of a successful connection test that the user confirms before the destination is
    /// added (`ConnectionTestResultDto.notes`).
    ConnectionNoteCode {
        /// The folder is on a CIFS/SMB share, which restic advises against for repositories.
        CifsShare => "cifs_share",
        /// The S3 bucket does not exist yet; adding the destination creates it.
        BucketMissing => "bucket_missing",
    }
}

code_enum! {
    /// Destination type offered by the wizard (`DestinationTypeDto.kind`), in the order of the
    /// cards: each maps to one restic backend.
    DestinationTypeCode {
        /// A local repository on a removable drive.
        Drive => "drive",
        /// A local repository in a folder or on another internal disk.
        Folder => "folder",
        Sftp => "sftp",
        Rest => "rest",
        S3 => "s3",
        B2 => "b2",
        Azure => "azure",
        Gs => "gs",
        Swift => "swift",
        Rclone => "rclone",
    }
}

code_enum! {
    /// Code of a D-Bus error: the error name `net.daester.Kestic.Error.<Name>`
    /// (`dbus-api.md` §Fehler-Namen). The error message is raw detail only.
    ServiceErrorCode {
        InvalidInput => "InvalidInput",
        NotFound => "NotFound",
        InUse => "InUse",
        ManualRunBlocked => "ManualRunBlocked",
        DestinationUnreachable => "DestinationUnreachable",
        WrongPassword => "WrongPassword",
        AuthFailed => "AuthFailed",
        SecretStoreLocked => "SecretStoreLocked",
        ResticUnavailable => "ResticUnavailable",
        AlreadyRunning => "AlreadyRunning",
        /// Unlock refused: the lock belongs to an active or remote process.
        LockInUse => "LockInUse",
        /// The repository location is already used by another destination.
        DuplicateLocation => "DuplicateLocation",
        /// SFTP: the server's host key is not known yet.
        HostKeyUnknown => "HostKeyUnknown",
        /// SFTP: the server shows another host key than before; the message is the file.
        HostKeyChanged => "HostKeyChanged",
        /// A restore to the original location or into a folder with content needs a finished,
        /// fresh preview with the same parameters.
        PreviewRequired => "PreviewRequired",
        /// A destination cannot be changed or removed while a restore from it starts, waits or
        /// runs.
        RestoreRunning => "RestoreRunning",
        Internal => "Internal",
        /// No D-Bus error name: the daemon could not be reached (transport error, set by the GUI).
        DaemonUnreachable => "DaemonUnreachable",
    }
}

code_enum! {
    /// Why installing the app-managed restic failed (`ResticInstallStatusDto.error_kind`).
    ResticInstallCode {
        /// No answer, a failed transfer, or an answer larger than its limit.
        Network => "network",
        /// A request or redirect left the allowed restic release hosts.
        HostNotAllowed => "host_not_allowed",
        /// The release or one of its files is missing or not in the expected form.
        AssetMissing => "asset_missing",
        UnsupportedArchitecture => "unsupported_architecture",
        GpgvMissing => "gpgv_missing",
        SignatureInvalid => "signature_invalid",
        ChecksumMismatch => "checksum_mismatch",
        /// The new binary reports another version than the release.
        VersionMismatch => "version_mismatch",
        /// The release or the new binary is older than the minimum restic version, or the
        /// release is older than the installed restic managed by Kestic.
        TooOld => "too_old",
        WriteFailed => "write_failed",
    }
}

code_enum! {
    /// Step of a running installation of the app-managed restic (`ResticInstallStatusDto.phase`).
    ResticInstallPhase {
        FetchingRelease => "fetching_release",
        Downloading => "downloading",
        Verifying => "verifying",
        /// Downloaded and checked; the binary is replaced once no restic runs.
        WaitingForRuns => "waiting_for_runs",
        Replacing => "replacing",
    }
}

code_enum! {
    /// Result of a finished installation (`ResticInstallStatusDto.outcome`).
    ResticInstallOutcome {
        Installed => "installed",
        /// The app-managed restic already was the latest release; nothing was downloaded.
        AlreadyCurrent => "already_current",
    }
}

code_enum! {
    /// Aggregated overall status precedence (see `docs/spec/ui/zustaende.md`).
    OverallStatusKind {
        Blocked => "blocked",
        Error => "error",
        Overdue => "overdue",
        Warning => "warning",
        Running => "running",
        Paused => "paused",
        Pending => "pending",
        Interrupted => "interrupted",
        Cancelled => "cancelled",
        NeverRun => "never_run",
        ManualOnly => "manual_only",
        Ok => "ok",
    }
}

code_enum! {
    /// Kind of a snapshot query (`SnapshotQueryRequest.kind`).
    SnapshotQueryKind {
        /// One folder level of a snapshot (`ls`).
        Ls => "ls",
        /// Search over snapshots (`find`).
        Find => "find",
        /// Changes between two snapshots (`diff`).
        Diff => "diff",
        /// Preview of a restore (`restore --dry-run`).
        RestorePreview => "restore_preview",
    }
}

code_enum! {
    /// Kind of a change between two snapshots (`DiffChangeDto.change`), from the modifier
    /// `restic diff --json` reports.
    SnapshotChange {
        /// Added (`+`).
        Added => "added",
        /// Removed (`-`).
        Removed => "removed",
        /// Content modified (`M`).
        Modified => "modified",
        /// The type changed, e.g. a file became a link (`T`).
        TypeChanged => "type_changed",
        /// The content changed while all metadata stayed the same (`?`).
        Bitrot => "bitrot",
    }
}

code_enum! {
    /// Kind of a run (`ActiveRunProgressDto.kind`, signal `RunProgress`).
    RunKindCode {
        Backup => "backup",
        Maintenance => "maintenance",
        Restore => "restore",
        Meta => "meta",
    }
}

impl Default for RunKindCode {
    /// Progress from a daemon that does not name the kind belongs to a backup.
    fn default() -> Self {
        Self::Backup
    }
}

code_enum! {
    /// What a restore does with a file that already exists at the target
    /// (`RestoreRequest.overwrite`, `RestoreStats.overwrite`); passed to restic as
    /// `--overwrite`.
    RestoreOverwrite {
        Never => "never",
        IfNewer => "if_newer",
        IfChanged => "if_changed",
        Always => "always",
    }
}

impl Default for RestoreOverwrite {
    /// Existing files stay unless the user chooses otherwise for one restore.
    fn default() -> Self {
        Self::Never
    }
}

code_enum! {
    /// What a restore would do with one file (`RestorePreviewEntryDto.outcome`).
    RestoreOutcome {
        /// The file does not exist at the target yet.
        New => "new",
        /// The file exists and is replaced.
        Overwritten => "overwritten",
        /// The file exists with other content and stays as it is.
        Kept => "kept",
        /// The file exists with the same content.
        Identical => "identical",
    }
}

code_enum! {
    /// Result of the daemon's check of a restore target (`RestoreTargetCheckDto.code`).
    RestoreTargetCode {
        Ok => "ok",
        /// An existing folder with content: only with a preview.
        ExistsNotEmpty => "exists_not_empty",
        NotADirectory => "not_a_directory",
        NotWritable => "not_writable",
        /// The folder of the original location does not exist; Kestic does not create it.
        MissingMappedDir => "missing_mapped_dir",
        /// The target lies inside the repository of a local destination, or the selection
        /// would write into such a repository below the target.
        InsideRepository => "inside_repository",
        /// The selection would write into Kestic's own configuration or data folder below
        /// the target.
        KesticData => "kestic_data",
    }
}

code_enum! {
    /// A note of a restore preview (`RestorePreviewResultDto.notes`).
    RestoreNoteCode {
        /// The snapshot belongs to another user; the restored files belong to the current one.
        OwnershipNotRestored => "ownership_not_restored",
        /// A file that is overwritten has further hard links.
        HardLinksAffected => "hard_links_affected",
        /// The selection matches no file of the snapshot.
        NothingSelectedMatches => "nothing_selected_matches",
        /// The target holds a symbolic link where the snapshot has an entry.
        SymlinkInTarget => "symlink_in_target",
        /// restic sets mode and modification time of existing folders at the target from the
        /// snapshot, in every overwrite mode.
        FolderMetadataReset => "folder_metadata_reset",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    fn json<T: Serialize>(value: T) -> String {
        serde_json::to_string(&value).expect("serialize code")
    }

    /// The typed codes keep the exact JSON strings of the former string fields.
    #[test]
    fn codes_serialize_as_the_former_snake_case_strings() {
        assert_eq!(json(ErrorKind::RepoMissing), "\"repo_missing\"");
        assert_eq!(
            json(ErrorKind::IncompleteSnapshot),
            "\"incomplete_snapshot\""
        );
        assert_eq!(json(PendingReasonCode::Queue), "\"queue\"");
        assert_eq!(json(PendingReasonCode::WrongNetwork), "\"wrong_network\"");
        assert_eq!(json(SourceStatus::DriveNotMounted), "\"drive_not_mounted\"");
        assert_eq!(json(SkipReason::PermanentError), "\"permanent_error\"");
        assert_eq!(
            json(ResticStatusCode::InvalidCustomPath),
            "\"invalid_custom_path\""
        );
        assert_eq!(
            json(BlockingKind::SecretStoreLocked),
            "\"secret_store_locked\""
        );
        assert_eq!(json(OverallStatusKind::NeverRun), "\"never_run\"");
    }

    /// `parse` finds every variant by its code string and rejects unknown codes.
    #[test]
    fn parse_finds_every_code() {
        for v in BlockedReason::ALL {
            assert_eq!(BlockedReason::parse(v.as_str()), Some(v));
            assert_eq!(BlockedReason::parse("nope"), None);
        }
        for v in SkipReason::ALL {
            assert_eq!(SkipReason::parse(v.as_str()), Some(v));
            assert_eq!(SkipReason::parse("nope"), None);
        }
        for v in ResticStatusCode::ALL {
            assert_eq!(ResticStatusCode::parse(v.as_str()), Some(v));
            assert_eq!(ResticStatusCode::parse("nope"), None);
        }
        for v in OverallStatusKind::ALL {
            assert_eq!(OverallStatusKind::parse(v.as_str()), Some(v));
            assert_eq!(OverallStatusKind::parse("nope"), None);
        }
        for v in ErrorKind::ALL {
            assert_eq!(ErrorKind::parse(v.as_str()), Some(v));
            assert_eq!(ErrorKind::parse("nope"), None);
        }
        for v in SourceStatus::ALL {
            assert_eq!(SourceStatus::parse(v.as_str()), Some(v));
            assert_eq!(SourceStatus::parse("nope"), None);
        }
        for v in BlockingKind::ALL {
            assert_eq!(BlockingKind::parse(v.as_str()), Some(v));
            assert_eq!(BlockingKind::parse("nope"), None);
        }
        for v in PendingReasonCode::ALL {
            assert_eq!(PendingReasonCode::parse(v.as_str()), Some(v));
            assert_eq!(PendingReasonCode::parse("nope"), None);
        }
        for v in RecoveryAction::ALL {
            assert_eq!(RecoveryAction::parse(v.as_str()), Some(v));
            assert_eq!(RecoveryAction::parse("nope"), None);
        }
        for v in ConnectionTestCode::ALL {
            assert_eq!(ConnectionTestCode::parse(v.as_str()), Some(v));
            assert_eq!(ConnectionTestCode::parse("nope"), None);
        }
        for v in ConnectionNoteCode::ALL {
            assert_eq!(ConnectionNoteCode::parse(v.as_str()), Some(v));
            assert_eq!(ConnectionNoteCode::parse("nope"), None);
        }
        for v in DestinationTypeCode::ALL {
            assert_eq!(DestinationTypeCode::parse(v.as_str()), Some(v));
            assert_eq!(DestinationTypeCode::parse("nope"), None);
        }
        for v in SnapshotQueryKind::ALL {
            assert_eq!(SnapshotQueryKind::parse(v.as_str()), Some(v));
            assert_eq!(SnapshotQueryKind::parse("nope"), None);
        }
        for v in SnapshotChange::ALL {
            assert_eq!(SnapshotChange::parse(v.as_str()), Some(v));
            assert_eq!(SnapshotChange::parse("nope"), None);
        }
        for v in RunProgressPhase::ALL {
            assert_eq!(RunProgressPhase::parse(v.as_str()), Some(v));
            assert_eq!(RunProgressPhase::parse("nope"), None);
        }
        for v in RunKindCode::ALL {
            assert_eq!(RunKindCode::parse(v.as_str()), Some(v));
        }
        for v in RestoreOverwrite::ALL {
            assert_eq!(RestoreOverwrite::parse(v.as_str()), Some(v));
        }
        for v in RestoreOutcome::ALL {
            assert_eq!(RestoreOutcome::parse(v.as_str()), Some(v));
        }
        for v in RestoreTargetCode::ALL {
            assert_eq!(RestoreTargetCode::parse(v.as_str()), Some(v));
        }
        for v in RestoreNoteCode::ALL {
            assert_eq!(RestoreNoteCode::parse(v.as_str()), Some(v));
        }
    }
}
