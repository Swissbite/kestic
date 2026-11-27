// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.Dialog {
    id: dialog
    objectName: "removeDestinationDialog"
    title: Texts.labelArgs("remove_from_kestic_question", [dialog.destName])
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing

    required property var rootWindow
    required property var appBackend
    property string destId: ""
    property string destName: ""

    customFooterActions: [
        Kirigami.Action {
            icon.name: "list-remove"
            text: i18n("Remove Destination")
            onTriggered: {
                dialog.rootWindow.afterDaemonCall(dialog.appBackend.removeDestination(dialog.destId), function(ok) {
                    if (ok) {
                        dialog.close();
                        dialog.rootWindow.showPassiveNotification(i18n("Destination removed"));
                        dialog.rootWindow.switchTab(2);
                    }
                });
            }
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 22
        Controls.Label {
            text: i18n("Snapshots on this destination stay as they are.")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
