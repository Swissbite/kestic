// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    readonly property SharedTexts sharedTexts: SharedTexts {}
    id: page
    objectName: "backupDetailPage"
    title: currentData ? currentData.name : i18n("Backup")

    required property var rootWindow
    required property var appBackend
    required property string backupId

    property var currentData: null
    property var nestedWarnings: []
    property var lintWarnings: []

    function refreshFromModel() {
        var found = rootWindow.findBackupData(backupId);
        currentData = found;
        if (found) {
            try {
                nestedWarnings = JSON.parse(found.nestedWarningsJson || "[]");
            } catch (e) {
                nestedWarnings = [];
            }
            try {
                lintWarnings = JSON.parse(found.excludeLintJson || "[]");
            } catch (e) {
                lintWarnings = [];
            }
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
                ? i18n("Cancel")
                : (currentData && currentData.hasError && currentData.errorAction === "update_stored_password"
                    ? appBackend.recoveryActionLabel("update_stored_password")
                    : sharedTexts.label("back_up_now"))
            enabled: currentData !== null && (currentData.isRunning
                || (currentData.hasError && currentData.errorAction === "update_stored_password" && appBackend.blocking_kind === "")
                || (currentData.available && appBackend.blocking_kind === ""))
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
            id: detailPauseAction
            objectName: "backupDetailPauseAction"
            icon.name: currentData && currentData.isPaused ? "media-playback-start" : "state-pause"
            text: currentData && currentData.isPaused ? i18n("Resume") : i18n("Pause…")
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            tooltip: currentData && currentData.isPaused ? currentData.pausedLine : i18n("Temporarily pause this backup's schedule")
            onTriggered: currentData && currentData.isPaused ? appBackend.resumeBackup(backupId) : backupPauseMenu.popup()
        },
        Kirigami.Action {
            id: detailBackUpAnywayAction
            objectName: "backupDetailBackUpAnywayAction"
            icon.name: "media-playback-start"
            text: i18n("Back Up Now Anyway")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            visible: currentData !== null && !currentData.isRunning && currentData.hasError && currentData.errorAction === "update_stored_password"
            enabled: currentData !== null && currentData.available && !currentData.isRunning && appBackend.blocking_kind === ""
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
            text: i18n("Rename…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: if (currentData) rootWindow.openRenameBackupDialog(backupId, currentData.name)
        },
        Kirigami.Action {
            objectName: "backupDetailShowCommandAction"
            icon.name: "utilities-terminal"
            text: i18n("Show Command")
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
            text: i18n("Remove from Kestic…")
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
            text: sharedTexts.label("pause_one_hour")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "1h")
        }
        Controls.MenuItem {
            objectName: "pauseBackup3hItem"
            text: sharedTexts.label("pause_three_hours")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "3h")
        }
        Controls.MenuItem {
            objectName: "pauseBackupTomorrowItem"
            text: sharedTexts.label("pause_until_tomorrow")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "tomorrow")
        }
        Controls.MenuItem {
            objectName: "pauseBackupIndefiniteItem"
            text: sharedTexts.label("pause_until_resume")
            Accessible.name: text
            onTriggered: appBackend.pauseBackup(backupId, "indefinite")
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing
        visible: currentData !== null

        Kirigami.InlineMessage {
            id: pausedBanner
            objectName: "backupDetailPausedBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: currentData !== null && currentData.isPaused
            text: currentData ? currentData.pausedLine : ""
            actions: [
                Kirigami.Action {
                    objectName: "backupDetailResumeAction"
                    icon.name: "media-playback-start"
                    text: i18n("Resume")
                    onTriggered: appBackend.resumeBackup(backupId)
                }
            ]
        }

        Kirigami.InlineMessage {
            id: errorBanner
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
                ? i18n("This source overlaps with backup '%1' (%2) on the same destination. Some files may be saved twice or with other exclude rules.",
                       page.nestedWarnings[0].other_backup_name,
                       page.nestedWarnings[0].other_source_path)
                : ""
        }

        ColumnLayout {
            Layout.fillWidth: true
            visible: page.lintWarnings.length > 0
            spacing: Kirigami.Units.smallSpacing

            Kirigami.InlineMessage {
                id: excludeLintWarningBanner
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
                        id: backupStatusLine
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
                id: backupNextScheduledLabel
                objectName: "backupDetailNextScheduledLabel"
                visible: currentData !== null && !currentData.isRunning && currentData.nextScheduledAtText !== ""
                text: currentData ? currentData.nextScheduledAtText : ""
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Controls.ProgressBar {
                visible: currentData !== null && currentData.isRunning
                from: 0
                to: 100
                value: currentData ? currentData.activePercent : 0
                Layout.fillWidth: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Controls.Button {
                    id: inlineBackUpNowButton
                    objectName: "backupDetailBackUpNowButton"
                    visible: !(currentData !== null && !currentData.isRunning && currentData.hasError && currentData.errorAction === "update_stored_password")
                    icon.name: currentData && currentData.isRunning ? "process-stop" : "media-playback-start"
                    text: currentData && currentData.isRunning ? i18n("Cancel") : sharedTexts.label("back_up_now")
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
                    id: inlineUpdatePasswordButton
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

        Controls.ItemDelegate {
            id: showSnapshotsButton
            objectName: "showSnapshotsButton"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Show all snapshots")
            onClicked: {
                rootWindow.nextTriggerItem = showSnapshotsButton;
                rootWindow.openBackupSnapshotsPage(backupId, currentData ? currentData.name : "");
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (showSnapshotsButton.activeFocus || showSnapshotsButton.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "view-list-details"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: i18n("Show all snapshots")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Settings")
        }

        Controls.ItemDelegate {
            id: editSourceRow
            objectName: "editSourceRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Edit source folder")
            onClicked: {
                rootWindow.nextTriggerItem = editSourceRow;
                rootWindow.openBackupSourcePage(backupId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (editSourceRow.activeFocus || editSourceRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: currentData && currentData.sourceKind === "drive" ? "drive-harddisk" : "folder"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: currentData
                        ? i18n("Source: %1 (%2)", currentData.sourcePath, currentData.sourceKindLabel)
                        : i18n("Source")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Controls.ItemDelegate {
            id: editExcludesRow
            objectName: "editExcludesRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Edit exclude rules")
            onClicked: {
                rootWindow.nextTriggerItem = editExcludesRow;
                rootWindow.openBackupExcludesPage(backupId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (editExcludesRow.activeFocus || editExcludesRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "filter-symbolic"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: currentData
                        ? i18n("Exclude rules: %1", currentData.excludeSummary)
                        : i18n("Exclude rules")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Controls.ItemDelegate {
            id: editRetentionRow
            objectName: "editRetentionRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Edit retention policy")
            onClicked: {
                rootWindow.nextTriggerItem = editRetentionRow;
                rootWindow.openBackupRetentionPage(backupId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (editRetentionRow.activeFocus || editRetentionRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "chronometer"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: currentData
                        ? i18n("Retention: %1", currentData.retentionSummary)
                        : i18n("Retention")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Controls.ItemDelegate {
            id: editScheduleRow
            objectName: "editScheduleRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Edit schedule")
            onClicked: {
                rootWindow.nextTriggerItem = editScheduleRow;
                rootWindow.openBackupSchedulePage(backupId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (editScheduleRow.activeFocus || editScheduleRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "view-calendar"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: currentData
                        ? i18n("Schedule: %1", currentData.scheduleSummary)
                        : i18n("Schedule")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Controls.ItemDelegate {
            id: editConditionsRow
            objectName: "editConditionsRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Edit run conditions")
            onClicked: {
                rootWindow.nextTriggerItem = editConditionsRow;
                rootWindow.openBackupConditionsPage(backupId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (editConditionsRow.activeFocus || editConditionsRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "preferences-system-power"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: currentData
                        ? i18n("Run conditions: %1", currentData.conditionsSummary)
                        : i18n("Run conditions")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Controls.ItemDelegate {
            id: editDestinationRow
            objectName: "editDestinationRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Assign destination")
            onClicked: {
                rootWindow.nextTriggerItem = editDestinationRow;
                rootWindow.openBackupReassignPage(backupId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (editDestinationRow.activeFocus || editDestinationRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "network-server"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: i18n("Destination: %1", currentData ? currentData.destinationName : "")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Controls.ItemDelegate {
            id: viewFullConfigRow
            objectName: "viewFullConfigRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("View full configuration")
            onClicked: {
                rootWindow.nextTriggerItem = viewFullConfigRow;
                rootWindow.openBackupFullConfigPage(backupId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (viewFullConfigRow.activeFocus || viewFullConfigRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "document-properties"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: i18n("View full configuration")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }
    }
}
