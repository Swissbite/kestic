// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ApplicationWindow {
    id: root
    objectName: "kesticMainWindow"
    title: "Kestic"
    width: 1180
    height: 740
    minimumWidth: Math.round(Kirigami.Units.gridUnit * 26)
    minimumHeight: Math.round(Kirigami.Units.gridUnit * 24)
    visible: true

    // Exactly 4 primary tabs in M1 (Overview, Backups, Destinations, History; Restore enters in M4)
    property int activeTab: 0
    onActiveTabChanged: syncNavChecked()
    readonly property bool narrowMode: width < Kirigami.Units.gridUnit * 35
    readonly property bool inWizardLayer: wizardPageStack.depth > 0

    property string cmdDialogText: ""
    property string cmdDialogTitle: ""
    property string pwdDialogDestId: ""
    property string pwdDialogDestName: ""
    property string renameBackupId: ""
    property string renameBackupCurrentName: ""
    property string removeBackupId: ""
    property string removeBackupName: ""
    property string removeDestId: ""
    property string removeDestName: ""
    property var pendingNavAction: null
    property var pendingNavCancel: null
    property bool navGuardBypass: false

    onActiveChanged: appBackend.windowActive(active, tabNameForIndex(activeTab))
    onClosing: appBackend.windowActive(false, "")

    readonly property var backendRef: appBackend
    readonly property var backupsModelRef: backupsModel
    readonly property var destinationsModelRef: destinationsModel
    readonly property var runsModelRef: runsModel
    readonly property var snapshotsModelRef: snapshotsModel
    readonly property var runLogsModelRef: runLogsModel

    function tabNameForIndex(idx) {
        switch (idx) {
        case 0: return "overview";
        case 1: return "backups";
        case 2: return "destinations";
        case 3: return "history";
        default: return "overview";
        }
    }

    function syncAllModels() {
        backupsModel.sync();
        destinationsModel.sync();
        runsModel.sync();
        snapshotsModel.sync();
        runLogsModel.sync();
    }

    function findBackupData(backupId) {
        return modelLookup.findBackupData(backupId);
    }

    function findDestinationData(destId) {
        return modelLookup.findDestinationData(destId);
    }

    property alias pendingRestoreFocusItem: focusManager.pendingRestoreFocusItem
    property alias nextTriggerItem: focusManager.nextTriggerItem
    property alias breadcrumbRevision: focusManager.breadcrumbRevision

    onNarrowModeChanged: focusManager.updatePageStackViewportAndFocusability()

    function clearSavedFocus() { focusManager.clearSavedFocus(); }
    function saveFocusForDepth(depth, item) { focusManager.saveFocusForDepth(depth, item); }
    function saveFocusForCurrentDepth(item) { focusManager.saveFocusForCurrentDepth(item); }
    function isItemInsidePage(item, pageObjectName) { return focusManager.isItemInsidePage(item, pageObjectName); }
    function updatePageStackViewportAndFocusability() { focusManager.updatePageStackViewportAndFocusability(); }
    function canNavigateBackInBreadcrumb(rev) { return focusManager.canNavigateBackInBreadcrumb(rev); }
    function defaultTitleForDepth(idx, rev) { return focusManager.defaultTitleForDepth(idx, rev); }
    function focusPrimaryListSelection() { focusManager.focusPrimaryListSelection(); }
    function removeTopPageFromStack() { focusManager.removeTopPageFromStack(); }
    function navigateToBreadcrumbIndex(targetIdx) { focusManager.navigateToBreadcrumbIndex(targetIdx); }
    function cancelFocusRestoreTimer() { focusManager.cancelFocusRestoreTimer(); }
    function focusItemInPageStack(targetItem) { return focusManager.focusItemInPageStack(targetItem); }
    function focusPageContent(pageItem) { focusManager.focusPageContent(pageItem); }
    function pushSubPageAtDepth(pageObj, targetDepth) { focusManager.pushSubPageAtDepth(pageObj, targetDepth); }
    function pushSubPage(pageObj) { focusManager.pushSubPage(pageObj); }
    function popSubPage() { focusManager.popSubPage(); }
    function replaceRootPage(pageObj) { focusManager.replaceRootPage(pageObj); }

    function syncNavChecked() {
        for (var i = 0; i < navActions.length; ++i) {
            navActions[i].checked = (i === root.activeTab);
        }
    }

    function isActivePageDirty() {
        if (appBackend.has_unsaved_changes) return true;
        var cur = pageStack.depth > 0 ? pageStack.get(pageStack.depth - 1) : null;
        return cur !== null && cur.dirty === true;
    }

    function saveActiveDirtyPage() {
        var cur = pageStack.depth > 0 ? pageStack.get(pageStack.depth - 1) : null;
        if (!cur) return true;
        if (typeof cur.requestSave === "function") {
            if (cur.pruneDirty === true) {
                cur.requestSave(false);
                return false;
            }
            return typeof cur.commitSave === "function" ? cur.commitSave(false) : false;
        }
        if (typeof cur.triggerSave === "function") {
            cur.triggerSave(false);
            return !appBackend.has_unsaved_changes && cur.dirty !== true;
        }
        if (typeof cur.saveChanges === "function") {
            return cur.saveChanges();
        }
        if (typeof cur.commitSave === "function") {
            if (cur.objectName === "backupSourceEditPage") {
                return cur.commitSave();
            }
            return cur.commitSave(false);
        }
        return false;
    }

    // AC-M3-14-9: every navigation that would leave a dirty editor goes through this guard.
    function guardDirtyNavigation(action, cancel) {
        if (navGuardBypass || !isActivePageDirty()) return false;
        pendingNavAction = action;
        pendingNavCancel = cancel || null;
        syncNavChecked();
        unsavedTabSwitchDialog.open();
        return true;
    }

    function runPendingNavigation() {
        var action = pendingNavAction;
        pendingNavAction = null;
        pendingNavCancel = null;
        if (action) {
            navGuardBypass = true;
            try {
                action();
            } finally {
                navGuardBypass = false;
            }
        }
    }

    function cancelPendingNavigation() {
        var cancel = pendingNavCancel;
        pendingNavAction = null;
        pendingNavCancel = null;
        if (cancel) cancel();
        syncNavChecked();
    }

    function switchTab(idx) {
        if (guardDirtyNavigation(function() { performSwitchTab(idx); })) return;
        performSwitchTab(idx);
    }

    function performSwitchTab(idx) {
        appBackend.has_unsaved_changes = false;
        while (wizardPageStack.depth > 0) {
            closeWizardLayer();
        }
        clearSavedFocus();
        activeTab = idx;
        syncNavChecked();
        var primaryPage = null;
        if (idx === 0) {
            primaryPage = overviewPageComponent.createObject(pageStack, {});
            replaceRootPage(primaryPage);
        } else if (idx === 1) {
            var firstId = backupsModel.rowCount(backupsModel.index(-1, -1)) > 0
                ? (backupsModel.data(backupsModel.index(0, 0), 0) || "")
                : "";
            primaryPage = backupsPageComponent.createObject(pageStack, { selectedBackupId: firstId });
            replaceRootPage(primaryPage);
            if (!narrowMode && firstId !== "") {
                pageStack.push(backupDetailComponent.createObject(pageStack, { backupId: firstId }));
                pageStack.currentIndex = 0;
            }
        } else if (idx === 2) {
            var firstDest = destinationsModel.rowCount(destinationsModel.index(-1, -1)) > 0
                ? (destinationsModel.data(destinationsModel.index(0, 0), 0) || "")
                : "";
            primaryPage = destinationsPageComponent.createObject(pageStack, { selectedDestinationId: firstDest });
            replaceRootPage(primaryPage);
            if (!narrowMode && firstDest !== "") {
                pageStack.push(destinationDetailComponent.createObject(pageStack, { destinationId: firstDest }));
                pageStack.currentIndex = 0;
            }
        } else if (idx === 3) {
            runsModel.setFilter("", false, "");
            var firstRun = runsModel.rowCount(runsModel.index(-1, -1)) > 0
                ? (runsModel.data(runsModel.index(0, 0), 0) || "")
                : "";
            primaryPage = historyPageComponent.createObject(pageStack, { selectedRunId: firstRun });
            replaceRootPage(primaryPage);
            if (!narrowMode && firstRun !== "") {
                pageStack.push(runDetailComponent.createObject(pageStack, { runId: firstRun }));
                pageStack.currentIndex = 0;
            }
        }
        updatePageStackViewportAndFocusability();
        if (primaryPage) {
            focusPageContent(primaryPage);
        }
    }

    function canActivateFocusedControlWithReturn() {
        var it = root.activeFocusItem;
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
            || (typeof it.clicked === "function" && (typeof it.down !== "undefined" || typeof it.showClickFeedback !== "undefined"));
    }

    function activateFocusedControlWithReturn() {
        var it = root.activeFocusItem;
        if (!canActivateFocusedControlWithReturn()) return;
        if (it.checkable && typeof it.animateClick === "function") {
            it.animateClick();
            if (it && it.visible && it.enabled) {
                it.forceActiveFocus(Qt.TabFocusReason);
            }
        } else if (typeof it.clicked === "function") {
            it.clicked();
        } else if (typeof it.animateClick === "function") {
            it.animateClick();
        }
    }

    function handleGlobalEscape() {
        var it = root.activeFocusItem;
        if (it && it.hasOwnProperty("popup") && it.popup && (it.popup.opened || it.popup.visible)) {
            it.popup.close();
            return true;
        }
        if (unsavedTabSwitchDialog.opened || unsavedTabSwitchDialog.visible) {
            unsavedTabSwitchDialog.close();
            root.cancelPendingNavigation();
            return true;
        }
        if (hamburgerMenu.visible) {
            hamburgerMenu.close();
            return true;
        }
        if (quitConfirmDialog.opened || quitConfirmDialog.visible) {
            quitConfirmDialog.close();
            return true;
        }
        if (removeDestinationDialog.opened || removeDestinationDialog.visible) {
            removeDestinationDialog.close();
            return true;
        }
        if (removeBackupDialog.opened || removeBackupDialog.visible) {
            removeBackupDialog.close();
            return true;
        }
        if (renameBackupDialog.opened || renameBackupDialog.visible) {
            renameBackupDialog.close();
            return true;
        }
        if (updatePasswordDialog.opened || updatePasswordDialog.visible) {
            updatePasswordDialog.close();
            return true;
        }
        if (showCommandDialog.opened || showCommandDialog.visible) {
            showCommandDialog.close();
            return true;
        }
        if (conditionOverrideDialog.opened || conditionOverrideDialog.visible) {
            conditionOverrideDialog.close();
            return true;
        }
        if (root.inWizardLayer) {
            var wiz = wizardPageStack.currentItem;
            if (wiz && typeof wiz.handleEscape === "function") {
                if (wiz.handleEscape()) return true;
            }
            closeWizardLayer();
            return true;
        }
        var cur = pageStack.depth > 0 ? pageStack.get(pageStack.depth - 1) : pageStack.currentItem;
        if (cur && typeof cur.handleEscape === "function") {
            if (cur.handleEscape()) return true;
        }
        if (pageStack.depth > 1) {
            var backEvent = { accepted: false };
            if (cur && typeof cur.backRequested === "function") {
                cur.backRequested(backEvent);
            }
            if (!backEvent.accepted) {
                if (!root.narrowMode && pageStack.depth === 2 && (root.activeTab === 1 || root.activeTab === 2 || root.activeTab === 3)) {
                    focusPrimaryListSelection();
                } else {
                    popSubPage();
                }
            }
            return true;
        }
        return false;
    }

    function openBackupDetailById(backupId) {
        if (guardDirtyNavigation(function() { openBackupDetailById(backupId); })) return;
        if (activeTab !== 1) {
            activeTab = 1;
            clearSavedFocus();
            replaceRootPage(backupsPageComponent.createObject(pageStack, { selectedBackupId: backupId }));
            var detailFromOtherTab = backupDetailComponent.createObject(pageStack, { backupId: backupId });
            pageStack.push(detailFromOtherTab);
            updatePageStackViewportAndFocusability();
            focusPageContent(detailFromOtherTab);
            return;
        }
        var prevFocus = root.activeFocusItem;
        while (pageStack.depth > 2) {
            removeTopPageFromStack();
        }
        var existingDetail = pageStack.depth === 2 ? pageStack.get(1) : null;
        if (existingDetail && existingDetail.objectName === "backupDetailPage") {
            existingDetail.backupId = backupId;
            existingDetail.refreshFromModel();
        } else {
            while (pageStack.depth > 1) removeTopPageFromStack();
            pageStack.push(backupDetailComponent.createObject(pageStack, { backupId: backupId }));
        }
        updatePageStackViewportAndFocusability();
        Qt.callLater(function() {
            updatePageStackViewportAndFocusability();
            if (prevFocus && prevFocus.visible && prevFocus.enabled) {
                focusItemInPageStack(prevFocus);
            }
        });
    }

    function openDestinationDetailById(destId) {
        if (guardDirtyNavigation(function() { openDestinationDetailById(destId); })) return;
        if (activeTab !== 2) {
            activeTab = 2;
            clearSavedFocus();
            replaceRootPage(destinationsPageComponent.createObject(pageStack, { selectedDestinationId: destId }));
            var destDetailFromOtherTab = destinationDetailComponent.createObject(pageStack, { destinationId: destId });
            pageStack.push(destDetailFromOtherTab);
            updatePageStackViewportAndFocusability();
            focusPageContent(destDetailFromOtherTab);
            return;
        }
        var prevFocus = root.activeFocusItem;
        while (pageStack.depth > 2) {
            removeTopPageFromStack();
        }
        var existingDestDetail = pageStack.depth === 2 ? pageStack.get(1) : null;
        if (existingDestDetail && existingDestDetail.objectName === "destinationDetailPage") {
            existingDestDetail.destinationId = destId;
            existingDestDetail.refreshFromModel();
        } else {
            while (pageStack.depth > 1) removeTopPageFromStack();
            pageStack.push(destinationDetailComponent.createObject(pageStack, { destinationId: destId }));
        }
        updatePageStackViewportAndFocusability();
        Qt.callLater(function() {
            updatePageStackViewportAndFocusability();
            if (prevFocus && prevFocus.visible && prevFocus.enabled) {
                focusItemInPageStack(prevFocus);
            }
        });
    }

    function targetDepthForBackupEditor() {
        var trig = nextTriggerItem || root.activeFocusItem;
        if (trig && isItemInsidePage(trig, "backupFullConfigPage")) {
            return 3;
        }
        return 2;
    }

    function openDestinationEditPage(destId) {
        pushSubPageAtDepth(destinationEditComponent.createObject(pageStack, { destinationId: destId }), 2);
    }

    function openDestinationMaintenancePage(destId) {
        pushSubPageAtDepth(destinationMaintenanceComponent.createObject(pageStack, { destinationId: destId }), 2);
    }

    function openBackupSourcePage(backupId) {
        pushSubPageAtDepth(backupSourceEditComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupExcludesPage(backupId) {
        pushSubPageAtDepth(backupExcludesEditComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupRetentionPage(backupId) {
        pushSubPageAtDepth(backupRetentionComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupSchedulePage(backupId) {
        pushSubPageAtDepth(backupScheduleEditComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupConditionsPage(backupId) {
        pushSubPageAtDepth(backupConditionsEditComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupDryRunPreviewPage(backupId, excludesJson) {
        var topPage = pageStack.depth > 0 ? pageStack.get(pageStack.depth - 1) : pageStack.currentItem;
        if (topPage && topPage.objectName === "backupPreviewPage") {
            topPage.backupId = backupId;
            topPage.mode = "dry_run";
            topPage.excludesJson = excludesJson || "";
            topPage.startDryRunIfNeeded();
            focusPageContent(topPage);
            return;
        }
        pushSubPageAtDepth(backupPreviewComponent.createObject(pageStack, {
            backupId: backupId,
            mode: "dry_run",
            excludesJson: excludesJson || ""
        }), Math.min(pageStack.depth, 3));
    }

    function openRetentionPreviewPage(backupId, retentionJson) {
        var topPage = pageStack.depth > 0 ? pageStack.get(pageStack.depth - 1) : pageStack.currentItem;
        if (topPage && topPage.objectName === "backupPreviewPage") {
            topPage.backupId = backupId;
            topPage.mode = "retention";
            topPage.retentionJson = retentionJson || "";
            topPage.startRetentionPreviewIfNeeded();
            focusPageContent(topPage);
            return;
        }
        pushSubPageAtDepth(backupPreviewComponent.createObject(pageStack, {
            backupId: backupId,
            mode: "retention",
            retentionJson: retentionJson || ""
        }), Math.min(pageStack.depth, 3));
    }

    function openBackupReassignPage(backupId) {
        pushSubPageAtDepth(backupReassignComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupSnapshotsPage(backupId, backupName) {
        pushSubPageAtDepth(backupSnapshotsComponent.createObject(pageStack, { backupId: backupId, backupName: backupName }), 2);
    }

    function openBackupFullConfigPage(backupId) {
        pushSubPageAtDepth(backupFullConfigComponent.createObject(pageStack, { backupId: backupId }), 2);
    }

    function openRunDetailById(runId) {
        if (guardDirtyNavigation(function() { openRunDetailById(runId); })) return;
        if (activeTab !== 3) {
            activeTab = 3;
            clearSavedFocus();
            replaceRootPage(historyPageComponent.createObject(pageStack, { selectedRunId: runId }));
            var runDetailFromOtherTab = runDetailComponent.createObject(pageStack, { runId: runId });
            pageStack.push(runDetailFromOtherTab);
            updatePageStackViewportAndFocusability();
            focusPageContent(runDetailFromOtherTab);
            return;
        }
        var prevFocus = root.activeFocusItem;
        while (pageStack.depth > 2) {
            removeTopPageFromStack();
        }
        var listPage = pageStack.depth >= 1 ? pageStack.get(0) : null;
        if (listPage && typeof listPage.selectedRunId !== "undefined") {
            listPage.selectedRunId = runId;
        }
        var existingRunDetail = pageStack.depth === 2 ? pageStack.get(1) : null;
        if (existingRunDetail && existingRunDetail.objectName === "runDetailPage") {
            existingRunDetail.runId = runId;
            existingRunDetail.refreshDetail();
        } else {
            while (pageStack.depth > 1) removeTopPageFromStack();
            pageStack.push(runDetailComponent.createObject(pageStack, { runId: runId }));
        }
        updatePageStackViewportAndFocusability();
        Qt.callLater(function() {
            updatePageStackViewportAndFocusability();
            if (prevFocus && prevFocus.visible && prevFocus.enabled) {
                focusItemInPageStack(prevFocus);
            }
        });
    }

    function closeWizardLayer() {
        if (wizardPageStack.depth > 1) {
            wizardPageStack.pop();
        } else if (wizardPageStack.depth === 1) {
            var p = wizardPageStack.currentItem;
            wizardPageStack.clear();
            if (p) {
                p.destroy();
            }
        }
    }

    function openNewBackupWizard(firstSetup) {
        var topLayer = wizardPageStack.currentItem;
        if (topLayer && topLayer.objectName === "newBackupWizard") {
            return;
        }
        var hasDestinations = destinationsModel.rowCount(destinationsModel.index(-1, -1)) > 0;
        var integrated = !hasDestinations;
        var titleText = firstSetup ? Texts.label("set_up_first_backup") : Texts.label("new_backup");
        wizardPageStack.push(newBackupWizardComponent.createObject(wizardPageStack, {
            integratedDestFlow: integrated,
            wizardTitle: titleText
        }));
    }

    function openAddDestinationWizard(callback, initialSetupMode) {
        var topLayer = wizardPageStack.currentItem;
        if (topLayer && topLayer.objectName === "addDestinationWizard") {
            return;
        }
        wizardPageStack.push(addDestinationWizardComponent.createObject(wizardPageStack, {
            destinationCreatedCallback: callback || null,
            setupMode: initialSetupMode || "init"
        }));
    }

    function openShowCommandDialog(cmdText, title) {
        cmdDialogText = cmdText;
        cmdDialogTitle = title;
        showCommandDialog.open();
    }

    function openUpdatePasswordDialog(destId, destName) {
        pwdDialogDestId = destId;
        pwdDialogDestName = destName;
        updatePasswordDialog.clearPasswordField();
        updatePasswordDialog.open();
    }

    function openRenameBackupDialog(backupId, currentName) {
        renameBackupId = backupId;
        renameBackupCurrentName = currentName;
        renameBackupDialog.setRenameText(currentName);
        renameBackupDialog.open();
    }

    function openRemoveBackupDialog(backupId, backupName) {
        removeBackupId = backupId;
        removeBackupName = backupName;
        removeBackupDialog.open();
    }

    function openRemoveDestinationDialog(destId, destName) {
        removeDestId = destId;
        removeDestName = destName;
        removeDestinationDialog.open();
    }

    function openSettingsWindow() {
        settingsWindow.openWindow();
    }

    function resizeToMinWidthForSmoke() {
        width = Math.round(Kirigami.Units.gridUnit * 26);
        height = Math.round(Kirigami.Units.gridUnit * 24);
    }

    function triggerBlockingSmokeSetup() {
        switchTab(0);
    }

    KesticApp {
        id: appBackend
        objectName: "appBackend"
        onModelsUpdated: root.syncAllModels()
        onPassiveNotification: function(message) {
            root.showPassiveNotification(message);
        }
        onShowWindowRequested: {
            root.show();
            if (Qt.platform.pluginName !== "offscreen") {
                root.raise();
            }
            root.requestActivate();
        }
        onQuitConfirmationRequested: function() {
            root.show();
            if (Qt.platform.pluginName !== "offscreen") {
                root.raise();
            }
            root.requestActivate();
            quitConfirmDialog.open();
        }
        onConditionOverrideRequested: function(backupId, backupName, message) {
            conditionOverrideDialog.open();
        }
    }

    BackupsModel { id: backupsModel; objectName: "backupsModel" }
    DestinationsModel { id: destinationsModel; objectName: "destinationsModel" }
    RunsModel { id: runsModel; objectName: "runsModel" }
    SnapshotsModel { id: snapshotsModel; objectName: "snapshotsModel" }
    RunLogsModel { id: runLogsModel; objectName: "runLogsModel" }

    ModelLookupHelper {
        id: modelLookup
        backupsModel: root.backupsModelRef
        destinationsModel: root.destinationsModelRef
    }

    PageFocusManager {
        id: focusManager
        rootWindow: root
        pageStack: root.pageStack
    }

    SettingsDialog {
        id: settingsWindow
        appBackend: root.backendRef
    }

    Timer {
        interval: 150
        running: true
        repeat: true
        onTriggered: appBackend.pollEvents()
    }

    Shortcut {
        sequence: "Ctrl+N"
        context: Qt.ApplicationShortcut
        enabled: appBackend.daemon_reachable && appBackend.blocking_kind !== "daemon_unreachable" && !root.inWizardLayer
        onActivated: root.openNewBackupWizard(false)
    }

    Shortcut {
        sequence: "Ctrl+Shift+,"
        context: Qt.ApplicationShortcut
        onActivated: root.openSettingsWindow()
    }

    function isMenuPopupFocused() {
        if (hamburgerMenu.visible) {
            return false;
        }
        var p = root.activeFocusItem;
        for (var i = 0; i < 8 && p; ++i) {
            if (p.Accessible && (p.Accessible.role === Accessible.MenuItem
                    || p.Accessible.role === Accessible.PopupMenu)) {
                return true;
            }
            if (("" + p).indexOf("QQuickMenu") !== -1) {
                return true;
            }
            p = p.parent;
        }
        return false;
    }

    // Static Hamburger Menu (§4.1): no New Backup…, no Add Destination…, no Quit Kestic…
    globalDrawer: Kirigami.GlobalDrawer {
        id: hamburgerMenu
        objectName: "hamburgerMenu"
        isMenu: true
        modal: true
        actions: [
            Kirigami.Action {
                icon.name: "configure"
                text: i18n("Configure Kestic…")
                shortcut: "Ctrl+Shift+,"
                onTriggered: root.openSettingsWindow()
            },
            Kirigami.Action {
                icon.name: "help-about"
                text: i18n("About Kestic")
                onTriggered: root.pushSubPageAtDepth(aboutPageComponent.createObject(pageStack, {}), Math.min(pageStack.depth, 2))
            }
        ]
    }

    property list<Kirigami.Action> navActions: [
        Kirigami.Action {
            objectName: "overviewTabAction"
            icon.name: "dashboard-show"
            text: Texts.label("overview")
            checked: root.activeTab === 0
            onTriggered: root.switchTab(0)
        },
        Kirigami.Action {
            objectName: "backupsTabAction"
            icon.name: "folder-sync"
            text: Texts.label("backups")
            checked: root.activeTab === 1
            onTriggered: root.switchTab(1)
        },
        Kirigami.Action {
            objectName: "destinationsTabAction"
            icon.name: "network-server"
            text: Texts.label("destinations")
            checked: root.activeTab === 2
            onTriggered: root.switchTab(2)
        },
        Kirigami.Action {
            objectName: "historyTabAction"
            icon.name: "view-history"
            text: Texts.label("history")
            checked: root.activeTab === 3
            onTriggered: root.switchTab(3)
        }
    ]

    header: MainHeaderArea {
        width: parent ? parent.width : 0
        rootWindow: root
        appBackend: root.backendRef
        pageStackRef: pageStack
    }

    footer: Kirigami.NavigationTabBar {
        id: bottomNavBar
        objectName: "bottomNavBar"
        visible: root.narrowMode && !root.inWizardLayer
        height: visible ? implicitHeight : 0
        position: Controls.ToolBar.Footer
        actions: root.navActions
        delegate: Kirigami.NavigationTabButton {
            required property Controls.Action modelData
            parent: bottomNavBar.contentItem
            action: modelData
            onActionChanged: action.checkable = true
            Layout.minimumWidth: bottomNavBar.buttonWidth
            Layout.maximumWidth: bottomNavBar.buttonWidth
            Layout.fillHeight: true

            Kirigami.Icon {
                source: "emblem-important"
                visible: (modelData.objectName === "backupsTabAction" && appBackend.backups_have_attention)
                      || (modelData.objectName === "destinationsTabAction" && appBackend.destinations_have_attention)
                color: Kirigami.Theme.negativeTextColor
                width: Kirigami.Units.iconSizes.small
                height: Kirigami.Units.iconSizes.small
                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: Kirigami.Units.smallSpacing
            }
        }
    }

    pageStack.visible: !root.inWizardLayer
    pageStack.defaultColumnWidth: Math.max(Kirigami.Units.gridUnit * 17, Math.floor((pageStack.width - 2) / 2))
    pageStack.columnView.interactive: false
    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.Breadcrumb
    pageStack.globalToolBar.showNavigationButtons: Kirigami.ApplicationHeaderStyle.NoNavigationButtons
    pageStack.globalToolBar.minimumHeight: 0
    pageStack.globalToolBar.preferredHeight: 0
    pageStack.globalToolBar.maximumHeight: 0

    Kirigami.PageRow {
        id: wizardPageStack
        anchors.fill: parent
        z: 50
        visible: depth > 0
        defaultColumnWidth: Math.max(width, Kirigami.Units.gridUnit * 40)
        columnView.columnResizeMode: Kirigami.ColumnView.SingleColumn
        globalToolBar.style: Kirigami.ApplicationHeaderStyle.Breadcrumb
        globalToolBar.showNavigationButtons: Kirigami.ApplicationHeaderStyle.NoNavigationButtons
        onPageRemoved: function(page) {
            if (page) {
                page.destroy();
            }
        }
    }

    Connections {
        target: pageStack
        function onDepthChanged() {
            root.updatePageStackViewportAndFocusability();
        }
        function onWidthChanged() {
            root.updatePageStackViewportAndFocusability();
        }
        function onPageRemoved(page) {
            if (page) {
                page.destroy();
            }
            root.updatePageStackViewportAndFocusability();
            if (root.pendingRestoreFocusItem) {
                var target = root.pendingRestoreFocusItem;
                root.pendingRestoreFocusItem = null;
                Qt.callLater(function() {
                    root.updatePageStackViewportAndFocusability();
                    if (target && target.visible && target.enabled) {
                        root.focusItemInPageStack(target);
                    }
                });
            }
        }
    }

    Component.onCompleted: {
        appBackend.initialize();
        pageStack.push(overviewPageComponent.createObject(pageStack, {}));
        appBackend.windowActive(true, "overview");
    }

    // Dialogs (§8.4, §9.4, §9.5, FR-34)
    ShowCommandDialog {
        id: showCommandDialog
        rootWindow: root
        appBackend: root.backendRef
        cmdText: root.cmdDialogText
        cmdTitle: root.cmdDialogTitle
    }

    UpdatePasswordDialog {
        id: updatePasswordDialog
        appBackend: root.backendRef
        destId: root.pwdDialogDestId
        destName: root.pwdDialogDestName
    }

    RenameBackupDialog {
        id: renameBackupDialog
        rootWindow: root
        appBackend: root.backendRef
        backupId: root.renameBackupId
        currentName: root.renameBackupCurrentName
    }

    RemoveBackupDialog {
        id: removeBackupDialog
        rootWindow: root
        appBackend: root.backendRef
        backupId: root.removeBackupId
        backupName: root.removeBackupName
    }

    RemoveDestinationDialog {
        id: removeDestinationDialog
        rootWindow: root
        appBackend: root.backendRef
        destId: root.removeDestId
        destName: root.removeDestName
    }

    QuitConfirmDialog {
        id: quitConfirmDialog
        appBackend: root.backendRef
    }

    ConditionOverrideDialog {
        id: conditionOverrideDialog
        appBackend: root.backendRef
    }

    UnsavedTabSwitchDialog {
        id: unsavedTabSwitchDialog
        rootWindow: root
        appBackend: root.backendRef
    }

    // Page Components
    Component {
        id: overviewPageComponent
        OverviewPage {
            rootWindow: root
            appBackend: root.backendRef
            backupsModel: root.backupsModelRef
        }
    }

    Component {
        id: backupsPageComponent
        BackupsPage {
            rootWindow: root
            appBackend: root.backendRef
            backupsModel: root.backupsModelRef
        }
    }

    Component {
        id: backupDetailComponent
        BackupDetailPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: backupSourceEditComponent
        BackupSourceEditPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: backupExcludesEditComponent
        BackupExcludesEditPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: backupRetentionComponent
        BackupRetentionPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: backupScheduleEditComponent
        BackupScheduleEditPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: backupConditionsEditComponent
        BackupConditionsEditPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: backupPreviewComponent
        BackupPreviewPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: backupReassignComponent
        BackupReassignPage {
            rootWindow: root
            appBackend: root.backendRef
            destinationsModel: root.destinationsModelRef
        }
    }

    Component {
        id: backupSnapshotsComponent
        BackupSnapshotsPage {
            rootWindow: root
            appBackend: root.backendRef
            snapshotsModel: root.snapshotsModelRef
        }
    }

    Component {
        id: backupFullConfigComponent
        BackupFullConfigPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: destinationsPageComponent
        DestinationsPage {
            rootWindow: root
            appBackend: root.backendRef
            destinationsModel: root.destinationsModelRef
        }
    }

    Component {
        id: destinationDetailComponent
        DestinationDetailPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: destinationEditComponent
        DestinationEditPage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: destinationMaintenanceComponent
        DestinationMaintenancePage {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: historyPageComponent
        HistoryPage {
            rootWindow: root
            appBackend: root.backendRef
            backupsModel: root.backupsModelRef
            runsModel: root.runsModelRef
        }
    }

    Component {
        id: runDetailComponent
        RunDetailPage {
            rootWindow: root
            appBackend: root.backendRef
            runLogsModel: root.runLogsModelRef
        }
    }

    Component {
        id: addDestinationWizardComponent
        AddDestinationWizard {
            rootWindow: root
            appBackend: root.backendRef
        }
    }

    Component {
        id: newBackupWizardComponent
        NewBackupWizard {
            rootWindow: root
            appBackend: root.backendRef
            destinationsModel: root.destinationsModelRef
        }
    }

    Component {
        id: aboutPageComponent
        MainAboutPage {
            appBackend: root.backendRef
        }
    }
}
