// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Restore area, right column: the snapshots of one backup, newest first, grouped by day.
// Return opens a snapshot for browsing; the context menu (Menu key, Shift+F10 or right click)
// also compares it with the previous one or restores it.
Kirigami.ScrollablePage {
    id: page
    objectName: "restoreSnapshotsPage"
    title: backupName

    required property var rootWindow
    required property var appBackend
    required property var restoreSnapshotsModel
    required property string backupId
    property string backupName: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: restoreSnapshotsListView

    signal browseRequested(var destinationId, var snapshotId)
    signal compareRequested(var destinationId, var fromSnapshotId, var toSnapshotId)
    signal restoreRequested(var destinationId, var snapshotId)

    function reload() {
        restoreSnapshotsListView.currentIndex = -1;
        appBackend.loadRestoreSnapshots(backupId);
    }

    // Opens the context menu of the snapshot in `row` below that row.
    function openSnapshotMenu(row) {
        restoreSnapshotsListView.currentIndex = row.index;
        snapshotMenu.destinationId = row.destinationId;
        snapshotMenu.snapshotId = row.snapshotId;
        snapshotMenu.previousSnapshotId = row.previousSnapshotId;
        snapshotMenu.popup(row, 0, row.height);
    }

    Component.onCompleted: reload()
    onBackupIdChanged: reload()

    Connections {
        target: appBackend
        function onRestoreSnapshotsUpdated() { restoreSnapshotsModel.sync(); }
    }

    actions: [
        Kirigami.Action {
            objectName: "restoreSnapshotsRefreshAction"
            icon.name: "view-refresh"
            text: Texts.label("refresh")
            enabled: !appBackend.restore_snapshots_loading
            onTriggered: page.reload()
        }
    ]

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        Kirigami.LoadingPlaceholder {
            Layout.fillWidth: true
            visible: appBackend.restore_snapshots_loading
            text: Texts.label("loading_snapshots")
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: !appBackend.restore_snapshots_loading && appBackend.restore_snapshots_error !== ""
            text: appBackend.restore_snapshots_error
            actions: [
                Kirigami.Action {
                    text: Texts.label("retry")
                    onTriggered: page.reload()
                }
            ]
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.gridUnit * 2
            visible: !appBackend.restore_snapshots_loading && restoreSnapshotsListView.count === 0
                && appBackend.restore_snapshots_error === ""
            icon.name: "view-history"
            text: Texts.label("no_snapshots_yet")
        }

        ListView {
            id: restoreSnapshotsListView
            objectName: "restoreSnapshotsListView"
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(contentHeight, Kirigami.Units.gridUnit * 12)
            visible: !appBackend.restore_snapshots_loading && count > 0
            interactive: false
            keyNavigationEnabled: true
            model: restoreSnapshotsModel
            spacing: Kirigami.Units.smallSpacing

            onActiveFocusChanged: {
                if (activeFocus && count > 0 && currentIndex < 0) {
                    currentIndex = 0;
                }
            }

            section.property: "sectionName"
            section.delegate: Kirigami.ListSectionHeader {
                required property string section
                objectName: "restoreSnapshotSectionHeader"
                width: ListView.view.width
                text: section
            }

            delegate: Controls.ItemDelegate {
                id: snapRow
                objectName: "restoreSnapshotListItem"
                required property int index
                required property string snapshotId
                required property string shortId
                required property string title
                required property string subtitle
                required property string previousSnapshotId
                required property string destinationId

                width: ListView.view.width
                activeFocusOnTab: true
                highlighted: ListView.isCurrentItem
                Accessible.name: title
                Accessible.description: subtitle

                onClicked: {
                    ListView.view.currentIndex = index;
                    page.browseRequested(destinationId, snapshotId);
                }

                Keys.onPressed: function(event) {
                    if (event.key === Qt.Key_Menu
                            || (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier))) {
                        page.openSnapshotMenu(snapRow);
                        event.accepted = true;
                    }
                }

                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: page.openSnapshotMenu(snapRow)
                }

                FocusRing {
                    target: snapRow
                    anchors.margins: snapRow.highlighted ? 2 : 0
                    ringColor: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                }

                contentItem: RowLayout {
                    spacing: Kirigami.Units.mediumSpacing

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2

                        RowLayout {
                            Layout.fillWidth: true
                            Controls.Label {
                                text: snapRow.title
                                color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                Layout.fillWidth: true
                            }
                            Controls.Label {
                                visible: appBackend.expert_mode
                                text: snapRow.shortId
                                font: Kirigami.Theme.fixedWidthFont
                                color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                opacity: 0.7
                            }
                        }
                        Controls.Label {
                            objectName: "restoreSnapshotSubtitle"
                            text: snapRow.subtitle
                            color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                            opacity: 0.75
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    Kirigami.Icon {
                        source: "go-next-symbolic"
                        color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    }
                }
            }
        }
    }

    Controls.Menu {
        id: snapshotMenu
        objectName: "snapshotContextMenu"

        property string destinationId: ""
        property string snapshotId: ""
        property string previousSnapshotId: ""

        Controls.MenuItem {
            objectName: "browseSnapshotMenuItem"
            icon.name: "document-open-folder"
            text: i18n("Browse")
            onTriggered: page.browseRequested(snapshotMenu.destinationId, snapshotMenu.snapshotId)
        }
        Controls.MenuItem {
            objectName: "compareWithPreviousMenuItem"
            icon.name: "kdiff3"
            text: i18n("Compare with Previous")
            enabled: snapshotMenu.previousSnapshotId !== ""
            onTriggered: page.compareRequested(snapshotMenu.destinationId,
                snapshotMenu.previousSnapshotId, snapshotMenu.snapshotId)
        }
        Controls.MenuItem {
            objectName: "restoreSnapshotMenuItem"
            icon.name: "document-revert"
            text: i18n("Restore…")
            onTriggered: page.restoreRequested(snapshotMenu.destinationId, snapshotMenu.snapshotId)
        }

        onClosed: {
            var item = restoreSnapshotsListView.itemAtIndex(restoreSnapshotsListView.currentIndex);
            if (item) item.forceActiveFocus(Qt.TabFocusReason);
        }
    }
}
