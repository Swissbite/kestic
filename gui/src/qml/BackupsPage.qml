// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "backupsListPage"
    title: Texts.label("backups")

    required property var rootWindow
    required property var appBackend
    required property var backupsModel

    property string selectedBackupId: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: backupsListView

    header: Kirigami.InlineViewHeader {
        objectName: "backupsListHeader"
        width: parent.width
        text: i18np("%1 Backup", "%1 Backups", backupsListView.count)
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

    KeyboardList {
        id: backupsListView
        objectName: "backupsListView"
        model: backupsModel
        spacing: Kirigami.Units.smallSpacing
        keyNavigationEnabled: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0) {
                if (currentIndex < 0) {
                    currentIndex = Math.max(0, backupsModel.rowOf(page.selectedBackupId));
                }
            }
        }

        Kirigami.PlaceholderMessage {
            objectName: "backupsEmptyPlaceholder"
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            // Nothing is known before the first state arrived.
            visible: backupsListView.count === 0 && !appBackend.state_loading
            icon.name: "folder-sync"
            text: Texts.label("no_backups")
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
            required property string sourceKind
            required property string statusText
            required property string statusIcon
            required property string statusColorKind
            required property string nextScheduledAtText
            required property bool isRunning

            width: ListView.view.width
            focusPolicy: Qt.ClickFocus
            highlighted: page.selectedBackupId === backupId
            Accessible.name: name

            onClicked: {
                ListView.view.currentIndex = index;
                page.selectedBackupId = backupId;
                row.forceActiveFocus(Qt.TabFocusReason);
                rootWindow.openBackupDetailById(backupId);
            }

            FocusRing {
                target: row
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                RowIcon {
                    iconSize: Kirigami.Units.iconSizes.medium
                    source: row.sourceKind === "drive" ? "drive-harddisk" : "folder"
                    onSelection: row.highlighted
                    backdropObjectName: "backupRowSourceIconBackdrop"
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Kirigami.Heading {
                        level: 4
                        text: row.name
                        color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }

                    Controls.Label {
                        text: row.statusText
                        color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        opacity: 0.8
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }

                    Controls.Label {
                        visible: !row.isRunning && row.nextScheduledAtText !== ""
                        text: row.nextScheduledAtText
                        color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        opacity: 0.65
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                }

                Kirigami.Icon {
                    source: row.statusIcon
                    color: rootWindow.iconColorFor(row.statusColorKind, page.Kirigami.Theme)
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
            }
        }
    }
}
