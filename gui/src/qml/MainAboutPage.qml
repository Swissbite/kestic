// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

Kirigami.AboutPage {
    required property var appBackend

    aboutData: {
        "displayName": "Kestic",
        "productName": "kestic",
        "componentName": "kestic",
        "shortDescription": i18n("Encrypted backup manager for Plasma powered by restic (%1)", appBackend.restic_version !== "" ? appBackend.restic_version : i18n("not available")),
        "version": appBackend.daemon_version !== "" ? appBackend.daemon_version : "0.1.0",
        "homepage": "https://daester.net",
        "bugAddress": "david@daester.net",
        "authors": [{ "name": "David Däster", "emailAddress": "david@daester.net" }],
        "licenses": [{ "name": "GPL-3.0-or-later", "spdx": "GPL-3.0-or-later" }]
    }
}
