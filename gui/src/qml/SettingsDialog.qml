// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

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

    function openWindow() {
        visible = true;
        if (Qt.platform.pluginName !== "offscreen") {
            raise();
        }
        requestActivate();
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
                text: i18n("Custom executable path")
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
                Accessible.name: i18n("Custom restic executable path")
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
                text: i18n("Shows technical identifiers, snapshot tags, and advanced fields.")
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
                Kirigami.FormData.label: i18n("Mark overdue after missed slots:")
                from: 1
                to: 99
                value: appBackend.default_overdue_threshold
                Accessible.name: i18n("Mark overdue after missed slots")
                onValueModified: settingsWindow.saveSettingsFromUi()
            }

            Controls.Label {
                text: i18n("A scheduled backup is shown as overdue when at least this many scheduled runs could not execute in a row.")
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
                Kirigami.FormData.label: i18n("Status:")
                text: appBackend.daemon_reachable
                    ? i18n("Running · starts with session")
                    : i18n("Not running")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Label {
                text: i18n("Closing the window does not exit Kestic; backups continue running. To quit, use the Kestic icon in the system tray.")
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
                text: i18n("Kestic requires restic 0.18.0 or newer for reliable JSON progress and lock handling.")
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
                    id: updateResticButton
                    objectName: "updateResticButton"
                    text: i18n("Update Now")
                    enabled: false
                    Accessible.name: i18n("Update restic now")
                    Controls.ToolTip.visible: hovered
                    Controls.ToolTip.text: i18n("App-managed restic updates will arrive in a future version.")
                }
            }
        }
    }
}
