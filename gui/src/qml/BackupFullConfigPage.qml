// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "backupFullConfigPage"
    title: i18n("Full Configuration")

    required property var rootWindow
    required property var appBackend
    required property string backupId

    property var backupData: rootWindow.findBackupData(backupId)

    actions: [
        Kirigami.Action {
            icon.name: "utilities-terminal"
            text: i18n("Show Command")
            onTriggered: if (backupData) rootWindow.openShowCommandDialog(
                appBackend.getBackupTransparentCommand(backupId),
                backupData.name
            )
        },
        Kirigami.Action {
            icon.name: "edit-copy"
            text: i18n("Copy as Text")
            onTriggered: {
                appBackend.copyToClipboard(appBackend.buildBackupConfigTomlSnippet(backupId));
                rootWindow.showPassiveNotification(i18n("Configuration copied to clipboard"));
            }
        }
    ]

    Kirigami.FormLayout {
        visible: backupData !== null

        Controls.Label {
            Kirigami.FormData.label: i18n("Name:")
            text: backupData ? backupData.name : ""
        }
        Controls.Label {
            Kirigami.FormData.label: i18n("ID:")
            text: backupData ? backupData.backupId : ""
            font: Kirigami.Theme.fixedWidthFont
        }
        Controls.Label {
            Kirigami.FormData.label: i18n("Source:")
            text: backupData ? backupData.sourcePath : ""
        }
        Controls.Label {
            Kirigami.FormData.label: i18n("Destination:")
            text: backupData ? backupData.destinationName : ""
        }
        Controls.Label {
            Kirigami.FormData.label: i18n("Exclude rules:")
            text: backupData && backupData.excludesText !== "" ? backupData.excludesText : i18n("None")
            wrapMode: Text.WordWrap
        }
        Controls.Label {
            Kirigami.FormData.label: i18n("Schedule:")
            text: backupData ? backupData.scheduleSummary : ""
        }
        Controls.Label {
            Kirigami.FormData.label: i18n("Retention:")
            text: backupData ? backupData.retentionSummary : ""
        }
        Controls.Label {
            Kirigami.FormData.label: i18n("Run conditions:")
            text: backupData ? backupData.conditionsSummary : ""
        }
    }
}
