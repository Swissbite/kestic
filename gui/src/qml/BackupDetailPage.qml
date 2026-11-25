// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "backupDetailPage"
    title: currentData ? currentData.name : i18n("Backup")

    required property var rootWindow
    required property var appBackend
    required property string backupId

    // Shortcut of «Back Up Now» for the backup shown here.
    readonly property string backUpNowShortcut: "Ctrl+Shift+B"
    property var currentData: null
    property var nestedWarnings: []
    property var lintWarnings: []

    function refreshFromModel() {
        var found = rootWindow.findBackupData(backupId);
        currentData = found;
        if (found) {
            nestedWarnings = rootWindow.parseJsonOr(found.nestedWarningsJson, []);
            lintWarnings = rootWindow.parseJsonOr(found.excludeLintJson, []);
        } else {
            nestedWarnings = [];
            lintWarnings = [];
        }
    }

    Connections {
        target: appBackend
        function onModelsUpdated() {
            page.refreshFromModel();
        }
    }

    Component.onCompleted: refreshFromModel()

    actions: [
        Kirigami.Action {
            id: detailBackupNowAction
            objectName: "detailBackupNowAction"
            icon.name: currentData && currentData.isRunning
                ? "process-stop"
                : (currentData && currentData.hasError && currentData.errorAction === "update_stored_password"
                    ? "dialog-password"
                    : "media-playback-start")
            text: currentData && currentData.isRunning
                ? Texts.label("cancel")
                : (currentData && currentData.hasError && currentData.errorAction === "update_stored_password"
                    ? appBackend.recoveryActionLabel("update_stored_password")
                    : Texts.label("back_up_now"))
            enabled: currentData !== null && (currentData.isRunning
                || (currentData.hasError && currentData.errorAction === "update_stored_password" && appBackend.blocking_kind === "")
                || (currentData.available && appBackend.blocking_kind === ""))
            // Ctrl+Shift+B starts this backup; it never cancels a run or opens the password dialog.
            shortcut: currentData && !currentData.isRunning
                && !(currentData.hasError && currentData.errorAction === "update_stored_password")
                ? page.backUpNowShortcut : ""
            tooltip: currentData && currentData.hasError && currentData.errorAction === "update_stored_password"
                ? i18n("Update saved repository password before backing up")
                : (currentData && !currentData.available
                    ? currentData.availabilityReason
                    : (appBackend.blocking_kind !== "" ? appBackend.blocking_message : i18n("Start backup now")))
            onTriggered: {
                if (!currentData) return;
                if (currentData.isRunning) {
                    appBackend.cancelRun(currentData.activeRunId);
                } else if (currentData.hasError && currentData.errorAction === "update_stored_password") {
                    rootWindow.openUpdatePasswordDialog(currentData.destinationId, currentData.destinationName);
                } else {
                    appBackend.startBackup(backupId);
                }
            }
        },
        Kirigami.Action {
            objectName: "backupDetailPauseAction"
            icon.name: currentData && currentData.isPaused ? "media-playback-start" : "state-pause"
            text: currentData && currentData.isPaused ? Texts.label("resume") : i18n("Pause…")
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            tooltip: currentData && currentData.isPaused ? currentData.pausedLine : i18n("Temporarily pause this backup's schedule")
            onTriggered: currentData && currentData.isPaused ? appBackend.resumeBackup(backupId) : backupPauseMenu.popup()
        },
        Kirigami.Action {
            objectName: "backupDetailBackUpAnywayAction"
            icon.name: "media-playback-start"
            text: Texts.label("back_up_now_anyway")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            visible: currentData !== null && !currentData.isRunning && currentData.hasError && currentData.errorAction === "update_stored_password"
            enabled: currentData !== null && currentData.available && !currentData.isRunning && appBackend.blocking_kind === ""
            shortcut: visible ? page.backUpNowShortcut : ""
            onTriggered: appBackend.startBackup(backupId)
        },
        Kirigami.Action {
            objectName: "backupDetailPreviewAction"
            icon.name: "document-preview"
            text: i18n("Preview: What Would Be Backed Up?")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: if (currentData) rootWindow.openBackupDryRunPreviewPage(backupId, "")
        },
        Kirigami.Action {
            objectName: "backupDetailRenameAction"
            icon.name: "edit-rename"
            text: Texts.label("rename_ellipsis")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: if (currentData) rootWindow.openRenameBackupDialog(backupId, currentData.name)
        },
        Kirigami.Action {
            objectName: "backupDetailShowCommandAction"
            icon.name: "utilities-terminal"
            text: Texts.label("show_command")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null
            onTriggered: if (currentData) rootWindow.openShowCommandDialog(
                appBackend.getBackupTransparentCommand(backupId),
                currentData.name
            )
        },
        Kirigami.Action {
            objectName: "backupDetailRemoveAction"
            icon.name: "list-remove"
            text: Texts.label("remove_from_kestic_ellipsis")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: if (currentData) rootWindow.openRemoveBackupDialog(backupId, currentData.name)
        }
    ]

    Controls.Menu {
        id: backupPauseMenu
        objectName: "backupPauseMenu"

        Controls.MenuItem {
            objectName: "pauseBackup1hItem"
            text: Texts.label("pause_one_hour")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "1h")
        }
        Controls.MenuItem {
            objectName: "pauseBackup3hItem"
            text: Texts.label("pause_three_hours")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "3h")
        }
        Controls.MenuItem {
            objectName: "pauseBackupTomorrowItem"
            text: Texts.label("pause_until_tomorrow")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "tomorrow")
        }
        Controls.MenuItem {
            objectName: "pauseBackupIndefiniteItem"
            text: Texts.label("pause_until_resume")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "indefinite")
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing
        visible: currentData !== null

        Kirigami.InlineMessage {
            objectName: "backupDetailPausedBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: currentData !== null && currentData.isPaused
            text: currentData ? currentData.pausedLine : ""
            actions: [
                Kirigami.Action {
                    objectName: "backupDetailResumeAction"
                    icon.name: "media-playback-start"
                    text: Texts.label("resume")
                    onTriggered: appBackend.resumeBackup(backupId)
                }
            ]
        }

        Kirigami.InlineMessage {
            objectName: "backupDetailErrorBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: currentData !== null && currentData.hasError
            text: currentData ? currentData.errorSummary : ""
            actions: [
                Kirigami.Action {
                    visible: currentData !== null && currentData.errorAction !== ""
                    text: currentData ? appBackend.recoveryActionLabel(currentData.errorAction) : ""
                    onTriggered: {
                        if (!currentData) return;
                        if (currentData.errorAction === "update_stored_password") {
                            rootWindow.openUpdatePasswordDialog(currentData.destinationId, currentData.destinationName);
                        } else if (currentData.errorAction === "add_to_excludes") {
                            rootWindow.openBackupExcludesPage(backupId);
                        } else if (currentData.errorAction === "check_restic_source") {
                            rootWindow.openSettingsWindow();
                        } else {
                            appBackend.startBackup(backupId);
                        }
                    }
                }
            ]
        }

        Kirigami.InlineMessage {
            id: nestedSourceWarningBanner
            objectName: "nestedSourceWarningBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: page.nestedWarnings.length > 0
            text: page.nestedWarnings.length > 0
                ? Texts.labelArgs("source_overlap", [page.nestedWarnings[0].other_backup_name,
                       page.nestedWarnings[0].other_source_path])
                : ""
        }

        ColumnLayout {
            Layout.fillWidth: true
            visible: page.lintWarnings.length > 0
            spacing: Kirigami.Units.smallSpacing

            Kirigami.InlineMessage {
                objectName: "backupDetailLintWarningBanner"
                Layout.fillWidth: true
                type: Kirigami.MessageType.Warning
                visible: page.lintWarnings.length > 0
                text: page.lintWarnings.length > 0
                    ? i18n("Exclude rule warning on line %1: '%2' has no effect with '%3'.",
                           page.lintWarnings[0].line_number,
                           page.lintWarnings[0].rule,
                           page.lintWarnings[0].parent_rule)
                    : ""
            }

            Controls.Button {
                id: reviewExcludeRulesButton
                Layout.alignment: Qt.AlignRight
                text: i18n("Review Exclude Rules…")
                Accessible.name: text
                onClicked: {
                    rootWindow.nextTriggerItem = reviewExcludeRulesButton;
                    rootWindow.openBackupExcludesPage(backupId);
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Item {
                Layout.fillWidth: true
                implicitHeight: detailStatusRow.implicitHeight

                RowLayout {
                    id: detailStatusRow
                    anchors.fill: parent
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.Icon {
                        source: currentData ? currentData.statusIcon : "state-information"
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    }

                    Controls.Label {
                        objectName: "backupDetailStatusLine"
                        text: currentData ? (currentData.statusText + (currentData.lastRunSummary !== "" ? " · " + currentData.lastRunSummary : "")) : ""
                        color: Kirigami.Theme.textColor
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }
                }

                MouseArea {
                    id: detailStatusHoverArea
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.NoButton
                }

                Controls.ToolTip.visible: detailStatusHoverArea.containsMouse
                    && currentData !== null
                    && currentData.pendingReasonsTooltip !== ""
                Controls.ToolTip.text: currentData ? currentData.pendingReasonsTooltip : ""
            }

            Controls.Label {
                objectName: "backupDetailNextScheduledLabel"
                visible: currentData !== null && !currentData.isRunning && currentData.nextScheduledAtText !== ""
                text: currentData ? currentData.nextScheduledAtText : ""
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Controls.ProgressBar {
                // Fusion leaves the bar 0 px high and draws nothing; 6 px is the bar of Breeze.
                Layout.minimumHeight: topInset + bottomInset + Math.round(Kirigami.Units.smallSpacing * 1.5)
                visible: currentData !== null && currentData.isRunning
                from: 0
                to: 100
                indeterminate: currentData !== null && currentData.activeScanning
                value: currentData ? currentData.activePercent : 0
                Layout.fillWidth: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Controls.Button {
                    objectName: "backupDetailBackUpNowButton"
                    visible: !(currentData !== null && !currentData.isRunning && currentData.hasError && currentData.errorAction === "update_stored_password")
                    icon.name: currentData && currentData.isRunning ? "process-stop" : "media-playback-start"
                    text: currentData && currentData.isRunning ? Texts.label("cancel") : Texts.label("back_up_now")
                    enabled: currentData !== null && (currentData.isRunning || (currentData.available && appBackend.blocking_kind === ""))
                    Accessible.name: text
                    Controls.ToolTip.visible: hovered && !enabled
                    Controls.ToolTip.text: currentData && !currentData.available
                        ? currentData.availabilityReason
                        : appBackend.blocking_message
                    onClicked: {
                        if (!currentData) return;
                        if (currentData.isRunning) {
                            appBackend.cancelRun(currentData.activeRunId);
                        } else {
                            appBackend.startBackup(backupId);
                        }
                    }
                }

                Controls.Button {
                    objectName: "backupDetailUpdatePasswordButton"
                    visible: currentData !== null && !currentData.isRunning && currentData.hasError && currentData.errorAction === "update_stored_password"
                    icon.name: "dialog-password"
                    text: appBackend.recoveryActionLabel("update_stored_password")
                    enabled: appBackend.blocking_kind === ""
                    Accessible.name: text
                    onClicked: if (currentData) rootWindow.openUpdatePasswordDialog(currentData.destinationId, currentData.destinationName)
                }

                Controls.Label {
                    id: backupDetailUnavailableReason
                    objectName: "backupDetailUnavailableReason"
                    visible: currentData !== null && !currentData.isRunning && !currentData.available && currentData.availabilityReason !== ""
                    text: currentData ? currentData.availabilityReason : ""
                    opacity: 0.85
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Snapshots")
        }

        NavRow {
            id: showSnapshotsButton
            objectName: "showSnapshotsButton"
            accessibleName: i18n("Show all snapshots")
            iconSource: "view-list-details"
            labelText: i18n("Show all snapshots")
            onClicked: {
                rootWindow.nextTriggerItem = showSnapshotsButton;
                rootWindow.openBackupSnapshotsPage(backupId, currentData ? currentData.name : "");
            }
        }

        Kirigami.Heading {
            level: 3
            text: Texts.label("settings")
        }

        NavRow {
            id: editSourceRow
            objectName: "editSourceRow"
            accessibleName: i18n("Edit source folder")
            iconSource: currentData && currentData.sourceKind === "drive" ? "drive-harddisk" : "folder"
            labelText: currentData
                ? i18n("Source: %1 (%2)", currentData.sourcePath, currentData.sourceKindLabel)
                : Texts.label("source")
            onClicked: {
                rootWindow.nextTriggerItem = editSourceRow;
                rootWindow.openBackupSourcePage(backupId);
            }
        }

        NavRow {
            id: editExcludesRow
            objectName: "editExcludesRow"
            accessibleName: i18n("Edit exclude rules")
            iconSource: "filter-symbolic"
            labelText: currentData
                ? i18n("Exclude rules: %1", currentData.excludeSummary)
                : i18n("Exclude rules")
            onClicked: {
                rootWindow.nextTriggerItem = editExcludesRow;
                rootWindow.openBackupExcludesPage(backupId);
            }
        }

        NavRow {
            id: editRetentionRow
            objectName: "editRetentionRow"
            accessibleName: i18n("Edit retention")
            iconSource: "chronometer"
            labelText: currentData
                ? i18n("Retention: %1", currentData.retentionSummary)
                : Texts.label("retention")
            onClicked: {
                rootWindow.nextTriggerItem = editRetentionRow;
                rootWindow.openBackupRetentionPage(backupId);
            }
        }

        NavRow {
            id: editScheduleRow
            objectName: "editScheduleRow"
            accessibleName: i18n("Edit schedule")
            iconSource: "view-calendar"
            labelText: currentData
                ? i18n("Schedule: %1", currentData.scheduleSummary)
                : Texts.label("schedule")
            onClicked: {
                rootWindow.nextTriggerItem = editScheduleRow;
                rootWindow.openBackupSchedulePage(backupId);
            }
        }

        NavRow {
            id: editConditionsRow
            objectName: "editConditionsRow"
            accessibleName: i18n("Edit run conditions")
            iconSource: "preferences-system-power"
            labelText: currentData
                ? i18n("Run conditions: %1", currentData.conditionsSummary)
                : Texts.label("run_conditions")
            onClicked: {
                rootWindow.nextTriggerItem = editConditionsRow;
                rootWindow.openBackupConditionsPage(backupId);
            }
        }

        NavRow {
            id: editDestinationRow
            objectName: "editDestinationRow"
            accessibleName: i18n("Assign destination")
            iconSource: "network-server"
            labelText: i18n("Destination: %1", currentData ? currentData.destinationName : "")
            onClicked: {
                rootWindow.nextTriggerItem = editDestinationRow;
                rootWindow.openBackupReassignPage(backupId);
            }
        }

        NavRow {
            id: viewFullConfigRow
            objectName: "viewFullConfigRow"
            accessibleName: i18n("View full configuration")
            iconSource: "document-properties"
            labelText: i18n("View full configuration")
            onClicked: {
                rootWindow.nextTriggerItem = viewFullConfigRow;
                rootWindow.openBackupFullConfigPage(backupId);
            }
        }
    }
}
