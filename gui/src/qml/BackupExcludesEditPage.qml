// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "backupExcludesEditPage"
    title: i18n("Exclude rules")

    required property var rootWindow
    required property var appBackend
    required property string backupId

    property var initialData: rootWindow.findBackupData(backupId)
    property string excludesText: initialData ? initialData.excludesText : ""
    property bool excludeCaches: initialData ? initialData.excludeCaches : false
    readonly property bool dirty: initialData !== null && (
        excludesText !== initialData.excludesText || excludeCaches !== initialData.excludeCaches
    )

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
        if (!initialData) return false;
        if (appBackend.updateBackup(
                backupId,
                initialData.name,
                initialData.sourcePath,
                initialData.destinationId,
                excludesText,
                excludeCaches
            )) {
            initialData = rootWindow.findBackupData(backupId);
            rootWindow.showPassiveNotification(i18n("Exclude rules saved"));
            return true;
        }
        return false;
    }

    actions: [
        Kirigami.Action {
            id: saveExcludesAction
            objectName: "saveExcludesAction"
            icon.name: "document-save"
            text: i18n("Save")
            enabled: page.dirty && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.saveChanges()
        }
    ]

    Kirigami.PromptDialog {
        id: unsavedDialog
        objectName: "unsavedExcludesDialog"
        title: i18n("Save changes?")
        subtitle: i18n("You have unsaved changes to exclude rules.")
        standardButtons: Kirigami.Dialog.Save | Kirigami.Dialog.Discard | Kirigami.Dialog.Cancel
        onAccepted: {
            if (page.saveChanges()) {
                rootWindow.pageStack.pop();
            }
        }
        onDiscarded: {
            appBackend.has_unsaved_changes = false;
            if (page.initialData) {
                page.excludesText = page.initialData.excludesText;
                page.excludeCaches = page.initialData.excludeCaches;
            }
            rootWindow.pageStack.pop();
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        Controls.Label {
            text: i18n("Rules in restic syntax (one per line: globs, **, !exceptions, $VAR; no regex):")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Controls.ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 12

            Controls.TextArea {
                id: excludesTextArea
                objectName: "excludesTextArea"
                font: Kirigami.Theme.fixedWidthFont
                text: page.excludesText
                onTextChanged: page.excludesText = text
                placeholderText: "*.tmp\n.cache\n!$HOME/.cache/keep"
                Accessible.name: i18n("Exclude rules in restic syntax")

                Keys.onTabPressed: function(event) {
                    var next = nextItemInFocusChain(true);
                    if (next) next.forceActiveFocus(Qt.TabFocusReason);
                    event.accepted = true;
                }
                Keys.onBacktabPressed: function(event) {
                    var prev = nextItemInFocusChain(false);
                    if (prev) prev.forceActiveFocus(Qt.BacktabFocusReason);
                    event.accepted = true;
                }

                Rectangle {
                    anchors.fill: parent
                    z: 10
                    color: "transparent"
                    radius: Kirigami.Units.cornerRadius
                    border.width: excludesTextArea.activeFocus ? 2 : 0
                    border.color: Kirigami.Theme.highlightColor
                    visible: border.width > 0
                }
            }
        }

        Controls.CheckBox {
            id: excludeCachesCheckbox
            objectName: "excludeCachesCheckbox"
            text: i18n("Exclude cache folders (with CACHEDIR.TAG)")
            checked: page.excludeCaches
            onToggled: page.excludeCaches = checked
            Accessible.name: i18n("Exclude cache folders with CACHEDIR.TAG")
        }
    }
}
