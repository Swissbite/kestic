// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0
import org.kde.kirigamiaddons.dateandtime as DateAndTime

EditorPage {
    id: page
    objectName: "backupScheduleEditPage"
    title: Texts.label("schedule")

    required property string backupId

    property var initialData: null
    property string savedScheduleJson: "{}"
    property int savedOverdueThreshold: 0
    property bool hydrating: false

    property string scheduleKind: "daily"
    property var selectedWeekdays: ["monday"]
    property int intervalDays: 3
    property string startDate: ""
    property int hour: 12
    property int minute: 0
    property bool useGlobalOverdue: true
    property int customOverdueThreshold: 2
    property var previewSlots: []

    readonly property var frequencyOptions: [
        { key: "daily", label: Texts.label("daily") },
        { key: "specific_weekdays", label: i18n("On specific weekdays") },
        { key: "every_n_days", label: i18n("Every n days") },
        { key: "manual_only", label: Texts.label("manual_only") }
    ]

    readonly property var weekdayOptions: [
        { key: "monday", shortLabel: Texts.label("mon"), fullLabel: Texts.label("monday") },
        { key: "tuesday", shortLabel: Texts.label("tue"), fullLabel: Texts.label("tuesday") },
        { key: "wednesday", shortLabel: Texts.label("wed"), fullLabel: Texts.label("wednesday") },
        { key: "thursday", shortLabel: Texts.label("thu"), fullLabel: Texts.label("thursday") },
        { key: "friday", shortLabel: Texts.label("fri"), fullLabel: Texts.label("friday") },
        { key: "saturday", shortLabel: Texts.label("sat"), fullLabel: Texts.label("saturday") },
        { key: "sunday", shortLabel: Texts.label("sun"), fullLabel: Texts.label("sunday") }
    ]

    function todayIsoDate() {
        return appBackend.todayIsoDate();
    }

    function isoWeekdayToKey(num) {
        var keys = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
        if (num >= 1 && num <= 7) return keys[num - 1];
        return "monday";
    }

    function parseIsoDateOrToday(s) {
        var m = /^(\d{4})-(\d{2})-(\d{2})$/.exec((s || "").trim());
        if (m) {
            var y = parseInt(m[1], 10);
            var mo = parseInt(m[2], 10);
            var d = parseInt(m[3], 10);
            var dt = new Date(y, mo - 1, d);
            if (!isNaN(dt.getTime())) {
                return dt;
            }
        }
        var fallback = /^(\d{4})-(\d{2})-(\d{2})$/.exec(todayIsoDate());
        if (fallback) {
            return new Date(parseInt(fallback[1], 10), parseInt(fallback[2], 10) - 1, parseInt(fallback[3], 10));
        }
        return new Date();
    }

    readonly property var validationResult: rootWindow.parseJsonOr(
        appBackend.validateScheduleJson(currentScheduleJson()), { field: "", message: "" })
    readonly property string validationField: validationResult.field || ""
    readonly property string validationError: validationResult.message || ""
    readonly property bool intervalInvalid: validationField === "interval"
    readonly property bool startDateInvalid: validationField === "start_date"
    readonly property bool weekdaysInvalid: validationField === "weekdays"
    readonly property bool valid: validationError === ""

    function currentScheduleObj() {
        if (scheduleKind === "manual_only") {
            return { kind: "manual_only" };
        }
        if (scheduleKind === "specific_weekdays") {
            return {
                kind: "specific_weekdays",
                weekdays: selectedWeekdays.slice(),
                hour: hour,
                minute: minute
            };
        }
        if (scheduleKind === "every_n_days") {
            return {
                kind: "every_n_days",
                interval_days: intervalDays,
                start_date: startDate.trim(),
                hour: hour,
                minute: minute
            };
        }
        return {
            kind: "daily",
            hour: hour,
            minute: minute
        };
    }

    function currentScheduleJson() {
        return JSON.stringify(currentScheduleObj());
    }

    function normalizedScheduleJson(rawJson) {
        var obj = rootWindow.parseJsonOr(rawJson, {});
        if (obj.kind === "weekly") {
            return JSON.stringify({
                kind: "specific_weekdays",
                weekdays: [isoWeekdayToKey(obj.weekday || 1)],
                hour: obj.hour !== undefined ? obj.hour : 12,
                minute: obj.minute !== undefined ? obj.minute : 0
            });
        }
        if (obj.kind === "specific_weekdays") {
            var days = (obj.weekdays || []).slice();
            days.sort();
            return JSON.stringify({
                kind: "specific_weekdays",
                weekdays: days,
                hour: obj.hour !== undefined ? obj.hour : 12,
                minute: obj.minute !== undefined ? obj.minute : 0
            });
        }
        if (obj.kind === "every_n_days") {
            return JSON.stringify({
                kind: "every_n_days",
                interval_days: obj.interval_days !== undefined ? obj.interval_days : 3,
                start_date: (obj.start_date || todayIsoDate()).trim(),
                hour: obj.hour !== undefined ? obj.hour : 12,
                minute: obj.minute !== undefined ? obj.minute : 0
            });
        }
        if (obj.kind === "manual_only") {
            return JSON.stringify({ kind: "manual_only" });
        }
        return JSON.stringify({
            kind: "daily",
            hour: obj.hour !== undefined ? obj.hour : 12,
            minute: obj.minute !== undefined ? obj.minute : 0
        });
    }

    readonly property int effectiveOverdueThreshold: useGlobalOverdue ? 0 : customOverdueThreshold
    dirty: initialData !== null && (
        normalizedScheduleJson(currentScheduleJson()) !== normalizedScheduleJson(savedScheduleJson)
        || effectiveOverdueThreshold !== savedOverdueThreshold
    )

    function refreshPreview() {
        if (!valid || scheduleKind === "manual_only") {
            previewSlots = [];
            return;
        }
        previewSlots = rootWindow.parseJsonOr(
            appBackend.previewScheduleSlotsJson(currentScheduleJson()), []);
    }

    function loadFromSaved(schedJson, thresholdVal) {
        hydrating = true;
        var obj = rootWindow.parseJsonOr(schedJson, {});
        if (obj.kind === "weekly") {
            scheduleKind = "specific_weekdays";
            selectedWeekdays = [isoWeekdayToKey(obj.weekday || 1)];
            hour = obj.hour !== undefined ? obj.hour : 12;
            minute = obj.minute !== undefined ? obj.minute : 0;
        } else if (obj.kind === "specific_weekdays") {
            scheduleKind = "specific_weekdays";
            selectedWeekdays = (obj.weekdays && obj.weekdays.length > 0) ? obj.weekdays.slice() : ["monday"];
            hour = obj.hour !== undefined ? obj.hour : 12;
            minute = obj.minute !== undefined ? obj.minute : 0;
        } else if (obj.kind === "every_n_days") {
            scheduleKind = "every_n_days";
            intervalDays = obj.interval_days !== undefined ? obj.interval_days : 3;
            startDate = obj.start_date || todayIsoDate();
            hour = obj.hour !== undefined ? obj.hour : 12;
            minute = obj.minute !== undefined ? obj.minute : 0;
        } else if (obj.kind === "manual_only") {
            scheduleKind = "manual_only";
            hour = 12;
            minute = 0;
        } else {
            scheduleKind = "daily";
            hour = obj.hour !== undefined ? obj.hour : 12;
            minute = obj.minute !== undefined ? obj.minute : 0;
        }
        if (!startDate || startDate === "") {
            startDate = todayIsoDate();
        }
        useGlobalOverdue = thresholdVal <= 0;
        customOverdueThreshold = thresholdVal > 0 ? thresholdVal : Math.max(1, appBackend.default_overdue_threshold);
        hydrating = false;
        refreshPreview();
    }

    function toggleWeekday(key) {
        var idx = selectedWeekdays.indexOf(key);
        var next = selectedWeekdays.slice();
        if (idx >= 0) {
            next.splice(idx, 1);
        } else {
            next.push(key);
        }
        selectedWeekdays = next;
        refreshPreview();
    }

    // Saves; `done(ok)` runs once the daemon has answered.
    function commitSave(popOnSuccess, done) {
        if (!dirty || !valid) {
            if (done) done(false);
            return;
        }
        saving = true;
        rootWindow.afterDaemonCall(appBackend.updateBackupSchedule(backupId, currentScheduleJson(), effectiveOverdueThreshold), function(ok) {
            saving = false;
            if (ok) {
                initialData = rootWindow.findBackupData(backupId);
                if (initialData) {
                    savedScheduleJson = initialData.scheduleJson;
                    savedOverdueThreshold = initialData.overdueThreshold;
                    loadFromSaved(savedScheduleJson, savedOverdueThreshold);
                }
                rootWindow.showPassiveNotification(i18n("Schedule saved"));
                if (popOnSuccess && rootWindow.pageStack.currentItem === page) {
                    rootWindow.popSubPage();
                }
            }
            if (done) done(ok);
        });
    }

    function saveForGuard(done) {
        commitSave(false, done);
    }

    Component.onCompleted: {
        initialData = rootWindow.findBackupData(backupId);
        if (initialData) {
            savedScheduleJson = initialData.scheduleJson;
            savedOverdueThreshold = initialData.overdueThreshold;
            loadFromSaved(savedScheduleJson, savedOverdueThreshold);
        } else {
            loadFromSaved("{\"kind\":\"daily\",\"hour\":12,\"minute\":0}", 0);
        }
    }

    actions: [
        Kirigami.Action {
            id: saveScheduleAction
            objectName: "saveScheduleAction"
            icon.name: "document-save"
            text: Texts.label("save")
            enabled: page.dirty && page.valid && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.commitSave(false)
        },
        Kirigami.Action {
            id: closeScheduleAction
            objectName: "closeScheduleAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: page.requestClose()
        }
    ]

    DateAndTime.DatePopup {
        id: startDatePickerPopup
        objectName: "scheduleStartDatePopup"
        parent: page.rootWindow ? page.rootWindow.overlay : page
        onAccepted: {
            var dt = value;
            var yyyy = String(dt.getFullYear());
            var mm = String(dt.getMonth() + 1).padStart(2, "0");
            var dd = String(dt.getDate()).padStart(2, "0");
            page.startDate = yyyy + "-" + mm + "-" + dd;
            page.refreshPreview();
        }
    }

    unsavedObjectName: "unsavedScheduleDialog"
    unsavedSubtitle: i18n("You have unsaved changes to the backup schedule.")
    extraDialogs: [startDatePickerPopup]

    onUnsavedSaveRequested: page.commitSave(true)
    onUnsavedDiscardRequested: {
        page.loadFromSaved(page.savedScheduleJson, page.savedOverdueThreshold);
        rootWindow.popSubPage();
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing

        Kirigami.InlineMessage {
            id: scheduleValidationBanner
            objectName: "scheduleValidationBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: page.validationError !== ""
            text: page.validationError
        }

        Kirigami.InlineMessage {
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: appBackend.last_action_error !== ""
            text: appBackend.last_action_error
        }

        Kirigami.FormLayout {
            Layout.fillWidth: true

            Controls.ComboBox {
                objectName: "scheduleFrequencyCombo"
                Kirigami.FormData.label: Texts.label("frequency_label")
                model: page.frequencyOptions
                textRole: "label"
                valueRole: "key"
                focusPolicy: Qt.StrongFocus
                Accessible.name: i18n("Backup frequency")
                currentIndex: {
                    for (var i = 0; i < page.frequencyOptions.length; ++i) {
                        if (page.frequencyOptions[i].key === page.scheduleKind) return i;
                    }
                    return 0;
                }
                onActivated: function(idx) {
                    page.scheduleKind = page.frequencyOptions[idx].key;
                    if (page.scheduleKind === "every_n_days" && page.startDateInvalid) {
                        page.startDate = page.todayIsoDate();
                    }
                    page.refreshPreview();
                }
            }

            ColumnLayout {
                Kirigami.FormData.label: i18n("Weekdays:")
                visible: page.scheduleKind === "specific_weekdays"
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Rectangle {
                    objectName: "scheduleWeekdaysFrame"
                    color: "transparent"
                    radius: Kirigami.Units.cornerRadius
                    border.width: page.weekdaysInvalid ? 2 : 0
                    border.color: Kirigami.Theme.negativeTextColor
                    implicitWidth: weekdaysFlow.implicitWidth + Kirigami.Units.smallSpacing * 2
                    implicitHeight: weekdaysFlow.implicitHeight + Kirigami.Units.smallSpacing * 2
                    Layout.fillWidth: true

                    Flow {
                        id: weekdaysFlow
                        anchors.fill: parent
                        anchors.margins: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        // Not a list: a row of buttons, one per weekday.
                        Repeater {
                            model: page.weekdayOptions
                            delegate: Controls.Button {
                                required property var modelData
                                objectName: "weekdayButton_" + modelData.key
                                text: modelData.shortLabel
                                checkable: true
                                checked: page.selectedWeekdays.indexOf(modelData.key) >= 0
                                Accessible.name: modelData.fullLabel
                                onClicked: page.toggleWeekday(modelData.key)
                            }
                        }
                    }
                }
            }

            RowLayout {
                Kirigami.FormData.label: i18n("Interval:")
                visible: page.scheduleKind === "every_n_days"
                spacing: Kirigami.Units.smallSpacing

                Controls.Label {
                    text: i18n("Every")
                }

                Item {
                    implicitWidth: intervalSpinBox.implicitWidth + 4
                    implicitHeight: intervalSpinBox.implicitHeight + 4

                    Controls.SpinBox {
                        id: intervalSpinBox
                        objectName: "scheduleIntervalSpinBox"
                        // The digits are centred in the whole box: Fusion makes the box lower than the
                        // padding of the style allows and would cut them.
                        topPadding: 0
                        bottomPadding: 0
                        anchors.centerIn: parent
                        from: 0
                        to: 365
                        value: page.intervalDays
                        editable: true
                        Accessible.name: i18n("Interval in days")
                        onValueModified: {
                            if (!page.hydrating) {
                                page.intervalDays = value;
                                page.refreshPreview();
                            }
                        }
                        onValueChanged: {
                            if (!page.hydrating && page.intervalDays !== value) {
                                page.intervalDays = value;
                                page.refreshPreview();
                            }
                        }
                    }

                    Rectangle {
                        objectName: "scheduleIntervalInvalidBorder"
                        anchors.fill: parent
                        color: "transparent"
                        radius: Kirigami.Units.cornerRadius
                        border.width: page.intervalInvalid ? 2 : 0
                        border.color: Kirigami.Theme.negativeTextColor
                        visible: page.intervalInvalid
                    }
                }

                Controls.Label {
                    text: i18np("day", "days", page.intervalDays)
                }
            }

            RowLayout {
                Kirigami.FormData.label: i18n("Start date:")
                visible: page.scheduleKind === "every_n_days"
                spacing: Kirigami.Units.smallSpacing

                Controls.Button {
                    objectName: "scheduleStartDatePickerButton"
                    icon.name: "view-calendar"
                    text: page.startDate !== "" ? page.startDate : i18n("Choose date…")
                    Accessible.name: i18n("Choose start date")
                    Controls.ToolTip.visible: hovered
                    Controls.ToolTip.text: i18n("Choose start date")

                    Rectangle {
                        objectName: "scheduleStartDateInvalidBorder"
                        anchors.fill: parent
                        color: "transparent"
                        radius: Kirigami.Units.cornerRadius
                        border.width: 2
                        border.color: Kirigami.Theme.negativeTextColor
                        visible: page.startDateInvalid
                    }

                    onClicked: {
                        startDatePickerPopup.value = page.parseIsoDateOrToday(page.startDate);
                        startDatePickerPopup.open();
                    }
                }
            }

            RowLayout {
                Kirigami.FormData.label: Texts.label("time_label")
                visible: page.scheduleKind === "daily" || page.scheduleKind === "specific_weekdays" || page.scheduleKind === "every_n_days"
                spacing: Kirigami.Units.smallSpacing

                Controls.SpinBox {
                    id: hourSpinBox
                    objectName: "scheduleHourSpinBox"
                    // The digits are centred in the whole box: Fusion makes the box lower than the
                    // padding of the style allows and would cut them.
                    topPadding: 0
                    bottomPadding: 0
                    from: 0
                    to: 23
                    value: page.hour
                    editable: true
                    textFromValue: function(v) { return v < 10 ? "0" + v : String(v); }
                    valueFromText: function(t) { var n = parseInt(t, 10); return isNaN(n) ? hourSpinBox.value : n; }
                    Accessible.name: i18n("Hour")
                    onValueModified: {
                        if (!page.hydrating) {
                            page.hour = value;
                            page.refreshPreview();
                        }
                    }
                    onValueChanged: {
                        if (!page.hydrating && page.hour !== value) {
                            page.hour = value;
                            page.refreshPreview();
                        }
                    }
                }

                Controls.Label { text: ":" }

                Controls.SpinBox {
                    id: minuteSpinBox
                    objectName: "scheduleMinuteSpinBox"
                    // The digits are centred in the whole box: Fusion makes the box lower than the
                    // padding of the style allows and would cut them.
                    topPadding: 0
                    bottomPadding: 0
                    from: 0
                    to: 59
                    value: page.minute
                    editable: true
                    textFromValue: function(v) { return v < 10 ? "0" + v : String(v); }
                    valueFromText: function(t) { var n = parseInt(t, 10); return isNaN(n) ? minuteSpinBox.value : n; }
                    Accessible.name: i18n("Minute")
                    onValueModified: {
                        if (!page.hydrating) {
                            page.minute = value;
                            page.refreshPreview();
                        }
                    }
                    onValueChanged: {
                        if (!page.hydrating && page.minute !== value) {
                            page.minute = value;
                            page.refreshPreview();
                        }
                    }
                }
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Monitoring")
                visible: page.scheduleKind !== "manual_only"
            }

            Controls.CheckBox {
                id: useGlobalOverdueCheck
                objectName: "useGlobalOverdueCheck"
                visible: page.scheduleKind !== "manual_only"
                Kirigami.FormData.label: i18n("Report overdue after:")
                text: i18np("Use global default (%1 missed run)", "Use global default (%1 missed runs)", appBackend.default_overdue_threshold)
                checked: page.useGlobalOverdue
                Accessible.name: i18n("Use the global default for overdue")
                onToggled: if (!page.hydrating) page.useGlobalOverdue = checked
            }

            RowLayout {
                visible: page.scheduleKind !== "manual_only" && !page.useGlobalOverdue
                spacing: Kirigami.Units.smallSpacing

                Controls.SpinBox {
                    id: overdueThresholdSpinBox
                    objectName: "overdueThresholdSpinBox"
                    // The digits are centred in the whole box: Fusion makes the box lower than the
                    // padding of the style allows and would cut them.
                    topPadding: 0
                    bottomPadding: 0
                    from: 1
                    to: 99
                    value: page.customOverdueThreshold
                    editable: true
                    Accessible.name: i18n("Missed runs before reporting overdue")
                    onValueModified: if (!page.hydrating) page.customOverdueThreshold = value
                    onValueChanged: if (!page.hydrating && page.customOverdueThreshold !== value) page.customOverdueThreshold = value
                }

                Controls.Label {
                    text: i18np("missed run", "missed runs", page.customOverdueThreshold)
                }
            }
        }

        Kirigami.InlineMessage {
            objectName: "scheduleManualOnlyHint"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: page.scheduleKind === "manual_only"
            text: i18n("Manual backups do not run automatically and are never marked as overdue.")
        }

        ColumnLayout {
            Layout.fillWidth: true
            visible: page.scheduleKind !== "manual_only" && page.valid && page.previewSlots.length > 0
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Heading {
                level: 4
                text: i18n("Next 3 scheduled runs:")
            }

            Repeater {
                model: page.previewSlots
                delegate: Controls.Label {
                    required property string modelData
                    objectName: "schedulePreviewSlotLabel"
                    text: "• " + modelData
                    opacity: 0.85
                    Layout.fillWidth: true
                }
            }
        }
    }
}
