// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

EditorPage {
    id: page
    objectName: "backupSourceEditPage"
    title: Texts.label("source")

    required property string backupId

    property var initialData: null
    property string sourcePath: ""
    property string detectedKindLabel: Texts.label("folder")
    property var nestedWarnings: []
    property bool hydrating: false

    dirty: initialData !== null && sourcePath.trim() !== initialData.sourcePath
    readonly property bool valid: sourcePath.trim().length > 0

    function refreshSourceMetadata() {
        var trimmed = sourcePath.trim();
        if (trimmed === "") {
            detectedKindLabel = Texts.label("folder");
            nestedWarnings = [];
            return;
        }
        var det = rootWindow.detectSource(trimmed);
        if (det) {
            if (det.kind === "drive") {
                detectedKindLabel = det.drive_label && det.drive_label !== ""
                    ? Texts.labelArgs("drive_with_label", [det.drive_label])
                    : Texts.label("drive");
            } else {
                detectedKindLabel = Texts.label("folder");
            }
        }
        if (initialData) {
            nestedWarnings = rootWindow.nestedSourceWarnings(backupId, trimmed, initialData.destinationId);
        }
    }

    Component.onCompleted: {
        hydrating = true;
        initialData = rootWindow.findBackupData(backupId);
        if (initialData) {
            sourcePath = initialData.sourcePath;
            detectedKindLabel = initialData.sourceKindLabel;
        }
        hydrating = false;
        refreshSourceMetadata();
    }

    function commitSave() {
        if (!initialData || !valid) return false;
        if (appBackend.updateBackupSource(backupId, sourcePath.trim())) {
            initialData = rootWindow.findBackupData(backupId);
            refreshSourceMetadata();
            rootWindow.showPassiveNotification(i18n("Source saved"));
            return true;
        }
        return false;
    }

    function saveForGuard() {
        return commitSave();
    }

    actions: [
        Kirigami.Action {
            id: saveSourceAction
            objectName: "saveSourceAction"
            icon.name: "document-save"
            text: Texts.label("save")
            enabled: page.dirty && page.valid && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: confirmChangeDialog.open()
        },
        Kirigami.Action {
            id: closeSourceAction
            objectName: "closeSourceAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: page.requestClose()
        }
    ]

    FolderDialog {
        id: folderDialog
        title: Texts.label("choose_source_folder_title")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                page.sourcePath = local;
                page.refreshSourceMetadata();
            }
        }
    }

    Kirigami.PromptDialog {
        id: confirmChangeDialog
        objectName: "confirmChangeSourceDialog"
        title: i18n("Change Source?")
        subtitle: i18n("Your snapshots stay. The next backup reads the new folder completely. Please check exclude rules that use full paths.")
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                id: confirmSourceChangeAction
                objectName: "confirmSourceChangeAction"
                icon.name: "document-save"
                text: i18n("Change Source")
                onTriggered: {
                    confirmChangeDialog.close();
                    page.commitSave();
                }
            }
        ]
    }

    unsavedObjectName: "unsavedSourceDialog"
    unsavedSubtitle: i18n("You have unsaved changes to the source folder.")
    extraDialogs: [confirmChangeDialog]

    onUnsavedSaveRequested: confirmChangeDialog.open()
    onUnsavedDiscardRequested: {
        if (page.initialData) {
            page.sourcePath = page.initialData.sourcePath;
            page.refreshSourceMetadata();
        }
        rootWindow.popSubPage();
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        Kirigami.InlineMessage {
            id: driveUnmountedInfoBanner
            objectName: "driveUnmountedInfoBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: page.initialData !== null && !page.dirty && page.initialData.sourceKind === "drive" && !page.initialData.available
            text: page.initialData && page.initialData.availabilityReason !== ""
                ? page.initialData.availabilityReason
                : i18n("The drive is not mounted; backups wait until it is mounted.")
        }

        Kirigami.InlineMessage {
            id: sourceNestedWarningBanner
            objectName: "sourceNestedWarningBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: page.nestedWarnings.length > 0
            text: page.nestedWarnings.length > 0
                ? Texts.labelArgs("source_overlap", [page.nestedWarnings[0].other_backup_name,
                       page.nestedWarnings[0].other_source_path])
                : ""
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.Label {
                visible: !page.valid
                text: i18n("Source path must not be empty.")
                color: Kirigami.Theme.negativeTextColor
                Kirigami.FormData.label: ""
            }

            RowLayout {
                Kirigami.FormData.label: i18n("Folder:")
                Layout.fillWidth: true

                Controls.TextField {
                    id: sourcePathField
                    objectName: "sourcePathField"
                    text: page.sourcePath
                    onTextChanged: {
                        if (!page.hydrating) {
                            page.sourcePath = text;
                            page.refreshSourceMetadata();
                        }
                    }
                    onAccepted: if (page.dirty && page.valid && appBackend.blocking_kind !== "daemon_unreachable") confirmChangeDialog.open()
                    placeholderText: "/home/user"
                    Accessible.name: Texts.label("source_folder_path")
                    Layout.fillWidth: true
                }

                Controls.Button {
                    text: Texts.label("choose")
                    Accessible.name: Texts.label("choose_source_folder")
                    onClicked: folderDialog.open()
                }
            }

            Controls.Label {
                Kirigami.FormData.label: i18n("Kind:")
                text: page.detectedKindLabel
            }
        }
    }
}
