// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    objectName: "updatePasswordDialog"
    title: i18n("Update Saved Password")
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing

    required property var appBackend
    property string destId: ""
    property string destName: ""

    function clearPasswordField() {
        updatePasswordField.text = "";
    }

    customFooterActions: [
        Kirigami.Action {
            icon.name: "dialog-ok-apply"
            text: i18n("Verify and Save")
            enabled: updatePasswordField.text.length > 0
            onTriggered: {
                if (dialog.appBackend.updateDestinationPassword(dialog.destId, updatePasswordField.text)) {
                    dialog.close();
                }
            }
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 24
        spacing: Kirigami.Units.smallSpacing

        Controls.Label {
            text: i18n("The saved password in your password store for '%1' does not match. The destination's own password is not changed.", dialog.destName)
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Controls.TextField {
            id: updatePasswordField
            echoMode: TextInput.Password
            placeholderText: i18n("Repository password")
            Accessible.name: i18n("Repository password")
            Layout.fillWidth: true
            onAccepted: {
                if (text.length > 0 && dialog.appBackend.updateDestinationPassword(dialog.destId, text)) {
                    dialog.close();
                }
            }
        }

        Controls.Label {
            visible: dialog.appBackend.last_action_error !== ""
            text: dialog.appBackend.last_action_error
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
