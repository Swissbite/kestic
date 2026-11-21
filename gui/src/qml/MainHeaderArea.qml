// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

ColumnLayout {
    id: headerArea
    spacing: 0

    required property var rootWindow
    required property var appBackend
    required property var pageStackRef

    // Priority order: Daemon unreachable -> Secret store locked -> restic unavailable
    Kirigami.InlineMessage {
        id: blockingStatusBanner
        objectName: "blockingStatusBanner"
        Layout.fillWidth: true
        position: Kirigami.InlineMessage.Position.Header
        type: headerArea.appBackend.blocking_kind === "secret_store_locked"
            ? Kirigami.MessageType.Warning
            : Kirigami.MessageType.Error
        visible: headerArea.appBackend.blocking_kind !== ""
        text: headerArea.appBackend.blocking_message

        actions: [
            Kirigami.Action {
                id: retryDaemonAction
                objectName: "retryDaemonAction"
                visible: headerArea.appBackend.blocking_kind === "daemon_unreachable"
                icon.name: "view-refresh"
                text: Texts.label("retry")
                onTriggered: headerArea.appBackend.retryStartDaemon()
            },
            Kirigami.Action {
                id: unlockSecretStoreAction
                objectName: "unlockSecretStoreAction"
                visible: headerArea.appBackend.blocking_kind === "secret_store_locked"
                icon.name: "dialog-password"
                text: Texts.label("unlock_ellipsis")
                onTriggered: headerArea.appBackend.unlockSecretStore()
            },
            Kirigami.Action {
                visible: headerArea.appBackend.blocking_kind === "secret_store_locked"
                icon.name: "view-refresh"
                text: Texts.label("retry")
                onTriggered: headerArea.appBackend.checkAvailability(true)
            },
            Kirigami.Action {
                id: downloadResticDisabledAction
                objectName: "downloadResticDisabledAction"
                visible: headerArea.appBackend.blocking_kind === "restic_unavailable"
                text: i18n("Download restic…")
                enabled: false
                tooltip: i18n("Automatic restic download will be added in a future release.")
            },
            Kirigami.Action {
                id: openResticSettingsAction
                objectName: "openResticSettingsAction"
                visible: headerArea.appBackend.blocking_kind === "restic_unavailable"
                icon.name: "configure"
                text: Texts.label("settings")
                onTriggered: headerArea.rootWindow.openSettingsWindow()
            }
        ]
    }

    Kirigami.InlineMessage {
        id: manualRunBlockedBanner
        objectName: "manualRunBlockedBanner"
        Layout.fillWidth: true
        position: Kirigami.InlineMessage.Position.Header
        type: Kirigami.MessageType.Warning
        visible: headerArea.appBackend.manual_run_blocked_message !== ""
        showCloseButton: true
        text: headerArea.appBackend.manual_run_blocked_message
        onVisibleChanged: if (!visible && headerArea.appBackend.manual_run_blocked_message !== "") headerArea.appBackend.dismissManualRunBlocked()
    }

    Kirigami.InlineMessage {
        id: globalPauseBanner
        objectName: "globalPauseBanner"
        Layout.fillWidth: true
        position: Kirigami.InlineMessage.Position.Header
        type: Kirigami.MessageType.Information
        visible: headerArea.appBackend.global_paused
        text: headerArea.appBackend.global_paused_line
        actions: [
            Kirigami.Action {
                objectName: "globalPauseResumeAction"
                icon.name: "media-playback-start"
                text: Texts.label("resume")
                onTriggered: headerArea.appBackend.resumeGlobal()
            }
        ]
    }

    Kirigami.NavigationTabBar {
        id: topNavBar
        objectName: "topNavBar"
        Layout.fillWidth: true
        visible: !headerArea.rootWindow.narrowMode && !headerArea.rootWindow.inWizardLayer
        height: visible ? implicitHeight : 0
        maximumContentWidth: visibleActions.length * Kirigami.Units.gridUnit * 9
        position: Controls.ToolBar.Header
        actions: headerArea.rootWindow.navActions
        delegate: Kirigami.NavigationTabButton {
            required property Controls.Action modelData
            parent: topNavBar.contentItem
            action: modelData
            onActionChanged: action.checkable = true
            Layout.minimumWidth: topNavBar.buttonWidth
            Layout.maximumWidth: topNavBar.buttonWidth
            Layout.fillHeight: true

            Kirigami.Icon {
                source: "emblem-important"
                visible: (modelData.objectName === "backupsTabAction" && headerArea.appBackend.backups_have_attention)
                      || (modelData.objectName === "destinationsTabAction" && headerArea.appBackend.destinations_have_attention)
                color: Kirigami.Theme.negativeTextColor
                width: Kirigami.Units.iconSizes.small
                height: Kirigami.Units.iconSizes.small
                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: Kirigami.Units.smallSpacing
            }
        }
    }

    Controls.ToolBar {
        id: breadcrumbToolBar
        objectName: "breadcrumbToolBar"
        Layout.fillWidth: true
        visible: !headerArea.rootWindow.inWizardLayer && headerArea.rootWindow.breadcrumbRevision >= 0 && headerArea.pageStackRef.depth > 1
        height: visible ? implicitHeight : 0
        position: Controls.ToolBar.Header

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: Kirigami.Units.smallSpacing
            anchors.rightMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            Controls.ToolButton {
                id: breadcrumbBackButton
                objectName: "breadcrumbBackButton"
                visible: headerArea.rootWindow.canNavigateBackInBreadcrumb(headerArea.rootWindow.breadcrumbRevision)
                enabled: visible
                activeFocusOnTab: visible
                icon.name: "go-previous"
                text: Texts.label("back")
                display: Controls.AbstractButton.TextBesideIcon
                Accessible.name: Texts.label("back")
                onClicked: headerArea.rootWindow.handleGlobalEscape()

                FocusRing {
                    target: breadcrumbBackButton
                    z: 0
                }
            }

            Controls.ToolSeparator {
                visible: breadcrumbBackButton.visible
            }

            Repeater {
                model: headerArea.rootWindow.breadcrumbRevision >= 0 ? headerArea.pageStackRef.depth : 0

                delegate: RowLayout {
                    id: crumbDelegate
                    required property int index
                    spacing: Kirigami.Units.smallSpacing

                    readonly property Item pageItem: (headerArea.rootWindow.breadcrumbRevision >= 0 && index >= 0 && index < headerArea.pageStackRef.depth)
                        ? headerArea.pageStackRef.get(index)
                        : null
                    readonly property string crumbTitle: (pageItem && pageItem.title !== "")
                        ? pageItem.title
                        : headerArea.rootWindow.defaultTitleForDepth(index, headerArea.rootWindow.breadcrumbRevision)
                    readonly property bool isCurrentCrumb: headerArea.rootWindow.breadcrumbRevision >= 0 && index === headerArea.pageStackRef.depth - 1

                    Kirigami.Icon {
                        visible: crumbDelegate.index > 0
                        source: "go-next-symbolic"
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                        opacity: 0.65
                    }

                    Controls.ToolButton {
                        id: crumbButton
                        objectName: "breadcrumbButton_" + crumbDelegate.index
                        visible: !crumbDelegate.isCurrentCrumb
                        enabled: visible
                        activeFocusOnTab: visible
                        flat: true
                        text: crumbDelegate.crumbTitle
                        Accessible.name: crumbDelegate.crumbTitle
                        onClicked: headerArea.rootWindow.navigateToBreadcrumbIndex(crumbDelegate.index)

                        FocusRing {
                            target: crumbButton
                            z: 0
                        }
                    }

                    Controls.Label {
                        objectName: "breadcrumbCurrentLabel"
                        visible: crumbDelegate.isCurrentCrumb
                        text: crumbDelegate.crumbTitle
                        font.bold: true
                        leftPadding: Kirigami.Units.smallSpacing
                        rightPadding: Kirigami.Units.smallSpacing
                    }
                }
            }

            Item {
                Layout.fillWidth: true
            }
        }
    }
}
