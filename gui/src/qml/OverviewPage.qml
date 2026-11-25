// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "overviewPage"
    title: Texts.label("overview")

    required property var rootWindow
    required property var appBackend
    required property var backupsModel

    actions: [
        Kirigami.Action {
            id: backUpAllAction
            objectName: "backUpAllAction"
            icon.name: "media-playback-start"
            text: i18n("Back Up All Now")
            visible: appBackend.has_config_file && appBackend.backups_count > 0
            enabled: appBackend.blocking_kind === ""
            tooltip: appBackend.blocking_kind !== ""
                ? appBackend.blocking_message
                : (appBackend.back_up_all_hint !== "" ? appBackend.back_up_all_hint : i18n("Start all available backups"))
            onTriggered: appBackend.startAllBackups()
        },
        Kirigami.Action {
            id: pauseSchedulesAction
            objectName: "pauseSchedulesAction"
            icon.name: appBackend.global_paused ? "media-playback-start" : "state-pause"
            text: appBackend.global_paused ? Texts.label("resume_schedules") : Texts.label("pause_schedules")
            visible: appBackend.has_config_file && appBackend.backups_count > 0
            enabled: appBackend.blocking_kind !== "daemon_unreachable"
            tooltip: appBackend.global_paused ? appBackend.global_paused_line : i18n("Temporarily pause scheduled backups")
            onTriggered: appBackend.global_paused ? appBackend.resumeGlobal() : globalPauseMenu.popup()
        }
    ]

    Controls.Menu {
        id: globalPauseMenu
        objectName: "globalPauseMenu"

        Controls.MenuItem {
            objectName: "pauseGlobal1hItem"
            text: Texts.label("pause_one_hour")
            Accessible.name: text
            onTriggered: appBackend.pauseGlobal("1h")
        }
        Controls.MenuItem {
            objectName: "pauseGlobal3hItem"
            text: Texts.label("pause_three_hours")
            Accessible.name: text
            onTriggered: appBackend.pauseGlobal("3h")
        }
        Controls.MenuItem {
            objectName: "pauseGlobalTomorrowItem"
            text: Texts.label("pause_until_tomorrow")
            Accessible.name: text
            onTriggered: appBackend.pauseGlobal("tomorrow")
        }
        Controls.MenuItem {
            objectName: "pauseGlobalIndefiniteItem"
            text: Texts.label("pause_until_resume")
            Accessible.name: text
            onTriggered: appBackend.pauseGlobal("indefinite")
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing

        // Welcome view when no config exists
        ColumnLayout {
            id: welcomeView
            objectName: "welcomeView"
            visible: !appBackend.has_config_file || appBackend.backups_count === 0
            Layout.fillWidth: true
            Layout.topMargin: rootWindow.narrowMode ? Kirigami.Units.smallSpacing : Kirigami.Units.gridUnit * 2
            spacing: rootWindow.narrowMode ? Kirigami.Units.mediumSpacing : Kirigami.Units.largeSpacing

            Kirigami.Icon {
                source: "kestic"
                fallback: "kestic-symbolic"
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: rootWindow.narrowMode ? 72 : 96
                Layout.preferredHeight: rootWindow.narrowMode ? 72 : 96
            }

            Kirigami.Heading {
                level: 1
                text: i18n("Welcome to Kestic")
                horizontalAlignment: Text.AlignHCenter
                Layout.alignment: Qt.AlignHCenter
                Layout.fillWidth: true
            }

            Controls.Label {
                text: i18n("Kestic encrypts your files and backs them up regularly to a drive, a server, or the cloud.")
                wrapMode: Text.WordWrap
                horizontalAlignment: Text.AlignHCenter
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumWidth: Kirigami.Units.gridUnit * 28
                Layout.fillWidth: true
            }

            Controls.Button {
                id: setupFirstBackupButton
                objectName: "setupFirstBackupButton"
                text: Texts.label("set_up_first_backup")
                icon.name: "list-add"
                highlighted: true
                enabled: appBackend.daemon_reachable && appBackend.blocking_kind !== "daemon_unreachable"
                Accessible.name: Texts.label("set_up_first_backup")
                Layout.alignment: Qt.AlignHCenter
                onClicked: rootWindow.openNewBackupWizard(true)
            }

            Controls.ToolButton {
                id: connectExistingButton
                objectName: "connectExistingButton"
                text: i18n("Connect Existing Backup…")
                enabled: appBackend.daemon_reachable && appBackend.blocking_kind === ""
                Accessible.name: i18n("Connect Existing Backup")
                Layout.alignment: Qt.AlignHCenter
                onClicked: rootWindow.openAddDestinationWizard(null, "connect")
            }
        }

        // Populated overview
        ColumnLayout {
            id: overviewContent
            objectName: "overviewContent"
            visible: appBackend.has_config_file && appBackend.backups_count > 0
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            Kirigami.InlineMessage {
                id: startAllSkippedBanner
                objectName: "startAllSkippedBanner"
                Layout.fillWidth: true
                type: Kirigami.MessageType.Information
                visible: appBackend.start_all_skipped_message !== ""
                showCloseButton: true
                text: appBackend.start_all_skipped_message
                onVisibleChanged: if (!visible && appBackend.start_all_skipped_message !== "") appBackend.dismissStartAllSkipped()
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.largeSpacing

                Kirigami.Icon {
                    source: appBackend.overall_icon !== "" ? appBackend.overall_icon : "state-ok"
                    color: rootWindow.iconColorFor(appBackend.overall_color_kind, page.Kirigami.Theme)
                    Layout.preferredWidth: Kirigami.Units.iconSizes.large
                    Layout.preferredHeight: Kirigami.Units.iconSizes.large
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.Heading {
                        id: overallHeadlineLabel
                        objectName: "overallHeadlineLabel"
                        level: 1
                        text: appBackend.overall_headline
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    Controls.Label {
                        objectName: "backUpAllHintLabel"
                        visible: appBackend.back_up_all_hint !== ""
                        text: appBackend.back_up_all_hint
                        color: Kirigami.Theme.textColor
                        opacity: 0.8
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }

                    Controls.Label {
                        visible: appBackend.overall_subtitle !== ""
                        text: appBackend.overall_subtitle
                        color: Kirigami.Theme.textColor
                        opacity: 0.8
                        wrapMode: Text.WordWrap
                        Layout.fillWidth: true
                    }
                }
            }

            Kirigami.CardsLayout {
                objectName: "overviewCardsLayout"
                Layout.fillWidth: true

                // Not a list: the backup cards, a card grid; each card is a Tab stop of its own.
                Repeater {
                    model: backupsModel

                    delegate: Kirigami.Card {
                        id: card
                        objectName: "overviewBackupCard"
                        required property string backupId
                        required property string name
                        required property string sourcePath
                        required property string sourceKind
                        required property string destinationId
                        required property string destinationName
                        required property string statusText
                        required property string statusIcon
                        required property string statusColorKind
                        required property string availabilityReason
                        required property bool available
                        required property bool hasError
                        required property bool isRunning
                        required property string activeRunId
                        required property double activePercent
                        required property string activeProgressText
                        required property bool activeScanning
                        required property string errorAction
                        required property string nextScheduledAtText
                        required property string pendingReasonsTooltip

                        // Without banner and actions: Kirigami's empty banner has a negative height
                        // and pulled the content onto the top border, its empty action bar added
                        // space below.
                        header: null
                        footer: null
                        showClickFeedback: true
                        activeFocusOnTab: true
                        Accessible.name: card.name
                        onClicked: rootWindow.openBackupDetailById(backupId)
                        Keys.onSpacePressed: function(event) {
                            event.accepted = true;
                            rootWindow.openBackupDetailById(card.backupId);
                        }

                        FocusRing {
                            target: card
                        }

                        contentItem: ColumnLayout {
                            spacing: Kirigami.Units.smallSpacing

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Kirigami.Units.smallSpacing

                                Kirigami.Icon {
                                    source: card.sourceKind === "drive" ? "drive-harddisk" : "folder"
                                    Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                                    Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 2

                                    Kirigami.Heading {
                                        level: 4
                                        text: card.name
                                        wrapMode: Text.WordWrap
                                        Layout.fillWidth: true
                                    }

                                    Controls.Label {
                                        text: i18n("%1 → %2", card.sourcePath, card.destinationName)
                                        opacity: 0.75
                                        elide: Text.ElideMiddle
                                        Layout.fillWidth: true
                                    }
                                }
                            }

                            Item {
                                Layout.fillWidth: true
                                implicitHeight: cardStatusRow.implicitHeight

                                RowLayout {
                                    id: cardStatusRow
                                    anchors.fill: parent
                                    spacing: Kirigami.Units.smallSpacing

                                    Kirigami.Icon {
                                        source: card.statusIcon
                                        color: rootWindow.iconColorFor(card.statusColorKind, page.Kirigami.Theme)
                                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                                    }

                                    Controls.Label {
                                        text: card.statusText
                                        color: Kirigami.Theme.textColor
                                        wrapMode: Text.WordWrap
                                        Layout.fillWidth: true
                                    }
                                }

                                MouseArea {
                                    id: statusHoverArea
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    acceptedButtons: Qt.NoButton
                                }

                                Controls.ToolTip.visible: statusHoverArea.containsMouse && card.pendingReasonsTooltip !== ""
                                Controls.ToolTip.text: card.pendingReasonsTooltip
                            }

                            Controls.Label {
                                visible: !card.isRunning && card.nextScheduledAtText !== ""
                                text: card.nextScheduledAtText
                                opacity: 0.7
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }

                            Controls.ProgressBar {
                                // Fusion leaves the bar 0 px high and draws nothing; 6 px is the bar of Breeze.
                                Layout.minimumHeight: topInset + bottomInset + Math.round(Kirigami.Units.smallSpacing * 1.5)
                                objectName: "backupCardProgressBar"
                                visible: card.isRunning
                                from: 0
                                to: 100
                                indeterminate: card.activeScanning
                                value: card.activePercent
                                Layout.fillWidth: true
                            }

                            Controls.Label {
                                visible: card.isRunning && card.activeProgressText !== ""
                                text: card.activeProgressText
                                opacity: 0.8
                                Layout.fillWidth: true
                            }

                            // A card stretched to a taller neighbour keeps its rows together and
                            // its buttons at the bottom.
                            Item {
                                Layout.fillHeight: true
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Kirigami.Units.smallSpacing

                                Controls.Button {
                                    objectName: "overviewBackUpNowButton"
                                    visible: !card.isRunning && !(card.hasError && card.errorAction === "update_stored_password")
                                    icon.name: "media-playback-start"
                                    text: Texts.label("back_up_now")
                                    enabled: card.available && appBackend.blocking_kind === ""
                                    Accessible.name: i18n("Back Up Now: %1", card.name)
                                    Controls.ToolTip.visible: hovered && !enabled
                                    Controls.ToolTip.text: appBackend.blocking_kind !== ""
                                        ? appBackend.blocking_message
                                        : card.availabilityReason
                                    onClicked: appBackend.startBackup(card.backupId)
                                }

                                Controls.Button {
                                    objectName: "overviewUpdatePasswordButton"
                                    visible: !card.isRunning && card.hasError && card.errorAction === "update_stored_password"
                                    icon.name: "dialog-password"
                                    text: appBackend.recoveryActionLabel("update_stored_password")
                                    enabled: appBackend.blocking_kind === ""
                                    Accessible.name: i18n("Update Saved Password for %1", card.name)
                                    onClicked: rootWindow.openUpdatePasswordDialog(card.destinationId, card.destinationName)
                                }

                                Controls.Button {
                                    visible: card.isRunning
                                    icon.name: "process-stop"
                                    text: Texts.label("cancel")
                                    Accessible.name: i18n("Cancel backup %1", card.name)
                                    onClicked: appBackend.cancelRun(card.activeRunId)
                                }

                                Item { Layout.fillWidth: true }

                                Controls.ToolButton {
                                    icon.name: "overflow-menu"
                                    Accessible.name: i18n("More actions for %1", card.name)
                                    onClicked: cardMenu.popup()

                                    Controls.Menu {
                                        id: cardMenu
                                        Controls.MenuItem {
                                            objectName: "overviewCardBackUpMenuItem"
                                            text: (card.hasError && card.errorAction === "update_stored_password")
                                                ? Texts.label("back_up_now_anyway")
                                                : Texts.label("back_up_now")
                                            enabled: card.available && !card.isRunning && appBackend.blocking_kind === ""
                                            onTriggered: appBackend.startBackup(card.backupId)
                                        }
                                        Controls.MenuItem {
                                            text: Texts.label("show_command")
                                            onTriggered: rootWindow.openShowCommandDialog(
                                                appBackend.getBackupTransparentCommand(card.backupId),
                                                card.name
                                            )
                                        }
                                        Controls.MenuItem {
                                            text: Texts.label("rename_ellipsis")
                                            enabled: appBackend.blocking_kind !== "daemon_unreachable"
                                            onTriggered: rootWindow.openRenameBackupDialog(card.backupId, card.name)
                                        }
                                        Controls.MenuItem {
                                            text: Texts.label("remove_from_kestic_ellipsis")
                                            enabled: appBackend.blocking_kind !== "daemon_unreachable"
                                            onTriggered: rootWindow.openRemoveBackupDialog(card.backupId, card.name)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
