// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    readonly property SharedTexts sharedTexts: SharedTexts {}
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
            text: appBackend.recoveryActionLabel("update_stored_password")
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
                ? sharedTexts.label("remove_backups_first")
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
            text: i18n("The saved password does not match this destination.")
            actions: [
                Kirigami.Action {
                    text: appBackend.recoveryActionLabel("update_stored_password")
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
                ? i18np("%1 backup still waits for retention because the destination was locked.", "%1 backups still wait for retention because the destination was locked.", currentData.pendingForgetCount)
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
                text: currentData ? i18np("%1 configured backup", "%1 configured backups", currentData.backupCount) : ""
            }
        }

        Kirigami.Heading {
            level: 3
            text: i18n("Maintenance")
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.Label {
                objectName: "maintenanceStatusLabel"
                visible: currentData !== null && currentData.maintenanceStatusText !== ""
                Kirigami.FormData.label: i18n("Status:")
                text: currentData ? currentData.maintenanceStatusText : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                Kirigami.FormData.label: i18n("Clean up:")
                text: currentData ? currentData.pruneSummary : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.lastPruneAtText !== ""
                Kirigami.FormData.label: i18n("Last clean up:")
                text: currentData ? currentData.lastPruneAtText : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.nextPruneAtText !== ""
                Kirigami.FormData.label: i18n("Next clean up:")
                text: currentData ? currentData.nextPruneAtText : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                Kirigami.FormData.label: i18n("Verify:")
                text: currentData ? currentData.checkSummary : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.lastCheckAtText !== ""
                Kirigami.FormData.label: i18n("Last verify:")
                text: currentData ? currentData.lastCheckAtText : ""
                wrapMode: Text.WordWrap
            }
            Controls.Label {
                visible: currentData !== null && currentData.nextCheckAtText !== ""
                Kirigami.FormData.label: i18n("Next verify:")
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
            text: sharedTexts.label("remove_backups_first")
            opacity: 0.75
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
