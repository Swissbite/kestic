// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "backupRetentionPage"
    title: i18n("Retention")

    required property var rootWindow
    required property var appBackend
    required property string backupId

    property var initialData: rootWindow.findBackupData(backupId)
    property string savedPolicyJson: initialData ? initialData.retentionJson : "{}"
    property string presetName: initialData ? initialData.retentionPreset : "standard"
    property bool hydrating: false
    property string validationError: ""

    // Count-based rules
    property bool keepLastOn: false
    property int keepLastNum: 10
    property bool keepLastAll: false

    property bool keepHourlyOn: false
    property int keepHourlyNum: 24
    property bool keepHourlyAll: false

    property bool keepDailyOn: false
    property int keepDailyNum: 7
    property bool keepDailyAll: false

    property bool keepWeeklyOn: false
    property int keepWeeklyNum: 4
    property bool keepWeeklyAll: false

    property bool keepMonthlyOn: false
    property int keepMonthlyNum: 12
    property bool keepMonthlyAll: false

    property bool keepYearlyOn: false
    property int keepYearlyNum: 5
    property bool keepYearlyAll: false

    // Duration-based rules (keep-within*)
    property bool withinAllOn: false
    property int withinAllNum: 30
    property string withinAllUnit: "d"

    property bool withinHourlyOn: false
    property int withinHourlyNum: 48
    property string withinHourlyUnit: "h"

    property bool withinDailyOn: false
    property int withinDailyNum: 14
    property string withinDailyUnit: "d"

    property bool withinWeeklyOn: false
    property int withinWeeklyNum: 6
    property string withinWeeklyUnit: "m"

    property bool withinMonthlyOn: false
    property int withinMonthlyNum: 1
    property string withinMonthlyUnit: "y"

    property bool withinYearlyOn: false
    property int withinYearlyNum: 5
    property string withinYearlyUnit: "y"

    property string pendingSaveRetentionJson: ""
    property int pendingRemoveCount: 0
    property var pendingRemoveSnapshots: []

    readonly property var unitValues: ["h", "d", "m", "y"]

    function parseDuration(raw, defaultNum, defaultUnit) {
        if (!raw || typeof raw !== "string") {
            return { on: false, num: defaultNum, unit: defaultUnit };
        }
        var m = /^(\d+)([hdmy])$/.exec(raw.trim());
        if (!m) {
            return { on: false, num: defaultNum, unit: defaultUnit };
        }
        return { on: true, num: Math.max(1, parseInt(m[1], 10)), unit: m[2] };
    }

    function applyKeepCountField(val, defaultNum) {
        if (val === "unlimited") {
            return { on: true, all: true, num: defaultNum };
        }
        if (typeof val === "number" && val > 0) {
            return { on: true, all: false, num: val };
        }
        return { on: false, all: false, num: defaultNum };
    }

    function loadFromPolicyJson(jsonText) {
        hydrating = true;
        var obj = {};
        try {
            obj = JSON.parse(jsonText || "{}");
        } catch (e) {
            obj = {};
        }
        var kl = applyKeepCountField(obj.keep_last, 10);
        keepLastOn = kl.on; keepLastAll = kl.all; keepLastNum = kl.num;

        var kh = applyKeepCountField(obj.keep_hourly, 24);
        keepHourlyOn = kh.on; keepHourlyAll = kh.all; keepHourlyNum = kh.num;

        var kd = applyKeepCountField(obj.keep_daily, 7);
        keepDailyOn = kd.on; keepDailyAll = kd.all; keepDailyNum = kd.num;

        var kw = applyKeepCountField(obj.keep_weekly, 4);
        keepWeeklyOn = kw.on; keepWeeklyAll = kw.all; keepWeeklyNum = kw.num;

        var km = applyKeepCountField(obj.keep_monthly, 12);
        keepMonthlyOn = km.on; keepMonthlyAll = km.all; keepMonthlyNum = km.num;

        var ky = applyKeepCountField(obj.keep_yearly, 5);
        keepYearlyOn = ky.on; keepYearlyAll = ky.all; keepYearlyNum = ky.num;

        var wa = parseDuration(obj.keep_within, 30, "d");
        withinAllOn = wa.on; withinAllNum = wa.num; withinAllUnit = wa.unit;

        var wh = parseDuration(obj.keep_within_hourly, 48, "h");
        withinHourlyOn = wh.on; withinHourlyNum = wh.num; withinHourlyUnit = wh.unit;

        var wd = parseDuration(obj.keep_within_daily, 14, "d");
        withinDailyOn = wd.on; withinDailyNum = wd.num; withinDailyUnit = wd.unit;

        var ww = parseDuration(obj.keep_within_weekly, 6, "m");
        withinWeeklyOn = ww.on; withinWeeklyNum = ww.num; withinWeeklyUnit = ww.unit;

        var wm = parseDuration(obj.keep_within_monthly, 1, "y");
        withinMonthlyOn = wm.on; withinMonthlyNum = wm.num; withinMonthlyUnit = wm.unit;

        var wy = parseDuration(obj.keep_within_yearly, 5, "y");
        withinYearlyOn = wy.on; withinYearlyNum = wy.num; withinYearlyUnit = wy.unit;

        detectPresetFromFields();
        hydrating = false;
    }

    function encodeKeepCount(on, all, num) {
        if (!on) return undefined;
        if (all) return "unlimited";
        return Math.max(1, num);
    }

    function currentPolicyObj() {
        var obj = {};
        if (keepLastOn) obj.keep_last = encodeKeepCount(keepLastOn, keepLastAll, keepLastNum);
        if (keepHourlyOn) obj.keep_hourly = encodeKeepCount(keepHourlyOn, keepHourlyAll, keepHourlyNum);
        if (keepDailyOn) obj.keep_daily = encodeKeepCount(keepDailyOn, keepDailyAll, keepDailyNum);
        if (keepWeeklyOn) obj.keep_weekly = encodeKeepCount(keepWeeklyOn, keepWeeklyAll, keepWeeklyNum);
        if (keepMonthlyOn) obj.keep_monthly = encodeKeepCount(keepMonthlyOn, keepMonthlyAll, keepMonthlyNum);
        if (keepYearlyOn) obj.keep_yearly = encodeKeepCount(keepYearlyOn, keepYearlyAll, keepYearlyNum);

        if (withinAllOn) obj.keep_within = Math.max(1, withinAllNum) + withinAllUnit;
        if (withinHourlyOn) obj.keep_within_hourly = Math.max(1, withinHourlyNum) + withinHourlyUnit;
        if (withinDailyOn) obj.keep_within_daily = Math.max(1, withinDailyNum) + withinDailyUnit;
        if (withinWeeklyOn) obj.keep_within_weekly = Math.max(1, withinWeeklyNum) + withinWeeklyUnit;
        if (withinMonthlyOn) obj.keep_within_monthly = Math.max(1, withinMonthlyNum) + withinMonthlyUnit;
        if (withinYearlyOn) obj.keep_within_yearly = Math.max(1, withinYearlyNum) + withinYearlyUnit;
        return obj;
    }

    function currentPolicyJson() {
        return JSON.stringify(currentPolicyObj());
    }

    function normalizePolicyString(jsonText) {
        try {
            var obj = JSON.parse(jsonText || "{}");
            var keys = [
                "keep_last", "keep_hourly", "keep_daily", "keep_weekly", "keep_monthly", "keep_yearly",
                "keep_within", "keep_within_hourly", "keep_within_daily", "keep_within_weekly",
                "keep_within_monthly", "keep_within_yearly"
            ];
            var out = {};
            keys.forEach(k => { if (obj[k] !== undefined && obj[k] !== null) out[k] = obj[k]; });
            return JSON.stringify(out);
        } catch (e) {
            return "{}";
        }
    }

    function detectPresetFromFields() {
        var cur = normalizePolicyString(currentPolicyJson());
        if (cur === normalizePolicyString(appBackend.retentionPresetJson("standard"))) {
            presetName = "standard";
        } else if (cur === normalizePolicyString(appBackend.retentionPresetJson("economical"))) {
            presetName = "economical";
        } else if (cur === normalizePolicyString(appBackend.retentionPresetJson("long"))) {
            presetName = "long";
        } else {
            presetName = "custom";
        }
    }

    function onFieldEdited() {
        if (!hydrating) {
            detectPresetFromFields();
        }
    }

    readonly property bool hasAnyRule: keepLastOn || keepHourlyOn || keepDailyOn || keepWeeklyOn || keepMonthlyOn || keepYearlyOn
        || withinAllOn || withinHourlyOn || withinDailyOn || withinWeeklyOn || withinMonthlyOn || withinYearlyOn

    readonly property bool dirty: initialData !== null && normalizePolicyString(currentPolicyJson()) !== normalizePolicyString(savedPolicyJson)

    onDirtyChanged: appBackend.has_unsaved_changes = dirty

    Component.onCompleted: {
        if (initialData) {
            savedPolicyJson = initialData.retentionJson;
            loadFromPolicyJson(savedPolicyJson);
        }
    }

    onBackRequested: function(event) {
        if (dirty) {
            event.accepted = true;
            unsavedDialog.open();
        }
    }

    function handleEscape() {
        if (confirmRemoveDialog.opened || confirmRemoveDialog.visible) {
            confirmRemoveDialog.close();
            return true;
        }
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

    function commitSave(jsonStr) {
        if (appBackend.updateBackupRetention(backupId, jsonStr)) {
            initialData = rootWindow.findBackupData(backupId);
            if (initialData) {
                savedPolicyJson = initialData.retentionJson;
                loadFromPolicyJson(savedPolicyJson);
            }
            validationError = "";
            rootWindow.showPassiveNotification(i18n("Retention rules saved"));
            return true;
        }
        validationError = appBackend.last_action_error;
        return false;
    }

    function triggerSave() {
        if (!hasAnyRule) return false;
        validationError = "";
        var jsonStr = currentPolicyJson();
        var previewRaw = appBackend.previewRetention(backupId, jsonStr);
        if (previewRaw !== "") {
            try {
                var parsed = JSON.parse(previewRaw);
                if (parsed.remove_count && parsed.remove_count > 0) {
                    pendingSaveRetentionJson = jsonStr;
                    pendingRemoveCount = parsed.remove_count;
                    pendingRemoveSnapshots = (parsed.snapshots || [])
                        .filter(s => s.action === "remove")
                        .map(s => s.short_id + " (" + s.time + ")");
                    confirmRemoveDialog.open();
                    return false;
                }
            } catch (e) {
            }
        }
        return commitSave(jsonStr);
    }

    function openPreviewPage() {
        if (!hasAnyRule) return;
        validationError = "";
        var jsonStr = currentPolicyJson();
        var previewRaw = appBackend.previewRetention(backupId, jsonStr);
        if (previewRaw === "") {
            validationError = appBackend.last_action_error || i18n("Could not preview retention rules.");
            return;
        }
        rootWindow.openRetentionPreviewPage(backupId, previewRaw);
    }

    actions: [
        Kirigami.Action {
            id: previewRetentionAction
            objectName: "previewRetentionAction"
            icon.name: "document-preview"
            text: i18n("Preview…")
            enabled: page.hasAnyRule && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.openPreviewPage()
        },
        Kirigami.Action {
            id: saveRetentionAction
            objectName: "saveRetentionAction"
            icon.name: "document-save"
            text: i18n("Save")
            enabled: page.dirty && page.hasAnyRule && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.triggerSave()
        }
    ]

    Kirigami.PromptDialog {
        id: unsavedDialog
        objectName: "unsavedRetentionDialog"
        title: i18n("Save changes?")
        subtitle: i18n("You have unsaved changes to retention rules.")
        standardButtons: Kirigami.Dialog.Save | Kirigami.Dialog.Discard | Kirigami.Dialog.Cancel
        onAccepted: {
            if (page.triggerSave()) {
                rootWindow.pageStack.pop();
            }
        }
        onDiscarded: {
            appBackend.has_unsaved_changes = false;
            page.loadFromPolicyJson(page.savedPolicyJson);
            rootWindow.pageStack.pop();
        }
    }

    Kirigami.PromptDialog {
        id: confirmRemoveDialog
        objectName: "confirmRetentionRemoveDialog"
        title: i18n("%1 snapshots will be permanently removed on the next run", page.pendingRemoveCount)
        subtitle: page.pendingRemoveSnapshots.length > 0
            ? i18n("Snapshots that will be removed:\n%1", page.pendingRemoveSnapshots.slice(0, 8).join("\n"))
            : i18n("Saving these rules will mark %1 existing snapshots for removal on the next run.", page.pendingRemoveCount)
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                id: confirmRetentionSaveAction
                objectName: "confirmRetentionSaveAction"
                icon.name: "document-save"
                text: i18n("Save Rules")
                onTriggered: {
                    confirmRemoveDialog.close();
                    if (page.commitSave(page.pendingSaveRetentionJson)) {
                        if (rootWindow.pageStack.currentItem === page && !page.dirty) {
                            // stay on page after explicit save; if popped via unsavedDialog, user can go back
                        }
                    }
                }
            }
        ]
    }

    component CountRuleRow: RowLayout {
        id: countRow
        required property string labelText
        required property string rowObjectPrefix
        property bool ruleEnabled: false
        property int ruleNumber: 7
        property bool ruleUnlimited: false
        signal edited()

        Layout.fillWidth: true
        spacing: Kirigami.Units.smallSpacing

        Controls.CheckBox {
            objectName: countRow.rowObjectPrefix + "Check"
            text: countRow.labelText
            checked: countRow.ruleEnabled
            Layout.minimumWidth: Kirigami.Units.gridUnit * 8
            Accessible.name: i18n("Keep %1", countRow.labelText)
            onToggled: {
                countRow.ruleEnabled = checked;
                countRow.edited();
            }
        }

        Controls.SpinBox {
            objectName: countRow.rowObjectPrefix + "Spin"
            from: 1
            to: 9999
            editable: true
            enabled: countRow.ruleEnabled && !countRow.ruleUnlimited
            value: countRow.ruleNumber
            Accessible.name: i18n("Number of %1 snapshots", countRow.labelText)
            onValueModified: {
                countRow.ruleNumber = value;
                countRow.edited();
            }
        }

        Controls.CheckBox {
            objectName: countRow.rowObjectPrefix + "UnlimitedCheck"
            text: i18n("All (unlimited)")
            enabled: countRow.ruleEnabled
            checked: countRow.ruleUnlimited
            Accessible.name: i18n("Keep all %1 snapshots without limit", countRow.labelText)
            onToggled: {
                countRow.ruleUnlimited = checked;
                countRow.edited();
            }
        }
    }

    component WithinRuleRow: RowLayout {
        id: withinRow
        required property string labelText
        required property string rowObjectPrefix
        property bool ruleEnabled: false
        property int ruleNumber: 7
        property string ruleUnit: "d"
        signal edited()

        Layout.fillWidth: true
        spacing: Kirigami.Units.smallSpacing

        Controls.CheckBox {
            objectName: withinRow.rowObjectPrefix + "Check"
            text: withinRow.labelText
            checked: withinRow.ruleEnabled
            Layout.minimumWidth: Kirigami.Units.gridUnit * 8
            Accessible.name: i18n("Keep %1 by time period", withinRow.labelText)
            onToggled: {
                withinRow.ruleEnabled = checked;
                withinRow.edited();
            }
        }

        Controls.SpinBox {
            objectName: withinRow.rowObjectPrefix + "Spin"
            from: 1
            to: 9999
            editable: true
            enabled: withinRow.ruleEnabled
            value: withinRow.ruleNumber
            Accessible.name: i18n("Time period length for %1", withinRow.labelText)
            onValueModified: {
                withinRow.ruleNumber = value;
                withinRow.edited();
            }
        }

        Controls.ComboBox {
            objectName: withinRow.rowObjectPrefix + "UnitCombo"
            enabled: withinRow.ruleEnabled
            model: [i18n("Hours"), i18n("Days"), i18n("Months"), i18n("Years")]
            currentIndex: Math.max(0, page.unitValues.indexOf(withinRow.ruleUnit))
            Accessible.name: i18n("Time unit for %1", withinRow.labelText)
            onActivated: function(idx) {
                withinRow.ruleUnit = page.unitValues[idx] || "d";
                withinRow.edited();
            }
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        Kirigami.InlineMessage {
            id: retentionValidationError
            objectName: "retentionValidationError"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: !page.hasAnyRule || page.validationError !== ""
            text: !page.hasAnyRule
                ? i18n("At least one retention rule must be active.")
                : page.validationError
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.ComboBox {
                id: retentionPresetCombo
                objectName: "retentionPresetCombo"
                Kirigami.FormData.label: i18n("Preset:")
                model: [
                    i18n("Standard (7 daily, 4 weekly, 12 monthly)"),
                    i18n("Economical (7 daily, 4 weekly)"),
                    i18n("Long (7 daily, 4 weekly, 12 monthly, all yearly)"),
                    i18n("Custom")
                ]
                readonly property var presetKeys: ["standard", "economical", "long", "custom"]
                currentIndex: Math.max(0, presetKeys.indexOf(page.presetName))
                Accessible.name: i18n("Retention preset")
                onActivated: function(idx) {
                    var key = presetKeys[idx];
                    if (key === "standard" || key === "economical" || key === "long") {
                        page.loadFromPolicyJson(appBackend.retentionPresetJson(key));
                    } else {
                        page.presetName = "custom";
                    }
                }
            }
        }

        Kirigami.Heading {
            level: 3
            text: i18n("By count: keep last n…")
        }

        CountRuleRow {
            labelText: i18n("Last backups:")
            rowObjectPrefix: "keepLast"
            ruleEnabled: page.keepLastOn
            ruleNumber: page.keepLastNum
            ruleUnlimited: page.keepLastAll
            onEdited: {
                page.keepLastOn = ruleEnabled;
                page.keepLastNum = ruleNumber;
                page.keepLastAll = ruleUnlimited;
                page.onFieldEdited();
            }
        }

        CountRuleRow {
            labelText: i18n("Hourly:")
            rowObjectPrefix: "keepHourly"
            ruleEnabled: page.keepHourlyOn
            ruleNumber: page.keepHourlyNum
            ruleUnlimited: page.keepHourlyAll
            onEdited: {
                page.keepHourlyOn = ruleEnabled;
                page.keepHourlyNum = ruleNumber;
                page.keepHourlyAll = ruleUnlimited;
                page.onFieldEdited();
            }
        }

        CountRuleRow {
            labelText: i18n("Daily:")
            rowObjectPrefix: "keepDaily"
            ruleEnabled: page.keepDailyOn
            ruleNumber: page.keepDailyNum
            ruleUnlimited: page.keepDailyAll
            onEdited: {
                page.keepDailyOn = ruleEnabled;
                page.keepDailyNum = ruleNumber;
                page.keepDailyAll = ruleUnlimited;
                page.onFieldEdited();
            }
        }

        CountRuleRow {
            labelText: i18n("Weekly:")
            rowObjectPrefix: "keepWeekly"
            ruleEnabled: page.keepWeeklyOn
            ruleNumber: page.keepWeeklyNum
            ruleUnlimited: page.keepWeeklyAll
            onEdited: {
                page.keepWeeklyOn = ruleEnabled;
                page.keepWeeklyNum = ruleNumber;
                page.keepWeeklyAll = ruleUnlimited;
                page.onFieldEdited();
            }
        }

        CountRuleRow {
            labelText: i18n("Monthly:")
            rowObjectPrefix: "keepMonthly"
            ruleEnabled: page.keepMonthlyOn
            ruleNumber: page.keepMonthlyNum
            ruleUnlimited: page.keepMonthlyAll
            onEdited: {
                page.keepMonthlyOn = ruleEnabled;
                page.keepMonthlyNum = ruleNumber;
                page.keepMonthlyAll = ruleUnlimited;
                page.onFieldEdited();
            }
        }

        CountRuleRow {
            labelText: i18n("Yearly:")
            rowObjectPrefix: "keepYearly"
            ruleEnabled: page.keepYearlyOn
            ruleNumber: page.keepYearlyNum
            ruleUnlimited: page.keepYearlyAll
            onEdited: {
                page.keepYearlyOn = ruleEnabled;
                page.keepYearlyNum = ruleNumber;
                page.keepYearlyAll = ruleUnlimited;
                page.onFieldEdited();
            }
        }

        Kirigami.Heading {
            level: 3
            text: i18n("By time period: keep all… for last")
        }

        WithinRuleRow {
            labelText: i18n("All backups:")
            rowObjectPrefix: "withinAll"
            ruleEnabled: page.withinAllOn
            ruleNumber: page.withinAllNum
            ruleUnit: page.withinAllUnit
            onEdited: {
                page.withinAllOn = ruleEnabled;
                page.withinAllNum = ruleNumber;
                page.withinAllUnit = ruleUnit;
                page.onFieldEdited();
            }
        }

        WithinRuleRow {
            labelText: i18n("Hourly:")
            rowObjectPrefix: "withinHourly"
            ruleEnabled: page.withinHourlyOn
            ruleNumber: page.withinHourlyNum
            ruleUnit: page.withinHourlyUnit
            onEdited: {
                page.withinHourlyOn = ruleEnabled;
                page.withinHourlyNum = ruleNumber;
                page.withinHourlyUnit = ruleUnit;
                page.onFieldEdited();
            }
        }

        WithinRuleRow {
            labelText: i18n("Daily:")
            rowObjectPrefix: "withinDaily"
            ruleEnabled: page.withinDailyOn
            ruleNumber: page.withinDailyNum
            ruleUnit: page.withinDailyUnit
            onEdited: {
                page.withinDailyOn = ruleEnabled;
                page.withinDailyNum = ruleNumber;
                page.withinDailyUnit = ruleUnit;
                page.onFieldEdited();
            }
        }

        WithinRuleRow {
            labelText: i18n("Weekly:")
            rowObjectPrefix: "withinWeekly"
            ruleEnabled: page.withinWeeklyOn
            ruleNumber: page.withinWeeklyNum
            ruleUnit: page.withinWeeklyUnit
            onEdited: {
                page.withinWeeklyOn = ruleEnabled;
                page.withinWeeklyNum = ruleNumber;
                page.withinWeeklyUnit = ruleUnit;
                page.onFieldEdited();
            }
        }

        WithinRuleRow {
            labelText: i18n("Monthly:")
            rowObjectPrefix: "withinMonthly"
            ruleEnabled: page.withinMonthlyOn
            ruleNumber: page.withinMonthlyNum
            ruleUnit: page.withinMonthlyUnit
            onEdited: {
                page.withinMonthlyOn = ruleEnabled;
                page.withinMonthlyNum = ruleNumber;
                page.withinMonthlyUnit = ruleUnit;
                page.onFieldEdited();
            }
        }

        WithinRuleRow {
            labelText: i18n("Yearly:")
            rowObjectPrefix: "withinYearly"
            ruleEnabled: page.withinYearlyOn
            ruleNumber: page.withinYearlyNum
            ruleUnit: page.withinYearlyUnit
            onEdited: {
                page.withinYearlyOn = ruleEnabled;
                page.withinYearlyNum = ruleNumber;
                page.withinYearlyUnit = ruleUnit;
                page.onFieldEdited();
            }
        }

        Kirigami.InlineMessage {
            id: retentionSemanticsInfo
            objectName: "retentionSemanticsInfo"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: true
            text: i18n("Count rules ('last 7 daily') skip days without backups. Time-period rules ('daily of the last 7 days') measure calendar days backwards from the newest snapshot.")
        }
    }
}
