// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "backupPreviewPage"
    title: mode === "retention" ? i18n("Retention Preview") : i18n("Backup Preview")

    required property var rootWindow
    required property var appBackend
    required property string backupId
    required property string mode // "dry_run" | "retention"
    property string excludesJson: ""
    property string retentionPreviewJson: ""

    property string previewId: ""
    property string dryRunState: "running"
    property var dryRunResult: null
    property string dryRunError: ""

    readonly property var retentionPreviewData: {
        if (mode !== "retention" || retentionPreviewJson === "") return null;
        try {
            return JSON.parse(retentionPreviewJson);
        } catch (e) {
            return null;
        }
    }

    function formatSize(bytes) {
        var b = Number(bytes || 0);
        if (b >= 1000000000) return (b / 1000000000).toFixed(1) + " GB";
        if (b >= 1000000) return (b / 1000000).toFixed(1) + " MB";
        if (b >= 1000) return Math.round(b / 1000) + " KB";
        return b + " B";
    }

    function startDryRunIfNeeded() {
        if (mode !== "dry_run") return;
        dryRunState = "running";
        dryRunResult = null;
        dryRunError = "";
        previewId = appBackend.startBackupDryRun(backupId, excludesJson);
        if (previewId === "") {
            dryRunState = "error";
            dryRunError = appBackend.dry_run_error || appBackend.last_action_error || i18n("Preview could not be started.");
        } else {
            pollTimer.start();
        }
    }

    Component.onCompleted: startDryRunIfNeeded()

    Component.onDestruction: {
        pollTimer.stop();
        if (mode === "dry_run" && dryRunState === "running" && previewId !== "") {
            appBackend.cancelBackupDryRun(previewId);
        }
    }

    Timer {
        id: pollTimer
        interval: 150
        repeat: true
        onTriggered: {
            if (page.previewId === "") {
                stop();
                return;
            }
            var raw = appBackend.pollBackupDryRun(page.previewId);
            if (raw === "") {
                stop();
                page.dryRunState = "error";
                page.dryRunError = appBackend.dry_run_error || i18n("Preview failed.");
                return;
            }
            try {
                var dto = JSON.parse(raw);
                page.dryRunState = dto.state || "error";
                if (dto.state !== "running") {
                    stop();
                }
                if (dto.result) {
                    page.dryRunResult = dto.result;
                }
                if (dto.error) {
                    page.dryRunError = dto.error;
                }
            } catch (e) {
                stop();
                page.dryRunState = "error";
                page.dryRunError = i18n("Invalid preview response.");
            }
        }
    }

    actions: [
        Kirigami.Action {
            id: showDryRunCommandAction
            objectName: "showDryRunCommandAction"
            icon.name: "utilities-terminal"
            text: i18n("Show Command")
            visible: page.mode === "dry_run" && page.dryRunResult !== null && (page.dryRunResult.transparent_command || "") !== ""
            onTriggered: rootWindow.openShowCommandDialog(
                page.dryRunResult.transparent_command,
                i18n("Dry-Run Preview")
            )
        }
    ]

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        // --- DRY-RUN MODE (M2-S04) ---
        ColumnLayout {
            visible: page.mode === "dry_run"
            Layout.fillWidth: true
            spacing: Kirigami.Units.mediumSpacing

            ColumnLayout {
                visible: page.dryRunState === "running"
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Controls.Label {
                    text: i18n("Calculating files to back up…")
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                Controls.ProgressBar {
                    indeterminate: true
                    Layout.fillWidth: true
                }

                Controls.Button {
                    id: cancelDryRunButton
                    objectName: "cancelDryRunButton"
                    icon.name: "process-stop"
                    text: i18n("Cancel")
                    Accessible.name: i18n("Cancel backup preview")
                    onClicked: appBackend.cancelBackupDryRun(page.previewId)
                }
            }

            Kirigami.InlineMessage {
                id: dryRunErrorBanner
                objectName: "dryRunErrorBanner"
                Layout.fillWidth: true
                type: Kirigami.MessageType.Error
                visible: page.dryRunState === "error"
                text: page.dryRunError !== "" ? page.dryRunError : i18n("Preview failed.")
            }

            Kirigami.PlaceholderMessage {
                Layout.fillWidth: true
                visible: page.dryRunState === "cancelled"
                icon.name: "process-stop"
                text: i18n("Preview cancelled")
            }

            ColumnLayout {
                visible: page.dryRunState === "done" && page.dryRunResult !== null
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Information
                    visible: page.dryRunResult !== null && page.dryRunResult.total_files === 0
                    text: i18n("Nothing to back up")
                }

                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    visible: page.dryRunResult !== null

                    Controls.Label {
                        Kirigami.FormData.label: i18n("Files to back up:")
                        text: page.dryRunResult ? String(page.dryRunResult.total_files) : "0"
                    }
                    Controls.Label {
                        Kirigami.FormData.label: i18n("Total size:")
                        text: page.dryRunResult ? page.formatSize(page.dryRunResult.total_bytes) : "0 B"
                    }
                    Controls.Label {
                        Kirigami.FormData.label: i18n("Changes:")
                        text: page.dryRunResult
                            ? i18n("%1 new · %2 changed · %3 unchanged",
                                   page.dryRunResult.files_new,
                                   page.dryRunResult.files_changed,
                                   page.dryRunResult.files_unmodified)
                            : ""
                    }
                    Controls.Label {
                        Kirigami.FormData.label: i18n("Estimated data added:")
                        text: page.dryRunResult ? page.formatSize(page.dryRunResult.data_added) : "0 B"
                    }
                }

                Kirigami.Heading {
                    level: 4
                    visible: page.dryRunResult !== null && page.dryRunResult.largest_excluded_folders && page.dryRunResult.largest_excluded_folders.length > 0
                    text: i18n("Largest excluded folders")
                }

                Repeater {
                    model: page.dryRunResult && page.dryRunResult.largest_excluded_folders ? page.dryRunResult.largest_excluded_folders : []
                    delegate: RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        Controls.Label {
                            text: modelData.path
                            font: Kirigami.Theme.fixedWidthFont
                            elide: Text.ElideMiddle
                            Layout.fillWidth: true
                        }
                        Controls.Label {
                            text: page.formatSize(modelData.bytes)
                            opacity: 0.8
                        }
                    }
                }

                Kirigami.Heading {
                    level: 4
                    visible: page.dryRunResult !== null && page.dryRunResult.files && page.dryRunResult.files.length > 0
                    text: i18n("Files (%1)", page.dryRunResult && page.dryRunResult.files ? page.dryRunResult.files.length : 0)
                }

                ListView {
                    id: dryRunFilesListView
                    objectName: "dryRunFilesListView"
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.min(Kirigami.Units.gridUnit * 16, contentHeight + Kirigami.Units.smallSpacing)
                    clip: true
                    keyNavigationEnabled: true
                    activeFocusOnTab: true
                    visible: count > 0
                    model: page.dryRunResult && page.dryRunResult.files ? page.dryRunResult.files : []
                    delegate: Controls.Label {
                        required property string modelData
                        width: ListView.view.width
                        text: modelData
                        font: Kirigami.Theme.fixedWidthFont
                        elide: Text.ElideMiddle
                    }
                }
            }
        }

        // --- RETENTION MODE (M2-S06) ---
        ColumnLayout {
            visible: page.mode === "retention"
            Layout.fillWidth: true
            spacing: Kirigami.Units.mediumSpacing

            Kirigami.InlineMessage {
                id: retentionPreviewSummaryBanner
                objectName: "retentionPreviewSummaryBanner"
                Layout.fillWidth: true
                visible: page.retentionPreviewData !== null
                type: page.retentionPreviewData && page.retentionPreviewData.remove_count > 0
                    ? Kirigami.MessageType.Warning
                    : Kirigami.MessageType.Information
                text: {
                    if (!page.retentionPreviewData) return "";
                    if (page.retentionPreviewData.remove_count === 0) {
                        return i18n("No snapshots would be removed (%1 kept).", page.retentionPreviewData.keep_count);
                    }
                    return i18n("%1 snapshots kept · %2 snapshots would be removed.",
                                page.retentionPreviewData.keep_count,
                                page.retentionPreviewData.remove_count);
                }
            }

            Kirigami.PlaceholderMessage {
                Layout.fillWidth: true
                visible: page.retentionPreviewData !== null && (!page.retentionPreviewData.snapshots || page.retentionPreviewData.snapshots.length === 0)
                icon.name: "view-list-details"
                text: i18n("No snapshots in this backup yet")
            }

            Repeater {
                model: page.retentionPreviewData && page.retentionPreviewData.snapshots ? page.retentionPreviewData.snapshots : []

                delegate: Controls.ItemDelegate {
                    id: snapRow
                    required property var modelData
                    Layout.fillWidth: true
                    activeFocusOnTab: true
                    Accessible.name: i18n("Snapshot %1: %2", modelData.short_id, modelData.action === "remove" ? i18n("Remove") : i18n("Keep"))

                    Rectangle {
                        anchors.fill: parent
                        z: 10
                        color: "transparent"
                        radius: Kirigami.Units.cornerRadius
                        border.width: (snapRow.activeFocus || snapRow.visualFocus) ? 2 : 0
                        border.color: Kirigami.Theme.highlightColor
                        visible: border.width > 0
                    }

                    contentItem: RowLayout {
                        spacing: Kirigami.Units.smallSpacing

                        Kirigami.Icon {
                            source: modelData.action === "remove" ? "edit-delete" : "emblem-ok-symbolic"
                            color: modelData.action === "remove" ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.positiveTextColor
                            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            Controls.Label {
                                text: modelData.short_id + " · " + modelData.time
                                font: Kirigami.Theme.fixedWidthFont
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }

                            Controls.Label {
                                text: (modelData.action === "remove" ? i18n("Remove") : i18n("Keep"))
                                    + (modelData.reasons && modelData.reasons.length > 0 ? " — " + modelData.reasons.join(", ") : "")
                                opacity: 0.85
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }
                        }
                    }
                }
            }
        }
    }
}
