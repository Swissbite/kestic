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

    readonly property var excludesObj: {
        if (!backupData || !backupData.excludesJson) return {};
        try {
            return JSON.parse(backupData.excludesJson);
        } catch (e) {
            return {};
        }
    }

    Connections {
        target: appBackend
        function onModelsUpdated() {
            page.backupData = rootWindow.findBackupData(page.backupId);
        }
    }

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
        },
        Kirigami.Action {
            id: closeFullConfigAction
            objectName: "closeFullConfigAction"
            icon.name: "window-close"
            text: i18n("Close")
            onTriggered: rootWindow.popSubPage()
        }
    ]

    component NavSummaryRow: Controls.ItemDelegate {
        id: navRow
        required property string rowTitle
        required property string rowValue
        required property string rowObjectName
        property bool monospace: false
        signal navigate()

        objectName: rowObjectName
        Layout.fillWidth: true
        activeFocusOnTab: true
        Accessible.name: rowTitle + ": " + rowValue
        onClicked: {
            rootWindow.nextTriggerItem = navRow;
            navigate();
        }

        Rectangle {
            anchors.fill: parent
            z: 10
            color: "transparent"
            radius: Kirigami.Units.cornerRadius
            border.width: (navRow.activeFocus || navRow.visualFocus) ? 2 : 0
            border.color: Kirigami.Theme.highlightColor
            visible: border.width > 0
        }

        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Controls.Label {
                    text: navRow.rowTitle
                    font.bold: true
                    Layout.fillWidth: true
                }
                Controls.Label {
                    text: navRow.rowValue
                    font: navRow.monospace ? Kirigami.Theme.fixedWidthFont : Kirigami.Theme.defaultFont
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }
            Kirigami.Icon {
                source: "go-next"
                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                Layout.preferredHeight: Kirigami.Units.iconSizes.small
            }
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing
        visible: backupData !== null

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.Label {
                Kirigami.FormData.label: i18n("Name:")
                text: backupData ? backupData.name : ""
            }
            Controls.Label {
                Kirigami.FormData.label: i18n("ID:")
                text: backupData ? backupData.backupId : ""
                font: Kirigami.Theme.fixedWidthFont
            }
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Source")
        }

        NavSummaryRow {
            rowObjectName: "fullConfigSourceRow"
            rowTitle: i18n("Folder and kind")
            rowValue: backupData ? (backupData.sourcePath + " (" + backupData.sourceKindLabel + ")") : ""
            onNavigate: rootWindow.openBackupSourcePage(backupId)
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Destination")
        }

        NavSummaryRow {
            rowObjectName: "fullConfigDestinationRow"
            rowTitle: i18n("Repository destination")
            rowValue: backupData
                ? (backupData.destinationName + "\n" + i18n("Password in KWallet (Secret Service); never stored in plain text"))
                : ""
            onNavigate: rootWindow.openBackupReassignPage(backupId)
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Exclude Rules")
        }

        NavSummaryRow {
            rowObjectName: "fullConfigExcludesRow"
            rowTitle: i18n("Configured exclude rules")
            monospace: true
            rowValue: {
                var lines = [];
                var f = page.excludesObj.folders || [];
                if (f.length > 0) lines.push(i18n("Folders:") + " " + f.join(", "));
                if (page.excludesObj.exclude_caches) lines.push(i18n("Exclude cache folders (CACHEDIR.TAG)"));
                if (page.excludesObj.exclude_trash) lines.push(i18n("Exclude trash folders"));
                if (page.excludesObj.exclude_larger_than) lines.push(i18n("Files larger than %1", page.excludesObj.exclude_larger_than));
                if (page.excludesObj.ignore_case) lines.push(i18n("Case-insensitive (--iexclude)"));
                if (page.excludesObj.exclude_if_present && page.excludesObj.exclude_if_present.length > 0) {
                    lines.push(i18n("Marker files:") + " " + page.excludesObj.exclude_if_present.join(", "));
                }
                var expertLines = page.excludesObj.lines || [];
                if (expertLines.length > 0) {
                    lines.push(i18n("Expert rules:") + "\n" + expertLines.join("\n"));
                }
                if (lines.length === 0) lines.push(i18n("None"));
                if (backupData && backupData.excludeFilePath) {
                    lines.push(i18n("Exclude file:") + " " + backupData.excludeFilePath);
                }
                return lines.join("\n");
            }
            onNavigate: rootWindow.openBackupExcludesPage(backupId)
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Retention")
        }

        NavSummaryRow {
            rowObjectName: "fullConfigRetentionRow"
            rowTitle: i18n("Retention policy")
            rowValue: backupData ? backupData.retentionSummary : ""
            onNavigate: rootWindow.openBackupRetentionPage(backupId)
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Schedule & Run Conditions")
        }

        NavSummaryRow {
            rowObjectName: "fullConfigScheduleRow"
            rowTitle: i18n("Schedule")
            rowValue: backupData ? backupData.scheduleSummary : ""
            onNavigate: rootWindow.openBackupSchedulePage(backupId)
        }

        NavSummaryRow {
            rowObjectName: "fullConfigConditionsRow"
            rowTitle: i18n("Run conditions")
            rowValue: backupData ? backupData.conditionsSummary : ""
            onNavigate: rootWindow.openBackupConditionsPage(backupId)
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.Label {
                Kirigami.FormData.label: i18n("Destination maintenance:")
                text: backupData && backupData.pendingForget
                    ? i18n("Retention pending (forget not yet executed due to lock)")
                    : i18n("Up to date")
                wrapMode: Text.WordWrap
            }
        }

        Controls.Button {
            visible: backupData !== null && backupData.pendingForget
            icon.name: "view-refresh"
            text: i18n("Run Destination Maintenance Now")
            Accessible.name: i18n("Run destination maintenance now")
            onClicked: if (backupData) appBackend.runDestinationMaintenance(backupData.destinationId)
        }
    }
}
