// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "backupReassignPage"
    title: Texts.label("destination")

    required property var rootWindow
    required property var appBackend
    required property var destinationsModel
    required property string backupId

    property var backupData: rootWindow.findBackupData(backupId)
    property string selectedDestId: ""
    property string selectedDestName: ""

    readonly property bool canSwitch: selectedDestId !== "" && backupData !== null && selectedDestId !== backupData.destinationId

    function handleEscape() {
        if (confirmReassignDialog.opened || confirmReassignDialog.visible) {
            confirmReassignDialog.close();
            return true;
        }
        return false;
    }

    actions: [
        Kirigami.Action {
            id: switchDestinationAction
            objectName: "switchDestinationAction"
            icon.name: "document-swap"
            text: i18n("Switch Destination…")
            enabled: page.canSwitch && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: confirmReassignDialog.open()
        },
        Kirigami.Action {
            id: closeReassignAction
            objectName: "closeReassignAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: rootWindow.popSubPage()
        }
    ]

    Kirigami.PromptDialog {
        id: confirmReassignDialog
        objectName: "confirmReassignDialog"
        title: backupData
            ? i18n("Back up '%1' to '%2'?", backupData.name, page.selectedDestName)
            : i18n("Switch Destination?")
        subtitle: backupData
            ? i18n("On '%1' the snapshots start fresh, so the next backup reads all files again. The old snapshots stay on '%2', and you can still restore them there.", page.selectedDestName, backupData.destinationName)
            : ""
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                text: i18n("Switch Destination")
                icon.name: "document-swap"
                onTriggered: {
                    if (appBackend.reassignBackup(backupId, page.selectedDestId)) {
                        confirmReassignDialog.close();
                        rootWindow.showPassiveNotification(i18n("Destination updated"));
                        rootWindow.popSubPage();
                    }
                }
            }
        ]
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Heading {
            level: 3
            text: i18n("Current Destination")
        }

        Controls.ItemDelegate {
            Layout.fillWidth: true
            enabled: false
            icon.name: "network-server"
            text: backupData ? backupData.destinationName : ""
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Choose Another Destination")
        }

        KeyboardList {
            objectName: "reassignDestinationsList"
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(contentHeight, Kirigami.Units.gridUnit * 8)
            model: destinationsModel
            currentIndex: -1
            spacing: Kirigami.Units.smallSpacing
            activeFocusOnTab: count > 1
            keyNavigationEnabled: true

            // Every destination except the backup's current one can be chosen.
            function selectable(i) {
                return !page.backupData
                    || destinationsModel.idAt(i) !== page.backupData.destinationId;
            }

            function firstSelectableIndex() {
                for (var i = 0; i < count; ++i) {
                    if (selectable(i)) {
                        return i;
                    }
                }
                return -1;
            }

            onActiveFocusChanged: {
                if (activeFocus && count > 0) {
                    if (currentIndex < 0 || (currentItem && currentItem.isCurrent)) {
                        var idx = firstSelectableIndex();
                        if (idx >= 0) {
                            currentIndex = idx;
                        }
                    }
                }
            }

            Keys.onUpPressed: function(event) {
                for (var i = currentIndex - 1; i >= 0; --i) {
                    if (selectable(i)) {
                        currentIndex = i;
                        event.accepted = true;
                        return;
                    }
                }
                event.accepted = false;
            }

            Keys.onDownPressed: function(event) {
                for (var i = currentIndex + 1; i < count; ++i) {
                    if (selectable(i)) {
                        currentIndex = i;
                        event.accepted = true;
                        return;
                    }
                }
                event.accepted = false;
            }

            delegate: Controls.ItemDelegate {
                id: row
                focusPolicy: Qt.ClickFocus
                objectName: "reassignDestItem"
                required property int index
                required property string destinationId
                required property string name
                required property string kindLabel
                required property string statusText
                required property string statusIcon

                readonly property bool isCurrent: backupData !== null && destinationId === backupData.destinationId
                visible: !isCurrent
                height: visible ? implicitHeight : 0
                width: ListView.view.width
                highlighted: page.selectedDestId === destinationId
                Accessible.name: name

                onClicked: {
                    ListView.view.currentIndex = index;
                    page.selectedDestId = destinationId;
                    page.selectedDestName = name;
                }

                FocusRing {
                    target: row
                }

                contentItem: RowLayout {
                    spacing: Kirigami.Units.mediumSpacing
                    Kirigami.Icon {
                        source: row.statusIcon
                        Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                        Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        Controls.Label {
                            text: row.name
                            font.bold: true
                            Layout.fillWidth: true
                        }
                        Controls.Label {
                            text: row.kindLabel + " · " + row.statusText
                            opacity: 0.75
                            Layout.fillWidth: true
                        }
                    }
                }
            }
        }

        Controls.Button {
            text: Texts.label("add_new_destination_ellipsis")
            icon.name: "list-add"
            enabled: appBackend.blocking_kind === ""
            Accessible.name: Texts.label("add_new_destination")
            onClicked: rootWindow.openAddDestinationWizard()
        }
    }
}
