// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.Dialog {
    id: dialog
    objectName: "removeBackupDialog"
    title: Texts.labelArgs("remove_from_kestic_question", [dialog.backupName])
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing

    required property var rootWindow
    required property var appBackend
    property string backupId: ""
    property string backupName: ""
    // The removal is on its way; the dialog takes no second one.
    property bool busy: false

    customFooterActions: [
        Kirigami.Action {
            id: confirmRemoveBackupAction
            objectName: "confirmRemoveBackupAction"
            icon.name: "list-remove"
            text: Texts.label("remove")
            enabled: !dialog.busy
            onTriggered: {
                dialog.busy = true;
                dialog.rootWindow.afterDaemonCall(dialog.appBackend.removeBackup(dialog.backupId), function(ok) {
                    dialog.busy = false;
                    if (ok) {
                        dialog.close();
                        dialog.rootWindow.showPassiveNotification(i18n("Backup removed from Kestic"));
                        dialog.rootWindow.switchTab(1);
                    }
                });
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
