// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

WizardPage {
    id: page
    objectName: "newBackupWizard"
    title: wizardTitle
    hasInput: hasAnyDestInput
    canGoBack: stepIndex > 0
    discardTitle: i18n("Discard backup setup?")
    onBackClicked: stepIndex -= 1

    required property var appBackend
    required property var destinationsModel

    property string wizardTitle: Texts.label("new_backup")
    property bool integratedDestFlow: false

    // Linear stage progression:
    // - Existing destinations: source -> choose_dest -> summary (3 steps)
    // - No destinations yet (new repo): source -> dest_type -> dest_details -> dest_password -> summary (5 steps)
    // - No destinations yet (connect existing repo): source -> dest_type -> dest_details -> summary (4 steps)
    readonly property var stages: {
        if (!integratedDestFlow) {
            return ["source", "choose_dest", "summary"];
        }
        if (destDraft.setupMode === "connect") {
            return ["source", "dest_type", "dest_details", "summary"];
        }
        return ["source", "dest_type", "dest_details", "dest_password", "summary"];
    }
    property int stepIndex: 0
    readonly property int displayStep: stepIndex + 1
    readonly property int totalSteps: stages.length
    readonly property string currentStage: stages[Math.min(stepIndex, stages.length - 1)]

    // Backup state
    property string sourcePath: appBackend.homeDir()
    property string selectedDestId: ""
    property string selectedDestName: ""
    property string backupName: ""
    property string excludesText: ""
    property bool nameManuallyEdited: false
    property string saveErrorText: ""
    property string detectedSourceKind: "folder"
    property string detectedDriveLabel: ""
    property var nestedWarnings: []

    function refreshSourceMetadata() {
        var trimmed = sourcePath.trim();
        if (trimmed === "") {
            detectedSourceKind = "folder";
            detectedDriveLabel = "";
            nestedWarnings = [];
            return;
        }
        var det = rootWindow.detectSource(trimmed);
        if (det) {
            detectedSourceKind = det.kind || "folder";
            detectedDriveLabel = det.drive_label || "";
        }
        if (!integratedDestFlow && selectedDestId !== "") {
            nestedWarnings = rootWindow.nestedSourceWarnings("", trimmed, selectedDestId);
        } else {
            nestedWarnings = [];
        }
    }

    // Integrated destination creation state (used when integratedDestFlow === true)
    property string createdDestId: ""

    DestinationDraft {
        id: destDraft
        appBackend: page.appBackend
    }

    readonly property bool destDetailsValid: destDraft.detailsValid && (destDraft.setupMode !== "connect" || destDraft.repoPassword.length > 0)
    readonly property bool sourceValid: sourcePath.trim().length > 0
    readonly property bool destValid: selectedDestId.length > 0
    readonly property bool hasAnyDestInput: integratedDestFlow && destDraft.hasAnyInput

    readonly property bool summaryValid: {
        if (backupName.trim().length === 0 || !sourceValid) return false;
        if (integratedDestFlow) {
            return destDetailsValid && destDraft.connectionAccepted && destDraft.passwordsMatch;
        }
        return destValid;
    }

    readonly property bool canAdvance: {
        switch (currentStage) {
        case "source":
            return sourceValid;
        case "choose_dest":
            return destValid;
        case "dest_type":
            return typeChooser.choiceValid;
        case "dest_details":
            return destDetailsValid && destDraft.connectionAccepted;
        case "dest_password":
            return destDraft.passwordsMatch;
        default:
            return summaryValid && appBackend.blocking_kind === "";
        }
    }

    function deriveDefaultName(path) {
        var trimmed = path.replace(/\/+$/, "");
        var idx = trimmed.lastIndexOf("/");
        var base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
        return base.length > 0 ? base : i18n("Personal folder");
    }

    function preselectFirstDestinationIfNeeded() {
        var firstId = destinationsModel.idAt(0);
        if (firstId !== "" && selectedDestId === "") {
            selectedDestId = firstId;
            selectedDestName = destinationsModel.rowMap(firstId).name || "";
        }
    }

    function submitBackup(startImmediately) {
        if (!summaryValid) return;
        saveErrorText = "";
        var targetDestId = selectedDestId;
        if (integratedDestFlow) {
            if (createdDestId === "") {
                var newDestId = appBackend.addDestination(
                    destDraft.fieldsJson(),
                    destDraft.setupMode,
                    destDraft.repoPassword,
                    destDraft.backendSecretsJson()
                );
                if (newDestId === "") {
                    saveErrorText = appBackend.last_action_error;
                    return;
                }
                createdDestId = newDestId;
            }
            targetDestId = createdDestId;
            selectedDestId = createdDestId;
            selectedDestName = destDraft.name.trim();
        }

        var createdId = appBackend.addBackup(
            backupName.trim(),
            sourcePath.trim(),
            targetDestId,
            excludesText
        );
        if (createdId !== "") {
            rootWindow.showPassiveNotification(i18n("Backup saved"));
            rootWindow.closeWizardLayer();
            if (startImmediately) {
                appBackend.startBackup(createdId);
            }
        } else {
            saveErrorText = appBackend.last_action_error;
        }
    }

    Component.onCompleted: {
        backupName = deriveDefaultName(sourcePath);
        preselectFirstDestinationIfNeeded();
    }

    Connections {
        target: appBackend
        function onModelsUpdated() {
            page.preselectFirstDestinationIfNeeded();
        }
    }

    FolderDialog {
        id: sourcePickDialog
        title: Texts.label("choose_source_folder_title")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                page.sourcePath = local;
                if (!page.nameManuallyEdited) {
                    page.backupName = page.deriveDefaultName(local);
                }
            }
        }
    }

    ColumnLayout {
        width: parent.width

        ColumnLayout {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 44
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.largeSpacing

            WizardProgress {
                step: page.displayStep
                totalSteps: page.totalSteps
                labelObjectName: "wizBackupStepLabel"
            }

            // Stage: Source
            ColumnLayout {
                visible: page.currentStage === "source"
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Heading {
                    level: 2
                    text: i18n("What should be backed up?")
                }

                Kirigami.FormLayout {
                    Layout.fillWidth: true

                    RowLayout {
                        Kirigami.FormData.label: i18n("Source folder*:")
                        Layout.fillWidth: true

                        Controls.TextField {
                            objectName: "wizBackupSourceField"
                            text: page.sourcePath
                            onTextChanged: {
                                page.sourcePath = text;
                                if (!page.nameManuallyEdited) {
                                    page.backupName = page.deriveDefaultName(text);
                                }
                                page.refreshSourceMetadata();
                            }
                            Accessible.name: Texts.label("source_folder_path")
                            Layout.fillWidth: true
                        }

                        Controls.Button {
                            text: i18n("Choose Other Folder…")
                            icon.name: "folder-open"
                            Accessible.name: Texts.label("choose_source_folder")
                            onClicked: sourcePickDialog.open()
                        }
                    }
                }

                Kirigami.InlineMessage {
                    id: wizDriveDetectedBanner
                    objectName: "wizDriveDetectedBanner"
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Information
                    visible: page.detectedSourceKind === "drive"
                    text: page.detectedDriveLabel !== ""
                        ? i18n("External drive '%1' detected — backed up only when plugged in.", page.detectedDriveLabel)
                        : i18n("External drive detected — backed up only when plugged in.")
                }
            }

            // Stage: Choose existing destination (when destinations already exist)
            ColumnLayout {
                visible: page.currentStage === "choose_dest"
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Heading {
                    level: 2
                    text: i18n("Where should the backup be stored?")
                }

                ListView {
                    objectName: "wizBackupDestList"
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.max(contentHeight, Kirigami.Units.gridUnit * 8)
                    model: destinationsModel
                    spacing: Kirigami.Units.smallSpacing
                    activeFocusOnTab: visible && count > 0
                    keyNavigationEnabled: true

                    onActiveFocusChanged: {
                        if (activeFocus && count > 0 && currentIndex < 0) {
                            currentIndex = Math.max(0, destinationsModel.rowOf(page.selectedDestId));
                        }
                    }

                    delegate: Controls.ItemDelegate {
                        id: destRow
                        objectName: "wizBackupDestItem"
                        required property int index
                        required property string destinationId
                        required property string name
                        required property string kindLabel
                        required property string statusText
                        required property string statusIcon

                        width: ListView.view.width
                        highlighted: page.selectedDestId === destinationId
                        Accessible.name: name

                        onClicked: {
                            ListView.view.currentIndex = index;
                            page.selectedDestId = destinationId;
                            page.selectedDestName = name;
                            page.refreshSourceMetadata();
                        }

                        FocusRing {
                            target: destRow
                            ringColor: destRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                        }

                        contentItem: RowLayout {
                            spacing: Kirigami.Units.mediumSpacing
                            Kirigami.Icon {
                                source: destRow.statusIcon
                                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2
                                Controls.Label {
                                    text: destRow.name
                                    font.bold: true
                                    Layout.fillWidth: true
                                }
                                Controls.Label {
                                    text: destRow.kindLabel + " · " + destRow.statusText
                                    opacity: 0.75
                                    Layout.fillWidth: true
                                }
                            }
                        }
                    }
                }

                Controls.Button {
                    text: Texts.label("add_new_destination_ellipsis")
                    icon.name: "list-add"
                    enabled: appBackend.blocking_kind === ""
                    Accessible.name: Texts.label("add_new_destination")
                    onClicked: rootWindow.openAddDestinationWizard(function(newId, newName) {
                        page.selectedDestId = newId;
                        page.selectedDestName = newName;
                    })
                }
            }

            DestinationTypeChooser {
                id: typeChooser
                visible: page.currentStage === "dest_type"
                Layout.fillWidth: true
                draft: destDraft
                namePrefix: "wizBackupDest"
            }

            DestinationDetailsForm {
                visible: page.currentStage === "dest_details"
                Layout.fillWidth: true
                draft: destDraft
                namePrefix: "wizBackupDest"
            }

            DestinationPasswordForm {
                visible: page.currentStage === "dest_password"
                Layout.fillWidth: true
                draft: destDraft
                namePrefix: "wizBackupDest"
            }

            // Stage: Summary & Excludes
            ColumnLayout {
                visible: page.currentStage === "summary"
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Heading {
                    level: 2
                    text: i18n("Summary")
                }

                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Error
                    visible: page.saveErrorText !== "" || appBackend.last_action_error !== ""
                    text: page.saveErrorText !== "" ? page.saveErrorText : appBackend.last_action_error
                }

                Kirigami.InlineMessage {
                    id: wizNestedSourceWarningBanner
                    objectName: "wizNestedSourceWarningBanner"
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Warning
                    visible: page.nestedWarnings.length > 0
                    text: page.nestedWarnings.length > 0
                        ? i18n("This source overlaps with backup '%1' (%2) on the same destination. You can still create the backup, but check the exclude rules.",
                               page.nestedWarnings[0].other_backup_name,
                               page.nestedWarnings[0].other_source_path)
                        : ""
                }

                Kirigami.FormLayout {
                    Layout.fillWidth: true

                    Controls.TextField {
                        id: wizBackupNameField
                        objectName: "wizBackupNameField"
                        Kirigami.FormData.label: Texts.label("required_name_label")
                        text: page.backupName
                        onTextEdited: {
                            page.nameManuallyEdited = true;
                            page.backupName = text;
                        }
                        Accessible.name: i18n("Backup name")
                    }

                    Controls.Label {
                        Kirigami.FormData.label: i18n("Source:")
                        text: page.sourcePath
                        wrapMode: Text.WordWrap
                    }

                    Controls.Label {
                        Kirigami.FormData.label: i18n("Destination:")
                        text: page.integratedDestFlow ? destDraft.name.trim() : page.selectedDestName
                    }

                    Controls.Label {
                        Kirigami.FormData.label: i18n("Defaults:")
                        text: i18n("Daily at 12:00 (AC power & network) · Standard retention (7 daily, 4 weekly, 12 monthly)")
                        wrapMode: Text.WordWrap
                    }

                    Controls.ScrollView {
                        Kirigami.FormData.label: i18n("Exclude rules:")
                        Layout.fillWidth: true
                        Layout.preferredHeight: Kirigami.Units.gridUnit * 7

                        TabNavTextArea {
                            id: wizBackupExcludesArea
                            objectName: "wizBackupExcludesArea"
                            font: Kirigami.Theme.fixedWidthFont
                            text: page.excludesText
                            onTextChanged: page.excludesText = text
                            placeholderText: "*.tmp\n.cache"
                            Accessible.name: Texts.label("exclude_rules_syntax")

                            FocusRing {
                                target: wizBackupExcludesArea
                            }
                        }
                    }
                }
            }
        }
    }

    footerButtons: [
        Controls.Button {
            objectName: "wizBackupNextButton"
            visible: page.currentStage !== "summary"
            text: Texts.label("next")
            highlighted: true
            enabled: page.canAdvance
            Accessible.name: Texts.label("next")
            onClicked: page.stepIndex += 1
        },
        Controls.Button {
            objectName: "wizBackupSaveLaterButton"
            visible: page.currentStage === "summary"
            text: i18n("Save, Back Up Later")
            enabled: page.canAdvance
            Accessible.name: i18n("Save, Back Up Later")
            onClicked: page.submitBackup(false)
        },
        Controls.Button {
            objectName: "wizBackupStartNowButton"
            visible: page.currentStage === "summary"
            icon.name: "media-playback-start"
            text: Texts.label("back_up_now")
            highlighted: true
            enabled: page.canAdvance
            Accessible.name: i18n("Save and Back Up Now")
            onClicked: page.submitBackup(true)
        }
    ]
}
