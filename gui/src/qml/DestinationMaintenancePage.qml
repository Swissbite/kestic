// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

EditorPage {
    id: page
    objectName: "destinationMaintenancePage"
    title: Texts.label("maintenance")

    required property string destinationId

    property var initialData: null
    property string savedMaintenanceJson: "{}"
    property bool hydrating: false

    // Prune entry
    property string pruneKind: "weekly"
    property string pruneNth: "first"
    property string pruneWeekday: "sunday"
    property int pruneHour: 12
    property int pruneMinute: 0

    // Check entry
    property string checkKind: "monthly"
    property string checkNth: "first"
    property string checkWeekday: "sunday"
    property int checkHour: 12
    property int checkMinute: 0

    property bool pendingPopAfterConfirm: false
    property string previewSummaryText: ""
    property bool previewSettled: false

    readonly property var freqOptions: [
        { key: "weekly", label: i18n("Weekly") },
        { key: "monthly", label: i18n("Monthly") },
        { key: "daily", label: Texts.label("daily") },
        { key: "manual_only", label: Texts.label("manual_only") }
    ]

    readonly property var nthOptions: [
        { key: "first", label: i18n("First") },
        { key: "second", label: i18n("Second") },
        { key: "third", label: i18n("Third") },
        { key: "fourth", label: i18n("Fourth") },
        { key: "last", label: i18n("Last") }
    ]

    readonly property var weekdayOptions: [
        { key: "monday", label: Texts.label("monday") },
        { key: "tuesday", label: Texts.label("tuesday") },
        { key: "wednesday", label: Texts.label("wednesday") },
        { key: "thursday", label: Texts.label("thursday") },
        { key: "friday", label: Texts.label("friday") },
        { key: "saturday", label: Texts.label("saturday") },
        { key: "sunday", label: Texts.label("sunday") }
    ]

    function buildEntryObj(kind, nth, weekday, hour, minute) {
        if (kind === "manual_only") return { kind: "manual_only" };
        if (kind === "daily") return { kind: "daily", hour: hour, minute: minute };
        if (kind === "monthly") return { kind: "monthly", nth: nth, weekday: weekday, hour: hour, minute: minute };
        return { kind: "weekly", weekday: weekday, hour: hour, minute: minute };
    }

    function normalizeEntry(obj, fallbackKind) {
        var k = (obj && obj.kind) ? obj.kind : fallbackKind;
        return buildEntryObj(
            k,
            (obj && obj.nth) ? obj.nth : "first",
            (obj && obj.weekday) ? obj.weekday : "sunday",
            (obj && obj.hour !== undefined) ? obj.hour : 12,
            (obj && obj.minute !== undefined) ? obj.minute : 0
        );
    }

    function currentMaintObj() {
        return {
            prune: buildEntryObj(pruneKind, pruneNth, pruneWeekday, pruneHour, pruneMinute),
            check: buildEntryObj(checkKind, checkNth, checkWeekday, checkHour, checkMinute)
        };
    }

    function currentMaintJson() {
        return JSON.stringify(currentMaintObj());
    }

    function normalizedSavedObj() {
        var parsed = rootWindow.parseJsonOr(savedMaintenanceJson, {});
        return {
            prune: normalizeEntry(parsed.prune, "weekly"),
            check: normalizeEntry(parsed.check, "monthly")
        };
    }

    readonly property bool pruneDirty: initialData !== null
        && JSON.stringify(currentMaintObj().prune) !== JSON.stringify(normalizedSavedObj().prune)

    dirty: initialData !== null
        && JSON.stringify(currentMaintObj()) !== JSON.stringify(normalizedSavedObj())

    function loadFromSaved(maintJson) {
        hydrating = true;
        var parsed = rootWindow.parseJsonOr(maintJson, {});
        var p = normalizeEntry(parsed.prune, "weekly");
        pruneKind = p.kind;
        pruneNth = p.nth || "first";
        pruneWeekday = p.weekday || "sunday";
        pruneHour = p.hour !== undefined ? p.hour : 12;
        pruneMinute = p.minute !== undefined ? p.minute : 0;

        var c = normalizeEntry(parsed.check, "monthly");
        checkKind = c.kind;
        checkNth = c.nth || "first";
        checkWeekday = c.weekday || "sunday";
        checkHour = c.hour !== undefined ? c.hour : 12;
        checkMinute = c.minute !== undefined ? c.minute : 0;
        hydrating = false;
    }

    function commitSave(popOnSuccess) {
        if (!dirty) return false;
        if (appBackend.updateDestinationMaintenance(destinationId, currentMaintJson())) {
            initialData = rootWindow.findDestinationData(destinationId);
            if (initialData) {
                savedMaintenanceJson = initialData.maintenanceJson;
                loadFromSaved(savedMaintenanceJson);
            }
            rootWindow.showPassiveNotification(i18n("Maintenance schedule saved"));
            if (popOnSuccess) {
                rootWindow.popSubPage();
            }
            return true;
        }
        return false;
    }

    function formatPreviewSummary(rawJson) {
        var sum = rootWindow.parseJsonOr(rawJson, null);
        if (!sum) {
            return i18n("The preview could not read the repository. It may be unreachable or locked.");
        }
        var parts = [];
        if (sum.delete_packs !== null && sum.delete_packs !== undefined) {
            parts.push(i18n("Data blocks to remove: %1", sum.delete_packs));
        }
        if (sum.keep_packs !== null && sum.keep_packs !== undefined) {
            parts.push(i18n("Data blocks to keep: %1", sum.keep_packs));
        }
        // restic's own summary line is English text: show only the localized size.
        var sizeMatch = sum.freed_summary
            ? /(\d+(?:\.\d+)?\s?(?:B|KiB|MiB|GiB|TiB))/.exec(sum.freed_summary)
            : null;
        if (sizeMatch) {
            parts.push(i18n("Space to free: %1", sizeMatch[1]));
        }
        if (parts.length === 0) {
            parts.push(i18n("The preview reported no data to remove."));
        }
        return parts.join("\n");
    }

    Connections {
        target: appBackend
        function onMaintenancePreviewReady(destId, summaryJson, errorMessage) {
            if (destId !== page.destinationId) {
                return;
            }
            page.previewSettled = true;
            page.previewSummaryText = page.formatPreviewSummary(errorMessage !== "" ? "" : summaryJson);
        }
    }

    function requestSave(popOnSuccess) {
        if (!dirty) return;
        pendingPopAfterConfirm = Boolean(popOnSuccess);
        if (pruneDirty) {
            previewSummaryText = "";
            previewSettled = false;
            pruneConfirmDialog.open();
            appBackend.previewDestinationMaintenance(destinationId);
        } else {
            commitSave(popOnSuccess);
        }
    }

    function saveForGuard() {
        if (pruneDirty) {
            requestSave(false);
            return false;
        }
        return commitSave(false);
    }

    function indexOfOption(arr, key) {
        for (var i = 0; i < arr.length; ++i) {
            if (arr[i].key === key) return i;
        }
        return 0;
    }

    Component.onCompleted: {
        initialData = rootWindow.findDestinationData(destinationId);
        if (initialData) {
            savedMaintenanceJson = initialData.maintenanceJson;
            loadFromSaved(savedMaintenanceJson);
        } else {
            loadFromSaved("{}");
        }
    }

    actions: [
        Kirigami.Action {
            id: saveMaintenanceAction
            objectName: "saveMaintenanceAction"
            icon.name: "document-save"
            text: Texts.label("save")
            enabled: page.dirty && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.requestSave(false)
        },
        Kirigami.Action {
            id: closeMaintenanceAction
            objectName: "closeMaintenanceAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: page.requestClose()
        }
    ]

    Kirigami.Dialog {
        id: pruneConfirmDialog
        objectName: "pruneConfirmDialog"
        title: i18n("Confirm Clean Up Schedule")
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing

        customFooterActions: [
            Kirigami.Action {
                id: confirmPruneSaveAction
                objectName: "confirmPruneSaveAction"
                icon.name: "document-save"
                text: i18n("Save Maintenance Schedule")
                // confirmation is only possible once the preview loaded or failed.
                enabled: page.previewSettled && !appBackend.maintenance_preview_loading
                onTriggered: {
                    pruneConfirmDialog.close();
                    page.commitSave(page.pendingPopAfterConfirm);
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 26
            spacing: Kirigami.Units.mediumSpacing

            Controls.Label {
                text: i18n("Cleaning up deletes data on the destination that no snapshot uses anymore. Deleted data cannot be restored.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing
                visible: appBackend.maintenance_preview_loading

                Controls.BusyIndicator {
                    running: appBackend.maintenance_preview_loading
                    implicitWidth: Kirigami.Units.iconSizes.smallMedium
                    implicitHeight: Kirigami.Units.iconSizes.smallMedium
                }

                Controls.Label {
                    text: i18n("Loading test run preview…")
                    Layout.fillWidth: true
                }
            }

            Controls.Label {
                objectName: "prunePreviewSummaryLabel"
                visible: !appBackend.maintenance_preview_loading
                text: page.previewSummaryText
                font.bold: true
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    unsavedObjectName: "unsavedMaintenanceDialog"
    unsavedSubtitle: i18n("You have unsaved changes to the maintenance schedule.")
    extraDialogs: [pruneConfirmDialog]

    onUnsavedSaveRequested: page.requestSave(true)
    onUnsavedDiscardRequested: {
        page.loadFromSaved(page.savedMaintenanceJson);
        rootWindow.popSubPage();
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
                Kirigami.FormData.label: i18n("Clean up")
            }

            Controls.ComboBox {
                id: pruneFrequencyCombo
                objectName: "pruneFrequencyCombo"
                Kirigami.FormData.label: Texts.label("frequency_label")
                model: page.freqOptions
                textRole: "label"
                valueRole: "key"
                focusPolicy: Qt.StrongFocus
                Accessible.name: i18n("Clean up frequency")
                currentIndex: page.indexOfOption(page.freqOptions, page.pruneKind)
                onActivated: function(idx) { page.pruneKind = page.freqOptions[idx].key; }
            }

            Controls.ComboBox {
                id: pruneNthCombo
                objectName: "pruneNthCombo"
                visible: page.pruneKind === "monthly"
                Kirigami.FormData.label: i18n("Week of month:")
                model: page.nthOptions
                textRole: "label"
                valueRole: "key"
                focusPolicy: Qt.StrongFocus
                Accessible.name: i18n("Clean up week of month")
                currentIndex: page.indexOfOption(page.nthOptions, page.pruneNth)
                onActivated: function(idx) { page.pruneNth = page.nthOptions[idx].key; }
            }

            Controls.ComboBox {
                id: pruneWeekdayCombo
                objectName: "pruneWeekdayCombo"
                visible: page.pruneKind === "weekly" || page.pruneKind === "monthly"
                Kirigami.FormData.label: i18n("Weekday:")
                model: page.weekdayOptions
                textRole: "label"
                valueRole: "key"
                focusPolicy: Qt.StrongFocus
                Accessible.name: i18n("Clean up weekday")
                currentIndex: page.indexOfOption(page.weekdayOptions, page.pruneWeekday)
                onActivated: function(idx) { page.pruneWeekday = page.weekdayOptions[idx].key; }
            }

            RowLayout {
                visible: page.pruneKind !== "manual_only"
                Kirigami.FormData.label: Texts.label("time_label")
                spacing: Kirigami.Units.smallSpacing

                Controls.SpinBox {
                    id: pruneHourSpin
                    objectName: "pruneHourSpin"
                    from: 0
                    to: 23
                    value: page.pruneHour
                    editable: true
                    textFromValue: function(v) { return v < 10 ? "0" + v : String(v); }
                    valueFromText: function(t) { var n = parseInt(t, 10); return isNaN(n) ? pruneHourSpin.value : n; }
                    Accessible.name: i18n("Clean up hour")
                    onValueModified: if (!page.hydrating) page.pruneHour = value
                    onValueChanged: if (!page.hydrating && page.pruneHour !== value) page.pruneHour = value
                }
                Controls.Label { text: ":" }
                Controls.SpinBox {
                    id: pruneMinuteSpin
                    objectName: "pruneMinuteSpin"
                    from: 0
                    to: 59
                    value: page.pruneMinute
                    editable: true
                    textFromValue: function(v) { return v < 10 ? "0" + v : String(v); }
                    valueFromText: function(t) { var n = parseInt(t, 10); return isNaN(n) ? pruneMinuteSpin.value : n; }
                    Accessible.name: i18n("Clean up minute")
                    onValueModified: if (!page.hydrating) page.pruneMinute = value
                    onValueChanged: if (!page.hydrating && page.pruneMinute !== value) page.pruneMinute = value
                }
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Verify")
            }

            Controls.ComboBox {
                id: checkFrequencyCombo
                objectName: "checkFrequencyCombo"
                Kirigami.FormData.label: Texts.label("frequency_label")
                model: page.freqOptions
                textRole: "label"
                valueRole: "key"
                focusPolicy: Qt.StrongFocus
                Accessible.name: i18n("Verify frequency")
                currentIndex: page.indexOfOption(page.freqOptions, page.checkKind)
                onActivated: function(idx) { page.checkKind = page.freqOptions[idx].key; }
            }

            Controls.ComboBox {
                id: checkNthCombo
                objectName: "checkNthCombo"
                visible: page.checkKind === "monthly"
                Kirigami.FormData.label: i18n("Week of month:")
                model: page.nthOptions
                textRole: "label"
                valueRole: "key"
                focusPolicy: Qt.StrongFocus
                Accessible.name: i18n("Verify week of month")
                currentIndex: page.indexOfOption(page.nthOptions, page.checkNth)
                onActivated: function(idx) { page.checkNth = page.nthOptions[idx].key; }
            }

            Controls.ComboBox {
                id: checkWeekdayCombo
                objectName: "checkWeekdayCombo"
                visible: page.checkKind === "weekly" || page.checkKind === "monthly"
                Kirigami.FormData.label: i18n("Weekday:")
                model: page.weekdayOptions
                textRole: "label"
                valueRole: "key"
                focusPolicy: Qt.StrongFocus
                Accessible.name: i18n("Verify weekday")
                currentIndex: page.indexOfOption(page.weekdayOptions, page.checkWeekday)
                onActivated: function(idx) { page.checkWeekday = page.weekdayOptions[idx].key; }
            }

            RowLayout {
                visible: page.checkKind !== "manual_only"
                Kirigami.FormData.label: Texts.label("time_label")
                spacing: Kirigami.Units.smallSpacing

                Controls.SpinBox {
                    id: checkHourSpin
                    objectName: "checkHourSpin"
                    from: 0
                    to: 23
                    value: page.checkHour
                    editable: true
                    textFromValue: function(v) { return v < 10 ? "0" + v : String(v); }
                    valueFromText: function(t) { var n = parseInt(t, 10); return isNaN(n) ? checkHourSpin.value : n; }
                    Accessible.name: i18n("Verify hour")
                    onValueModified: if (!page.hydrating) page.checkHour = value
                    onValueChanged: if (!page.hydrating && page.checkHour !== value) page.checkHour = value
                }
                Controls.Label { text: ":" }
                Controls.SpinBox {
                    id: checkMinuteSpin
                    objectName: "checkMinuteSpin"
                    from: 0
                    to: 59
                    value: page.checkMinute
                    editable: true
                    textFromValue: function(v) { return v < 10 ? "0" + v : String(v); }
                    valueFromText: function(t) { var n = parseInt(t, 10); return isNaN(n) ? checkMinuteSpin.value : n; }
                    Accessible.name: i18n("Verify minute")
                    onValueModified: if (!page.hydrating) page.checkMinute = value
                    onValueChanged: if (!page.hydrating && page.checkMinute !== value) page.checkMinute = value
                }
            }
        }
    }
}
