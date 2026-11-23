// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

EditorPage {
    id: page
    objectName: "backupExcludesEditPage"
    title: Texts.label("exclude_rules")
    horizontalScrollBarPolicy: Qt.ScrollBarAlwaysOff

    required property string backupId

    property var initialData: rootWindow.findBackupData(backupId)
    property string savedExcludesJson: initialData ? initialData.excludesJson : "{}"
    property bool hydrating: false

    property var folders: []
    property string excludesText: ""
    property bool excludeCaches: false
    property bool excludeTrash: false
    property bool sizeLimitEnabled: false
    property int sizeLimitNumber: 100
    property string sizeLimitUnit: "M"
    property bool ignoreCase: false
    property string excludeIfPresentText: ""
    property string saveErrorText: ""
    property var lintWarnings: []
    property string testPathResultText: ""
    property bool testPathIncluded: true
    property bool hasTestPathResult: false

    readonly property var sizeUnitLabels: ["KB", "MB", "GB", "TB"]
    readonly property var sizeUnitSuffixes: ["K", "M", "G", "T"]

    function parseSizeLimit(raw) {
        if (!raw || typeof raw !== "string") {
            return { enabled: false, num: 100, unit: "M" };
        }
        var m = /^\s*(\d+)\s*([KkMmGgTt])[Bb]?\s*$/.exec(raw);
        if (!m) {
            return { enabled: false, num: 100, unit: "M" };
        }
        var unit = m[2].toUpperCase();
        return { enabled: true, num: Math.max(1, parseInt(m[1], 10)), unit: unit };
    }

    function loadFromExcludesJson(jsonStr) {
        hydrating = true;
        var obj = rootWindow.parseJsonOr(jsonStr, {});
        folders = Array.isArray(obj.folders) ? obj.folders.slice() : [];
        excludesText = Array.isArray(obj.lines) ? obj.lines.join("\n") : "";
        excludeCaches = Boolean(obj.exclude_caches);
        excludeTrash = Boolean(obj.exclude_trash);
        var sl = parseSizeLimit(obj.exclude_larger_than);
        sizeLimitEnabled = sl.enabled;
        sizeLimitNumber = sl.num;
        sizeLimitUnit = sl.unit;
        ignoreCase = Boolean(obj.ignore_case);
        excludeIfPresentText = Array.isArray(obj.exclude_if_present)
            ? obj.exclude_if_present.join(", ")
            : "";
        hydrating = false;
        refreshLint();
    }

    function splitNonEmptyLines(raw) {
        return raw === "" ? [] : (raw || "").split(/\r?\n/);
    }

    function splitMarkers(raw) {
        return (raw || "").split(",").map(s => s.trim()).filter(Boolean);
    }

    function currentExcludesObj() {
        var obj = {
            folders: folders.slice(),
            lines: splitNonEmptyLines(excludesText),
            exclude_caches: excludeCaches,
            exclude_trash: excludeTrash,
            ignore_case: ignoreCase,
            exclude_if_present: splitMarkers(excludeIfPresentText)
        };
        if (sizeLimitEnabled) {
            obj.exclude_larger_than = Math.max(1, sizeLimitNumber) + sizeLimitUnit;
        }
        return obj;
    }

    function currentExcludesJson() {
        return JSON.stringify(currentExcludesObj());
    }

    function canonicalExcludes(jsonStr) {
        var obj = rootWindow.parseJsonOr(jsonStr, {});
        return JSON.stringify({
            folders: Array.isArray(obj.folders) ? obj.folders : [],
            lines: Array.isArray(obj.lines) ? obj.lines : [],
            exclude_caches: Boolean(obj.exclude_caches),
            exclude_trash: Boolean(obj.exclude_trash),
            exclude_larger_than: obj.exclude_larger_than || null,
            ignore_case: Boolean(obj.ignore_case),
            exclude_if_present: Array.isArray(obj.exclude_if_present) ? obj.exclude_if_present : []
        });
    }

    readonly property int expertRuleCount: splitNonEmptyLines(excludesText)
        .filter(l => { var t = l.trim(); return t !== "" && !t.startsWith("#"); }).length

    dirty: initialData !== null && canonicalExcludes(currentExcludesJson()) !== canonicalExcludes(savedExcludesJson)

    Component.onCompleted: {
        if (initialData) {
            savedExcludesJson = initialData.excludesJson;
            loadFromExcludesJson(savedExcludesJson);
        }
    }

    function refreshLint() {
        if (hydrating) return;
        var raw = appBackend.lintExcludes(currentExcludesJson());
        lintWarnings = rootWindow.parseJsonOr(raw, []);
    }

    function addExcludeFolder(rawPath) {
        var trimmed = (rawPath || "").trim();
        if (trimmed === "") return;
        var normalized = appBackend.normalizeExcludeFolder(trimmed) || trimmed;
        if (folders.indexOf(normalized) === -1) {
            folders = folders.concat([normalized]);
        }
    }

    function removeExcludeFolder(idx) {
        if (idx >= 0 && idx < folders.length) {
            folders = folders.filter((_, i) => i !== idx);
        }
    }

    function applyLintSuggestion(warningItem) {
        if (!warningItem) return;
        var replaced = false;
        excludesText = splitNonEmptyLines(excludesText).map(line => {
            if (!replaced && line.trim() === warningItem.parent_rule) {
                replaced = true;
                return warningItem.suggested_fix;
            }
            return line;
        }).join("\n");
        refreshLint();
    }

    function runTestPath() {
        var tp = testExcludePathField.text.trim();
        if (tp === "") return;
        var raw = appBackend.testExcludePath(backupId, currentExcludesJson(), tp);
        var res = rootWindow.parseJsonOr(raw, null);
        if (!res) return;
        testPathIncluded = Boolean(res.included);
        hasTestPathResult = true;
        if (res.included) {
            testPathResultText = i18n("Will be backed up.");
        } else {
            var ruleLabel = res.matched_rule || "";
            if (res.matched_line) {
                testPathResultText = i18n("Excluded by line %1 (%2).", res.matched_line, ruleLabel);
            } else {
                testPathResultText = i18n("Excluded by rule: %1", ruleLabel);
            }
        }
    }

    function saveForGuard() {
        if (!initialData) return false;
        saveErrorText = "";
        if (appBackend.updateBackupExcludes(backupId, currentExcludesJson())) {
            initialData = rootWindow.findBackupData(backupId);
            if (initialData) {
                savedExcludesJson = initialData.excludesJson;
                loadFromExcludesJson(savedExcludesJson);
            }
            rootWindow.showPassiveNotification(i18n("Exclude rules saved"));
            return true;
        }
        saveErrorText = appBackend.last_action_error;
        return false;
    }

    actions: [
        Kirigami.Action {
            id: previewExcludesAction
            objectName: "previewExcludesAction"
            icon.name: "document-preview"
            text: Texts.label("preview_ellipsis")
            enabled: appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: rootWindow.openBackupDryRunPreviewPage(backupId, page.currentExcludesJson())
        },
        Kirigami.Action {
            id: saveExcludesAction
            objectName: "saveExcludesAction"
            icon.name: "document-save"
            text: Texts.label("save")
            enabled: page.dirty && appBackend.blocking_kind !== "daemon_unreachable"
            onTriggered: page.saveForGuard()
        },
        Kirigami.Action {
            id: closeExcludesAction
            objectName: "closeExcludesAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: page.requestClose()
        }
    ]

    FolderDialog {
        id: addFolderDialog
        title: i18n("Choose Folder to Exclude")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                page.addExcludeFolder(local);
            }
        }
    }

    unsavedObjectName: "unsavedExcludesDialog"
    unsavedSubtitle: i18n("You have unsaved changes to exclude rules.")
    extraDialogs: [enableExpertModeDialog]

    onUnsavedSaveRequested: {
        if (page.saveForGuard()) {
            rootWindow.popSubPage();
        }
    }
    onUnsavedDiscardRequested: {
        page.loadFromExcludesJson(page.savedExcludesJson);
        rootWindow.popSubPage();
    }

    Kirigami.PromptDialog {
        id: enableExpertModeDialog
        objectName: "enableExpertModeDialog"
        title: i18n("Enable Expert Mode?")
        subtitle: i18n("In Expert Mode you write exclude rules in restic syntax, including patterns and exceptions. You can turn it off at any time in Settings (menu at the top right).")
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                id: confirmEnableExpertModeAction
                objectName: "confirmEnableExpertModeAction"
                icon.name: "configure"
                text: i18n("Enable Expert Mode")
                onTriggered: {
                    appBackend.updateExpertMode(true);
                    enableExpertModeDialog.close();
                }
            }
        ]
    }

    ColumnLayout {
        width: page.flickable.width
        spacing: Kirigami.Units.mediumSpacing

        Kirigami.InlineMessage {
            id: excludeSaveErrorBanner
            objectName: "excludeSaveErrorBanner"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: page.saveErrorText !== ""
            text: page.saveErrorText
        }

        // Section: Exclude these folders (Simple Mode)
        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Heading {
                level: 3
                text: i18n("Exclude these folders")
                Layout.fillWidth: true
            }

            Controls.Button {
                id: addExcludeFolderButton
                objectName: "addExcludeFolderButton"
                icon.name: "list-add"
                text: i18n("Add Folder…")
                Accessible.name: i18n("Add folder to exclude")
                onClicked: addFolderDialog.open()
            }
        }

        Controls.Label {
            visible: page.folders.length === 0
            text: i18n("No folders excluded yet.")
            opacity: 0.75
            Layout.fillWidth: true
        }

        Repeater {
            model: page.folders

            delegate: RowLayout {
                required property int index
                required property string modelData
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Kirigami.Icon {
                    source: "folder"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }

                Controls.Label {
                    text: modelData
                    font: Kirigami.Theme.fixedWidthFont
                    elide: Text.ElideMiddle
                    Layout.fillWidth: true
                }

                Controls.ToolButton {
                    objectName: "removeExcludeFolderButton"
                    icon.name: "edit-delete"
                    Accessible.name: i18n("Remove excluded folder %1", modelData)
                    onClicked: {
                        page.removeExcludeFolder(index);
                        addExcludeFolderButton.forceActiveFocus(Qt.TabFocusReason);
                    }
                }
            }
        }

        // Section: Templates
        Kirigami.Heading {
            level: 3
            text: i18n("Templates")
        }

        Controls.CheckBox {
            id: excludeCachesCheckbox
            objectName: "excludeCachesCheckbox"
            text: i18n("Exclude cache folders (with CACHEDIR.TAG)")
            checked: page.excludeCaches
            onToggled: {
                page.excludeCaches = checked;
                excludeCachesCheckbox.forceActiveFocus(Qt.TabFocusReason);
            }
            Accessible.name: i18n("Exclude cache folders with CACHEDIR.TAG")
        }

        Controls.CheckBox {
            id: excludeTrashCheckbox
            objectName: "excludeTrashCheckbox"
            text: Texts.label("exclude_trash")
            checked: page.excludeTrash
            onToggled: {
                page.excludeTrash = checked;
                excludeTrashCheckbox.forceActiveFocus(Qt.TabFocusReason);
            }
            Accessible.name: Texts.label("exclude_trash")
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Controls.CheckBox {
                id: excludeLargerThanCheckbox
                objectName: "excludeLargerThanCheckbox"
                text: i18n("Exclude files larger than")
                checked: page.sizeLimitEnabled
                onToggled: {
                    page.sizeLimitEnabled = checked;
                    excludeLargerThanCheckbox.forceActiveFocus(Qt.TabFocusReason);
                }
                Accessible.name: i18n("Exclude files larger than")
            }

            Controls.SpinBox {
                id: sizeLimitNumberField
                objectName: "sizeLimitNumberField"
                // The digits are centred in the whole box: Fusion makes the box lower than the
                // padding of the style allows and would cut them.
                topPadding: 0
                bottomPadding: 0
                from: 1
                to: 999999
                editable: true
                enabled: page.sizeLimitEnabled
                value: page.sizeLimitNumber
                onValueModified: page.sizeLimitNumber = value
                Accessible.name: i18n("File size limit value")
            }

            Controls.ComboBox {
                id: sizeLimitUnitCombo
                objectName: "sizeLimitUnitCombo"
                enabled: page.sizeLimitEnabled
                model: page.sizeUnitLabels
                currentIndex: Math.max(0, page.sizeUnitSuffixes.indexOf(page.sizeLimitUnit))
                onActivated: function(idx) {
                    page.sizeLimitUnit = page.sizeUnitSuffixes[idx] || "M";
                }
                Accessible.name: i18n("File size limit unit")
            }
        }

        // Simple mode read-only notice for restic rules
        ColumnLayout {
            Layout.fillWidth: true
            visible: !appBackend.expert_mode
            spacing: Kirigami.Units.smallSpacing

            Kirigami.InlineMessage {
                id: expertRulesReadonlyInfo
                objectName: "expertRulesReadonlyInfo"
                Layout.fillWidth: true
                visible: !appBackend.expert_mode
                type: Kirigami.MessageType.Information
                text: page.expertRuleCount > 0
                    ? i18np("%1 more rule in restic syntax. Enable Expert Mode to edit it.", "%1 more rules in restic syntax. Enable Expert Mode to edit them.", page.expertRuleCount)
                    : i18n("Additional rules in restic syntax can be edited when Expert Mode is enabled.")
            }

            Controls.Button {
                id: enableExpertModeAction
                objectName: "enableExpertModeAction"
                Layout.alignment: Qt.AlignRight
                icon.name: "configure"
                text: i18n("Enable Expert Mode…")
                Accessible.name: text
                function trigger() { enableExpertModeDialog.open(); }
                onClicked: trigger()
            }
        }

        // Expert mode section
        ColumnLayout {
            objectName: "excludeExpertSection"
            visible: appBackend.expert_mode
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Heading {
                level: 3
                text: i18n("Expert Rules")
            }

            Controls.Label {
                text: i18n("Rules in restic syntax (one per line: globs, **, !exceptions, $VAR; no regex):")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Controls.ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: Kirigami.Units.gridUnit * 12

                TabNavTextArea {
                    id: excludesTextArea
                    objectName: "excludesTextArea"
                    Kirigami.Theme.colorSet: Kirigami.Theme.View
                    Kirigami.Theme.inherit: false
                    font: Kirigami.Theme.fixedWidthFont
                    color: Kirigami.Theme.textColor
                    text: page.excludesText
                    onTextChanged: {
                        page.excludesText = text;
                        page.refreshLint();
                    }
                    placeholderText: "*.tmp\n.cache\n!$HOME/.cache/keep"
                    Accessible.name: Texts.label("exclude_rules_syntax")

                    background: Rectangle {
                        color: Kirigami.Theme.backgroundColor
                        radius: Kirigami.Units.cornerRadius
                        border.width: excludesTextArea.activeFocus ? 2 : 1
                        border.color: excludesTextArea.activeFocus
                            ? Kirigami.Theme.highlightColor
                            : Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.textColor, 0.25)
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                visible: excludeLintWarningMessage.activeWarn !== null
                spacing: Kirigami.Units.smallSpacing

                Kirigami.InlineMessage {
                    id: excludeLintWarningMessage
                    objectName: "excludeLintWarningMessage"
                    Layout.fillWidth: true
                    readonly property var activeWarn: page.lintWarnings.length > 0 ? page.lintWarnings[0] : null
                    visible: activeWarn !== null
                    type: Kirigami.MessageType.Warning
                    text: activeWarn
                        ? i18n("Line %1: '%2' has no effect because '%3' excludes the folder itself. Use '%4' instead.",
                               activeWarn.line_number,
                               activeWarn.rule,
                               activeWarn.parent_rule,
                               activeWarn.suggested_fix)
                        : ""
                }

                Controls.Button {
                    id: applyLintSuggestionAction
                    objectName: "applyLintSuggestionAction"
                    Layout.alignment: Qt.AlignRight
                    text: i18n("Use '%1'", excludeLintWarningMessage.activeWarn ? excludeLintWarningMessage.activeWarn.suggested_fix : "/*")
                    Accessible.name: text
                    function trigger() {
                        if (excludeLintWarningMessage.activeWarn) {
                            page.applyLintSuggestion(excludeLintWarningMessage.activeWarn);
                        }
                    }
                    onClicked: trigger()
                }
            }

            Controls.CheckBox {
                id: ignoreCaseCheckbox
                objectName: "ignoreCaseCheckbox"
                text: i18n("Ignore upper/lower case (--iexclude)")
                checked: page.ignoreCase
                onToggled: {
                    page.ignoreCase = checked;
                    ignoreCaseCheckbox.forceActiveFocus(Qt.TabFocusReason);
                }
                Accessible.name: i18n("Ignore upper and lower case")
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Controls.Label {
                    text: i18n("Exclude folders with marker file:")
                }

                Controls.TextField {
                    id: excludeIfPresentField
                    objectName: "excludeIfPresentField"
                    Layout.fillWidth: true
                    text: page.excludeIfPresentText
                    placeholderText: ".nobackup"
                    onTextChanged: page.excludeIfPresentText = text
                    Accessible.name: i18n("Marker file name for excluding folders")
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                Controls.TextField {
                    id: testExcludePathField
                    objectName: "testExcludePathField"
                    Layout.fillWidth: true
                    placeholderText: i18n("Enter relative or absolute path to test…")
                    Accessible.name: i18n("Path to test against exclude rules")
                    onAccepted: page.runTestPath()
                }

                Controls.Button {
                    id: testExcludePathButton
                    objectName: "testExcludePathButton"
                    text: i18n("Test Path…")
                    Accessible.name: i18n("Test path against exclude rules")
                    onClicked: page.runTestPath()
                }
            }

            Kirigami.InlineMessage {
                id: testPathResultBanner
                objectName: "testPathResultBanner"
                Layout.fillWidth: true
                visible: page.hasTestPathResult
                type: page.testPathIncluded ? Kirigami.MessageType.Positive : Kirigami.MessageType.Information
                text: page.testPathResultText
            }
        }
    }
}
