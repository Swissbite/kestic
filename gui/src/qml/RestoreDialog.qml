// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Dialog «Restore n items»: where to (a new folder by default, the original location or a
// chosen folder) and, at the original location or in a folder with content, what happens to
// existing files. The preview runs on its own; «Restore Now» stays disabled until it is done.
// Overwriting names the consequence and turns the button into «Overwrite and Restore».
Kirigami.Dialog {
    id: dialog
    objectName: "restoreDialog"
    title: dialog.model ? dialog.model.title : ""
    // «Cancel» is a footer action like «Restore Now», before it, so both stand in the order they
    // show, also for Tab. Cancel and Esc close the dialog, so it has no close button of its own.
    standardButtons: Kirigami.Dialog.NoButton
    showCloseButton: false
    padding: Kirigami.Units.largeSpacing
    // Centred by its content height: positioning the popup may adjust `height`, which the
    // default binding of `y` reads.
    y: parent ? Math.max(0, Math.round((parent.height - implicitHeight) / 2)) : 0

    required property var model
    property bool listShown: false
    // The folder chooser returned a network folder (smb://, sftp://…), which is no target.
    property bool networkFolderRefused: false

    // Takes the folder the chooser returned. A local folder (also one mounted from the network
    // through fstab or systemd) becomes the target; restic writes only to local paths, so for a
    // network URL the dialog says so and keeps its target, without asking the daemon.
    function chooseFolderUrl(url) {
        var text = url.toString();
        if (!text.startsWith("file://")) {
            networkFolderRefused = true;
            return;
        }
        networkFolderRefused = false;
        var local = decodeURIComponent(text.replace(/^file:\/\//, ""));
        if (local.length > 0) dialog.model.chooseFolder(local);
    }

    // The list that gets the focus back when the dialog closes: the dialog may open from a
    // context menu, whose item is gone by then.
    property var returnFocusList: null

    // Opens the dialog for `selection` (absolute snapshot paths, empty = the whole snapshot);
    // `focusList` (optional) gets the focus back on its current row afterwards.
    function openFor(destinationId, snapshotId, selection, focusList) {
        returnFocusList = focusList || null;
        kindChips.uncheckAll();
        listShown = false;
        networkFolderRefused = false;
        previewFilter.resetFilter();
        model.open(destinationId, snapshotId, selection);
        open();
    }

    onOpened: (originalRadio.checked ? originalRadio : newFolderRadio).forceActiveFocus(Qt.TabFocusReason)
    // However the dialog closes (Cancel, Escape, a click outside), a running preview stops.
    onClosed: {
        model.cancel();
        var list = returnFocusList;
        returnFocusList = null;
        if (list) {
            if (list.currentItem) list.currentItem.forceActiveFocus(Qt.PopupFocusReason);
            else list.forceActiveFocus(Qt.PopupFocusReason);
        }
    }

    // When files would be overwritten, «Cancel» is the default: it holds the focus and the
    // default frame, so Return cancels. The focus stays on the list of modes while the user
    // steps through it.
    readonly property bool overwrites: dialog.model !== null && dialog.model.overwrites
    onOverwritesChanged: {
        var cancel = dialog.customFooterButton(cancelAction);
        if (cancel) {
            cancel.Accessible.defaultButton = overwrites;
            if (overwrites && !overwriteBox.activeFocus) cancel.forceActiveFocus(Qt.TabFocusReason);
        }
    }

    customFooterActions: [
        Kirigami.Action {
            id: cancelAction
            objectName: "restoreCancelAction"
            icon.name: "dialog-cancel"
            text: i18n("Cancel")
            onTriggered: dialog.reject()
        },
        Kirigami.Action {
            id: restoreNowAction
            objectName: "restoreNowAction"
            icon.name: dialog.overwrites ? "document-replace" : "document-revert"
            text: dialog.model && dialog.model.overwrites
                ? i18n("Overwrite and Restore")
                : i18n("Restore Now")
            enabled: dialog.model !== null && dialog.model.state === "done" && dialog.model.can_restore
            onTriggered: {
                dialog.model.startRestore();
                dialog.close();
            }
        }
    ]

    Timer {
        interval: 150
        repeat: true
        running: dialog.visible && dialog.model !== null && dialog.model.state === "running"
        onTriggered: dialog.model.poll()
    }

    FolderDialog {
        id: folderDialog
        title: i18n("Choose Target Folder")
        onAccepted: dialog.chooseFolderUrl(selectedFolder)
    }

    ColumnLayout {
        spacing: Kirigami.Units.mediumSpacing

        // The width of the dialog: a layout computes its own implicit width from its children,
        // so the heading carries it, also when only a short message follows.
        Kirigami.Heading {
            Layout.preferredWidth: Math.min(Kirigami.Units.gridUnit * 30,
                dialog.absoluteMaximumWidth - dialog.leftPadding - dialog.rightPadding - Kirigami.Units.gridUnit)
            level: 4
            text: i18n("Where to")
        }

        Controls.RadioButton {
            id: newFolderRadio
            objectName: "restoreNewFolderRadio"
            Layout.fillWidth: true
            text: i18n("Into a new folder")
            onClicked: dialog.model.chooseFolder("")
        }
        // A click must not break the link to the model: the dialog opens again with its state.
        Binding {
            target: newFolderRadio
            property: "checked"
            value: dialog.model !== null && !dialog.model.target_original
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.gridUnit
            spacing: Kirigami.Units.smallSpacing
            // Without a preview there is no folder to show: the selection has to change first.
            visible: dialog.model !== null && dialog.model.folder_instead.length === 0

            Controls.Label {
                objectName: "restoreTargetDirLabel"
                Layout.fillWidth: true
                text: dialog.model
                    ? (dialog.model.target_original ? dialog.model.new_folder : dialog.model.target_dir)
                    : ""
                textFormat: Text.PlainText
                elide: Text.ElideMiddle
                enabled: newFolderRadio.checked
            }
            Controls.Button {
                objectName: "restoreChooseFolderButton"
                text: Texts.label("choose")
                icon.name: "folder-open"
                onClicked: folderDialog.open()
            }
        }

        Controls.RadioButton {
            id: originalRadio
            objectName: "restoreOriginalRadio"
            Layout.fillWidth: true
            enabled: dialog.model !== null && dialog.model.original_available
            text: i18n("To the original location")
            onClicked: dialog.model.chooseOriginal()
        }
        Binding {
            target: originalRadio
            property: "checked"
            value: dialog.model !== null && dialog.model.target_original
        }

        Controls.Label {
            objectName: "restoreOriginalDirLabel"
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.gridUnit
            visible: dialog.model !== null && dialog.model.original_dir.length > 0
            text: dialog.model ? dialog.model.original_dir : ""
            textFormat: Text.PlainText
            elide: Text.ElideMiddle
            enabled: originalRadio.checked
        }

        Controls.Label {
            objectName: "restoreMappingLabel"
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.gridUnit
            visible: dialog.model !== null && dialog.model.mapping_text.length > 0
            text: dialog.model ? dialog.model.mapping_text : ""
            textFormat: Text.PlainText
            wrapMode: Text.Wrap
            opacity: 0.75
        }

        Controls.Label {
            objectName: "restoreOriginalNote"
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.gridUnit
            visible: dialog.model !== null && dialog.model.state === "done" && !dialog.model.original_available
            text: dialog.model ? dialog.model.original_note : ""
            textFormat: Text.PlainText
            wrapMode: Text.WordWrap
            opacity: 0.75
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.gridUnit
            visible: dialog.model !== null && dialog.model.preview_required
            spacing: Kirigami.Units.smallSpacing

            Controls.Label {
                text: i18n("Existing files:")
            }
            Controls.ComboBox {
                id: overwriteBox
                objectName: "restoreOverwriteBox"
                model: [i18n("Never overwrite"), i18n("If older"), i18n("If changed"), i18n("Always")]
                Accessible.name: i18n("Existing files:")
                onActivated: function(index) { dialog.model.chooseOverwrite(index); }
            }
            // Choosing a mode must not break the link to the model: the dialog opens again
            // with «Never».
            Binding {
                target: overwriteBox
                property: "currentIndex"
                value: dialog.model ? dialog.model.overwrite : 0
            }
        }

        LiteralMessage {
            objectName: "restoreNetworkFolderMessage"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: dialog.networkFolderRefused
            plainText: i18n("Network folders such as smb:// cannot be chosen directly. Mount the folder first or choose a local folder.")
        }

        LiteralMessage {
            objectName: "restoreCheckMessage"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: dialog.model !== null && dialog.model.check_text.length > 0
            plainText: dialog.model ? dialog.model.check_text : ""
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            visible: dialog.model !== null && dialog.model.state === "running"
            Controls.BusyIndicator {
                running: parent.visible
                Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                Layout.preferredWidth: Kirigami.Units.iconSizes.medium
            }
            Controls.Label {
                Layout.fillWidth: true
                text: i18n("Checking what the restore would change…")
                wrapMode: Text.WordWrap
            }
        }

        // A refused original location stays refused: the error offers the new folder instead.
        LiteralMessage {
            objectName: "restorePreviewError"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: dialog.model !== null && dialog.model.state === "error"
            plainText: dialog.model ? dialog.model.error_text : ""
            actions: [
                Kirigami.Action {
                    objectName: "restoreFolderInsteadAction"
                    icon.name: "folder"
                    text: i18n("Restore Folder")
                    visible: dialog.model !== null && dialog.model.folder_instead.length > 0
                    onTriggered: dialog.model.restoreFolderInstead()
                },
                Kirigami.Action {
                    objectName: "restoreNewFolderInsteadAction"
                    icon.name: "folder-new"
                    text: i18n("Into a New Folder")
                    visible: dialog.model !== null && dialog.model.target_original
                        && dialog.model.folder_instead.length === 0
                    onTriggered: dialog.model.chooseFolder("")
                },
                Kirigami.Action {
                    objectName: "restoreRetryAction"
                    icon.name: "view-refresh"
                    text: Texts.label("retry")
                    // A refused target is refused again: only the new folder helps then.
                    visible: dialog.model !== null && dialog.model.folder_instead.length === 0
                        && !dialog.model.target_refused
                    onTriggered: dialog.model.retry()
                }
            ]
        }

        Controls.Label {
            objectName: "restoreDefaultTargetLabel"
            Layout.fillWidth: true
            visible: dialog.model !== null && dialog.model.state === "done" && !dialog.model.preview_required
                && dialog.model.check_text.length === 0
            text: i18n("Everything is restored into this folder; nothing is overwritten.")
            wrapMode: Text.WordWrap
        }

        RowLayout {
            Layout.fillWidth: true
            visible: dialog.model !== null && dialog.model.state === "done" && dialog.model.preview_required
                && dialog.model.summary_text.length > 0
            spacing: Kirigami.Units.smallSpacing

            Controls.Label {
                objectName: "restoreSummaryLabel"
                Layout.fillWidth: true
                text: dialog.model ? dialog.model.summary_text : ""
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
            }
            Controls.Button {
                objectName: "restoreListButton"
                text: i18n("List")
                icon.name: dialog.listShown ? "arrow-up" : "arrow-down"
                Accessible.name: dialog.listShown ? i18n("Hide the list of files") : i18n("Show the list of files")
                checkable: true
                checked: dialog.listShown
                onToggled: dialog.listShown = checked
            }
        }

        Kirigami.InlineMessage {
            objectName: "restoreConsequenceMessage"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Warning
            visible: dialog.model !== null && dialog.model.state === "done" && dialog.model.overwrites
            text: dialog.model ? dialog.model.consequence_text : ""
        }

        Kirigami.InlineMessage {
            objectName: "restoreNotesMessage"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Information
            visible: dialog.model !== null && dialog.model.state === "done" && dialog.model.notes_text.length > 0
            text: dialog.model ? dialog.model.notes_text : ""
        }

        RowLayout {
            Layout.fillWidth: true
            visible: dialog.listShown && dialog.model !== null && dialog.model.state === "done"
                && dialog.model.preview_required
            spacing: Kirigami.Units.mediumSpacing

            Controls.Label {
                objectName: "restorePreviewCountLabel"
                Layout.fillWidth: true
                text: dialog.model ? dialog.model.count_text : ""
                elide: Text.ElideRight
                opacity: 0.8
            }
            FilterField {
                id: previewFilter
                objectName: "restorePreviewFilter"
                Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                visible: dialog.model !== null && !dialog.model.too_large
                onFilterRequested: function(text) { dialog.model.setFilter(text); }
            }
        }

        Controls.Label {
            objectName: "restorePreviewTooLarge"
            Layout.fillWidth: true
            visible: dialog.listShown && dialog.model !== null && dialog.model.state === "done"
                && dialog.model.too_large
            text: dialog.model ? dialog.model.too_large_text : ""
            wrapMode: Text.WordWrap
        }

        KindChips {
            id: kindChips
            objectName: "restoreKindChips"
            Layout.fillWidth: true
            // One kind alone filters nothing.
            visible: dialog.listShown && restoreListView.visible
                && dialog.model !== null && dialog.model.kind_codes.length > 1
            listModel: dialog.model
            namePrefix: "restoreKindChip_"
        }

        KeyboardList {
            id: restoreListView
            objectName: "restorePreviewList"
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(contentHeight, Kirigami.Units.gridUnit * 10)
            visible: dialog.listShown && dialog.model !== null && dialog.model.state === "done"
                && dialog.model.preview_required && !dialog.model.too_large
            clip: true
            keyNavigationEnabled: true
            model: dialog.model
            Controls.ScrollBar.vertical: Controls.ScrollBar {}
            delegate: Controls.ItemDelegate {
                id: row
                focusPolicy: Qt.ClickFocus
                required property string path
                required property string outcomeText
                required property string sizeText
                width: ListView.view.width
                Accessible.name: row.path + ", " + row.outcomeText
                contentItem: RowLayout {
                    spacing: Kirigami.Units.mediumSpacing
                    Controls.Label {
                        Layout.fillWidth: true
                        text: row.path
                        textFormat: Text.PlainText
                        elide: Text.ElideMiddle
                    }
                    // A column of its own, so the outcomes line up.
                    Controls.Label {
                        text: row.outcomeText
                        opacity: 0.75
                        elide: Text.ElideRight
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    }
                    Controls.Label {
                        text: row.sizeText
                        opacity: 0.75
                        horizontalAlignment: Text.AlignRight
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 4
                    }
                }
            }
        }
    }
}
