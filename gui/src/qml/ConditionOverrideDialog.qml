// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    objectName: "conditionOverrideDialog"
    title: i18n("Run Conditions Not Met")
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing

    required property var appBackend

    customFooterActions: [
        Kirigami.Action {
            id: confirmConditionOverrideAction
            objectName: "confirmConditionOverrideAction"
            icon.name: "media-playback-start"
            text: i18n("Run Anyway")
            onTriggered: {
                dialog.appBackend.startBackupOverride(dialog.appBackend.condition_override_backup_id);
                dialog.close();
            }
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 24
        spacing: Kirigami.Units.smallSpacing
        Controls.Label {
            text: dialog.appBackend.condition_override_backup_name !== ""
                ? i18n("Backup: %1", dialog.appBackend.condition_override_backup_name)
                : ""
            font.bold: true
            visible: text !== ""
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
        Controls.Label {
            text: dialog.appBackend.condition_override_message !== ""
                ? dialog.appBackend.condition_override_message
                : i18n("One or more run conditions are currently not met. Start this backup anyway?")
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
