// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "destinationsListPage"
    title: ""

    required property var rootWindow
    required property var appBackend
    required property var destinationsModel

    property string selectedDestinationId: ""

    header: Kirigami.InlineViewHeader {
        width: parent.width
        text: i18n("%1 Destinations", destinationsListView.count)
        actions: [
            Kirigami.Action {
                id: addDestinationHeaderAction
                objectName: "addDestinationHeaderAction"
                icon.name: "list-add"
                text: i18n("Add Destination…")
                enabled: appBackend.blocking_kind === ""
                onTriggered: rootWindow.openAddDestinationWizard()
            }
        ]
    }

    ListView {
        id: destinationsListView
        objectName: "destinationsListView"
        model: destinationsModel
        spacing: Kirigami.Units.smallSpacing
        keyNavigationEnabled: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0 && currentIndex < 0) {
                var targetIdx = 0;
                for (var i = 0; i < count; ++i) {
                    if (destinationsModel.data(destinationsModel.index(i, 0), 0) === page.selectedDestinationId) {
                        targetIdx = i;
                        break;
                    }
                }
                currentIndex = targetIdx;
            }
        }

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: destinationsListView.count === 0
            icon.name: "network-server"
            text: i18n("No destinations configured yet")
            explanation: i18n("Add a local folder or REST server destination to store encrypted backups.")
            helpfulAction: Kirigami.Action {
                id: destinationsEmptyAddAction
                objectName: "destinationsEmptyAddAction"
                icon.name: "list-add"
                text: i18n("Add Destination…")
                enabled: appBackend.blocking_kind === ""
                onTriggered: rootWindow.openAddDestinationWizard()
            }
        }

        delegate: Controls.ItemDelegate {
            id: row
            objectName: "destinationListItem"
            required property int index
            required property string destinationId
            required property string name
            required property string kindName
            required property string kindLabel
            required property int backupCount
            required property string statusText
            required property string statusIcon
            required property bool hasProblem

            width: ListView.view.width
            highlighted: page.selectedDestinationId === destinationId
            Accessible.name: name

            onClicked: {
                ListView.view.currentIndex = index;
                page.selectedDestinationId = destinationId;
                rootWindow.openDestinationDetailById(destinationId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (row.activeFocus || row.visualFocus) ? 2 : 0
                border.color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Icon {
                    source: row.kindName === "rest" ? "network-server" : "folder"
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
                        text: i18n("%1 · %2 backups", row.statusText, row.backupCount)
                        color: Kirigami.Theme.textColor
                        opacity: 0.8
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                }

                Kirigami.Icon {
                    source: row.hasProblem ? "emblem-important" : row.statusIcon
                    color: row.hasProblem ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.textColor
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
            }
        }
    }
}
