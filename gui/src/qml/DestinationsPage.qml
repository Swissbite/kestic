// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "destinationsListPage"
    title: Texts.label("destinations")

    required property var rootWindow
    required property var appBackend
    required property var destinationsModel

    property string selectedDestinationId: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: destinationsListView

    header: Kirigami.InlineViewHeader {
        objectName: "destinationsListHeader"
        width: parent.width
        text: i18np("%1 Destination", "%1 Destinations", destinationsListView.count)
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
                currentIndex = Math.max(0, destinationsModel.rowOf(page.selectedDestinationId));
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
            required property int backupCount
            required property string statusText
            required property string statusIcon
            required property bool hasProblem

            width: ListView.view.width
            activeFocusOnTab: true
            highlighted: page.selectedDestinationId === destinationId
            Accessible.name: name

            onClicked: {
                ListView.view.currentIndex = index;
                page.selectedDestinationId = destinationId;
                row.forceActiveFocus(Qt.TabFocusReason);
                rootWindow.openDestinationDetailById(destinationId);
            }

            FocusRing {
                target: row
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                Item {
                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.medium

                    // The folder icon has the colour of the selection: on the selected row it
                    // sits on the background colour, or its body would vanish.
                    Rectangle {
                        objectName: "destinationRowIconBackdrop"
                        visible: row.highlighted
                        anchors.fill: parent
                        anchors.margins: -2
                        radius: Kirigami.Units.cornerRadius
                        color: Kirigami.Theme.backgroundColor
                    }

                    Kirigami.Icon {
                        anchors.fill: parent
                        source: row.kindName === "rest" ? "network-server" : "folder"
                    }
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
                        text: i18np("%2 · %1 backup", "%2 · %1 backups", row.backupCount, row.statusText)
                        color: row.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
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
