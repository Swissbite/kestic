// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "destinationDetailPage"
    title: currentData ? currentData.name : i18n("Destination")

    required property var rootWindow
    required property var appBackend
    required property string destinationId

    property var currentData: null

    function refreshFromModel() {
        currentData = rootWindow.findDestinationData(destinationId);
    }

    Connections {
        target: appBackend
        function onModelsUpdated() {
            page.refreshFromModel();
        }
    }

    Component.onCompleted: refreshFromModel()

    actions: [
        Kirigami.Action {
            icon.name: "view-refresh"
            text: i18n("Check Connection")
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: appBackend.checkAvailability(true)
        },
        Kirigami.Action {
            objectName: "destEditConnectionAction"
            icon.name: "document-edit"
            text: i18n("Edit Connection…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: if (currentData) rootWindow.openDestinationEditPage(destinationId)
        },
        Kirigami.Action {
            objectName: "destUpdatePasswordAction"
            icon.name: "dialog-password"
            text: i18n("Update Saved Password…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null && appBackend.blocking_kind === ""
            onTriggered: if (currentData) rootWindow.openUpdatePasswordDialog(destinationId, currentData.name)
        },
        Kirigami.Action {
            objectName: "destRemoveAction"
            icon.name: "list-remove"
            text: i18n("Remove Destination…")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            enabled: currentData !== null && currentData.backupCount === 0 && appBackend.blocking_kind !== "daemon_unreachable"
            tooltip: currentData && currentData.backupCount > 0
                ? i18n("First remove backups from Kestic or assign them to another destination")
                : i18n("Remove destination from Kestic")
            onTriggered: if (currentData) rootWindow.openRemoveDestinationDialog(destinationId, currentData.name)
        }
    ]

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing
        visible: currentData !== null

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: currentData !== null && currentData.passwordRejected
            text: i18n("The stored password does not match this destination.")
            actions: [
                Kirigami.Action {
                    text: i18n("Update Saved Password…")
                    onTriggered: rootWindow.openUpdatePasswordDialog(destinationId, currentData.name)
                }
            ]
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: currentData !== null && currentData.hasStaleLock
            text: i18n("An interrupted run left a repository lock on this destination.")
            actions: [
                Kirigami.Action {
                    objectName: "destUnlockAction"
                    icon.name: "unlock"
                    text: i18n("Unlock…")
                    enabled: appBackend.blocking_kind === ""
                    onTriggered: appBackend.unlockDestination(destinationId)
                }
            ]
        }

        Kirigami.InlineMessage {
            id: pendingForgetBanner
            objectName: "destPendingForgetBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: currentData !== null && currentData.pendingForgetCount > 0
            text: currentData
                ? i18n("%1 backup has pending retention cleanup from a previous locked run.", currentData.pendingForgetCount)
                : ""
            actions: [
                Kirigami.Action {
                    objectName: "runDestinationMaintenanceAction"
                    icon.name: "view-refresh"
                    text: i18n("Run Maintenance Now")
                    onTriggered: appBackend.runDestinationMaintenance(destinationId)
                }
            ]
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: currentData ? currentData.statusIcon : "network-connect"
                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                Layout.preferredHeight: Kirigami.Units.iconSizes.small
            }

            Controls.Label {
                text: currentData
                    ? i18n("%1 · %2 · %3", currentData.statusText, currentData.kindLabel, currentData.location)
                    : ""
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }

        Kirigami.FormLayout {
            Controls.Label {
                Kirigami.FormData.label: i18n("Type:")
                text: currentData ? currentData.kindLabel : ""
            }
            Controls.Label {
                Kirigami.FormData.label: i18n("Location:")
                text: currentData ? currentData.location : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.kindName === "rest" && currentData.username !== ""
                Kirigami.FormData.label: i18n("Username:")
                text: currentData ? currentData.username : ""
            }
            Controls.Label {
                Kirigami.FormData.label: i18n("Backups:")
                text: currentData ? i18n("%1 configured backups", currentData.backupCount) : ""
            }
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Maintenance")
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.Label {
                Kirigami.FormData.label: i18n("Cleanup (Prune):")
                text: currentData ? currentData.pruneSummary : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.lastPruneAtText !== ""
                Kirigami.FormData.label: i18n("Last cleanup:")
                text: currentData ? currentData.lastPruneAtText : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.nextPruneAtText !== ""
                Kirigami.FormData.label: i18n("Next cleanup:")
                text: currentData ? currentData.nextPruneAtText : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                Kirigami.FormData.label: i18n("Integrity Check:")
                text: currentData ? currentData.checkSummary : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.lastCheckAtText !== ""
                Kirigami.FormData.label: i18n("Last check:")
                text: currentData ? currentData.lastCheckAtText : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.nextCheckAtText !== ""
                Kirigami.FormData.label: i18n("Next check:")
                text: currentData ? currentData.nextCheckAtText : ""
                wrapMode: Text.WordWrap
            }
        }

        Controls.ItemDelegate {
            id: editMaintenanceRow
            objectName: "editMaintenanceRow"
            Layout.fillWidth: true
            activeFocusOnTab: true
            Accessible.name: i18n("Edit maintenance schedule")
            onClicked: {
                rootWindow.nextTriggerItem = editMaintenanceRow;
                rootWindow.openDestinationMaintenancePage(destinationId);
            }

            Rectangle {
                anchors.fill: parent
                z: 10
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: (editMaintenanceRow.activeFocus || editMaintenanceRow.visualFocus) ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Kirigami.Icon {
                    source: "tools-wizard"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                Controls.Label {
                    text: i18n("Edit maintenance schedule…")
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }

        Controls.Label {
            visible: currentData !== null && currentData.backupCount > 0
            text: i18n("Remove destination requires removing or reassigning its backups first.")
            opacity: 0.75
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
