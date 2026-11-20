// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Wizard step: password of a new repository.
ColumnLayout {
    id: form

    required property var draft
    // Prefix of the object names (`<prefix>PasswordField`, `<prefix>PasswordConfirmField`).
    required property string namePrefix
    // Shows the error of the last save attempt above the fields.
    property bool showErrors: false

    spacing: Kirigami.Units.mediumSpacing

    Kirigami.Heading {
        level: 2
        text: i18n("Backup Encryption Password")
    }

    Controls.Label {
        text: i18n("Your backups are encrypted with this password. Kestic stores it securely in your password store.")
        wrapMode: Text.WordWrap
        Layout.fillWidth: true
    }

    Kirigami.InlineMessage {
        Layout.fillWidth: true
        type: Kirigami.MessageType.Error
        visible: form.showErrors && form.draft.testFeedback !== "" && !form.draft.testFeedbackOk
        text: form.draft.testFeedback
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true

        Controls.TextField {
            objectName: form.namePrefix + "PasswordField"
            Kirigami.FormData.label: i18n("Backup password*:")
            echoMode: TextInput.Password
            text: form.draft.repoPassword
            onTextChanged: form.draft.repoPassword = text
            Accessible.name: i18n("Backup password")
        }

        Controls.TextField {
            objectName: form.namePrefix + "PasswordConfirmField"
            visible: form.draft.setupMode === "init"
            Kirigami.FormData.label: i18n("Repeat password*:")
            echoMode: TextInput.Password
            text: form.draft.repoPasswordConfirm
            onTextChanged: form.draft.repoPasswordConfirm = text
            Accessible.name: i18n("Repeat backup password")
        }

        Controls.Label {
            visible: form.draft.setupMode === "init" && form.draft.repoPasswordConfirm.length > 0 && form.draft.repoPassword !== form.draft.repoPasswordConfirm
            text: i18n("Passwords do not match.")
            color: Kirigami.Theme.negativeTextColor
        }
    }
}
