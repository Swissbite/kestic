// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ApplicationWindow {
    id: settingsWindow
    objectName: "settingsWindow"
    title: i18n("Configure Kestic")
    width: 720
    height: 640
    minimumWidth: 520
    minimumHeight: 460
    visible: false

    required property var appBackend

    property string pendingProviderKind: "system"
    property string pendingCustomPath: ""

    function saveSettingsFromUi() {
        appBackend.updateSettings(
            defaultOverdueSpinBox.value,
            notifErrorSwitch.checked,
            notifWarningSwitch.checked,
            notifOverdueSwitch.checked,
            notifMaintenanceSwitch.checked,
            notifResticUpdateSwitch.checked
        );
    }

    onClosing: function(close) {
        close.accepted = false;
        visible = false;
    }

    // Opens the window; with `section` "restic" it scrolls to the restic section.
    function openWindow(section) {
        visible = true;
        if (Qt.platform.pluginName !== "offscreen") {
            raise();
        }
        requestActivate();
        if (section === "restic") {
            Qt.callLater(showResticSection);
        }
    }

    // The restic section is the last one: scrolling to the end shows it.
    function showResticSection() {
        var page = pageStack.initialPage;
        var flick = page ? page.flickable : null;
        if (!flick) return;
        flick.contentY = Math.max(0, flick.contentHeight - flick.height);
    }

    // Opens the restic section and starts the download offered there.
    function downloadRestic() {
        openWindow("restic");
        if (appBackend.restic_offer_download && appBackend.restic_offer_enabled) {
            appBackend.startResticInstall();
        }
    }

    // Polls the installation while it runs, also while this window is hidden.
    Timer {
        objectName: "resticInstallPollTimer"
        interval: 500
        repeat: true
        running: appBackend.restic_install_state === "running"
        onTriggered: appBackend.pollResticInstall()
    }

    function canActivateFocusedControlWithReturn() {
        var it = settingsWindow.activeFocusItem;
        if (!it) return false;
        if (it.hasOwnProperty("textDocument") || it.hasOwnProperty("acceptableInput")
                || it.hasOwnProperty("popup") || it.hasOwnProperty("highlightedIndex")
                || it.hasOwnProperty("subMenu")) {
            return false;
        }
        if (it.Accessible && (it.Accessible.role === Accessible.MenuItem
                || it.Accessible.role === Accessible.ComboBox
                || it.Accessible.role === Accessible.EditableText)) {
            return false;
        }
        return typeof it.animateClick === "function"
            || (typeof it.clicked === "function" && typeof it.down !== "undefined");
    }

    function activateFocusedControlWithReturn() {
        var it = settingsWindow.activeFocusItem;
        if (!canActivateFocusedControlWithReturn()) return;
        if (it.checkable && typeof it.animateClick === "function") {
            it.animateClick();
        } else if (typeof it.clicked === "function") {
            it.clicked();
        } else if (typeof it.animateClick === "function") {
            it.animateClick();
        }
    }

    function handleEscape() {
        if (resticSourceDialog.opened || resticSourceDialog.visible) {
            resticSourceDialog.close();
            return true;
        }
        settingsWindow.close();
        return true;
    }

    Kirigami.Dialog {
        id: resticSourceDialog
        objectName: "resticSourceDialog"
        title: i18n("Change restic Source")
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing

        customFooterActions: [
            Kirigami.Action {
                id: applyResticSourceAction
                objectName: "applyResticSourceAction"
                icon.name: "dialog-ok-apply"
                text: i18n("Apply")
                enabled: settingsWindow.pendingProviderKind === "system" || settingsWindow.pendingCustomPath.trim().length > 0
                onTriggered: {
                    if (appBackend.setResticProvider(settingsWindow.pendingProviderKind, settingsWindow.pendingCustomPath)) {
                        resticSourceDialog.close();
                    }
                }
            }
        ]

        ColumnLayout {
            spacing: Kirigami.Units.mediumSpacing
            implicitWidth: Kirigami.Units.gridUnit * 24

            Controls.RadioButton {
                text: i18n("System restic (from PATH)")
                checked: settingsWindow.pendingProviderKind === "system"
                Accessible.name: text
                onToggled: if (checked) settingsWindow.pendingProviderKind = "system"
            }

            Controls.RadioButton {
                text: i18n("Custom program path")
                checked: settingsWindow.pendingProviderKind === "custom"
                Accessible.name: text
                onToggled: if (checked) settingsWindow.pendingProviderKind = "custom"
            }

            Controls.TextField {
                id: customResticPathField
                objectName: "customResticPathField"
                enabled: settingsWindow.pendingProviderKind === "custom"
                text: settingsWindow.pendingCustomPath
                onTextChanged: settingsWindow.pendingCustomPath = text
                onAccepted: {
                    if (settingsWindow.pendingProviderKind === "system" || settingsWindow.pendingCustomPath.trim().length > 0) {
                        if (appBackend.setResticProvider(settingsWindow.pendingProviderKind, settingsWindow.pendingCustomPath)) {
                            resticSourceDialog.close();
                        }
                    }
                }
                placeholderText: "/usr/local/bin/restic"
                Accessible.name: i18n("Custom restic program path")
                Layout.fillWidth: true
            }

            Controls.Label {
                visible: appBackend.last_action_error !== ""
                text: appBackend.last_action_error
                color: Kirigami.Theme.negativeTextColor
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    pageStack.initialPage: Kirigami.ScrollablePage {
        title: i18n("Configure Kestic")

        Kirigami.FormLayout {
            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Appearance")
            }

            Controls.Switch {
                id: expertModeSwitch
                objectName: "expertModeSwitch"
                Kirigami.FormData.label: i18n("Expert mode:")
                checked: appBackend.expert_mode
                Accessible.name: i18n("Expert mode")
                onToggled: appBackend.updateExpertMode(checked)
            }

            Controls.Label {
                text: i18n("Shows technical identifiers, snapshot tags, and expert options.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Monitoring & Notifications")
            }

            Controls.SpinBox {
                id: defaultOverdueSpinBox
                objectName: "defaultOverdueSpinBox"
                Kirigami.FormData.label: i18n("Mark overdue after missed runs:")
                from: 1
                to: 99
                value: appBackend.default_overdue_threshold
                Accessible.name: i18n("Mark overdue after missed runs")
                onValueModified: settingsWindow.saveSettingsFromUi()
            }

            Controls.Label {
                text: i18n("A scheduled backup counts as overdue when this many scheduled runs in a row did not happen.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Switch {
                id: notifErrorSwitch
                objectName: "notifErrorSwitch"
                Kirigami.FormData.label: i18n("Notify on backup errors:")
                checked: appBackend.notif_on_error
                Accessible.name: i18n("Notify on backup errors")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifWarningSwitch
                objectName: "notifWarningSwitch"
                Kirigami.FormData.label: i18n("Notify on warnings:")
                checked: appBackend.notif_on_warning
                Accessible.name: i18n("Notify on warnings")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifOverdueSwitch
                objectName: "notifOverdueSwitch"
                Kirigami.FormData.label: i18n("Notify when backup is overdue:")
                checked: appBackend.notif_on_overdue
                Accessible.name: i18n("Notify when backup is overdue")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifMaintenanceSwitch
                objectName: "notifMaintenanceSwitch"
                Kirigami.FormData.label: i18n("Notify on maintenance failures:")
                checked: appBackend.notif_on_maintenance_failed
                Accessible.name: i18n("Notify on maintenance failures")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifResticUpdateSwitch
                objectName: "notifResticUpdateSwitch"
                Kirigami.FormData.label: i18n("Notify on restic updates:")
                checked: appBackend.notif_on_restic_update
                Accessible.name: i18n("Notify on restic updates")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Background Service")
            }

            Controls.Label {
                Kirigami.FormData.label: Texts.label("status_label")
                text: appBackend.daemon_reachable
                    ? i18n("Running · starts when you log in")
                    : i18n("Not running")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Label {
                text: i18n("Closing the window does not quit Kestic; backups keep running. To quit, use the Kestic icon in the system tray.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("restic")
            }

            Controls.Label {
                Kirigami.FormData.label: i18n("restic source:")
                text: appBackend.restic_valid
                    ? i18n("%1 (%2 · %3)", appBackend.restic_provider_kind, appBackend.restic_version, appBackend.restic_binary_path)
                    : appBackend.restic_message
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Label {
                text: i18n("Kestic needs restic 0.18.0 or newer.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            RowLayout {
                spacing: Kirigami.Units.smallSpacing

                Controls.Button {
                    id: changeResticSourceButton
                    objectName: "changeResticSourceButton"
                    text: i18n("Change restic Source…")
                    Accessible.name: i18n("Change restic Source")
                    onClicked: {
                        settingsWindow.pendingProviderKind = appBackend.restic_provider_kind === "custom" ? "custom" : "system";
                        settingsWindow.pendingCustomPath = appBackend.restic_binary_path;
                        resticSourceDialog.open();
                    }
                }

                Controls.Button {
                    id: downloadResticButton
                    objectName: "downloadResticButton"
                    visible: appBackend.restic_offer_download
                    enabled: appBackend.restic_offer_enabled
                    icon.name: "download"
                    text: i18n("Download restic")
                    Accessible.name: text
                    onClicked: appBackend.startResticInstall()
                }

                Controls.Button {
                    id: updateResticButton
                    objectName: "updateResticButton"
                    visible: appBackend.restic_offer_update
                    enabled: appBackend.restic_offer_enabled
                    icon.name: "update-none"
                    text: i18n("Update Now")
                    Accessible.name: i18n("Update restic now")
                    onClicked: appBackend.startResticInstall()
                }
            }

            // Progress, result or error of the last installation of the restic managed by Kestic.
            ColumnLayout {
                id: resticInstallPanel
                objectName: "resticInstallPanel"
                spacing: Kirigami.Units.smallSpacing
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26

                readonly property string installState: appBackend.restic_install_state
                readonly property bool running: installState === "running"
                property bool detailsExpanded: false

                visible: installState === "running" || installState === "done" || installState === "error"

                onInstallStateChanged: detailsExpanded = false

                Controls.Label {
                    objectName: "resticInstallPhaseLabel"
                    visible: resticInstallPanel.running
                    text: appBackend.restic_install_phase_text
                    textFormat: Text.PlainText
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                RowLayout {
                    visible: resticInstallPanel.running
                    spacing: Kirigami.Units.smallSpacing
                    Layout.fillWidth: true

                    Controls.ProgressBar {
                        objectName: "resticInstallProgressBar"
                        from: 0
                        to: 1
                        indeterminate: appBackend.restic_install_progress < 0
                        value: Math.max(0, appBackend.restic_install_progress)
                        Accessible.name: appBackend.restic_install_phase_text
                        Layout.fillWidth: true
                    }

                    Controls.Button {
                        objectName: "cancelResticInstallButton"
                        icon.name: "dialog-cancel"
                        text: Texts.label("cancel")
                        enabled: appBackend.restic_install_cancellable
                        Accessible.name: text
                        onClicked: appBackend.cancelResticInstall()
                    }
                }

                Kirigami.InlineMessage {
                    objectName: "resticInstallResultMessage"
                    type: Kirigami.MessageType.Positive
                    visible: resticInstallPanel.installState === "done"
                    text: appBackend.restic_install_result
                    Layout.fillWidth: true
                }

                Kirigami.InlineMessage {
                    objectName: "resticInstallErrorMessage"
                    type: Kirigami.MessageType.Error
                    visible: resticInstallPanel.installState === "error"
                    text: appBackend.restic_install_error
                    Layout.fillWidth: true
                    actions: [
                        Kirigami.Action {
                            objectName: "resticInstallDetailsAction"
                            visible: appBackend.restic_install_error_detail !== ""
                            icon.name: resticInstallPanel.detailsExpanded ? "go-up" : "go-down"
                            text: resticInstallPanel.detailsExpanded ? Texts.label("hide_details") : Texts.label("details")
                            onTriggered: resticInstallPanel.detailsExpanded = !resticInstallPanel.detailsExpanded
                        }
                    ]
                }

                // Raw text from the network or from gpgv: shown as plain text, never as markup.
                Controls.Label {
                    objectName: "resticInstallErrorDetail"
                    visible: resticInstallPanel.installState === "error" && resticInstallPanel.detailsExpanded
                        && appBackend.restic_install_error_detail !== ""
                    text: appBackend.restic_install_error_detail
                    textFormat: Text.PlainText
                    font.family: "monospace"
                    wrapMode: Text.WrapAnywhere
                    Layout.fillWidth: true
                }
            }
        }
    }
}
