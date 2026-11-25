// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! GUI texts that more than one page or Rust module shows. Each msgid is written only here:
//! QML asks for it by code (`Texts.label("close")`, `Texts.labelArgs("…", [a, b])`), Rust by
//! variant (`GuiText::Close.text()`). Texts that the tray shows too live in
//! `kestic_i18n::messages::Label`.

use kestic_i18n::messages::{Message, i18n_noop};

macro_rules! gui_texts {
    ($($variant:ident = $code:literal => $msgid:expr,)+) => {
        /// A GUI text used in more than one place.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum GuiText {
            $($variant,)+
        }

        impl GuiText {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            /// Code used by QML.
            pub fn code(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            /// English msgid in `po/de/kestic.po`.
            pub fn msgid(self) -> &'static str {
                match self {
                    $(Self::$variant => $msgid,)+
                }
            }
        }
    };
}

gui_texts! {
    // Actions
    Back = "back" => i18n_noop("Back"),
    BackUpNowAnyway = "back_up_now_anyway" => i18n_noop("Back Up Now Anyway"),
    Cancel = "cancel" => i18n_noop("Cancel"),
    CheckAgain = "check_again" => i18n_noop("Check Again"),
    Choose = "choose" => i18n_noop("Choose…"),
    Close = "close" => i18n_noop("Close"),
    Copy = "copy" => i18n_noop("Copy"),
    Details = "details" => i18n_noop("Details"),
    HideDetails = "hide_details" => i18n_noop("Hide Details"),
    Next = "next" => i18n_noop("Next"),
    PreviewEllipsis = "preview_ellipsis" => i18n_noop("Preview…"),
    Remove = "remove" => i18n_noop("Remove"),
    RemoveFromKesticEllipsis = "remove_from_kestic_ellipsis" => i18n_noop("Remove from Kestic…"),
    RenameEllipsis = "rename_ellipsis" => i18n_noop("Rename…"),
    Resume = "resume" => i18n_noop("Resume"),
    Refresh = "refresh" => i18n_noop("Refresh"),
    Retry = "retry" => i18n_noop("Retry"),
    Save = "save" => i18n_noop("Save"),
    Settings = "settings" => i18n_noop("Settings"),
    ShowCommand = "show_command" => i18n_noop("Show Command"),
    UnlockEllipsis = "unlock_ellipsis" => i18n_noop("Unlock…"),
    SetUpFirstBackup = "set_up_first_backup" => i18n_noop("Set Up First Backup"),
    NewBackup = "new_backup" => i18n_noop("New Backup"),
    AddNewDestination = "add_new_destination" => i18n_noop("Add New Destination"),
    AddNewDestinationEllipsis = "add_new_destination_ellipsis" => i18n_noop("Add New Destination…"),
    ChooseSourceFolderTitle = "choose_source_folder_title" => i18n_noop("Choose Source Folder"),
    ChooseSourceFolder = "choose_source_folder" => i18n_noop("Choose source folder"),

    // Areas and sections
    AboutKestic = "about_kestic" => i18n_noop("About Kestic"),
    Overview = "overview" => i18n_noop("Overview"),
    Restore = "restore" => i18n_noop("Restore"),
    Backups = "backups" => i18n_noop("Backups"),
    Destinations = "destinations" => i18n_noop("Destinations"),
    History = "history" => i18n_noop("History"),
    Source = "source" => i18n_noop("Source"),
    Destination = "destination" => i18n_noop("Destination"),
    Schedule = "schedule" => i18n_noop("Schedule"),
    Retention = "retention" => i18n_noop("Retention"),
    RunConditions = "run_conditions" => i18n_noop("Run conditions"),
    Maintenance = "maintenance" => i18n_noop("Maintenance"),
    ExcludeRules = "exclude_rules" => i18n_noop("Exclude Rules"),

    // Field labels
    Name = "name" => i18n_noop("Name"),
    NameLabel = "name_label" => i18n_noop("Name:"),
    RequiredNameLabel = "required_name_label" => i18n_noop("Name*:"),
    UsernameLabel = "username_label" => i18n_noop("Username:"),
    StatusLabel = "status_label" => i18n_noop("Status:"),
    FrequencyLabel = "frequency_label" => i18n_noop("Frequency:"),
    TimeLabel = "time_label" => i18n_noop("Time:"),
    RepositoryPassword = "repository_password" => i18n_noop("Repository password"),
    SourceFolderPath = "source_folder_path" => i18n_noop("Source folder path"),
    ExcludeTrash = "exclude_trash" => i18n_noop("Exclude trash folders"),
    ExcludeRulesSyntax = "exclude_rules_syntax" => i18n_noop("Exclude rules in restic syntax"),
    Filter = "filter" => i18n_noop("Filter"),
    SearchInBackupSnapshots = "search_in_backup_snapshots" => i18n_noop("Search in the snapshots of this backup"),

    // Values
    None = "none" => i18n_noop("None"),
    Folder = "folder" => i18n_noop("Folder"),
    Drive = "drive" => i18n_noop("Drive"),
    DriveWithLabel = "drive_with_label" => i18n_noop("Drive (%1)"),
    RestServer = "rest_server" => i18n_noop("REST Server"),
    Sftp = "sftp" => i18n_noop("SFTP"),
    S3Compatible = "s3_compatible" => i18n_noop("S3-Compatible Storage"),
    BackblazeB2 = "backblaze_b2" => i18n_noop("Backblaze B2"),
    Azure = "azure" => i18n_noop("Azure"),
    GoogleCloud = "google_cloud" => i18n_noop("Google Cloud"),
    Swift = "swift" => i18n_noop("Swift"),
    Rclone = "rclone" => i18n_noop("rclone"),
    ManualOnly = "manual_only" => i18n_noop("Manual only"),
    DeletedBackup = "deleted_backup" => i18n_noop("Deleted backup"),
    Interrupted = "interrupted" => i18n_noop("Interrupted"),
    Daily = "daily" => i18n_noop("Daily"),
    PresetStandard = "preset_standard" => i18n_noop("Standard (7 daily, 4 weekly, 12 monthly)"),
    PresetEconomical = "preset_economical" => i18n_noop("Economical (7 daily, 4 weekly)"),
    PresetLong = "preset_long" => i18n_noop("Long (7 daily, 4 weekly, 12 monthly, all yearly)"),
    Monday = "monday" => i18n_noop("Monday"),
    Tuesday = "tuesday" => i18n_noop("Tuesday"),
    Wednesday = "wednesday" => i18n_noop("Wednesday"),
    Thursday = "thursday" => i18n_noop("Thursday"),
    Friday = "friday" => i18n_noop("Friday"),
    Saturday = "saturday" => i18n_noop("Saturday"),
    Sunday = "sunday" => i18n_noop("Sunday"),
    Mon = "mon" => i18n_noop("Mon"),
    Tue = "tue" => i18n_noop("Tue"),
    Wed = "wed" => i18n_noop("Wed"),
    Thu = "thu" => i18n_noop("Thu"),
    Fri = "fri" => i18n_noop("Fri"),
    Sat = "sat" => i18n_noop("Sat"),
    Sun = "sun" => i18n_noop("Sun"),
    TwoParts = "two_parts" => i18n_noop("%1 · %2"),
    EarlierSnapshotsOn = "earlier_snapshots_on" => i18n_noop("Earlier snapshots on '%1'"),
    LoadingSnapshots = "loading_snapshots" => i18n_noop("Loading snapshots…"),
    NoEntriesMatchFilter = "no_entries_match_filter" => i18n_noop("No entries match the filter."),
    NoSnapshotsYet = "no_snapshots_yet" => i18n_noop("No snapshots yet. The first backup is still pending."),

    // Messages
    SaveChangesQuestion = "save_changes_question" => i18n_noop("Save changes?"),
    FillRequiredFields = "fill_required_fields" => i18n_noop("Please fill in all required fields."),
    DestinationSaved = "destination_saved" => i18n_noop("Destination saved"),
    CommandCopied = "command_copied" => i18n_noop("Command copied to clipboard"),
    NothingCreatedYet = "nothing_created_yet" => i18n_noop("No repository or configuration has been created yet."),
    RemoveFromKesticQuestion = "remove_from_kestic_question" => i18n_noop("Remove '%1' from Kestic?"),
    SourceOverlap = "source_overlap" => i18n_noop("This source overlaps with backup '%1' (%2) on the same destination. Some files may be saved twice or with other exclude rules."),
}

impl GuiText {
    /// Parses a QML code; `None` for unknown codes.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.code() == code)
    }

    /// The text in the GUI language.
    pub fn text(self) -> String {
        self.with_args(&[])
    }

    /// The text in the GUI language with `%1`, `%2`, … filled from `args`.
    pub fn with_args(self, args: &[&str]) -> String {
        crate::state_cache::tr(&Message::with_args(self.msgid(), args))
    }
}
