// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "backupsListPage"
    title: i18n("Backups")

    required property var rootWindow
    required property var appBackend
    required property var backupsModel

    property string selectedBackupId: ""

    function focusSelectedRow() {
        for (var i = 0; i < backupsListView.count; ++i) {
            var item = backupsListView.itemAtIndex(i);
            if (item && item.highlighted) {
                backupsListView.currentIndex = i;
                rootWindow.focusItemInPageStack(item);
                return true;
            }
        }
        return false;
    }

    function iconColorFor(kind) {
        switch (kind) {
        case "positive": return Kirigami.Theme.positiveTextColor;
        case "highlight": return Kirigami.Theme.highlightColor;
        case "neutral": return Kirigami.Theme.neutralTextColor;
        case "negative": return Kirigami.Theme.negativeTextColor;
        default: return Kirigami.Theme.textColor;
        }
    }

    header: Kirigami.InlineViewHeader {
        width: parent.width
        text: i18n("%1 Backups", backupsListView.count)
        actions: [
            Kirigami.Action {
                id: newBackupHeaderAction
                objectName: "newBackupHeaderAction"
                icon.name: "list-add"
                text: i18n("New Backup…")
                enabled: appBackend.blocking_kind !== "daemon_unreachable" && appBackend.daemon_reachable
                onTriggered: rootWindow.openNewBackupWizard()
            }
        ]
    }

    ListView {
        id: backupsListView
        objectName: "backupsListView"
        model: backupsModel
        spacing: Kirigami.Units.smallSpacing
        keyNavigationEnabled: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0) {
                if (currentIndex < 0) {
                    var targetIdx = 0;
                    for (var i = 0; i < count; ++i) {
                        if (backupsModel.data(backupsModel.index(i, 0), 0) === page.selectedBackupId) {
                            targetIdx = i;
                            break;
                        }
                    }
                    currentIndex = targetIdx;
                }
            }
        }

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: backupsListView.count === 0
            icon.name: "folder-sync"
            text: i18n("No backups configured yet")
            explanation: i18n("Create a backup to define what folder to back up and where to store it.")
            helpfulAction: Kirigami.Action {
                id: backupsEmptyAddAction
                objectName: "backupsEmptyAddAction"
                icon.name: "list-add"
                text: i18n("New Backup…")
                enabled: appBackend.daemon_reachable
                onTriggered: rootWindow.openNewBackupWizard()
            }
        }

        delegate: Controls.ItemDelegate {
            id: row
            objectName: "backupListItem"
            required property int index
            required property string backupId
            required property string name
            required property string sourcePath
            required property string sourceKind
            required property string destinationId
            required property string destinationName
            required property string statusText
            required property string statusIcon
            required property string statusColorKind
            required property string nextScheduledAtText
            required property bool available
            required property bool isRunning

            width: ListView.view.width
            activeFocusOnTab: true
            highlighted: page.selectedBackupId === backupId
            Accessible.name: name

            onClicked: {
                ListView.view.currentIndex = index;
                page.selectedBackupId = backupId;
                row.forceActiveFocus(Qt.TabFocusReason);
                rootWindow.openBackupDetailById(backupId);
            }

            Rectangle {
                anchors.fill: parent
                anchors.margins: row.highlighted ? 2 : 0
                z: 10
                color: (row.activeFocus || row.visualFocus) && row.highlighted
                    ? Qt.rgba(1, 1, 1, 0.14)
                    : "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (row.activeFocus || row.visualFocus) ? 2 : 0
                border.color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Icon {
                    source: row.sourceKind === "drive" ? "drive-harddisk" : "folder"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Kirigami.Heading {
                        level: 4
                        text: row.name
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }

                    Controls.Label {
                        text: row.statusText
                        color: Kirigami.Theme.textColor
                        opacity: 0.8
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }

                    Controls.Label {
                        visible: !row.isRunning && row.nextScheduledAtText !== ""
                        text: row.nextScheduledAtText
                        color: Kirigami.Theme.textColor
                        opacity: 0.65
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                }

                Kirigami.Icon {
                    source: row.statusIcon
                    color: page.iconColorFor(row.statusColorKind)
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
            }
        }
    }
}
