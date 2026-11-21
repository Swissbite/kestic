// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import net.daester.kestic 1.0

QtObject {
    id: manager

    required property var rootWindow
    required property var pageStack

    property Item savedFocusDepth1: null
    property Item savedFocusDepth2: null
    property Item savedFocusDepth3: null
    property Item pendingRestoreFocusItem: null
    property Item nextTriggerItem: null
    property int breadcrumbRevision: 0

    property Timer focusRestoreTimer: Timer {
        interval: 35
        repeat: true
        property int remainingTicks: 0
        property Item targetItem: null
        property Item targetPage: null
        onTriggered: {
            manager.updatePageStackViewportAndFocusability();
            if (!targetItem && targetPage) {
                targetItem = manager.findFirstTabFocusable(targetPage);
            }
            if (!targetItem || !targetItem.visible || !targetItem.enabled) {
                remainingTicks -= 1;
                if (remainingTicks <= 0) stop();
                return;
            }
            var cur = manager.rootWindow.activeFocusItem;
            if (cur && cur !== targetItem && cur !== targetPage && manager.isInteractiveTabLeaf(cur)) {
                // Only override cur if cur is the default PageRow/Kirigami auto-forwarded first child of targetPage
                // when we are explicitly restoring a saved trigger item (such as editExcludesRow after pop).
                var curFirstInPage = targetPage ? manager.findFirstTabFocusable(targetPage) : null;
                if (cur !== curFirstInPage) {
                    stop();
                    return;
                }
            }
            if (targetPage) {
                for (var i = 0; i < manager.pageStack.depth; ++i) {
                    if (manager.pageStack.get(i) === targetPage) {
                        if (manager.pageStack.currentIndex !== i) {
                            manager.pageStack.currentIndex = i;
                        }
                        break;
                    }
                }
            }
            manager.updatePageStackViewportAndFocusability();
            targetItem.forceActiveFocus(Qt.TabFocusReason);
            remainingTicks -= 1;
            if (remainingTicks <= 0) {
                stop();
            }
        }
    }

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
        if (!pageStack || !rootWindow) return;
        var d = pageStack.depth;
        var minVisibleIdx = rootWindow.narrowMode ? Math.max(0, d - 1) : Math.max(0, d - 2);
        for (var i = 0; i < d; ++i) {
            var pg = pageStack.get(i);
            if (pg) {
                pg.enabled = (i >= minVisibleIdx);
            }
        }
        if (!rootWindow.narrowMode && pageStack.columnView && d > 0) {
            var firstVis = pageStack.get(minVisibleIdx);
            if (firstVis && pageStack.columnView.contentX !== firstVis.x) {
                pageStack.columnView.contentX = firstVis.x;
            }
        }
        breadcrumbRevision += 1;
    }

    function canNavigateBackInBreadcrumb(rev) {
        if (!pageStack || !rootWindow || rev < 0 || rootWindow.inWizardLayer) return false;
        if (rootWindow.narrowMode) return pageStack.depth > 1;
        if (rootWindow.activeTab === 1 || rootWindow.activeTab === 2 || rootWindow.activeTab === 3) {
            return pageStack.depth > 2;
        }
        return pageStack.depth > 1;
    }

    function defaultTitleForDepth(idx, rev) {
        if (rev >= 0 && idx === 0) {
            switch (rootWindow.activeTab) {
            case 0: return Texts.label("overview");
            case 1: return Texts.label("backups");
            case 2: return Texts.label("destinations");
            case 3: return Texts.label("history");
            default: return Texts.label("overview");
            }
        }
        return "";
    }

    function focusPrimaryListSelection() {
        if (pageStack.depth <= 0) return;
        var listPage = pageStack.get(0);
        if (!listPage) return;
        var list = listPage.primaryList;
        if (list) {
            for (var i = 0; i < list.count; ++i) {
                var item = list.itemAtIndex(i);
                if (item && item.highlighted) {
                    list.currentIndex = i;
                    focusItemInPageStack(item);
                    return;
                }
            }
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
        if (!rootWindow.narrowMode && targetIdx === 0 && (rootWindow.activeTab === 1 || rootWindow.activeTab === 2 || rootWindow.activeTab === 3)) {
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
        var leavingDepth = (typeof targetDepth === "number" && targetDepth >= 1)
            ? Math.min(targetDepth, pageStack.depth)
            : pageStack.depth;
        if (pageStack.depth > leavingDepth
                && rootWindow.guardDirtyNavigation(function() { pushSubPageAtDepth(pageObj, targetDepth); },
                                                    function() {
                                                        rootWindow.nextTriggerItem = null;
                                                        pageObj.destroy();
                                                    })) {
            return;
        }
        var trig = nextTriggerItem;
        nextTriggerItem = null;
        var focusCandidate = (trig && trig.visible && trig.enabled)
            ? trig
            : rootWindow.activeFocusItem;
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
}
