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
    standardButtons: Kirigami.Dialog.Cancel
    padding: Kirigami.Units.largeSpacing
    // Centred by its content height: positioning the popup may adjust `height`, which the
    // default binding of `y` reads.
    y: parent ? Math.max(0, Math.round((parent.height - implicitHeight) / 2)) : 0

    required property var model
    // The kinds of outcome whose chip is checked; none shows all files.
    property var checkedKinds: []
    property bool listShown: false

    // Opens the dialog for `selection` (absolute snapshot paths, empty = the whole snapshot).
    function openFor(destinationId, snapshotId, selection) {
        checkedKinds = [];
        listShown = false;
        previewFilter.text = "";
        model.open(destinationId, snapshotId, selection);
        open();
    }

    function toggleKind(code, checked) {
        var kinds = checkedKinds.filter(function(k) { return k !== code; });
        if (checked) kinds.push(code);
        checkedKinds = kinds;
        model.setKinds(kinds);
    }

    onRejected: model.cancel()
    onOpened: (originalRadio.checked ? originalRadio : newFolderRadio).forceActiveFocus(Qt.TabFocusReason)

    // When files would be overwritten, «Cancel» is the default: it holds the focus and the
    // default frame, so Return cancels.
    readonly property bool overwrites: dialog.model !== null && dialog.model.overwrites
    onOverwritesChanged: {
        var cancel = dialog.standardButton(Kirigami.Dialog.Cancel);
        if (cancel) {
            cancel.Accessible.defaultButton = overwrites;
            if (overwrites) cancel.forceActiveFocus(Qt.TabFocusReason);
        }
    }

    customFooterActions: [
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
        onAccepted: {
            var local = decodeURIComponent(selectedFolder.toString().replace(/^file:\/\//, ""));
            if (local.length > 0) dialog.model.chooseFolder(local);
        }
    }

    ColumnLayout {
        implicitWidth: Math.min(Kirigami.Units.gridUnit * 30,
            dialog.absoluteMaximumWidth - dialog.leftPadding - dialog.rightPadding - Kirigami.Units.gridUnit)
        spacing: Kirigami.Units.mediumSpacing

        Kirigami.Heading {
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
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.gridUnit
            visible: dialog.model !== null && dialog.model.state === "done" && !dialog.model.original_available
            text: i18n("The original location cannot be used for this selection; choose a folder.")
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
                currentIndex: dialog.model ? dialog.model.overwrite : 0
                Accessible.name: i18n("Existing files:")
                onActivated: function(index) { dialog.model.chooseOverwrite(index); }
            }
        }

        Kirigami.InlineMessage {
            objectName: "restoreCheckMessage"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: dialog.model !== null && dialog.model.check_text.length > 0
            text: dialog.model ? dialog.model.check_text : ""
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

        Kirigami.InlineMessage {
            objectName: "restorePreviewError"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: dialog.model !== null && dialog.model.state === "error"
            text: dialog.model ? dialog.model.error_text : ""
            actions: [
                Kirigami.Action {
                    objectName: "restoreFolderInsteadAction"
                    icon.name: "folder"
                    text: i18n("Restore Folder")
                    visible: dialog.model !== null && dialog.model.folder_instead.length > 0
                    onTriggered: dialog.model.restoreFolderInstead()
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
            Kirigami.SearchField {
                id: previewFilter
                objectName: "restorePreviewFilter"
                Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                visible: dialog.model !== null && !dialog.model.too_large
                placeholderText: Texts.label("filter")
                Accessible.name: Texts.label("filter")
                focusSequences: []
                onTextChanged: previewFilterDelay.restart()
            }
        }

        Timer {
            id: previewFilterDelay
            interval: 300
            onTriggered: dialog.model.setFilter(previewFilter.text)
        }

        Controls.Label {
            objectName: "restorePreviewTooLarge"
            Layout.fillWidth: true
            visible: dialog.listShown && dialog.model !== null && dialog.model.state === "done"
                && dialog.model.too_large
            text: dialog.model ? dialog.model.too_large_text : ""
            wrapMode: Text.WordWrap
        }

        Flow {
            objectName: "restoreKindChips"
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            // One kind alone filters nothing.
            visible: dialog.listShown && restoreListView.visible
                && dialog.model !== null && dialog.model.kind_codes.length > 1

            Repeater {
                model: dialog.model ? dialog.model.kind_codes : []
                delegate: Kirigami.Chip {
                    required property int index
                    required property string modelData
                    objectName: "restoreKindChip_" + modelData
                    text: dialog.model.kind_texts[index]
                    checkable: true
                    closable: false
                    checked: dialog.checkedKinds.indexOf(modelData) >= 0
                    Accessible.name: text
                    onToggled: dialog.toggleKind(modelData, checked)
                }
            }
        }

        ListView {
            id: restoreListView
            objectName: "restorePreviewList"
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(contentHeight, Kirigami.Units.gridUnit * 10)
            visible: dialog.listShown && dialog.model !== null && dialog.model.state === "done"
                && dialog.model.preview_required && !dialog.model.too_large
            clip: true
            activeFocusOnTab: true
            keyNavigationEnabled: true
            model: dialog.model
            Controls.ScrollBar.vertical: Controls.ScrollBar {}
            delegate: Controls.ItemDelegate {
                id: row
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
                    Controls.Label {
                        text: row.outcomeText
                        opacity: 0.75
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
