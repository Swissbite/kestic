// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// Checkable chips that filter a list by kind (of change, of outcome), each with its number.
// No chip checked shows every entry. The chips are named `namePrefix` + kind code.
Flow {
    id: chips

    // The list model: `kind_codes`, `kind_texts` and `setKinds(codes)`.
    required property var listModel
    required property string namePrefix
    // The kinds whose chip is checked.
    property var checkedKinds: []

    // The checked kinds changed (the model filters by them already).
    signal kindsToggled()

    // Unchecks every chip; the model is not told (a new list starts unfiltered).
    function uncheckAll() {
        checkedKinds = [];
    }

    function toggleKind(code, checked) {
        var kinds = checkedKinds.filter(function(k) { return k !== code; });
        if (checked) kinds.push(code);
        checkedKinds = kinds;
        listModel.setKinds(kinds);
        kindsToggled();
    }

    spacing: Kirigami.Units.smallSpacing

    Repeater {
        model: chips.listModel ? chips.listModel.kind_codes : []
        delegate: Kirigami.Chip {
            id: kindChip
            required property int index
            required property string modelData
            objectName: chips.namePrefix + modelData
            text: chips.listModel.kind_texts[index]
            checkable: true
            closable: false
            checked: chips.checkedKinds.indexOf(modelData) >= 0
            Accessible.name: text
            onToggled: {
                chips.toggleKind(kindChip.modelData, kindChip.checked);
                kindChip.forceActiveFocus(Qt.TabFocusReason);
            }

            FocusRing {
                target: kindChip
                radius: Math.round(kindChip.height / 2)
                ringColor: kindChip.checked ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
            }
        }
    }
}
