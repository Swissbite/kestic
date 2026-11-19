// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    objectName: "renameBackupDialog"
    title: i18n("Rename Backup")
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing

    required property var rootWindow
    required property var appBackend
    property string backupId: ""
    property string currentName: ""

    function setRenameText(text) {
        renameBackupField.text = text;
    }

    customFooterActions: [
        Kirigami.Action {
            text: i18n("Rename")
            icon.name: "edit-rename"
            enabled: renameBackupField.text.trim().length > 0 && renameBackupField.text.trim() !== dialog.currentName
            onTriggered: {
                if (dialog.appBackend.renameBackup(dialog.backupId, renameBackupField.text.trim())) {
                    dialog.close();
                    dialog.rootWindow.showPassiveNotification(i18n("Backup renamed"));
                }
            }
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 22
        spacing: Kirigami.Units.smallSpacing

        Controls.TextField {
            id: renameBackupField
            Accessible.name: i18n("New backup name")
            Layout.fillWidth: true
            onAccepted: {
                if (text.trim().length > 0 && text.trim() !== dialog.currentName
                        && dialog.appBackend.renameBackup(dialog.backupId, text.trim())) {
                    dialog.close();
                    dialog.rootWindow.showPassiveNotification(i18n("Backup renamed"));
                }
            }
        }
    }
}
