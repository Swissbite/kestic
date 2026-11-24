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
    // The restore dialog's model; it follows the restore it started.
    required property var restoreModel

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

    Timer {
        interval: 1000
        repeat: true
        running: headerArea.restoreModel.run_state === "running"
        onTriggered: headerArea.restoreModel.pollRun()
    }

    // A restore is a run: while it runs, with progress text and «Cancel»; at the end its result,
    // with «Open Folder» after a successful one.
    LiteralMessage {
        id: restoreRunBanner
        objectName: "restoreRunBanner"
        Layout.fillWidth: true
        position: Kirigami.InlineMessage.Position.Header
        type: {
            switch (headerArea.restoreModel.run_state) {
            case "ok": return Kirigami.MessageType.Positive;
            case "warning": return Kirigami.MessageType.Warning;
            case "error": return Kirigami.MessageType.Error;
            default: return Kirigami.MessageType.Information;
            }
        }
        // Information keeps Kirigami's emblem; the running and cancelled restore show the icons
        // of their state, which exist in Breeze and Breeze Dark.
        icon.name: {
            switch (headerArea.restoreModel.run_state) {
            case "running": return "document-revert";
            case "cancelled": return "process-stop";
            default: return "";
            }
        }
        visible: headerArea.restoreModel.run_state !== ""
        showCloseButton: headerArea.restoreModel.run_state !== "running"
        plainText: headerArea.restoreModel.run_text
        onVisibleChanged: if (!visible && headerArea.restoreModel.run_state !== "") headerArea.restoreModel.dismissRun()
        actions: [
            Kirigami.Action {
                objectName: "restoreRunCancelAction"
                visible: headerArea.restoreModel.run_state === "running"
                icon.name: "dialog-cancel"
                text: Texts.label("cancel")
                onTriggered: headerArea.restoreModel.cancelRun()
            },
            Kirigami.Action {
                objectName: "restoreRunOpenFolderAction"
                visible: headerArea.restoreModel.run_state === "ok"
                icon.name: "document-open-folder"
                text: i18n("Open Folder")
                onTriggered: Qt.openUrlExternally("file://"
                    + headerArea.restoreModel.run_target.split("/").map(encodeURIComponent).join("/"))
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

    // Tabs (wide layout only) and, at the right end, the ☰ button of the main menu.
    RowLayout {
        id: topRow
        Layout.fillWidth: true
        spacing: 0
        visible: !headerArea.rootWindow.inWizardLayer

        Kirigami.NavigationTabBar {
            id: topNavBar
            objectName: "topNavBar"
            Layout.fillWidth: true
            visible: !headerArea.rootWindow.narrowMode
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
            id: mainMenuBar
            Layout.fillWidth: !topNavBar.visible
            Layout.fillHeight: true
            position: Controls.ToolBar.Header

            RowLayout {
                anchors.fill: parent

                Item {
                    Layout.fillWidth: true
                }

                Controls.ToolButton {
                    id: mainMenuButton
                    objectName: "mainMenuButton"
                    icon.name: "application-menu"
                    text: i18n("Main Menu")
                    display: Controls.AbstractButton.IconOnly
                    down: pressed || headerArea.rootWindow.mainMenuOpen
                    Accessible.name: text
                    Accessible.role: Accessible.ButtonMenu
                    Controls.ToolTip.text: text
                    Controls.ToolTip.visible: hovered && !headerArea.rootWindow.mainMenuOpen
                    Controls.ToolTip.delay: Kirigami.Units.toolTipDelay
                    onClicked: headerArea.rootWindow.openMainMenu(mainMenuButton)

                    FocusRing {
                        target: mainMenuButton
                        z: 0
                    }
                }
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

                    // Only the current crumb may become narrower than its text (it elides then).
                    Layout.fillWidth: isCurrentCrumb

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
                        id: currentCrumbLabel
                        objectName: "breadcrumbCurrentLabel"
                        visible: crumbDelegate.isCurrentCrumb
                        text: crumbDelegate.crumbTitle
                        font.bold: true
                        // Elides only when the bar is too narrow for the whole trail. The width of
                        // the whole text comes from TextMetrics: the label's implicit width
                        // shrinks with the elided text.
                        elide: Text.ElideRight
                        leftPadding: Kirigami.Units.smallSpacing
                        rightPadding: Kirigami.Units.smallSpacing
                        readonly property real fullWidth: Math.ceil(currentCrumbText.advanceWidth) + leftPadding + rightPadding
                        Layout.fillWidth: true
                        Layout.minimumWidth: Math.min(fullWidth, Kirigami.Units.gridUnit * 2)
                        Layout.preferredWidth: fullWidth
                        Layout.maximumWidth: fullWidth

                        TextMetrics {
                            id: currentCrumbText
                            font: currentCrumbLabel.font
                            text: currentCrumbLabel.text
                        }
                    }
                }
            }

            Item {
                Layout.fillWidth: true
            }
        }
    }
}
