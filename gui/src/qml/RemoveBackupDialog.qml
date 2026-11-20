// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    objectName: "removeBackupDialog"
    title: i18n("Remove '%1' from Kestic?", dialog.backupName)
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing

    required property var rootWindow
    required property var appBackend
    property string backupId: ""
    property string backupName: ""

    customFooterActions: [
        Kirigami.Action {
            id: confirmRemoveBackupAction
            objectName: "confirmRemoveBackupAction"
            icon.name: "list-remove"
            text: i18n("Remove")
            onTriggered: {
                if (dialog.appBackend.removeBackup(dialog.backupId)) {
                    dialog.close();
                    dialog.rootWindow.showPassiveNotification(i18n("Backup removed from Kestic"));
                    dialog.rootWindow.switchTab(1);
                }
            }
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 22
        Controls.Label {
            text: i18n("Snapshots stay on the destination.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
