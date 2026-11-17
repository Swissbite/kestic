// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "backupSourceEditPage"
    title: i18n("Source")

    required property var rootWindow
    required property var appBackend
    required property string backupId

    property var initialData: rootWindow.findBackupData(backupId)
    property string sourcePath: initialData ? initialData.sourcePath : ""
    readonly property bool dirty: initialData !== null && sourcePath.trim() !== initialData.sourcePath
    readonly property bool valid: sourcePath.trim().length > 0

    onDirtyChanged: appBackend.has_unsaved_changes = dirty

    onBackRequested: function(event) {
        if (dirty) {
            event.accepted = true;
            unsavedDialog.open();
        }
    }

    function handleEscape() {
        if (unsavedDialog.opened || unsavedDialog.visible) {
            unsavedDialog.close();
            return true;
        }
        if (dirty) {
            unsavedDialog.open();
            return true;
        }
        return false;
    }

    function saveChanges() {
        if (!initialData || !valid) return false;
        if (appBackend.updateBackup(
                backupId,
                initialData.name,
                sourcePath.trim(),
                initialData.destinationId,
                initialData.excludesText,
                initialData.excludeCaches
            )) {
            initialData = rootWindow.findBackupData(backupId);
            rootWindow.showPassiveNotification(i18n("Source saved"));
            return true;
        }
        return false;
    }

    actions: [
        Kirigami.Action {
            id: saveSourceAction
            objectName: "saveSourceAction"
            icon.name: "document-save"
            text: i18n("Save")
            enabled: page.dirty && page.valid && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.saveChanges()
        }
    ]

    FolderDialog {
        id: folderDialog
        title: i18n("Choose Source Folder")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                page.sourcePath = local;
            }
        }
    }

    Kirigami.PromptDialog {
        id: unsavedDialog
        objectName: "unsavedSourceDialog"
        title: i18n("Save changes?")
        subtitle: i18n("You have unsaved changes to the source folder.")
        standardButtons: Kirigami.Dialog.Save | Kirigami.Dialog.Discard | Kirigami.Dialog.Cancel
        onAccepted: {
            if (page.saveChanges()) {
                rootWindow.pageStack.pop();
            }
        }
        onDiscarded: {
            appBackend.has_unsaved_changes = false;
            if (page.initialData) {
                page.sourcePath = page.initialData.sourcePath;
            }
            rootWindow.pageStack.pop();
        }
    }

    Kirigami.FormLayout {
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
                onTextChanged: page.sourcePath = text
                onAccepted: if (page.dirty && page.valid && appBackend.blocking_kind !== "daemon_unreachable") page.saveChanges()
                placeholderText: "/home/user"
                Accessible.name: i18n("Source folder path")
                Layout.fillWidth: true
            }

            Controls.Button {
                text: i18n("Choose…")
                Accessible.name: i18n("Choose source folder")
                onClicked: folderDialog.open()
            }
        }

        Controls.Label {
            Kirigami.FormData.label: i18n("Kind:")
            text: page.initialData && page.initialData.sourceKind === "drive" ? i18n("Drive") : i18n("Folder")
        }
    }
}
