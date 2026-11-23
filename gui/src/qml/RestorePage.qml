// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Restore area, left column: «Which backup?» with the number of snapshots per backup, and below
// «Other snapshots»: unassigned and orphaned snapshots and backups not configured here.
Kirigami.ScrollablePage {
    id: page
    objectName: "restorePage"
    title: Texts.label("restore")

    required property var rootWindow
    required property var appBackend
    required property var restoreBackupsModel
    required property var restoreGroupsModel

    property string selectedBackupId: ""
    // The further group shown next to the list (`unassigned`, `orphaned`, `described`).
    property string selectedGroup: ""
    property int groupCount: 0

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: restoreBackupsListView

    // The page holds lists only: no padding, and the background of a list view.
    padding: 0
    Kirigami.Theme.colorSet: Kirigami.Theme.View
    Kirigami.Theme.inherit: false

    function syncModels() {
        restoreBackupsModel.sync();
        restoreGroupsModel.sync();
    }

    Component.onCompleted: {
        syncModels();
        appBackend.loadRestoreOverview();
    }

    Connections {
        target: appBackend
        function onRestoreOverviewUpdated() { page.syncModels(); }
        function onModelsUpdated() { page.syncModels(); }
    }

    header: Kirigami.InlineViewHeader {
        objectName: "restoreBackupsHeader"
        width: parent.width
        text: i18n("Which backup?")
        actions: [
            Kirigami.Action {
                objectName: "restoreRefreshAction"
                icon.name: "view-refresh"
                text: Texts.label("refresh")
                enabled: !appBackend.restore_overview_loading
                onTriggered: appBackend.loadRestoreOverview()
            }
        ]
    }

    // The backups, then the further groups: one column, so Tab reaches the rows in the order
    // they are shown.
    ColumnLayout {
        width: parent.width
        spacing: 0

        ListView {
            id: restoreBackupsListView
            objectName: "restoreBackupsListView"
            Layout.fillWidth: true
            Layout.preferredHeight: contentHeight
            interactive: false
            model: restoreBackupsModel
            spacing: Kirigami.Units.smallSpacing
            keyNavigationEnabled: true

            onActiveFocusChanged: {
                if (activeFocus && count > 0 && currentIndex < 0) {
                    currentIndex = Math.max(0, restoreBackupsModel.rowOf(page.selectedBackupId));
                }
            }

            delegate: Controls.ItemDelegate {
                id: row
                objectName: "restoreBackupListItem"
                required property int index
                required property string backupId
                required property string name
                required property string subtitle

                width: ListView.view.width
                activeFocusOnTab: true
                highlighted: page.selectedBackupId === backupId
                Accessible.name: name
                Accessible.description: subtitle

                onClicked: {
                    ListView.view.currentIndex = index;
                    page.selectedBackupId = backupId;
                    page.selectedGroup = "";
                    row.forceActiveFocus(Qt.TabFocusReason);
                    rootWindow.openRestoreSnapshots(backupId);
                }

                FocusRing {
                    target: row
                }

                contentItem: RowLayout {
                    spacing: Kirigami.Units.mediumSpacing

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
                            objectName: "restoreBackupSubtitle"
                            text: row.subtitle
                            color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                            opacity: 0.8
                            wrapMode: Text.Wrap
                            Layout.fillWidth: true
                        }
                    }

                    Kirigami.Icon {
                        source: "go-next-symbolic"
                        selected: row.highlighted
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    }
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            visible: groupRepeater.count > 0

            Kirigami.ListSectionHeader {
                objectName: "restoreGroupsHeader"
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.largeSpacing
                text: i18n("Other snapshots")
            }

            Repeater {
                id: groupRepeater
                model: page.restoreGroupsModel
                onCountChanged: page.groupCount = count

                delegate: Controls.ItemDelegate {
                    id: groupRow
                    objectName: "restoreGroupListItem"
                    required property string group
                    required property string name
                    required property string subtitle
                    required property string iconName

                    Layout.fillWidth: true
                    activeFocusOnTab: true
                    highlighted: page.selectedGroup === group
                    Accessible.name: name
                    Accessible.description: subtitle

                    onClicked: {
                        page.selectedBackupId = "";
                        page.selectedGroup = group;
                        restoreBackupsListView.currentIndex = -1;
                        groupRow.forceActiveFocus(Qt.TabFocusReason);
                        rootWindow.openRestoreGroup(group, name);
                    }

                    FocusRing {
                        target: groupRow
                    }

                    // The kind of group shows as an icon before the subtitle, so the names line
                    // up with those of the backups above.
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.mediumSpacing

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            Kirigami.Heading {
                                level: 4
                                text: groupRow.name
                                color: groupRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Kirigami.Units.smallSpacing

                                RowIcon {
                                    objectName: "restoreGroupIcon"
                                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                                    Layout.alignment: Qt.AlignTop
                                    source: groupRow.iconName
                                    onSelection: groupRow.highlighted
                                }
                                Controls.Label {
                                    objectName: "restoreGroupSubtitle"
                                    text: groupRow.subtitle
                                    color: groupRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                    opacity: 0.8
                                    wrapMode: Text.Wrap
                                    Layout.fillWidth: true
                                }
                            }
                        }

                        Kirigami.Icon {
                            source: "go-next-symbolic"
                            selected: groupRow.highlighted
                            Layout.preferredWidth: Kirigami.Units.iconSizes.small
                            Layout.preferredHeight: Kirigami.Units.iconSizes.small
                        }
                    }
                }
            }
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing * 2
            Layout.topMargin: Kirigami.Units.gridUnit * 2
            visible: restoreBackupsListView.count === 0 && page.groupCount === 0
            icon.name: "document-revert"
            text: Texts.label("no_backups")
        }
    }
}
