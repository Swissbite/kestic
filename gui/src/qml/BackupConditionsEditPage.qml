// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "backupConditionsEditPage"
    title: i18n("Run Conditions")

    required property var rootWindow
    required property var appBackend
    required property string backupId

    property var initialData: null
    property string savedConditionsJson: "{}"
    property bool hydrating: false

    property bool requireAcPower: true
    property bool requireNetwork: true
    property bool skipOnMetered: false
    property var allowedNetworks: []
    property var knownNetworks: []
    // Network conditions only apply to network destinations, not to a local folder or drive
    // (decision 2026-11-20).
    readonly property bool networkDestination: {
        var dest = initialData ? rootWindow.findDestinationData(initialData.destinationId) : null;
        return dest ? dest.kindName !== "local" : true;
    }

    function currentConditionsObj() {
        return {
            require_ac_power: requireAcPower,
            require_network: networkDestination && requireNetwork,
            skip_on_metered_network: networkDestination && skipOnMetered,
            allowed_networks: networkDestination ? allowedNetworks.slice() : []
        };
    }

    function currentConditionsJson() {
        return JSON.stringify(currentConditionsObj());
    }

    function normalizedConditionsJson(rawJson) {
        try {
            var obj = JSON.parse(rawJson || "{}");
            var nets = (obj.allowed_networks || []).slice();
            var net = page.networkDestination;
            return JSON.stringify({
                require_ac_power: obj.require_ac_power !== undefined ? Boolean(obj.require_ac_power) : true,
                require_network: net && (obj.require_network !== undefined ? Boolean(obj.require_network) : true),
                skip_on_metered_network: net && Boolean(obj.skip_on_metered_network),
                allowed_networks: net ? nets : []
            });
        } catch (e) {
            return "{}";
        }
    }

    readonly property bool dirty: initialData !== null
        && normalizedConditionsJson(currentConditionsJson()) !== normalizedConditionsJson(savedConditionsJson)

    onDirtyChanged: appBackend.has_unsaved_changes = dirty

    function loadKnownNetworks() {
        var list = [];
        try {
            list = JSON.parse(appBackend.listKnownNetworksJson() || "[]");
        } catch (e) {
            list = [];
        }
        var merged = list.slice();
        for (var i = 0; i < allowedNetworks.length; ++i) {
            if (merged.indexOf(allowedNetworks[i]) < 0) {
                merged.push(allowedNetworks[i]);
            }
        }
        knownNetworks = merged;
    }

    function loadFromSaved(condJson) {
        hydrating = true;
        var obj = {};
        try {
            obj = JSON.parse(condJson || "{}");
        } catch (e) {
            obj = {};
        }
        requireAcPower = obj.require_ac_power !== undefined ? Boolean(obj.require_ac_power) : true;
        requireNetwork = obj.require_network !== undefined ? Boolean(obj.require_network) : true;
        skipOnMetered = Boolean(obj.skip_on_metered_network);
        allowedNetworks = (obj.allowed_networks || []).slice();
        hydrating = false;
        loadKnownNetworks();
    }

    function toggleAllowedNetwork(ssid, enabled) {
        var next = allowedNetworks.slice();
        var idx = next.indexOf(ssid);
        if (enabled && idx < 0) {
            next.push(ssid);
        } else if (!enabled && idx >= 0) {
            next.splice(idx, 1);
        }
        allowedNetworks = next;
    }

    function addManualNetwork(raw) {
        var trimmed = (raw || "").trim();
        if (trimmed === "") return;
        if (allowedNetworks.indexOf(trimmed) < 0) {
            var next = allowedNetworks.slice();
            next.push(trimmed);
            allowedNetworks = next;
        }
        if (knownNetworks.indexOf(trimmed) < 0) {
            var kNext = knownNetworks.slice();
            kNext.push(trimmed);
            knownNetworks = kNext;
        }
        newNetworkField.text = "";
    }

    function commitSave(popOnSuccess) {
        if (!dirty) return false;
        if (appBackend.updateBackupConditions(backupId, currentConditionsJson())) {
            initialData = rootWindow.findBackupData(backupId);
            if (initialData) {
                savedConditionsJson = initialData.conditionsJson;
                loadFromSaved(savedConditionsJson);
            }
            rootWindow.showPassiveNotification(i18n("Run conditions saved"));
            if (popOnSuccess) {
                rootWindow.popSubPage();
            }
            return true;
        }
        return false;
    }

    Component.onCompleted: {
        initialData = rootWindow.findBackupData(backupId);
        if (initialData) {
            savedConditionsJson = initialData.conditionsJson;
            loadFromSaved(savedConditionsJson);
        } else {
            loadFromSaved("{}");
        }
    }

    onBackRequested: function(event) {
        if (dirty) {
            event.accepted = true;
            unsavedDialog.open();
        }
    }

    function handleEscape() {
        if (unsavedDialog.opened || unsavedDialog.visible) {
            unsavedDialog.close();
            return true;
        }
        if (dirty) {
            unsavedDialog.open();
            return true;
        }
        return false;
    }

    actions: [
        Kirigami.Action {
            id: saveConditionsAction
            objectName: "saveConditionsAction"
            icon.name: "document-save"
            text: i18n("Save")
            enabled: page.dirty && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.commitSave(false)
        },
        Kirigami.Action {
            id: closeConditionsAction
            objectName: "closeConditionsAction"
            icon.name: "window-close"
            text: i18n("Close")
            onTriggered: {
                if (page.dirty) {
                    unsavedDialog.open();
                } else {
                    rootWindow.popSubPage();
                }
            }
        }
    ]

    UnsavedChangesDialog {
        id: unsavedDialog
        objectName: "unsavedConditionsDialog"
        subtitle: i18n("You have unsaved changes to run conditions.")
        onSaveRequested: page.commitSave(true)
        onDiscardRequested: {
            appBackend.has_unsaved_changes = false;
            page.loadFromSaved(page.savedConditionsJson);
            rootWindow.popSubPage();
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: appBackend.last_action_error !== ""
            text: appBackend.last_action_error
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: page.networkDestination ? i18n("Power & Network") : i18n("Power")
            }

            Controls.CheckBox {
                id: requireAcCheck
                objectName: "requireAcCheck"
                Kirigami.FormData.label: i18n("Power:")
                text: i18n("Only on AC power (skip on battery)")
                checked: page.requireAcPower
                Accessible.name: text
                onToggled: if (!page.hydrating) page.requireAcPower = checked
            }

            Controls.CheckBox {
                id: requireNetworkCheck
                objectName: "requireNetworkCheck"
                visible: page.networkDestination
                Kirigami.FormData.label: i18n("Network:")
                text: i18n("Require network connection")
                checked: page.requireNetwork
                Accessible.name: text
                onToggled: if (!page.hydrating) page.requireNetwork = checked
            }

            Controls.CheckBox {
                id: skipMeteredCheck
                objectName: "skipMeteredCheck"
                visible: page.networkDestination
                text: i18n("Skip on metered network (e.g. mobile hotspot)")
                checked: page.skipOnMetered
                enabled: page.requireNetwork
                Accessible.name: text
                onToggled: if (!page.hydrating) page.skipOnMetered = checked
            }
        }

        Controls.Label {
            objectName: "localDestinationNetworkHint"
            visible: !page.networkDestination
            text: i18n("Network conditions do not apply: the destination is a local folder or drive.")
            opacity: 0.75
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            enabled: page.requireNetwork
            visible: page.networkDestination

            Kirigami.Heading {
                level: 4
                text: i18n("Allowed Wi-Fi Networks (optional)")
            }

            Controls.Label {
                text: page.allowedNetworks.length === 0
                    ? i18n("Leave all unchecked to allow any network.")
                    : i18n("Scheduled backups will only run when connected to one of the selected Wi-Fi networks.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Repeater {
                model: page.knownNetworks
                delegate: Controls.CheckBox {
                    required property string modelData
                    objectName: "knownNetworkCheck_" + modelData
                    text: modelData
                    checked: page.allowedNetworks.indexOf(modelData) >= 0
                    Accessible.name: i18n("Allow network %1", modelData)
                    onToggled: page.toggleAllowedNetwork(modelData, checked)
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Controls.TextField {
                    id: newNetworkField
                    objectName: "newNetworkField"
                    placeholderText: i18n("Add Wi-Fi name (SSID)…")
                    Accessible.name: i18n("Add Wi-Fi network SSID")
                    Layout.fillWidth: true
                    onAccepted: page.addManualNetwork(text)
                }

                Controls.Button {
                    id: addNetworkButton
                    objectName: "addNetworkButton"
                    icon.name: "list-add"
                    text: i18n("Add")
                    enabled: newNetworkField.text.trim().length > 0
                    Accessible.name: i18n("Add Wi-Fi network")
                    onClicked: page.addManualNetwork(newNetworkField.text)
                }
            }
        }

        Controls.Label {
            text: i18n("Manual backups ask for confirmation before running if any condition is currently unmet.")
            opacity: 0.75
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
