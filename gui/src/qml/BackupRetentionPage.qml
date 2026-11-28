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

    // The rules; loading the saved policy fills in the values core offers for rules that are off.
    property bool keepLastOn: false
    property int keepLastNum: 0
    property bool keepLastAll: false
    property bool keepHourlyOn: false
    property int keepHourlyNum: 0
    property bool keepHourlyAll: false
    property bool keepDailyOn: false
    property int keepDailyNum: 0
    property bool keepDailyAll: false
    property bool keepWeeklyOn: false
    property int keepWeeklyNum: 0
    property bool keepWeeklyAll: false
    property bool keepMonthlyOn: false
    property int keepMonthlyNum: 0
    property bool keepMonthlyAll: false
    property bool keepYearlyOn: false
    property int keepYearlyNum: 0
    property bool keepYearlyAll: false

    property bool withinAllOn: false
    property int withinAllNum: 0
    property string withinAllUnit: "d"
    property bool withinHourlyOn: false
    property int withinHourlyNum: 0
    property string withinHourlyUnit: "d"
    property bool withinDailyOn: false
    property int withinDailyNum: 0
    property string withinDailyUnit: "d"
    property bool withinWeeklyOn: false
    property int withinWeeklyNum: 0
    property string withinWeeklyUnit: "d"
    property bool withinMonthlyOn: false
    property int withinMonthlyNum: 0
    property string withinMonthlyUnit: "d"
    property bool withinYearlyOn: false
    property int withinYearlyNum: 0
    property string withinYearlyUnit: "d"

    property string pendingSaveRetentionJson: ""
    property int pendingRemoveCount: 0
    property var pendingRemoveSnapshots: []
    property bool saveBusy: false
    // The rules take no input while the preview before saving runs: the save commits the rules
    // the preview checked.
    enabled: !saving && !saveBusy
    property string savePreviewId: ""
    property bool pendingPopAfterSave: false

    readonly property var unitValues: ["h", "d", "m", "y"]

    // The rules of the form as core names them, with the prefix of their page properties.
    readonly property var countRules: ({
        keep_last: "keepLast", keep_hourly: "keepHourly", keep_daily: "keepDaily",
        keep_weekly: "keepWeekly", keep_monthly: "keepMonthly", keep_yearly: "keepYearly"
    })
    readonly property var withinRules: ({
        keep_within: "withinAll", keep_within_hourly: "withinHourly",
        keep_within_daily: "withinDaily", keep_within_weekly: "withinWeekly",
        keep_within_monthly: "withinMonthly", keep_within_yearly: "withinYearly"
    })

    // The fields as core reads them.
    function currentForm() {
        var form = {};
        for (var key in countRules) {
            var c = countRules[key];
            form[key] = { on: page[c + "On"], all: page[c + "All"], num: page[c + "Num"] };
        }
        for (var wkey in withinRules) {
            var w = withinRules[wkey];
            form[wkey] = { on: page[w + "On"], num: page[w + "Num"], unit: page[w + "Unit"] };
        }
        return form;
    }

    // What core makes of the fields: the policy, its preset, whether it differs from the saved
    // one and whether a rule is on.
    readonly property var formState: rootWindow.parseJsonOr(
        appBackend.retentionFormState(JSON.stringify(currentForm()), savedPolicyJson), ({}))

    function loadFromPolicyJson(jsonText) {
        hydrating = true;
        var form = rootWindow.parseJsonOr(appBackend.retentionFormJson(jsonText), {});
        for (var key in countRules) {
            var c = countRules[key];
            page[c + "On"] = form[key].on;
            page[c + "All"] = form[key].all;
            page[c + "Num"] = form[key].num;
        }
        for (var wkey in withinRules) {
            var w = withinRules[wkey];
            page[w + "On"] = form[wkey].on;
            page[w + "Num"] = form[wkey].num;
            page[w + "Unit"] = form[wkey].unit;
        }
        presetName = formState.preset || "custom";
        hydrating = false;
    }

    function currentPolicyJson() {
        return JSON.stringify(formState.policy || {});
    }

    function onFieldEdited() {
        if (!hydrating) {
            presetName = formState.preset || "custom";
        }
    }

    readonly property bool hasAnyRule: formState.has_rule === true

    dirty: initialData !== null && formState.dirty === true

    // Shows the backup as the daemon has it now, with its revision.
    function reloadFromDaemon() {
        initialData = rootWindow.findBackupData(backupId);
        revision = appBackend.backupRevision(backupId);
        if (initialData) {
            savedPolicyJson = initialData.retentionJson;
            loadFromPolicyJson(savedPolicyJson);
        }
    }

    Component.onCompleted: reloadFromDaemon()

    Component.onDestruction: {
        savePollTimer.stop();
        // A preview still starting is cancelled once the daemon has started it.
        appBackend.forgetDaemonCall(saveStartRequest);
        if (saveBusy && savePreviewId !== "") {
            appBackend.cancelRetentionPreview(savePreviewId);
        }
    }

    // Saves; `done(ok)` runs once the daemon has answered.
    function commitSave(jsonStr, done) {
        saving = true;
        rootWindow.afterDaemonCall(appBackend.updateBackupRetention(backupId, jsonStr, page.revision), function(ok, answer) {
            saving = false;
            page.afterSave(ok, answer, page.reloadFromDaemon);
            if (ok) {
                initialData = rootWindow.findBackupData(backupId);
                if (initialData) {
                    savedPolicyJson = initialData.retentionJson;
                    loadFromPolicyJson(savedPolicyJson);
                }
                validationError = "";
                rootWindow.showPassiveNotification(i18n("Retention rules saved"));
            } else if (answer !== "conflict") {
                validationError = appBackend.last_action_error;
            }
            if (done) done(ok);
        });
    }

    function finishSave(jsonStr, popOnSuccess, done) {
        commitSave(jsonStr, function(ok) {
            if (ok && popOnSuccess && rootWindow.pageStack.currentItem === page) {
                rootWindow.popSubPage();
            }
            if (done) done(ok);
        });
    }

    // Saves, first checking the rules with a preview where needed; `done(ok)` runs once the
    // daemon has answered (with `false` when the save waits for the preview).
    function triggerSave(popAfterSave, done) {
        if (!hasAnyRule || saveBusy || saving) {
            if (done) done(false);
            return false;
        }
        validationError = "";
        var jsonStr = currentPolicyJson();
        pendingSaveRetentionJson = jsonStr;
        pendingPopAfterSave = Boolean(popAfterSave);
        saveBusy = true;
        savePreviewId = "";
        saveStartRequest = appBackend.startRetentionPreview(backupId, jsonStr);
        if (saveStartRequest === 0) {
            saveBusy = false;
            finishSave(jsonStr, pendingPopAfterSave, done);
            return false;
        }
        if (done) done(false);
        return false;
    }

    // The preview before saving started (`id`), or could not start: then the rules are saved
    // without it.
    function savePreviewStarted(ok, id) {
        if (!ok || id === "") {
            saveBusy = false;
            finishSave(pendingSaveRetentionJson, pendingPopAfterSave);
            return;
        }
        savePreviewId = id;
        savePollTimer.start();
    }

    // The requests of the preview before saving (0: none); answers arrive through
    // `daemonCallDone`.
    property int saveStartRequest: 0
    property int savePollRequest: 0

    Connections {
        target: appBackend
        function onDaemonCallDone(request, ok, result) {
            if (request === page.saveStartRequest) {
                page.saveStartRequest = 0;
                page.savePreviewStarted(ok, result);
            } else if (request === page.savePollRequest) {
                page.savePollRequest = 0;
                page.savePreviewAnswered(ok ? rootWindow.parseJsonOr(result, null) : null);
            }
        }
    }

    function saveForGuard(done) {
        triggerSave(false, done);
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
            if (page.savePollRequest === 0) {
                page.savePollRequest = appBackend.pollRetentionPreview(page.savePreviewId);
            }
        }
    }

    // One answer `dto` (null: none) of the preview before saving.
    function savePreviewAnswered(dto) {
        if (dto && dto.state === "running") {
            return;
        }
        savePollTimer.stop();
        saveBusy = false;
        savePreviewId = "";
        if (dto && dto.state === "done" && dto.result && dto.result.remove_count && dto.result.remove_count > 0) {
            pendingRemoveCount = dto.result.remove_count;
            pendingRemoveSnapshots = (dto.result.snapshots || [])
                .filter(s => s.action === "remove")
                .map(s => s.short_id + " (" + s.time_text + ")");
            confirmRemoveDialog.open();
            return;
        }
        if (dto && dto.state === "cancelled") {
            return;
        }
        finishSave(pendingSaveRetentionJson, pendingPopAfterSave);
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
            enabled: page.dirty && page.hasAnyRule && !page.saveBusy && !page.saving && appBackend.blocking_kind !== "daemon_unreachable"
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
        title: i18np("%1 snapshot will be permanently removed on the next run", "%1 snapshots will be permanently removed on the next run", page.pendingRemoveCount)
        subtitle: page.pendingRemoveSnapshots.length > 0
            ? i18n("Snapshots that will be removed:\n%1", page.pendingRemoveSnapshots.slice(0, 8).join("\n"))
            : i18np("If you save these rules, the next run removes %1 existing snapshot.", "If you save these rules, the next run removes %1 existing snapshots.", page.pendingRemoveCount)
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
            // The digits are centred in the whole box: Fusion makes the box lower than the
            // padding of the style allows and would cut them.
            topPadding: 0
            bottomPadding: 0
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
            // The digits are centred in the whole box: Fusion makes the box lower than the
            // padding of the style allows and would cut them.
            topPadding: 0
            bottomPadding: 0
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
            // Fusion leaves the bar 0 px high and draws nothing; 6 px is the bar of Breeze.
            Layout.minimumHeight: topInset + bottomInset + Math.round(Kirigami.Units.smallSpacing * 1.5)
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
