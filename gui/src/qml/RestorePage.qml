// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Restore area, left column: «Which backup?» with the number of snapshots per backup.
Kirigami.ScrollablePage {
    id: page
    objectName: "restorePage"
    title: Texts.label("restore")

    required property var rootWindow
    required property var appBackend
    required property var restoreBackupsModel

    property string selectedBackupId: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: restoreBackupsListView

    Component.onCompleted: {
        restoreBackupsModel.sync();
        appBackend.loadRestoreOverview();
    }

    Connections {
        target: appBackend
        function onRestoreOverviewUpdated() { restoreBackupsModel.sync(); }
        function onModelsUpdated() { restoreBackupsModel.sync(); }
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

    ListView {
        id: restoreBackupsListView
        objectName: "restoreBackupsListView"
        model: restoreBackupsModel
        spacing: Kirigami.Units.smallSpacing
        keyNavigationEnabled: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0 && currentIndex < 0) {
                currentIndex = Math.max(0, restoreBackupsModel.rowOf(page.selectedBackupId));
            }
        }

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: restoreBackupsListView.count === 0
            icon.name: "document-revert"
            text: Texts.label("no_backups")
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
                row.forceActiveFocus(Qt.TabFocusReason);
                rootWindow.openRestoreSnapshots(backupId);
            }

            FocusRing {
                target: row
                anchors.margins: row.highlighted ? 2 : 0
                ringColor: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
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
                    color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }
    }
}
