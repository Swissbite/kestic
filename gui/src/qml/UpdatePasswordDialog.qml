// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

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

    // The request the dialog waits for; it closes once the daemon accepted the password.
    property int pendingRequest: 0

    function save(password) {
        if (dialog.pendingRequest !== 0) return;
        dialog.pendingRequest = dialog.appBackend.updateDestinationPassword(dialog.destId, password);
    }

    Connections {
        target: dialog.appBackend
        function onDaemonCallDone(request, ok, result) {
            if (request !== dialog.pendingRequest) return;
            dialog.pendingRequest = 0;
            if (ok) dialog.close();
        }
    }

    customFooterActions: [
        Kirigami.Action {
            icon.name: "dialog-ok-apply"
            text: i18n("Verify and Save")
            enabled: updatePasswordField.text.length > 0 && dialog.pendingRequest === 0
            onTriggered: dialog.save(updatePasswordField.text)
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 24
        spacing: Kirigami.Units.smallSpacing

        Controls.Label {
            text: i18n("The saved password for '%1' does not match. Here you only change the saved password, not the password of the destination.", dialog.destName)
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Controls.TextField {
            id: updatePasswordField
            echoMode: TextInput.Password
            placeholderText: Texts.label("repository_password")
            Accessible.name: Texts.label("repository_password")
            Layout.fillWidth: true
            onAccepted: {
                if (text.length > 0) dialog.save(text);
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
