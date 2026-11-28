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
    // The revision of the backup or destination from the page's opening (not bound to the
    // state: a change made elsewhere must not slip into it); the save sends it.
    property string revision: ""
    // «backup» or «destination»: what the message after a refused save names.
    property string revisionOf: "backup"
    // The last save was refused because the backup or destination changed meanwhile.
    property bool changedElsewhere: false

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

    // Takes the answer of a save: the new revision, or after a `Conflict` the current state
    // through `reload` (which also takes the current revision) with a message.
    function afterSave(ok, answer, reload) {
        if (ok) {
            revision = answer;
            changedElsewhere = false;
        } else if (answer === "conflict") {
            reload();
            changedElsewhere = true;
        }
    }

    header: Kirigami.InlineMessage {
        objectName: "editorChangedElsewhereMessage"
        type: Kirigami.MessageType.Warning
        position: Kirigami.InlineMessage.Position.Header
        visible: editor.changedElsewhere
        text: editor.revisionOf === "destination"
            ? i18n("This destination was changed elsewhere in the meantime. The current version is loaded; your change was not saved.")
            : i18n("This backup was changed elsewhere in the meantime. The current version is loaded; your change was not saved.")
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
