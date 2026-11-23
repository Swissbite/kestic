// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.AboutPage {
    id: page
    objectName: "mainAboutPage"

    required property var appBackend

    // Kirigami centres the text of its link buttons. The homepage and licence links fill their
    // rows under «Copyright», so they stood apart from the text around them; all links start
    // on the left instead.
    function alignLinksLeft(item) {
        for (let i = 0; i < item.children.length; ++i) {
            const child = item.children[i];
            if (child.normalColor !== undefined && child.horizontalAlignment !== undefined) {
                child.horizontalAlignment = Text.AlignLeft;
            }
            page.alignLinksLeft(child);
        }
    }

    Component.onCompleted: alignLinksLeft(page)

    // Version and build commit of one process; the commit links to GitHub when it is clean.
    component BuildRow: RowLayout {
        id: buildRow

        property string version
        property string commit
        readonly property bool known: version !== ""
        readonly property string shownCommit: BuildInfo.shownCommit(commit)
        readonly property string commitUrl: BuildInfo.commitUrl(commit)

        spacing: Kirigami.Units.smallSpacing

        Controls.Label {
            text: buildRow.known ? buildRow.version : i18n("not available")
        }
        Controls.Label {
            visible: buildRow.known && buildRow.shownCommit !== ""
            text: "·"
            Accessible.ignored: true
        }
        Kirigami.UrlButton {
            objectName: buildRow.objectName + "CommitLink"
            visible: buildRow.known && buildRow.commitUrl !== ""
            url: buildRow.commitUrl
            text: buildRow.shownCommit
            Accessible.name: i18n("Source code %1", buildRow.shownCommit)
        }
        Controls.Label {
            objectName: buildRow.objectName + "CommitText"
            visible: buildRow.known && buildRow.shownCommit !== "" && buildRow.commitUrl === ""
            text: buildRow.shownCommit
        }
    }

    aboutData: {
        "displayName": "Kestic",
        "productName": "kestic",
        "componentName": "kestic",
        "shortDescription": i18n("Encrypted backup manager for Plasma powered by restic (%1)", appBackend.restic_version !== "" ? appBackend.restic_version : i18n("not available")),
        "version": BuildInfo.appVersion(),
        "homepage": BuildInfo.homepageUrl(),
        "bugAddress": BuildInfo.issuesUrl(),
        "otherText": "",
        "copyrightStatement": "© 2026 David Däster",
        "desktopFileName": "net.daester.Kestic",
        "authors": [{ "name": "David Däster", "task": "", "emailAddress": "", "webAddress": "", "ocsUsername": "" }],
        "credits": [],
        "translators": [],
        "licenses": [{ "name": "GPL-3.0-or-later", "spdx": "GPL-3.0-or-later", "text": "https://www.gnu.org/licenses/gpl-3.0.html" }]
    }

    Kirigami.Heading {
        Kirigami.FormData.isSection: true
        text: i18n("Version")
    }

    BuildRow {
        objectName: "appBuildRow"
        Kirigami.FormData.label: i18n("Application:")
        version: BuildInfo.appVersion()
        commit: BuildInfo.appCommit()
    }

    BuildRow {
        objectName: "daemonBuildRow"
        Kirigami.FormData.label: i18n("Background service:")
        version: page.appBackend.daemon_version
        commit: page.appBackend.daemon_build_commit
    }
}
