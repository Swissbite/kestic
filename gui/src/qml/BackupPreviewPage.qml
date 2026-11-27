// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "backupPreviewPage"
    title: mode === "retention" ? i18n("Retention Preview") : i18n("Backup Preview")

    required property var rootWindow
    required property var appBackend
    required property string backupId
    required property string mode // "dry_run" | "retention"
    property string excludesJson: ""
    property string retentionJson: ""

    property string previewId: ""
    property string dryRunState: "running"
    property var dryRunResult: null
    property string dryRunError: ""

    property string retentionPreviewId: ""
    property string retentionPreviewState: "running"
    property var retentionPreviewData: null
    property string retentionPreviewError: ""

    // The requests the page waits for (0: none); answers arrive through `daemonCallDone`.
    property int dryRunStartRequest: 0
    property int dryRunPollRequest: 0
    property int retentionStartRequest: 0
    property int retentionPollRequest: 0

    function startDryRunIfNeeded() {
        if (mode !== "dry_run") return;
        retentionPollTimer.stop();
        dryRunState = "running";
        dryRunResult = null;
        dryRunError = "";
        previewId = "";
        dryRunStartRequest = appBackend.startBackupDryRun(backupId, excludesJson);
        if (dryRunStartRequest === 0) {
            dryRunStarted(false, "");
        }
    }

    function dryRunStarted(ok, id) {
        if (!ok || id === "") {
            dryRunState = "error";
            dryRunError = appBackend.dry_run_error || appBackend.last_action_error || i18n("Preview could not be started.");
            return;
        }
        previewId = id;
        pollTimer.start();
    }

    function startRetentionPreviewIfNeeded() {
        if (mode !== "retention") return;
        pollTimer.stop();
        retentionPreviewState = "running";
        retentionPreviewData = null;
        retentionPreviewError = "";
        retentionPreviewId = "";
        retentionStartRequest = appBackend.startRetentionPreview(backupId, retentionJson);
        if (retentionStartRequest === 0) {
            retentionPreviewStarted(false, "");
        }
    }

    function retentionPreviewStarted(ok, id) {
        if (!ok || id === "") {
            retentionPreviewState = "error";
            retentionPreviewError = appBackend.retention_preview_error || appBackend.last_action_error || i18n("Preview could not be started.");
            return;
        }
        retentionPreviewId = id;
        retentionPollTimer.start();
    }

    // Cancels the dry run; one still starting is cancelled once the daemon has started it.
    function cancelDryRun() {
        if (previewId !== "") {
            appBackend.cancelBackupDryRun(previewId);
            return;
        }
        appBackend.forgetDaemonCall(dryRunStartRequest);
        dryRunStartRequest = 0;
        dryRunState = "cancelled";
    }

    function cancelRetentionPreview() {
        if (retentionPreviewId !== "") {
            appBackend.cancelRetentionPreview(retentionPreviewId);
            return;
        }
        appBackend.forgetDaemonCall(retentionStartRequest);
        retentionStartRequest = 0;
        retentionPreviewState = "cancelled";
    }

    Connections {
        target: appBackend
        function onDaemonCallDone(request, ok, result) {
            if (request === page.dryRunStartRequest) {
                page.dryRunStartRequest = 0;
                page.dryRunStarted(ok, result);
            } else if (request === page.retentionStartRequest) {
                page.retentionStartRequest = 0;
                page.retentionPreviewStarted(ok, result);
            } else if (request === page.dryRunPollRequest) {
                page.dryRunPollRequest = 0;
                page.handlePollAnswer(pollTimer, ok ? result : "",
                                      appBackend.dry_run_error, page.applyDryRunAnswer);
            } else if (request === page.retentionPollRequest) {
                page.retentionPollRequest = 0;
                page.handlePollAnswer(retentionPollTimer, ok ? result : "",
                                      appBackend.retention_preview_error, page.applyRetentionAnswer);
            }
        }
    }

    Component.onCompleted: {
        startDryRunIfNeeded();
        startRetentionPreviewIfNeeded();
    }

    Component.onDestruction: {
        pollTimer.stop();
        retentionPollTimer.stop();
        // A preview still starting is cancelled once the daemon has started it.
        appBackend.forgetDaemonCall(dryRunStartRequest);
        appBackend.forgetDaemonCall(retentionStartRequest);
        if (mode === "dry_run" && dryRunState === "running" && previewId !== "") {
            appBackend.cancelBackupDryRun(previewId);
        }
        if (mode === "retention" && retentionPreviewState === "running" && retentionPreviewId !== "") {
            appBackend.cancelRetentionPreview(retentionPreviewId);
        }
    }

    // Handles one answer `raw` of a poll: stops `timer` once the preview is no longer running
    // and passes the new state, result and error text (null: unchanged) to `apply`.
    function handlePollAnswer(timer, raw, backendError, apply) {
        if (raw === "") {
            timer.stop();
            apply("error", null, backendError || i18n("Preview failed."));
            return;
        }
        var dto = rootWindow.parseJsonOr(raw, null);
        if (!dto) {
            timer.stop();
            apply("error", null, i18n("The preview result could not be read."));
            return;
        }
        if (dto.state !== "running") {
            timer.stop();
        }
        apply(dto.state || "error", dto.result || null,
              dto.state === "error" ? (backendError || i18n("Preview failed.")) : null);
    }

    function applyDryRunAnswer(state, result, error) {
        dryRunState = state;
        if (result) dryRunResult = result;
        if (error !== null) dryRunError = error;
    }

    function applyRetentionAnswer(state, result, error) {
        retentionPreviewState = state;
        if (result) retentionPreviewData = result;
        if (error !== null) retentionPreviewError = error;
    }

    // Asks for the status while no answer is on its way.
    Timer {
        id: pollTimer
        interval: 150
        repeat: true
        onTriggered: {
            if (page.previewId === "") {
                stop();
                return;
            }
            if (page.dryRunPollRequest === 0) {
                page.dryRunPollRequest = appBackend.pollBackupDryRun(page.previewId);
            }
        }
    }

    Timer {
        id: retentionPollTimer
        interval: 50
        repeat: true
        onTriggered: {
            if (page.retentionPreviewId === "") {
                stop();
                return;
            }
            if (page.retentionPollRequest === 0) {
                page.retentionPollRequest = appBackend.pollRetentionPreview(page.retentionPreviewId);
            }
        }
    }

    actions: [
        Kirigami.Action {
            id: showDryRunCommandAction
            objectName: "showDryRunCommandAction"
            icon.name: "utilities-terminal"
            text: Texts.label("show_command")
            visible: page.mode === "dry_run" && page.dryRunResult !== null && (page.dryRunResult.transparent_command || "") !== ""
            onTriggered: rootWindow.openShowCommandDialog(
                page.dryRunResult.transparent_command,
                i18n("Test Run Preview")
            )
        },
        Kirigami.Action {
            id: closePreviewAction
            objectName: "closePreviewAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: rootWindow.popSubPage()
        }
    ]

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        // --- DRY-RUN MODE ---
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
                    // Fusion leaves the bar 0 px high and draws nothing; 6 px is the bar of Breeze.
                    Layout.minimumHeight: topInset + bottomInset + Math.round(Kirigami.Units.smallSpacing * 1.5)
                    indeterminate: true
                    Layout.fillWidth: true
                }

                Controls.Button {
                    id: cancelDryRunButton
                    objectName: "cancelDryRunButton"
                    icon.name: "process-stop"
                    text: Texts.label("cancel")
                    Accessible.name: i18n("Cancel backup preview")
                    onClicked: page.cancelDryRun()
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
                        objectName: "dryRunTotalSize"
                        Kirigami.FormData.label: i18n("Total size:")
                        text: page.appBackend.formatBytes(page.dryRunResult ? page.dryRunResult.total_bytes : 0)
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
                        text: page.appBackend.formatBytes(page.dryRunResult ? page.dryRunResult.data_added : 0)
                    }
                }

                Kirigami.Heading {
                    level: 4
                    visible: page.dryRunResult !== null && page.dryRunResult.largest_excluded_folders && page.dryRunResult.largest_excluded_folders.length > 0
                    text: page.dryRunResult && page.dryRunResult.largest_excluded_folders && page.dryRunResult.largest_excluded_folders.length === 1
                        ? i18n("Excluded folder")
                        : i18n("Largest excluded folders")
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
                            text: page.appBackend.formatBytes(modelData.bytes || 0)
                            opacity: 0.8
                        }
                    }
                }

                Kirigami.Heading {
                    level: 4
                    visible: page.dryRunResult !== null && page.dryRunResult.files && page.dryRunResult.files.length > 0
                    text: i18n("Files (%1)", page.dryRunResult && page.dryRunResult.files ? page.dryRunResult.files.length : 0)
                }

                KeyboardList {
                    id: dryRunFilesListView
                    objectName: "dryRunFilesListView"
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.min(Kirigami.Units.gridUnit * 16, contentHeight + Kirigami.Units.smallSpacing)
                    clip: true
                    keyNavigationEnabled: true
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

        // --- RETENTION MODE ---
        ColumnLayout {
            visible: page.mode === "retention"
            Layout.fillWidth: true
            spacing: Kirigami.Units.mediumSpacing

            ColumnLayout {
                visible: page.retentionPreviewState === "running"
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Controls.Label {
                    text: i18n("Calculating retention preview…")
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                Controls.ProgressBar {
                    // Fusion leaves the bar 0 px high and draws nothing; 6 px is the bar of Breeze.
                    Layout.minimumHeight: topInset + bottomInset + Math.round(Kirigami.Units.smallSpacing * 1.5)
                    indeterminate: true
                    Layout.fillWidth: true
                }

                Controls.Button {
                    id: cancelRetentionPreviewButton
                    objectName: "cancelRetentionPreviewButton"
                    icon.name: "process-stop"
                    text: Texts.label("cancel")
                    Accessible.name: i18n("Cancel retention preview")
                    onClicked: page.cancelRetentionPreview()
                }
            }

            Kirigami.InlineMessage {
                id: retentionPreviewErrorBanner
                objectName: "retentionPreviewErrorBanner"
                Layout.fillWidth: true
                type: Kirigami.MessageType.Error
                visible: page.retentionPreviewState === "error"
                text: page.retentionPreviewError !== "" ? page.retentionPreviewError : i18n("Preview failed.")
            }

            Kirigami.PlaceholderMessage {
                Layout.fillWidth: true
                visible: page.retentionPreviewState === "cancelled"
                icon.name: "process-stop"
                text: i18n("Preview cancelled")
            }

            Kirigami.InlineMessage {
                id: retentionPreviewSummaryBanner
                objectName: "retentionPreviewSummaryBanner"
                Layout.fillWidth: true
                visible: page.retentionPreviewState === "done" && page.retentionPreviewData !== null
                type: page.retentionPreviewData && page.retentionPreviewData.remove_count > 0
                    ? Kirigami.MessageType.Warning
                    : Kirigami.MessageType.Information
                text: page.retentionPreviewData ? page.retentionPreviewData.summary : ""
            }

            Kirigami.PlaceholderMessage {
                Layout.fillWidth: true
                visible: page.retentionPreviewState === "done"
                    && page.retentionPreviewData !== null
                    && (!page.retentionPreviewData.snapshots || page.retentionPreviewData.snapshots.length === 0)
                icon.name: "view-list-details"
                text: i18n("No snapshots in this backup yet")
            }

            KeyboardList {
                objectName: "retentionPreviewListView"
                Layout.fillWidth: true
                implicitHeight: contentHeight
                interactive: false
                model: page.retentionPreviewState === "done"
                    && page.retentionPreviewData
                    && page.retentionPreviewData.snapshots
                    ? page.retentionPreviewData.snapshots
                    : []

                delegate: Controls.ItemDelegate {
                    id: snapRow
                    required property var modelData
                    width: ListView.view.width
                    focusPolicy: Qt.ClickFocus
                    Accessible.name: i18n("Snapshot %1: %2", modelData.short_id, modelData.action === "remove" ? Texts.label("remove") : i18n("Keep"))

                    FocusRing {
                        target: snapRow
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
                                objectName: "retentionPreviewSnapshotTime"
                                text: modelData.short_id + " · " + modelData.time_text
                                font: Kirigami.Theme.fixedWidthFont
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }

                            Controls.Label {
                                objectName: "retentionPreviewSnapshotResult"
                                text: (modelData.action === "remove" ? Texts.label("remove") : i18n("Keep"))
                                    + (modelData.reasons_text !== "" ? " — " + modelData.reasons_text : "")
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
