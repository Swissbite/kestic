// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// Guards every navigation that would leave a dirty editor.
UnsavedChangesDialog {
    id: dialog
    objectName: "unsavedTabSwitchDialog"
    subtitle: i18n("You have unsaved changes. Save or discard them before leaving?")

    required property var rootWindow
    required property var appBackend

    onSaveRequested: {
        if (dialog.rootWindow.saveActiveDirtyPage()) {
            dialog.rootWindow.runPendingNavigation();
        } else {
            // Save failed (invalid input): stay on the page and resync the tab bar.
            dialog.rootWindow.cancelPendingNavigation();
        }
    }
    onDiscardRequested: {
        dialog.appBackend.has_unsaved_changes = false;
        dialog.rootWindow.runPendingNavigation();
    }
    onCancelRequested: dialog.rootWindow.cancelPendingNavigation()
}
