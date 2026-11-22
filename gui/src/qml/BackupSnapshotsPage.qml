// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    objectName: "backupSnapshotsPage"
    title: i18n("Snapshots · %1", backupName)

    required property var rootWindow
    required property var appBackend
    required property var snapshotsModel
    required property string backupId
    required property string backupName

    Component.onCompleted: {
        appBackend.loadBackupSnapshots(backupId);
        snapshotsModel.sync();
    }

    Connections {
        target: appBackend
        function onSnapshotsUpdated() {
            snapshotsModel.sync();
        }
    }

    actions: [
        Kirigami.Action {
            icon.name: "view-refresh"
            text: Texts.label("refresh")
            onTriggered: appBackend.loadBackupSnapshots(backupId)
        },
        Kirigami.Action {
            id: closeSnapshotsAction
            objectName: "closeSnapshotsAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: rootWindow.popSubPage()
        }
    ]

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        Kirigami.LoadingPlaceholder {
            Layout.fillWidth: true
            visible: appBackend.snapshots_loading
            text: Texts.label("loading_snapshots")
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: !appBackend.snapshots_loading && appBackend.snapshots_error !== ""
            text: appBackend.snapshots_error
            actions: [
                Kirigami.Action {
                    text: Texts.label("retry")
                    onTriggered: appBackend.loadBackupSnapshots(backupId)
                }
            ]
        }

        Kirigami.PlaceholderMessage {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.gridUnit * 2
            visible: !appBackend.snapshots_loading && snapshotsListView.count === 0 && appBackend.snapshots_error === ""
            icon.name: "view-history"
            text: Texts.label("no_snapshots_yet")
        }

        ListView {
            id: snapshotsListView
            objectName: "snapshotsListView"
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(contentHeight, Kirigami.Units.gridUnit * 12)
            visible: !appBackend.snapshots_loading && count > 0
            activeFocusOnTab: visible && count > 0
            keyNavigationEnabled: true
            model: snapshotsModel
            spacing: Kirigami.Units.smallSpacing

            onActiveFocusChanged: {
                if (activeFocus && count > 0 && currentIndex < 0) {
                    currentIndex = 0;
                }
            }

            section.property: "sectionName"
            section.delegate: Kirigami.ListSectionHeader {
                required property string section
                objectName: "snapshotSectionHeader"
                width: ListView.view.width
                text: section
            }

            delegate: Controls.ItemDelegate {
                id: snapRow
                objectName: "snapshotListItem"
                required property int index
                required property string shortId
                required property string timeDisplay
                required property string pathsText
                required property string statsText

                width: ListView.view.width
                highlighted: ListView.isCurrentItem && snapshotsListView.activeFocus
                Accessible.name: timeDisplay
                onClicked: ListView.view.currentIndex = index

                FocusRing {
                    target: snapRow
                    ringColor: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                }

                contentItem: ColumnLayout {
                    spacing: 2
                    RowLayout {
                        Layout.fillWidth: true
                        Controls.Label {
                            text: snapRow.timeDisplay
                            font.bold: true
                            Layout.fillWidth: true
                        }
                        Controls.Label {
                            visible: appBackend.expert_mode
                            text: snapRow.shortId
                            font: Kirigami.Theme.fixedWidthFont
                            opacity: 0.7
                        }
                    }
                    Controls.Label {
                        text: snapRow.statsText + (snapRow.pathsText !== "" ? " · " + snapRow.pathsText : "")
                        opacity: 0.75
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                }
            }
        }
    }
}
