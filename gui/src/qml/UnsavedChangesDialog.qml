// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// «Save changes?» prompt (U18, AC-M3-14-9) with explicitly localized
// [Save] / [Discard] / [Cancel] buttons, independent of Qt's own catalogs.
Kirigami.PromptDialog {
    id: dialog
    title: Texts.label("save_changes_question")
    standardButtons: Kirigami.Dialog.NoButton

    signal saveRequested()
    signal discardRequested()
    signal cancelRequested()

    customFooterActions: [
        Kirigami.Action {
            objectName: "unsavedSaveAction"
            icon.name: "document-save"
            text: Texts.label("save")
            onTriggered: {
                dialog.close();
                dialog.saveRequested();
            }
        },
        Kirigami.Action {
            objectName: "unsavedDiscardAction"
            icon.name: "edit-delete"
            text: i18n("Discard")
            onTriggered: {
                dialog.close();
                dialog.discardRequested();
            }
        },
        Kirigami.Action {
            objectName: "unsavedCancelAction"
            icon.name: "dialog-cancel"
            text: Texts.label("cancel")
            onTriggered: {
                dialog.close();
                dialog.cancelRequested();
            }
        }
    ]

    onRejected: dialog.cancelRequested()
}
