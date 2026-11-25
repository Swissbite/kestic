// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// Why a form field is refused, below the field: in the negative colour and wrapped to the width
// of the form.
Controls.Label {
    color: Kirigami.Theme.negativeTextColor
    wrapMode: Text.Wrap
    Layout.fillWidth: true
}
