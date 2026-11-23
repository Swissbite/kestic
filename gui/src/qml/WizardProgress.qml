// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// «Step 1 of 3» with a progress bar at the top of a wizard.
RowLayout {
    id: progress

    required property int step
    required property int totalSteps
    property string labelObjectName: ""

    Layout.fillWidth: true
    spacing: Kirigami.Units.mediumSpacing

    Controls.Label {
        objectName: progress.labelObjectName
        text: i18n("Step %1 of %2", progress.step, progress.totalSteps)
        font.bold: true
    }

    Controls.ProgressBar {
        // Fusion leaves the bar 0 px high and draws nothing; 6 px is the bar of Breeze.
        Layout.minimumHeight: topInset + bottomInset + Math.round(Kirigami.Units.smallSpacing * 1.5)
        from: 1
        to: progress.totalSteps
        value: progress.step
        Layout.fillWidth: true
    }
}
