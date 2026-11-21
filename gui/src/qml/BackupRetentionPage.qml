// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

EditorPage {
    id: page
    objectName: "backupRetentionPage"
    title: Texts.label("retention")

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
    property bool saveBusy: false
    property string savePreviewId: ""
    property bool pendingPopAfterSave: false

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
        var obj = rootWindow.parseJsonOr(jsonText, {});
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
        var obj = rootWindow.parseJsonOr(jsonText, {});
        var keys = [
            "keep_last", "keep_hourly", "keep_daily", "keep_weekly", "keep_monthly", "keep_yearly",
            "keep_within", "keep_within_hourly", "keep_within_daily", "keep_within_weekly",
            "keep_within_monthly", "keep_within_yearly"
        ];
        var out = {};
        keys.forEach(k => { if (obj[k] !== undefined && obj[k] !== null) out[k] = obj[k]; });
        return JSON.stringify(out);
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

    dirty: initialData !== null && normalizePolicyString(currentPolicyJson()) !== normalizePolicyString(savedPolicyJson)

    Component.onCompleted: {
        if (initialData) {
            savedPolicyJson = initialData.retentionJson;
            loadFromPolicyJson(savedPolicyJson);
        }
    }

    Component.onDestruction: {
        savePollTimer.stop();
        if (saveBusy && savePreviewId !== "") {
            appBackend.cancelRetentionPreview(savePreviewId);
        }
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

    function finishSave(jsonStr, popOnSuccess) {
        var ok = commitSave(jsonStr);
        if (ok && popOnSuccess) {
            rootWindow.popSubPage();
        }
        return ok;
    }

    function triggerSave(popAfterSave) {
        if (!hasAnyRule || saveBusy) return false;
        validationError = "";
        var jsonStr = currentPolicyJson();
        pendingSaveRetentionJson = jsonStr;
        pendingPopAfterSave = Boolean(popAfterSave);
        saveBusy = true;
        savePreviewId = appBackend.startRetentionPreview(backupId, jsonStr);
        if (savePreviewId === "") {
            saveBusy = false;
            return finishSave(jsonStr, pendingPopAfterSave);
        }
        savePollTimer.start();
        return false;
    }

    function saveForGuard() {
        triggerSave(false);
        return !appBackend.has_unsaved_changes && !dirty;
    }

    function openPreviewPage() {
        if (!hasAnyRule) return;
        validationError = "";
        rootWindow.openRetentionPreviewPage(backupId, currentPolicyJson());
    }

    Timer {
        id: savePollTimer
        interval: 50
        repeat: true
        onTriggered: {
            if (page.savePreviewId === "") {
                stop();
                page.saveBusy = false;
                return;
            }
            var dto = rootWindow.parseJsonOr(appBackend.pollRetentionPreview(page.savePreviewId), null);
            if (dto && dto.state === "running") {
                return;
            }
            stop();
            page.saveBusy = false;
            page.savePreviewId = "";
            if (dto && dto.state === "done" && dto.result && dto.result.remove_count && dto.result.remove_count > 0) {
                page.pendingRemoveCount = dto.result.remove_count;
                page.pendingRemoveSnapshots = (dto.result.snapshots || [])
                    .filter(s => s.action === "remove")
                    .map(s => s.short_id + " (" + s.time + ")");
                confirmRemoveDialog.open();
                return;
            }
            if (dto && dto.state === "cancelled") {
                return;
            }
            page.finishSave(page.pendingSaveRetentionJson, page.pendingPopAfterSave);
        }
    }

    actions: [
        Kirigami.Action {
            id: previewRetentionAction
            objectName: "previewRetentionAction"
            icon.name: "document-preview"
            text: Texts.label("preview_ellipsis")
            enabled: page.hasAnyRule && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.openPreviewPage()
        },
        Kirigami.Action {
            id: saveRetentionAction
            objectName: "saveRetentionAction"
            icon.name: "document-save"
            text: Texts.label("save")
            enabled: page.dirty && page.hasAnyRule && !page.saveBusy && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.triggerSave(false)
        },
        Kirigami.Action {
            id: closeRetentionAction
            objectName: "closeRetentionAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: page.requestClose()
        }
    ]

    Kirigami.Dialog {
        id: retentionExamplesDialog
        objectName: "retentionExamplesDialog"
        title: i18n("How Retention Rules Work")
        standardButtons: Kirigami.Dialog.Close
        padding: Kirigami.Units.largeSpacing

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 28
            spacing: Kirigami.Units.mediumSpacing

            Controls.Label {
                text: i18n("1. By count ('keep last 7 daily'):\nOnly days with a backup count. Days without a backup are skipped.\nExample: Your PC runs only from Monday to Thursday. Then '7 daily' keeps the backups of the last 7 days with a backup, which covers almost 2 weeks.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Controls.Label {
                text: i18n("2. By time period ('daily for the last 7 days'):\nCounts calendar days back from the newest snapshot.\nExample: 'Daily for the last 7 days' keeps one backup per day for the 7 days before the newest backup.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    unsavedObjectName: "unsavedRetentionDialog"
    unsavedSubtitle: i18n("You have unsaved changes to retention rules.")
    extraDialogs: [retentionExamplesDialog, confirmRemoveDialog]

    onUnsavedSaveRequested: page.triggerSave(true)
    onUnsavedDiscardRequested: {
        page.loadFromPolicyJson(page.savedPolicyJson);
        rootWindow.popSubPage();
    }

    Kirigami.PromptDialog {
        id: confirmRemoveDialog
        objectName: "confirmRetentionRemoveDialog"
        title: i18n("%1 snapshots will be permanently removed on the next run", page.pendingRemoveCount)
        subtitle: page.pendingRemoveSnapshots.length > 0
            ? i18n("Snapshots that will be removed:\n%1", page.pendingRemoveSnapshots.slice(0, 8).join("\n"))
            : i18n("If you save these rules, the next run removes %1 existing snapshots.", page.pendingRemoveCount)
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                id: confirmRetentionSaveAction
                objectName: "confirmRetentionSaveAction"
                icon.name: "document-save"
                text: i18n("Save Rules")
                onTriggered: {
                    confirmRemoveDialog.close();
                    page.finishSave(page.pendingSaveRetentionJson, page.pendingPopAfterSave);
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
            id: countRuleCheck
            objectName: countRow.rowObjectPrefix + "Check"
            text: countRow.labelText
            checked: countRow.ruleEnabled
            Layout.minimumWidth: Kirigami.Units.gridUnit * 8
            Accessible.name: i18n("Keep %1", countRow.labelText)
            onToggled: {
                countRow.ruleEnabled = checked;
                countRow.edited();
                countRuleCheck.forceActiveFocus(Qt.TabFocusReason);
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
            id: countUnlimitedCheck
            objectName: countRow.rowObjectPrefix + "UnlimitedCheck"
            text: i18n("All (unlimited)")
            enabled: countRow.ruleEnabled
            checked: countRow.ruleUnlimited
            Accessible.name: i18n("Keep all %1 snapshots without limit", countRow.labelText)
            onToggled: {
                countRow.ruleUnlimited = checked;
                countRow.edited();
                countUnlimitedCheck.forceActiveFocus(Qt.TabFocusReason);
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
            id: withinRuleCheck
            objectName: withinRow.rowObjectPrefix + "Check"
            text: withinRow.labelText
            checked: withinRow.ruleEnabled
            Layout.minimumWidth: Kirigami.Units.gridUnit * 8
            Accessible.name: i18n("Keep %1 by time period", withinRow.labelText)
            onToggled: {
                withinRow.ruleEnabled = checked;
                withinRow.edited();
                withinRuleCheck.forceActiveFocus(Qt.TabFocusReason);
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

        Controls.ProgressBar {
            id: retentionSaveProgress
            objectName: "retentionSaveProgress"
            Layout.fillWidth: true
            indeterminate: true
            visible: page.saveBusy
        }

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
                    Texts.label("preset_standard"),
                    Texts.label("preset_economical"),
                    Texts.label("preset_long"),
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

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Heading {
                level: 3
                text: i18n("By count: keep last n…")
                Layout.fillWidth: true
            }

            Controls.ToolButton {
                id: countRulesHelpButton
                objectName: "countRulesHelpButton"
                icon.name: "help-contextual"
                text: "?"
                display: Controls.AbstractButton.IconOnly
                Accessible.name: i18n("Explain count rules")
                Controls.ToolTip.visible: hovered || activeFocus
                Controls.ToolTip.delay: 150
                Controls.ToolTip.text: i18n("Only days with a backup count. Click for an example.")
                onClicked: retentionExamplesDialog.open()
            }
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

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Heading {
                level: 3
                text: i18n("By time period: keep all… for last")
                Layout.fillWidth: true
            }

            Controls.ToolButton {
                id: withinRulesHelpButton
                objectName: "withinRulesHelpButton"
                icon.name: "help-contextual"
                text: "?"
                display: Controls.AbstractButton.IconOnly
                Accessible.name: i18n("Explain time rules")
                Controls.ToolTip.visible: hovered || activeFocus
                Controls.ToolTip.delay: 150
                Controls.ToolTip.text: i18n("Counts calendar days back from the newest snapshot. Click for an example.")
                onClicked: retentionExamplesDialog.open()
            }
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
            text: i18n("Count rules ('last 7 daily') skip days without a backup. Time rules ('daily for the last 7 days') count calendar days back from the newest snapshot.")
            actions: [
                Kirigami.Action {
                    icon.name: "help-contextual"
                    text: i18n("Show Example…")
                    onTriggered: retentionExamplesDialog.open()
                }
            ]
        }
    }
}
