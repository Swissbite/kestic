// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// Base of the editor pages with unsaved changes: reports `dirty` to the backend, asks
// «Save changes?» before leaving and closes the open dialogs one by one on Escape. A page
// binds `dirty`, sets the dialog texts, lists its own dialogs in `extraDialogs` and
// handles `unsavedSaveRequested` / `unsavedDiscardRequested`.
Kirigami.ScrollablePage {
    id: editor

    required property var rootWindow
    required property var appBackend

    property bool dirty: false
    // A save is on its way to the daemon: the page takes no input until it has answered, so the
    // answer cannot overwrite changes made meanwhile and a second save cannot start.
    property bool saving: false
    enabled: !saving
    property string unsavedObjectName: ""
    property string unsavedSubtitle: ""
    // Dialogs of the page that Escape closes before the «Save changes?» prompt.
    property var extraDialogs: []

    signal unsavedSaveRequested()
    signal unsavedDiscardRequested()

    onBackRequested: function(event) {
        if (dirty) {
            event.accepted = true;
            unsavedDialogItem.open();
        }
    }

    // Closes the page, asking «Save changes?» first when it has unsaved changes.
    function requestClose() {
        if (dirty) {
            unsavedDialogItem.open();
        } else {
            rootWindow.popSubPage();
        }
    }

    // Saves for the navigation guard; `done(ok)` runs with true once the page is clean. A page
    // that can be dirty overrides it.
    function saveForGuard(done) {
        done(false);
    }

    function handleEscape() {
        for (const dlg of extraDialogs.concat([unsavedDialogItem])) {
            if (rootWindow.closeIfOpen(dlg)) return true;
        }
        if (dirty) {
            unsavedDialogItem.open();
            return true;
        }
        return false;
    }

    UnsavedChangesDialog {
        id: unsavedDialogItem
        objectName: editor.unsavedObjectName
        subtitle: editor.unsavedSubtitle
        onSaveRequested: editor.unsavedSaveRequested()
        onDiscardRequested: editor.unsavedDiscardRequested()
    }
}
