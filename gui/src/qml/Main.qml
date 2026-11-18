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
        var count = backupsModel.rowCount(backupsModel.index(-1, -1));
        for (var i = 0; i < count; ++i) {
            var idx = backupsModel.index(i, 0);
            if (backupsModel.data(idx, 0) === backupId) {
                return {
                    backupId: backupsModel.data(idx, 0),
                    name: backupsModel.data(idx, 1),
                    sourcePath: backupsModel.data(idx, 2),
                    sourceKind: backupsModel.data(idx, 3),
                    destinationId: backupsModel.data(idx, 4),
                    destinationName: backupsModel.data(idx, 5),
                    excludesText: backupsModel.data(idx, 6),
                    excludeCaches: backupsModel.data(idx, 7),
                    statusText: backupsModel.data(idx, 8),
                    statusIcon: backupsModel.data(idx, 9),
                    statusColorKind: backupsModel.data(idx, 10),
                    lastRunSummary: backupsModel.data(idx, 11),
                    availabilityReason: backupsModel.data(idx, 12),
                    available: backupsModel.data(idx, 13),
                    hasError: backupsModel.data(idx, 14),
                    isOverdue: backupsModel.data(idx, 15),
                    isRunning: backupsModel.data(idx, 16),
                    activeRunId: backupsModel.data(idx, 17),
                    activePercent: backupsModel.data(idx, 18),
                    activeProgressText: backupsModel.data(idx, 19),
                    errorSummary: backupsModel.data(idx, 20),
                    errorAction: backupsModel.data(idx, 21),
                    unreadableFilesJson: backupsModel.data(idx, 22),
                    scheduleSummary: backupsModel.data(idx, 23),
                    retentionSummary: backupsModel.data(idx, 24),
                    conditionsSummary: backupsModel.data(idx, 25),
                    lastRunId: backupsModel.data(idx, 26),
                    excludesJson: backupsModel.data(idx, 29),
                    excludeFoldersCount: backupsModel.data(idx, 30),
                    expertRuleCount: backupsModel.data(idx, 31),
                    excludeSummary: backupsModel.data(idx, 32),
                    excludeFilePath: backupsModel.data(idx, 33),
                    excludeLintJson: backupsModel.data(idx, 34),
                    retentionJson: backupsModel.data(idx, 35),
                    retentionPreset: backupsModel.data(idx, 36),
                    sourceKindLabel: backupsModel.data(idx, 37),
                    driveLabel: backupsModel.data(idx, 38),
                    pendingForget: backupsModel.data(idx, 39),
                    nestedWarningsJson: backupsModel.data(idx, 40),
                    hasAttentionWarning: backupsModel.data(idx, 41)
                };
            }
        }
        return null;
    }

    function findDestinationData(destId) {
        var count = destinationsModel.rowCount(destinationsModel.index(-1, -1));
        for (var i = 0; i < count; ++i) {
            var idx = destinationsModel.index(i, 0);
            if (destinationsModel.data(idx, 0) === destId) {
                return {
                    destinationId: destinationsModel.data(idx, 0),
                    name: destinationsModel.data(idx, 1),
                    kindName: destinationsModel.data(idx, 2),
                    kindLabel: destinationsModel.data(idx, 3),
                    location: destinationsModel.data(idx, 4),
                    username: destinationsModel.data(idx, 5),
                    backupCount: destinationsModel.data(idx, 6),
                    statusText: destinationsModel.data(idx, 7),
                    statusIcon: destinationsModel.data(idx, 8),
                    reachable: destinationsModel.data(idx, 9),
                    passwordRejected: destinationsModel.data(idx, 10),
                    hasStaleLock: destinationsModel.data(idx, 11),
                    hasProblem: destinationsModel.data(idx, 12),
                    detailText: destinationsModel.data(idx, 13),
                    pendingForgetCount: destinationsModel.data(idx, 14)
                };
            }
        }
        return null;
    }

    property Item savedFocusDepth1: null
    property Item savedFocusDepth2: null
    property Item savedFocusDepth3: null
    property Item pendingRestoreFocusItem: null
    property Item nextTriggerItem: null
    property int breadcrumbRevision: 0

    onNarrowModeChanged: updatePageStackViewportAndFocusability()

    function clearSavedFocus() {
        savedFocusDepth1 = null;
        savedFocusDepth2 = null;
        savedFocusDepth3 = null;
        pendingRestoreFocusItem = null;
        nextTriggerItem = null;
    }

    function saveFocusForDepth(depth, item) {
        if (depth === 1) savedFocusDepth1 = item;
        else if (depth === 2) savedFocusDepth2 = item;
        else savedFocusDepth3 = item;
    }

    function saveFocusForCurrentDepth(item) {
        saveFocusForDepth(pageStack.depth, item);
    }

    function takeSavedFocusForDepth(depth) {
        var it = null;
        if (depth === 1) { it = savedFocusDepth1; savedFocusDepth1 = null; }
        else if (depth === 2) { it = savedFocusDepth2; savedFocusDepth2 = null; }
        else { it = savedFocusDepth3; savedFocusDepth3 = null; }
        return it;
    }

    function isItemInsidePage(item, pageObjectName) {
        for (var p = item; p; p = p.parent) {
            if (p.objectName === pageObjectName) {
                return true;
            }
        }
        return false;
    }

    function updatePageStackViewportAndFocusability() {
        var d = pageStack.depth;
        var minVisibleIdx = root.narrowMode ? Math.max(0, d - 1) : Math.max(0, d - 2);
        for (var i = 0; i < d; ++i) {
            var pg = pageStack.get(i);
            if (pg) {
                pg.enabled = (i >= minVisibleIdx);
            }
        }
        if (!root.narrowMode && pageStack.columnView && d > 0) {
            var firstVis = pageStack.get(minVisibleIdx);
            if (firstVis && pageStack.columnView.contentX !== firstVis.x) {
                pageStack.columnView.contentX = firstVis.x;
            }
        }
        breadcrumbRevision += 1;
    }

    function canNavigateBackInBreadcrumb(rev) {
        if (rev < 0 || root.inWizardLayer) return false;
        if (root.narrowMode) return pageStack.depth > 1;
        if (root.activeTab === 1 || root.activeTab === 2 || root.activeTab === 3) {
            return pageStack.depth > 2;
        }
        return pageStack.depth > 1;
    }

    function defaultTitleForDepth(idx, rev) {
        if (rev >= 0 && idx === 0) {
            switch (root.activeTab) {
            case 0: return i18n("Overview");
            case 1: return i18n("Backups");
            case 2: return i18n("Destinations");
            case 3: return i18n("History");
            default: return i18n("Overview");
            }
        }
        return "";
    }

    function focusPrimaryListSelection() {
        if (pageStack.depth <= 0) return;
        var listPage = pageStack.get(0);
        if (!listPage) return;
        if (typeof listPage.focusSelectedRow === "function") {
            if (listPage.focusSelectedRow()) return;
        }
        focusPageContent(listPage);
    }

    function clearObjectNamesRecursive(rootObj) {
        if (!rootObj) return;
        var visited = new Set();
        var stack = [rootObj];
        while (stack.length > 0) {
            var obj = stack.pop();
            if (!obj || visited.has(obj)) continue;
            visited.add(obj);
            if (typeof obj.objectName === "string" && obj.objectName !== "") {
                obj.objectName = "";
            }
            if (obj.contentItem && obj.contentItem !== obj && !visited.has(obj.contentItem)) {
                stack.push(obj.contentItem);
            }
            if (obj.actions) {
                for (var a = 0; a < obj.actions.length; ++a) {
                    var act = obj.actions[a];
                    if (act && !visited.has(act)) stack.push(act);
                }
            }
            if (obj.children) {
                for (var i = 0; i < obj.children.length; ++i) {
                    var ch = obj.children[i];
                    if (ch && !visited.has(ch)) stack.push(ch);
                }
            }
            if (obj.resources) {
                for (var j = 0; j < obj.resources.length; ++j) {
                    var res = obj.resources[j];
                    if (res && !visited.has(res)) stack.push(res);
                }
            }
            if (obj.data) {
                for (var k = 0; k < obj.data.length; ++k) {
                    var dt = obj.data[k];
                    if (dt && !visited.has(dt)) stack.push(dt);
                }
            }
        }
    }

    function removeTopPageFromStack() {
        if (pageStack.depth <= 0) return;
        var removed = pageStack.removePage(pageStack.depth - 1);
        if (removed) {
            clearObjectNamesRecursive(removed);
            removed.visible = false;
            removed.enabled = false;
        }
    }

    function navigateToBreadcrumbIndex(targetIdx) {
        if (targetIdx < 0 || targetIdx >= pageStack.depth) return;
        if (targetIdx === pageStack.depth - 1) {
            focusPageContent(pageStack.get(targetIdx));
            return;
        }
        var cur = pageStack.depth > 0 ? pageStack.get(pageStack.depth - 1) : null;
        if (cur && typeof cur.backRequested === "function") {
            var backEvent = { accepted: false };
            cur.backRequested(backEvent);
            if (backEvent.accepted) {
                return;
            }
        }
        if (!root.narrowMode && targetIdx === 0 && (root.activeTab === 1 || root.activeTab === 2 || root.activeTab === 3)) {
            while (pageStack.depth > 2) {
                removeTopPageFromStack();
            }
            updatePageStackViewportAndFocusability();
            Qt.callLater(function() {
                updatePageStackViewportAndFocusability();
                focusPrimaryListSelection();
            });
            return;
        }
        var desiredDepth = targetIdx + 1;
        var restoreItem = takeSavedFocusForDepth(desiredDepth);
        pendingRestoreFocusItem = restoreItem;
        while (pageStack.depth > desiredDepth) {
            removeTopPageFromStack();
        }
        if (pageStack.depth > 0) {
            pageStack.currentIndex = pageStack.depth - 1;
        }
        updatePageStackViewportAndFocusability();
        Qt.callLater(function() {
            updatePageStackViewportAndFocusability();
            if (restoreItem && restoreItem.visible && restoreItem.enabled) {
                focusItemInPageStack(restoreItem);
            } else if (pageStack.depth > 0) {
                focusPageContent(pageStack.get(pageStack.depth - 1));
            }
        });
    }

    function isInteractiveTabLeaf(item) {
        if (!item || !item.visible || !item.enabled || !item.activeFocusOnTab) return false;
        // Kirigami.ScrollablePage's internal ScrollView has a flickable child and is a container, not an interactive leaf control.
        if (typeof item.flickable !== "undefined" && item.flickable !== null && typeof item.text === "undefined") {
            return false;
        }
        return true;
    }

    function cancelFocusRestoreTimer() {
        focusRestoreTimer.stop();
        focusRestoreTimer.targetItem = null;
        focusRestoreTimer.targetPage = null;
        pendingRestoreFocusItem = null;
    }

    Timer {
        id: focusRestoreTimer
        interval: 35
        repeat: true
        property int remainingTicks: 0
        property Item targetItem: null
        property Item targetPage: null
        onTriggered: {
            root.updatePageStackViewportAndFocusability();
            if (!targetItem && targetPage) {
                targetItem = findFirstTabFocusable(targetPage);
            }
            if (!targetItem || !targetItem.visible || !targetItem.enabled) {
                remainingTicks -= 1;
                if (remainingTicks <= 0) stop();
                return;
            }
            var cur = root.activeFocusItem;
            if (cur && cur !== targetItem && cur !== targetPage && isInteractiveTabLeaf(cur)) {
                // Only override cur if cur is the default PageRow/Kirigami auto-forwarded first child of targetPage
                // when we are explicitly restoring a saved trigger item (such as editExcludesRow after pop).
                var curFirstInPage = targetPage ? findFirstTabFocusable(targetPage) : null;
                if (cur !== curFirstInPage) {
                    stop();
                    return;
                }
            }
            if (targetPage) {
                for (var i = 0; i < pageStack.depth; ++i) {
                    if (pageStack.get(i) === targetPage) {
                        if (pageStack.currentIndex !== i) {
                            pageStack.currentIndex = i;
                        }
                        break;
                    }
                }
            }
            root.updatePageStackViewportAndFocusability();
            targetItem.forceActiveFocus(Qt.TabFocusReason);
            remainingTicks -= 1;
            if (remainingTicks <= 0) {
                stop();
            }
        }
    }

    function focusItemInPageStack(targetItem) {
        if (!targetItem) return false;
        updatePageStackViewportAndFocusability();
        if (!targetItem.enabled) return false;
        var foundPage = null;
        for (var p = targetItem; p; p = p.parent) {
            for (var i = 0; i < pageStack.depth; ++i) {
                if (pageStack.get(i) === p) {
                    foundPage = p;
                    if (pageStack.currentIndex !== i) {
                        pageStack.currentIndex = i;
                    }
                    break;
                }
            }
            if (foundPage) break;
        }
        updatePageStackViewportAndFocusability();
        targetItem.forceActiveFocus(Qt.TabFocusReason);
        focusRestoreTimer.targetItem = targetItem;
        focusRestoreTimer.targetPage = foundPage;
        focusRestoreTimer.remainingTicks = 4;
        focusRestoreTimer.restart();
        return true;
    }

    function findFirstTabFocusable(item) {
        if (!item || !item.visible || !item.enabled) return null;
        if (isInteractiveTabLeaf(item)) {
            return item;
        }
        var children = [];
        if (item.flickable && item.flickable.contentItem) {
            children.push(item.flickable.contentItem);
        }
        if (item.contentItem) {
            children.push(item.contentItem);
        }
        if (item.children) {
            for (var i = 0; i < item.children.length; ++i) {
                if (item.header && item.children[i] === item.header) continue;
                children.push(item.children[i]);
            }
        }
        for (var j = 0; j < children.length; ++j) {
            var found = findFirstTabFocusable(children[j]);
            if (found) return found;
        }
        if (item.header) {
            var headerFound = findFirstTabFocusable(item.header);
            if (headerFound) return headerFound;
        }
        return null;
    }

    function focusPageContent(pageItem) {
        if (!pageItem) return;
        updatePageStackViewportAndFocusability();
        for (var i = 0; i < pageStack.depth; ++i) {
            if (pageStack.get(i) === pageItem) {
                if (pageStack.currentIndex !== i) {
                    pageStack.currentIndex = i;
                }
                break;
            }
        }
        updatePageStackViewportAndFocusability();
        focusRestoreTimer.targetPage = pageItem;
        focusRestoreTimer.targetItem = null;
        focusRestoreTimer.remainingTicks = 4;
        focusRestoreTimer.restart();
        Qt.callLater(function() {
            if (!pageItem) return;
            updatePageStackViewportAndFocusability();
            var first = findFirstTabFocusable(pageItem);
            if (first) {
                focusItemInPageStack(first);
            } else {
                focusItemInPageStack(pageItem);
            }
        });
    }

    function pushSubPageAtDepth(pageObj, targetDepth) {
        var trig = nextTriggerItem;
        nextTriggerItem = null;
        var focusCandidate = (trig && trig.visible && trig.enabled)
            ? trig
            : root.activeFocusItem;
        var desiredParentDepth = (typeof targetDepth === "number" && targetDepth >= 1)
            ? Math.min(targetDepth, pageStack.depth)
            : pageStack.depth;
        var parentPage = desiredParentDepth > 0 ? pageStack.get(desiredParentDepth - 1) : null;
        if (focusCandidate && (!parentPage || isItemInsidePage(focusCandidate, parentPage.objectName))) {
            saveFocusForDepth(desiredParentDepth, focusCandidate);
        }
        while (pageStack.depth > desiredParentDepth) {
            removeTopPageFromStack();
        }
        pendingRestoreFocusItem = null;
        if (pageStack.depth > 0) {
            pageStack.currentIndex = pageStack.depth - 1;
        }
        pageStack.push(pageObj);
        pageStack.currentIndex = pageStack.depth - 1;
        updatePageStackViewportAndFocusability();
        focusPageContent(pageObj);
    }

    function pushSubPage(pageObj) {
        pushSubPageAtDepth(pageObj, pageStack.depth);
    }

    function popSubPage() {
        if (pageStack.depth <= 1) return;
        var targetDepth = pageStack.depth - 1;
        var restoreItem = takeSavedFocusForDepth(targetDepth);
        pendingRestoreFocusItem = restoreItem;
        removeTopPageFromStack();
        if (pageStack.depth > 0) {
            pageStack.currentIndex = pageStack.depth - 1;
        }
        updatePageStackViewportAndFocusability();
        Qt.callLater(function() {
            updatePageStackViewportAndFocusability();
            if (restoreItem && restoreItem.visible && restoreItem.enabled) {
                focusItemInPageStack(restoreItem);
            } else if (pageStack.depth > 0) {
                focusPageContent(pageStack.get(pageStack.depth - 1));
            }
        });
    }

    function replaceRootPage(pageObj) {
        if (pageStack.depth > 1) {
            pageStack.currentIndex = 0;
            while (pageStack.depth > 1) {
                removeTopPageFromStack();
            }
        }
        if (pageStack.depth === 1) {
            pageStack.replace(pageObj);
        } else {
            pageStack.push(pageObj);
        }
        updatePageStackViewportAndFocusability();
    }

    function switchTab(idx) {
        while (wizardPageStack.depth > 0) {
            closeWizardLayer();
        }
        clearSavedFocus();
        activeTab = idx;
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

    function openBackupSourcePage(backupId) {
        pushSubPageAtDepth(backupSourceEditComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupExcludesPage(backupId) {
        pushSubPageAtDepth(backupExcludesEditComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
    }

    function openBackupRetentionPage(backupId) {
        pushSubPageAtDepth(backupRetentionComponent.createObject(pageStack, { backupId: backupId }), targetDepthForBackupEditor());
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
        var titleText = firstSetup ? i18n("Set Up First Backup") : i18n("New Backup");
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
        updatePasswordField.text = "";
        updatePasswordDialog.open();
    }

    function openRenameBackupDialog(backupId, currentName) {
        renameBackupId = backupId;
        renameBackupCurrentName = currentName;
        renameBackupField.text = currentName;
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
    }

    BackupsModel { id: backupsModel; objectName: "backupsModel" }
    DestinationsModel { id: destinationsModel; objectName: "destinationsModel" }
    RunsModel { id: runsModel; objectName: "runsModel" }
    SnapshotsModel { id: snapshotsModel; objectName: "snapshotsModel" }
    RunLogsModel { id: runLogsModel; objectName: "runLogsModel" }

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
            text: i18n("Overview")
            checked: root.activeTab === 0
            onTriggered: root.switchTab(0)
        },
        Kirigami.Action {
            objectName: "backupsTabAction"
            icon.name: "folder-sync"
            text: i18n("Backups")
            checked: root.activeTab === 1
            onTriggered: root.switchTab(1)
        },
        Kirigami.Action {
            objectName: "destinationsTabAction"
            icon.name: "network-server"
            text: i18n("Destinations")
            checked: root.activeTab === 2
            onTriggered: root.switchTab(2)
        },
        Kirigami.Action {
            objectName: "historyTabAction"
            icon.name: "view-history"
            text: i18n("History")
            checked: root.activeTab === 3
            onTriggered: root.switchTab(3)
        }
    ]

    header: ColumnLayout {
        width: parent.width
        spacing: 0

        // Priority order (§6.1): Daemon unreachable -> Secret store locked -> restic unavailable
        Kirigami.InlineMessage {
            id: blockingStatusBanner
            objectName: "blockingStatusBanner"
            Layout.fillWidth: true
            position: Kirigami.InlineMessage.Position.Header
            type: appBackend.blocking_kind === "secret_store_locked"
                ? Kirigami.MessageType.Warning
                : Kirigami.MessageType.Error
            visible: appBackend.blocking_kind !== ""
            text: appBackend.blocking_message

            actions: [
                Kirigami.Action {
                    id: retryDaemonAction
                    objectName: "retryDaemonAction"
                    visible: appBackend.blocking_kind === "daemon_unreachable"
                    icon.name: "view-refresh"
                    text: i18n("Retry")
                    onTriggered: appBackend.retryStartDaemon()
                },
                Kirigami.Action {
                    id: unlockSecretStoreAction
                    objectName: "unlockSecretStoreAction"
                    visible: appBackend.blocking_kind === "secret_store_locked"
                    icon.name: "dialog-password"
                    text: i18n("Unlock…")
                    onTriggered: appBackend.unlockSecretStore()
                },
                Kirigami.Action {
                    visible: appBackend.blocking_kind === "secret_store_locked"
                    icon.name: "view-refresh"
                    text: i18n("Retry")
                    onTriggered: appBackend.checkAvailability(true)
                },
                Kirigami.Action {
                    id: downloadResticDisabledAction
                    objectName: "downloadResticDisabledAction"
                    visible: appBackend.blocking_kind === "restic_unavailable"
                    text: i18n("Download restic…")
                    enabled: false
                    tooltip: i18n("Automatic restic download will be added in a future release.")
                },
                Kirigami.Action {
                    id: openResticSettingsAction
                    objectName: "openResticSettingsAction"
                    visible: appBackend.blocking_kind === "restic_unavailable"
                    icon.name: "configure"
                    text: i18n("Settings")
                    onTriggered: root.openSettingsWindow()
                }
            ]
        }

        Kirigami.InlineMessage {
            id: manualRunBlockedBanner
            objectName: "manualRunBlockedBanner"
            Layout.fillWidth: true
            position: Kirigami.InlineMessage.Position.Header
            type: Kirigami.MessageType.Warning
            visible: appBackend.manual_run_blocked_message !== ""
            showCloseButton: true
            text: appBackend.manual_run_blocked_message
            onVisibleChanged: if (!visible && appBackend.manual_run_blocked_message !== "") appBackend.dismissManualRunBlocked()
        }

        Kirigami.NavigationTabBar {
            id: topNavBar
            objectName: "topNavBar"
            Layout.fillWidth: true
            visible: !root.narrowMode && !root.inWizardLayer
            height: visible ? implicitHeight : 0
            maximumContentWidth: visibleActions.length * Kirigami.Units.gridUnit * 9
            position: Controls.ToolBar.Header
            actions: root.navActions
            delegate: Kirigami.NavigationTabButton {
                required property Controls.Action modelData
                parent: topNavBar.contentItem
                action: modelData
                onActionChanged: action.checkable = true
                Layout.minimumWidth: topNavBar.buttonWidth
                Layout.maximumWidth: topNavBar.buttonWidth
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

        Controls.ToolBar {
            id: breadcrumbToolBar
            objectName: "breadcrumbToolBar"
            Layout.fillWidth: true
            visible: !root.inWizardLayer && root.breadcrumbRevision >= 0 && pageStack.depth > 1
            height: visible ? implicitHeight : 0
            position: Controls.ToolBar.Header

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: Kirigami.Units.smallSpacing
                anchors.rightMargin: Kirigami.Units.smallSpacing
                spacing: Kirigami.Units.smallSpacing

                Controls.ToolButton {
                    id: breadcrumbBackButton
                    objectName: "breadcrumbBackButton"
                    visible: root.canNavigateBackInBreadcrumb(root.breadcrumbRevision)
                    enabled: visible
                    activeFocusOnTab: visible
                    icon.name: "go-previous"
                    text: i18n("Back")
                    display: Controls.AbstractButton.TextBesideIcon
                    Accessible.name: i18n("Back")
                    onClicked: root.handleGlobalEscape()

                    Rectangle {
                        anchors.fill: parent
                        color: "transparent"
                        radius: Kirigami.Units.cornerRadius
                        border.width: (breadcrumbBackButton.activeFocus || breadcrumbBackButton.visualFocus) ? 2 : 0
                        border.color: Kirigami.Theme.highlightColor
                        visible: border.width > 0
                    }
                }

                Controls.ToolSeparator {
                    visible: breadcrumbBackButton.visible
                }

                Repeater {
                    id: breadcrumbRepeater
                    model: root.breadcrumbRevision >= 0 ? pageStack.depth : 0

                    delegate: RowLayout {
                        id: crumbDelegate
                        required property int index
                        spacing: Kirigami.Units.smallSpacing

                        readonly property Item pageItem: (root.breadcrumbRevision >= 0 && index >= 0 && index < pageStack.depth)
                            ? pageStack.get(index)
                            : null
                        readonly property string crumbTitle: (pageItem && pageItem.title !== "")
                            ? pageItem.title
                            : root.defaultTitleForDepth(index, root.breadcrumbRevision)
                        readonly property bool isCurrentCrumb: root.breadcrumbRevision >= 0 && index === pageStack.depth - 1

                        Kirigami.Icon {
                            visible: crumbDelegate.index > 0
                            source: "go-next-symbolic"
                            Layout.preferredWidth: Kirigami.Units.iconSizes.small
                            Layout.preferredHeight: Kirigami.Units.iconSizes.small
                            opacity: 0.65
                        }

                        Controls.ToolButton {
                            id: crumbButton
                            objectName: "breadcrumbButton_" + crumbDelegate.index
                            visible: !crumbDelegate.isCurrentCrumb
                            enabled: visible
                            activeFocusOnTab: visible
                            flat: true
                            text: crumbDelegate.crumbTitle
                            Accessible.name: crumbDelegate.crumbTitle
                            onClicked: root.navigateToBreadcrumbIndex(crumbDelegate.index)

                            Rectangle {
                                anchors.fill: parent
                                color: "transparent"
                                radius: Kirigami.Units.cornerRadius
                                border.width: (crumbButton.activeFocus || crumbButton.visualFocus) ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor
                                visible: border.width > 0
                            }
                        }

                        Controls.Label {
                            objectName: "breadcrumbCurrentLabel"
                            visible: crumbDelegate.isCurrentCrumb
                            text: crumbDelegate.crumbTitle
                            font.bold: true
                            leftPadding: Kirigami.Units.smallSpacing
                            rightPadding: Kirigami.Units.smallSpacing
                        }
                    }
                }

                Item {
                    Layout.fillWidth: true
                }
            }
        }
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
    Kirigami.Dialog {
        id: showCommandDialog
        objectName: "showCommandDialog"
        title: root.cmdDialogTitle !== "" ? i18n("Command · %1", root.cmdDialogTitle) : i18n("Command")
        standardButtons: Kirigami.Dialog.Close
        padding: Kirigami.Units.largeSpacing

        customFooterActions: [
            Kirigami.Action {
                icon.name: "edit-copy"
                text: i18n("Copy")
                onTriggered: {
                    appBackend.copyToClipboard(root.cmdDialogText);
                    root.showPassiveNotification(i18n("Command copied to clipboard"));
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 28
            spacing: Kirigami.Units.smallSpacing

            Controls.Label {
                text: i18n("Transparent restic command without plaintext secrets:")
                opacity: 0.8
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Controls.TextArea {
                id: commandTextArea
                objectName: "showCommandTextArea"
                readOnly: true
                font: Kirigami.Theme.fixedWidthFont
                text: root.cmdDialogText
                wrapMode: TextEdit.Wrap
                Layout.fillWidth: true

                Keys.onTabPressed: function(event) {
                    var next = nextItemInFocusChain(true);
                    if (next) next.forceActiveFocus(Qt.TabFocusReason);
                    event.accepted = true;
                }
                Keys.onBacktabPressed: function(event) {
                    var prev = nextItemInFocusChain(false);
                    if (prev) prev.forceActiveFocus(Qt.BacktabFocusReason);
                    event.accepted = true;
                }

                Rectangle {
                    anchors.fill: parent
                    color: "transparent"
                    radius: Kirigami.Units.cornerRadius
                    border.width: commandTextArea.activeFocus ? 2 : 0
                    border.color: Kirigami.Theme.highlightColor
                    visible: border.width > 0
                }
            }
        }
    }

    Kirigami.Dialog {
        id: updatePasswordDialog
        objectName: "updatePasswordDialog"
        title: i18n("Update Saved Password")
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing

        customFooterActions: [
            Kirigami.Action {
                icon.name: "dialog-ok-apply"
                text: i18n("Verify and Save")
                enabled: updatePasswordField.text.length > 0
                onTriggered: {
                    if (appBackend.updateDestinationPassword(root.pwdDialogDestId, updatePasswordField.text)) {
                        updatePasswordDialog.close();
                    }
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 24
            spacing: Kirigami.Units.smallSpacing

            Controls.Label {
                text: i18n("The saved password in your password store for '%1' does not match. The destination's own password is not changed.", root.pwdDialogDestName)
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Controls.TextField {
                id: updatePasswordField
                echoMode: TextInput.Password
                placeholderText: i18n("Repository password")
                Accessible.name: i18n("Repository password")
                Layout.fillWidth: true
                onAccepted: {
                    if (text.length > 0 && appBackend.updateDestinationPassword(root.pwdDialogDestId, text)) {
                        updatePasswordDialog.close();
                    }
                }
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

    Kirigami.Dialog {
        id: renameBackupDialog
        objectName: "renameBackupDialog"
        title: i18n("Rename Backup")
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing

        customFooterActions: [
            Kirigami.Action {
                text: i18n("Rename")
                icon.name: "edit-rename"
                enabled: renameBackupField.text.trim().length > 0 && renameBackupField.text.trim() !== root.renameBackupCurrentName
                onTriggered: {
                    if (appBackend.renameBackup(root.renameBackupId, renameBackupField.text.trim())) {
                        renameBackupDialog.close();
                        root.showPassiveNotification(i18n("Backup renamed"));
                    }
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 22
            spacing: Kirigami.Units.smallSpacing

            Controls.TextField {
                id: renameBackupField
                Accessible.name: i18n("New backup name")
                Layout.fillWidth: true
                onAccepted: {
                    if (text.trim().length > 0 && text.trim() !== root.renameBackupCurrentName
                            && appBackend.renameBackup(root.renameBackupId, text.trim())) {
                        renameBackupDialog.close();
                        root.showPassiveNotification(i18n("Backup renamed"));
                    }
                }
            }
        }
    }

    Kirigami.Dialog {
        id: removeBackupDialog
        objectName: "removeBackupDialog"
        title: i18n("Remove '%1' from Kestic?", root.removeBackupName)
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing
        customFooterActions: [
            Kirigami.Action {
                id: confirmRemoveBackupAction
                objectName: "confirmRemoveBackupAction"
                icon.name: "list-remove"
                text: i18n("Remove")
                onTriggered: {
                    if (appBackend.removeBackup(root.removeBackupId)) {
                        removeBackupDialog.close();
                        root.showPassiveNotification(i18n("Backup removed from Kestic"));
                        root.switchTab(1);
                    }
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 22
            Controls.Label {
                text: i18n("Snapshots will remain in the destination.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    Kirigami.Dialog {
        id: removeDestinationDialog
        objectName: "removeDestinationDialog"
        title: i18n("Remove '%1' from Kestic?", root.removeDestName)
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing
        customFooterActions: [
            Kirigami.Action {
                icon.name: "list-remove"
                text: i18n("Remove Destination")
                onTriggered: {
                    if (appBackend.removeDestination(root.removeDestId)) {
                        removeDestinationDialog.close();
                        root.showPassiveNotification(i18n("Destination removed"));
                        root.switchTab(2);
                    }
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 22
            Controls.Label {
                text: i18n("Snapshots on this destination remain untouched.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    Kirigami.Dialog {
        id: quitConfirmDialog
        objectName: "quitConfirmDialog"
        title: i18n("Quit Kestic?")
        y: parent ? Math.round((parent.height - height) / 2) : 0
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing
        customFooterActions: [
            Kirigami.Action {
                id: confirmQuitAction
                objectName: "confirmQuitAction"
                icon.name: "application-exit"
                text: i18n("Quit Kestic")
                onTriggered: {
                    appBackend.confirmQuit();
                    quitConfirmDialog.close();
                    Qt.quit();
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 24
            Controls.Label {
                text: appBackend.quit_has_running_run
                    ? i18n("Scheduled backups will not run again until Kestic is started. The running backup '%1' will be cancelled.", appBackend.quit_running_backup_name)
                    : i18n("Scheduled backups will not run again until Kestic is started.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
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
        Kirigami.AboutPage {
            aboutData: {
                "displayName": "Kestic",
                "productName": "kestic",
                "componentName": "kestic",
                "shortDescription": i18n("Encrypted backup manager for Plasma powered by restic (%1)", root.backendRef.restic_version !== "" ? root.backendRef.restic_version : i18n("not available")),
                "version": root.backendRef.daemon_version !== "" ? root.backendRef.daemon_version : "0.1.0",
                "homepage": "https://daester.net",
                "bugAddress": "david@daester.net",
                "authors": [{ "name": "David Däster", "emailAddress": "david@daester.net" }],
                "licenses": [{ "name": "GPL-3.0-or-later", "spdx": "GPL-3.0-or-later" }]
            }
        }
    }
}
