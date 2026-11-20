// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    objectName: "quitConfirmDialog"
    title: i18n("Quit Kestic?")
    y: parent ? Math.round((parent.height - height) / 2) : 0
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing

    required property var appBackend

    customFooterActions: [
        Kirigami.Action {
            id: confirmQuitAction
            objectName: "confirmQuitAction"
            icon.name: "application-exit"
            text: i18n("Quit Kestic")
            onTriggered: {
                dialog.appBackend.confirmQuit();
                dialog.close();
                Qt.quit();
            }
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 24
        Controls.Label {
            text: dialog.appBackend.quit_has_running_run
                ? i18n("Scheduled backups will not run again until Kestic is started. The running backup '%1' will be cancelled.", dialog.appBackend.quit_running_backup_name)
                : i18n("Scheduled backups will not run again until Kestic is started.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
