// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Offscreen GUI smoke scenarios for Milestone M5. Each scenario returns 0 on success and a unique
// non-zero code on the first failed check.

#include "src/smoke_m3_util.h"

#include <QFile>
#include <QFont>
#include <QFontInfo>
#include <algorithm>
#include <QPointF>

namespace kestic::smoke {

namespace {

using namespace m3;

QByteArray card_name(const char* card) {
    return QByteArray("wizDestTypeCard_") + card;
}

QQuickItem* card_item(QQuickWindow* w, const char* card) {
    return lookup(w, card_name(card).constData());
}

double scene_y(QQuickItem* item) {
    return item->mapToScene(QPointF(0, 0)).y();
}

// The text item below `root` (itself included) that shows `text`; nullptr if there is none.
QQuickItem* text_item_showing(QQuickItem* root, const QString& text) {
    if (root->inherits("QQuickText") && root->property("text").toString() == text) {
        return root;
    }
    for (QQuickItem* child : root->childItems()) {
        if (QQuickItem* found = text_item_showing(child, text)) {
            return found;
        }
    }
    return nullptr;
}

// The pixel size of the font of the text item below `root` that shows `text`; -1 if there is none.
int text_pixel_size(QQuickItem* root, const QString& text) {
    QQuickItem* item = text_item_showing(root, text);
    return item ? QFontInfo(item->property("font").value<QFont>()).pixelSize() : -1;
}

// Whether the title of the title-and-subtitle row `row` is drawn in a font at least as large as its
// subtitle.
bool title_not_smaller_than_subtitle(QQuickItem* row) {
    const int title = text_pixel_size(row, row->property("text").toString());
    const int subtitle = text_pixel_size(row, row->property("subtitle").toString());
    return title > 0 && subtitle > 0 && title >= subtitle;
}

// Scrolls the page `page_name` so that `item` is in view (the narrow window does not show all of
// it).
void scroll_into_view_in(QQuickWindow* w, const char* page_name, QQuickItem* item) {
    QQuickItem* page = lookup(w, page_name);
    auto* flick = page ? qvariant_cast<QQuickItem*>(page->property("flickable")) : nullptr;
    auto* content = flick ? qvariant_cast<QQuickItem*>(flick->property("contentItem")) : nullptr;
    if (!content) {
        return;
    }
    const double top = item->mapToItem(content, QPointF(0, 0)).y();
    const double max_y = std::max(0.0, flick->property("contentHeight").toDouble() - flick->height());
    flick->setProperty("contentY", std::clamp(top - 20.0, 0.0, max_y));
    // quiet window: the page repaints at its new position.
    wait_ms(100);
}

// Scrolls the wizard page so that `item` is in view.
void scroll_into_view(QQuickWindow* w, QQuickItem* item) {
    scroll_into_view_in(w, "addDestinationWizard", item);
}

// Opens «Add Destination» on its first step, the destination type.
int open_destination_types(QQuickWindow* w) {
    QMetaObject::invokeMethod(w, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QStringLiteral("init")));
    CHECK(5001, wait_until([&] { return lookup(w, "addDestinationWizard") != nullptr; }, 3000), "addDestinationWizard did not open");
    CHECK(5002, wait_until([&] { return card_item(w, "drive") != nullptr; }, 3000), "destination type cards missing");
    wait_ms(250); // quiet window: the wizard ends its transition
    finish_incubation(w);
    return 0;
}

// Clicks the rclone entry and waits for its install panel.
int open_rclone_entry(QQuickWindow* w) {
    QQuickItem* rclone = card_item(w, "rclone");
    CHECK(5010, rclone != nullptr, "rclone card missing");
    scroll_into_view(w, rclone);
    CHECK(5011, click_ui_item(w, rclone, "rclone card"), "click on the rclone card failed");
    CHECK(5012, wait_until([&] { return count_visible(w, "wizDestInstallPanel") == 1; }, 2000), "install panel did not open");
    // quiet window: the page lays out its new height.
    wait_ms(150);
    scroll_into_view(w, rclone);
    return 0;
}

// AC-M5-04-1 (GUI) and AC-M5-04-3: with `ssh` and without `rclone` in the daemon's PATH, the five
// ready cards stand under «Ready» before «Needs Installation» with rclone; the opened rclone entry
// shows the package and `sudo apt install rclone` but no «Open in Discover» (rclone has no
// AppStream component).
int scenario_destination_types(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (const int rc = open_destination_types(w)) {
        return rc;
    }

    QQuickItem* ready_header = lookup(w, "wizDestReadySection");
    QQuickItem* needs_header = lookup(w, "wizDestNeedsInstallationSection");
    CHECK2(5020, ready_header != nullptr, ready_header->isVisible(), "«Ready» section missing");
    CHECK2(5021, needs_header != nullptr, needs_header->isVisible(), "«Needs Installation» section missing");
    CHECK_EQ(5022, ready_header->property("text").toString(), loc(c.lang, "Ready", "Bereit"), "ready section title");
    CHECK_EQ(5023, needs_header->property("text").toString(), loc(c.lang, "Needs Installation", "Braucht Installation"), "needs installation section title");

    QQuickItem* rclone = card_item(w, "rclone");
    CHECK2(5024, rclone != nullptr, rclone->isVisible(), "rclone card missing");
    // (b) rclone sits under «Needs Installation».
    CHECK(5025, scene_y(rclone) > scene_y(needs_header), "AC-M5-04-1 (b): rclone card is not under «Needs Installation»");
    CHECK(5026, !rclone->property("checkable").toBool(), "AC-M5-04-1 (b): rclone card is a ready card");
    int code = 5030;
    for (const char* ready : {"drive", "folder", "sftp", "rest", "cloud"}) {
        QQuickItem* card = card_item(w, ready);
        CHECK2(code, card != nullptr, card->isVisible(), card_name(ready).constData());
        // (a) under «Ready».
        CHECK(code + 1, scene_y(card) > scene_y(ready_header), "AC-M5-04-1 (a): ready card above the «Ready» section");
        CHECK(code + 2, card->property("checkable").toBool(), "AC-M5-04-1 (a): card is not a ready card");
        // (c) before every card that needs installation.
        CHECK(code + 3, scene_y(card) < scene_y(needs_header), "AC-M5-04-1 (c): ready card after «Needs Installation»");
        code += 4;
    }
    CHECK_EQ(5060, card_item(w, "sftp")->property("text").toString(), loc(c.lang, "NAS / Server via SSH", "NAS / Server per SSH"), "sftp card title");
    CHECK_EQ(5061, rclone->property("text").toString(), loc(c.lang, "Cloud Services via rclone", "Cloud-Dienste über rclone"), "rclone card title");
    for (const char* each : {"drive", "folder", "sftp", "rest", "cloud", "rclone"}) {
        CHECK(5062, title_not_smaller_than_subtitle(card_item(w, each)), card_name(each).constData());
    }
    // The chosen card's coloured icon sits on its backdrop, so it keeps its body on the selection.
    QQuickItem* chosen = card_item(w, "folder");
    CHECK(5063, chosen->property("checked").toBool(), "the folder card is not chosen");
    CHECK(5064, count_visible(w, "iconBackdrop_wizDestTypeCard_folder") == 1, "no icon backdrop on the chosen card");

    if (const int rc = open_rclone_entry(w)) {
        return rc;
    }
    QQuickItem* package = lookup(w, "wizDestInstallPackageLabel");
    QQuickItem* command = lookup(w, "wizDestInstallCommandField");
    // AC-M5-04-3 (a)
    CHECK2(5070, package != nullptr, package->isVisible(), "AC-M5-04-3 (a): package label missing");
    CHECK_EQ(5071, package->property("text").toString(), loc(c.lang, "Package: rclone", "Paket: rclone"), "AC-M5-04-3 (a): package");
    // AC-M5-04-3 (b)
    // The same lookup finds the button «Check Again» of the panel, so it would find a Discover one.
    CHECK(5073, visible_button(w->contentItem(), "Check Again|Erneut prüfen") != nullptr, "button lookup finds «Check Again»");
    CHECK(5072, visible_button(w->contentItem(), "Open in Discover|In Discover öffnen") == nullptr,
        "AC-M5-04-3 (b): rclone has no AppStream component, but «Open in Discover» is shown");
    // AC-M5-04-3 (c)
    CHECK2(5075, command != nullptr, command->isVisible(), "AC-M5-04-3 (c): install command missing");
    CHECK_EQ(5076, command->property("text").toString(), QStringLiteral("sudo apt install rclone"), "AC-M5-04-3 (c): command");
    QQuickItem* again = lookup(w, "wizDestCheckAgainButton");
    CHECK2(5077, again != nullptr, again->isVisible(), "«Check Again» missing");

    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestTypeCard_folder"),
        QStringLiteral("wizDestTypeCard_rclone"),
        QStringLiteral("wizDestInstallCommandField"),
        QStringLiteral("wizDestCopyInstallCommandButton"),
        QStringLiteral("wizDestCheckAgainButton"),
    });
    CHECK_EQ(5079, unnamed, QString(), "control without accessible name");
    CHECK_EQ(5080, accessible_name(lookup(w, "wizDestCopyInstallCommandButton")), loc(c.lang, "Copy", "Kopieren"), "accessible name of «Copy»");
    return shot(c, 5078);
}

// AC-M5-04-4 (GUI): rclone is put into the daemon's PATH while its entry is open; «Check Again»
// moves it under «Ready».
int scenario_check_again(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* rclone_card = card_item(w, "rclone");
    CHECK2(5100, rclone_card != nullptr, !rclone_card->property("checkable").toBool(), "precondition: rclone needs installation");
    if (const int rc = open_rclone_entry(w)) {
        return rc;
    }

    const QString dir = qEnvironmentVariable("KESTIC_SMOKE_PROGRAM_DIR");
    CHECK(9000, !dir.isEmpty(), "KESTIC_SMOKE_PROGRAM_DIR not set");
    QFile rclone_file(dir + QStringLiteral("/rclone"));
    CHECK(9001, rclone_file.open(QIODevice::WriteOnly), "cannot write the fake rclone");
    rclone_file.write("#!/bin/sh\nexit 0\n");
    rclone_file.close();
    CHECK(9002, rclone_file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner), "cannot make the fake rclone executable");

    QQuickItem* again = lookup(w, "wizDestCheckAgainButton");
    CHECK(9003, again != nullptr, "«Check Again» missing");
    scroll_into_view(w, again);
    CHECK(9004, click_ui_item(w, again, "Check Again"), "click on «Check Again» failed");
    // (a) rclone is ready now: a ready card, below «Ready», and no section «Needs Installation».
    CHECK(9005, wait_until([&] {
        QQuickItem* card = card_item(w, "rclone");
        return card && card->property("checkable").toBool();
    }, 3000), "AC-M5-04-4 (a): rclone did not become ready");
    wait_until([&] {
        QQuickItem* needs = lookup(w, "wizDestNeedsInstallationSection");
        return needs && lookup(w, "wizDestReadySection") && !needs->isVisible()
            && count_visible(w, "wizDestInstallPanel") == 0;
    }, 200);
    QQuickItem* needs_header = lookup(w, "wizDestNeedsInstallationSection");
    QQuickItem* ready_header = lookup(w, "wizDestReadySection");
    CHECK2(9006, needs_header != nullptr, ready_header != nullptr, "section headers missing");
    CHECK(9007, !needs_header->isVisible(), "AC-M5-04-4 (a): «Needs Installation» still shown");
    CHECK(9008, scene_y(card_item(w, "rclone")) > scene_y(ready_header), "AC-M5-04-4 (a): rclone not under «Ready»");
    CHECK(9009, count_visible(w, "wizDestInstallPanel") == 0, "AC-M5-04-4 (a): install panel still open");

    return shot(c, 5109);
}

// Chooses the ready card `card` and moves on to the step «destination details».
int open_details_of(QQuickWindow* w, const char* card) {
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* item = card_item(w, card);
    CHECK2(5200, item != nullptr, item->isVisible(), card_name(card).constData());
    scroll_into_view(w, item);
    CHECK(9010, click_ui_item(w, item, card), "click on the card failed");
    QQuickItem* next = lookup(w, "wizDestNextButton");
    CHECK(9011, wait_until([&] { return next && next->isEnabled(); }, 2000), "«Next» not enabled after choosing the card");
    CHECK(9012, click_ui_item(w, next, "Next"), "click on «Next» failed");
    CHECK(9013, wait_until([&] {
        QQuickItem* name = lookup(w, "wizDestNameField");
        return name && name->isVisible();
    }, 2000), "step «destination details» did not open");
    wait_ms(150); // quiet window: the step ends its transition
    return 0;
}

// AC-M5-05-1 (GUI): with the UDisks2 fake reporting the drive «USB-Disk» on
// /media/user/USB-Disk, the type «External Drive / USB» offers it with name and mount point;
// choosing it puts the mount point into the path.
int scenario_drive_list(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (const int rc = open_details_of(w, "drive")) {
        return rc;
    }
    CHECK(9014, wait_until([&] { return count_visible(w, "wizDestDrive_0") == 1; }, 3000), "AC-M5-05-1 (a): the drive is not offered");
    QQuickItem* drive = lookup(w, "wizDestDrive_0");
    CHECK_EQ(9015, drive->property("text").toString(), QStringLiteral("USB-Disk"), "AC-M5-05-1 (a): name");
    CHECK_EQ(9016, drive->property("subtitle").toString(), QStringLiteral("/media/user/USB-Disk"), "AC-M5-05-1 (a): mount point");
    CHECK(9017, count_visible(w, "wizDestDrive_1") == 0, "only the removable drive is offered");
    CHECK(9018, count_visible(w, "wizDestNoDriveMessage") == 0, "«no drive» message shown next to a drive");
    CHECK(9019, title_not_smaller_than_subtitle(drive), "drive name drawn smaller than its mount point");
    QQuickItem* drive_title = text_item_showing(drive, QStringLiteral("USB-Disk"));
    CHECK(9020, drive_title != nullptr && drive_title->property("textFormat").toInt() == Qt::PlainText,
        "a drive label could be read as markup");
    CHECK(9021, click_ui_item(w, drive, "drive"), "click on the drive failed");
    QQuickItem* location = lookup(w, "wizDestLocationField");
    CHECK(9022, wait_until([&] { return location && location->property("text").toString() == QLatin1String("/media/user/USB-Disk"); }, 2000),
        "choosing the drive did not set the path");
    CHECK(9023, drive->property("checked").toBool(), "chosen drive is not marked");
    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestDrive_0"),
        QStringLiteral("wizDestDrivesCheckAgainButton"),
    });
    CHECK_EQ(9024, unnamed, QString(), "control without accessible name");
    return shot(c, 5219);
}

// Whether `item` is `ancestor` or lies below it.
bool lies_in(QQuickItem* item, QQuickItem* ancestor) {
    for (QQuickItem* it = item; it; it = it->parentItem()) {
        if (it == ancestor) {
            return true;
        }
    }
    return false;
}

// The index of the row of `list` that holds the keyboard focus; -1 when the focus is elsewhere.
int focused_row_of(QQuickWindow* w, QQuickItem* list) {
    auto* content = list->property("contentItem").value<QQuickItem*>();
    for (QQuickItem* it = w->activeFocusItem(); it; it = it->parentItem()) {
        if (it->parentItem() == content) {
            return it->property("index").toInt();
        }
    }
    return -1;
}

// AC-M4-01-7 for the removable drives of «External Drive / USB» (two drives from the UDisks2
// fake): the list is one Tab stop. (a) Tab enters it on the first drive, (b) Down moves to the
// second, which Space chooses (a second Space keeps it chosen), Home and End move to the first
// and the last drive, (c) the next Tab leaves the list to «Check Again», Shift+Tab comes back to
// the second drive and the next Shift+Tab leaves the list to the control before it, and (d) no
// drive is a Tab stop of its own.
int scenario_drive_list_keys(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (const int rc = open_details_of(w, "drive")) {
        return rc;
    }
    CHECK(9025, wait_until([&] { return count_visible(w, "wizDestDrive_1") == 1; }, 3000), "the two drives are not offered");
    QQuickItem* list = lookup(w, "wizDestDriveList");
    CHECK(9026, list != nullptr && list->isVisible(), "the drives are no list");
    for (int i = 0; i < 40 && !lies_in(w->activeFocusItem(), list); ++i) {
        press_tab(w);
    }
    CHECK(9027, focused_row_of(w, list) == 0,
        qPrintable(QStringLiteral("(a) Tab enters the drives on the first one: ") + describe_focus_item(w->activeFocusItem())));
    CHECK(5233, press_until(w, Qt::Key_Down, [&] { return focused_row_of(w, list) == 1; }, 80),
        qPrintable(QStringLiteral("(b) Down moves to the second drive: ") + describe_focus_item(w->activeFocusItem())));
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    QQuickItem* location = lookup(w, "wizDestLocationField");
    QQuickItem* second = lookup(w, "wizDestDrive_1");
    CHECK(5234, wait_until([&] { return location && location->property("text").toString() == QLatin1String("/media/user/USB-Disk"); }, 2000),
        "(b) Space on the second drive does not choose it");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    wait_ms(80); // quiet window: a second Space that changed the path would show by now
    CHECK_EQ(9028, location->property("text").toString(), QStringLiteral("/media/user/USB-Disk"), "(b) a second Space keeps the path");
    CHECK(9029, second->property("checked").toBool(), "(b) a second Space unchecks the chosen drive");
    CHECK(9030, press_until(w, Qt::Key_Home, [&] { return focused_row_of(w, list) == 0; }, 80), qPrintable(QStringLiteral("(b) Home moves to the first drive: ") + describe_focus_item(w->activeFocusItem())));
    CHECK(9031, press_until(w, Qt::Key_End, [&] { return focused_row_of(w, list) == 1; }, 80), qPrintable(QStringLiteral("(b) End moves to the last drive: ") + describe_focus_item(w->activeFocusItem())));
    press_tab(w);
    CHECK_EQ(5235, w->activeFocusItem() ? w->activeFocusItem()->objectName() : QString(),
        QStringLiteral("wizDestDrivesCheckAgainButton"), "(c) the next Tab leaves the drives to «Check Again»");
    QQuickItem* check_again = w->activeFocusItem();
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
    CHECK(5236, focused_row_of(w, list) == 1,
        qPrintable(QStringLiteral("(c) Shift+Tab comes back to the second drive: ") + describe_focus_item(w->activeFocusItem())));
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
    QQuickItem* before = w->activeFocusItem();
    CHECK(9032, before && !lies_in(before, list) && before != check_again,
        qPrintable(QStringLiteral("(c) the next Shift+Tab leaves the drives to the control before them: ") + describe_focus_item(before)));
    auto* content = list->property("contentItem").value<QQuickItem*>();
    QList<QQuickItem*> items;
    collect_visual_descendants(content, items);
    int rows = 0;
    for (QQuickItem* it : items) {
        CHECK(5237, !(it->isVisible() && it->activeFocusOnTab()),
            qPrintable(QStringLiteral("(d) a drive is a Tab stop: ") + describe_focus_item(it)));
        rows += it->objectName().startsWith(QLatin1String("wizDestDrive_")) ? 1 : 0;
    }
    CHECK(5238, rows == 2, qPrintable(QStringLiteral("(d) drives checked: %1").arg(rows)));
    return shot(c, 5239);
}

// AC-M5-05-2 (GUI): a folder on a CIFS mount (mountinfo fake); after «Test Connection» Kestic
// (a) warns that restic advises against repositories on CIFS/SMB shares, «Next» stays disabled
// until the warning is confirmed, and (b) the destination can be saved after the confirmation.
int scenario_cifs_note(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString dir = qEnvironmentVariable("KESTIC_SMOKE_CIFS_DIR");
    CHECK(5300, !dir.isEmpty(), "KESTIC_SMOKE_CIFS_DIR not set");
    if (const int rc = open_details_of(w, "folder")) {
        return rc;
    }
    lookup(w, "wizDestNameField")->setProperty("text", QStringLiteral("NAS share"));
    lookup(w, "wizDestLocationField")->setProperty("text", dir);
    QQuickItem* test = lookup(w, "wizDestTestButton");
    CHECK(5301, wait_until([&] { return test && test->isEnabled(); }, 2000), "«Test Connection» not enabled");
    CHECK(5302, click_ui_item(w, test, "Test Connection"), "click on «Test Connection» failed");
    QQuickItem* note = lookup(w, "wizDestNoteBanner");
    QQuickItem* confirm = lookup(w, "wizDestNoteConfirmCheck");
    QQuickItem* next = lookup(w, "wizDestNextButton");
    // (a)
    CHECK(5303, wait_until([&] { return note && note->isVisible(); }, 5000), "AC-M5-05-2 (a): no warning after the connection test");
    CHECK_EQ(5304, note->property("text").toString(),
        loc(c.lang,
            "This folder is on a CIFS/SMB network share. restic advises against repositories on such shares.",
            "Dieser Ordner liegt auf einer CIFS/SMB-Netzwerkfreigabe. restic rät von Repositorys auf solchen Freigaben ab."),
        "AC-M5-05-2 (a): warning text");
    CHECK2(5305, confirm != nullptr, confirm->isVisible(), "AC-M5-05-2 (b): confirmation missing");
    CHECK(9033, !next->isEnabled(), "AC-M5-05-2: «Next» enabled before the confirmation");
    CHECK_EQ(9034, accessible_name(confirm), loc(c.lang, "Use this folder anyway", "Diesen Ordner trotzdem verwenden"), "confirmation text");
    // quiet window: the messages fade in before their text is drawn.
    wait_ms(600);
    CHECK(9035, save_screenshot(w, c.out_path), "screenshot failed");
    // (b)
    CHECK(9036, click_ui_item(w, confirm, "confirm"), "click on the confirmation failed");
    CHECK(9037, wait_until([&] { return next->isEnabled(); }, 2000), "AC-M5-05-2 (b): «Next» disabled after the confirmation");
    CHECK(9038, click_ui_item(w, next, "Next"), "click on «Next» failed");
    QQuickItem* pw = lookup(w, "wizDestPasswordField");
    CHECK(9039, wait_until([&] { return pw && pw->isVisible(); }, 2000), "step «password» did not open");
    pw->setProperty("text", QStringLiteral("correct horse battery"));
    lookup(w, "wizDestPasswordConfirmField")->setProperty("text", QStringLiteral("correct horse battery"));
    CHECK(9040, wait_until([&] { return next->isEnabled(); }, 2000), "«Save Destination» not enabled");
    CHECK(9041, click_ui_item(w, next, "Save Destination"), "click on «Save Destination» failed");
    CHECK(9042, wait_until([&] { return count_visible(w, "addDestinationWizard") == 0; }, 15000),
        "AC-M5-05-2 (b): the wizard did not close after saving");
    return 0;
}

// Fills the S3 form with the server of KESTIC_SMOKE_S3_* and the secret key `secret`, then
// runs «Test Connection» and waits for its banner.
int test_s3_connection(QQuickWindow* w, const QString& secret, int code) {
    lookup(w, "wizDestSecretKeyField")->setProperty("text", secret);
    QQuickItem* test = lookup(w, "wizDestTestButton");
    CHECK(code, wait_until([&] { return test && test->isEnabled(); }, 2000), "«Test Connection» not enabled");
    scroll_into_view(w, test);
    CHECK(code + 1, click_ui_item(w, test, "Test Connection"), "click on «Test Connection» failed");
    QQuickItem* banner = lookup(w, "wizDestFeedbackBanner");
    CHECK(code + 2, wait_until([&] { return banner && banner->isVisible(); }, 15000), "no result after the connection test");
    return 0;
}

// M5-S02 (GUI): the card «Cloud Storage» offers «S3-Compatible Storage» and opens the S3 form
// (endpoint, bucket, path, region, access key, secret key). With a wrong secret key the
// connection test (AC-M5-02-3 a) shows what to do and the action «Check Keys», which puts the
// focus into the secret key; with the right key the test passes and «Next» is enabled.
int scenario_s3_form(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString endpoint = qEnvironmentVariable("KESTIC_SMOKE_S3_ENDPOINT");
    const QString access = qEnvironmentVariable("KESTIC_SMOKE_S3_ACCESS_KEY");
    const QString secret = qEnvironmentVariable("KESTIC_SMOKE_S3_SECRET_KEY");
    CHECK(5400, !endpoint.isEmpty() && !access.isEmpty() && !secret.isEmpty(), "KESTIC_SMOKE_S3_* not set");
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* cloud = card_item(w, "cloud");
    CHECK2(5401, cloud != nullptr, cloud->isVisible(), "cloud card missing");
    scroll_into_view(w, cloud);
    CHECK(5402, click_ui_item(w, cloud, "cloud"), "click on the cloud card failed");
    QQuickItem* s3_choice = lookup(w, "wizDestCloudService_s3");
    CHECK(5403, wait_until([&] { return s3_choice && s3_choice->isVisible(); }, 2000), "«S3-Compatible Storage» not offered");
    CHECK(5404, s3_choice->property("checked").toBool(), "«S3-Compatible Storage» not chosen");
    CHECK_EQ(5405, s3_choice->property("text").toString(), loc(c.lang, "S3-Compatible Storage", "S3-kompatibler Speicher"), "S3 choice text");
    // quiet window: the choice appears below the card; the page knows its new height only after a layout pass.
    wait_ms(300);
    scroll_into_view(w, s3_choice);
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(300);
    if (!c.out_path.isEmpty()) {
        QString choice_shot = c.out_path;
        choice_shot.replace(QStringLiteral("s3-form-wrong-key"), QStringLiteral("s3-type-choice"));
        CHECK(5398, save_screenshot(w, choice_shot), "screenshot failed");
    }
    QQuickItem* next = lookup(w, "wizDestNextButton");
    CHECK(5406, wait_until([&] { return next && next->isEnabled(); }, 2000), "«Next» not enabled after choosing S3");
    CHECK(5407, click_ui_item(w, next, "Next"), "click on «Next» failed");
    CHECK(5408, wait_until([&] { return count_visible(w, "wizDestEndpointField") == 1; }, 2000), "S3 form did not open");
    wait_until([&] {
        for (const char* field : {"wizDestBucketField", "wizDestS3PathField", "wizDestRegionField", "wizDestAccessKeyField", "wizDestSecretKeyField"}) {
            if (count_visible(w, field) != 1) {
                return false;
            }
        }
        return count_visible(w, "wizDestLocationField") == 0 && count_visible(w, "wizDestKeysError") == 1;
    }, 150);
    int code = 5410;
    for (const char* field : {"wizDestBucketField", "wizDestS3PathField", "wizDestRegionField", "wizDestAccessKeyField", "wizDestSecretKeyField"}) {
        CHECK(code, count_visible(w, field) == 1, field);
        ++code;
    }
    CHECK(9043, count_visible(w, "wizDestLocationField") == 0, "the path field is shown for S3");
    // The path in the bucket has no spaces, quotes, #, ? or %; the form says so.
    CHECK(9044, count_visible(w, "wizDestS3PathError") == 0, "path hint shown for an empty path");
    CHECK(5397, count_visible(w, "wizDestKeysError") == 1, "no hint for the empty keys");
    lookup(w, "wizDestS3PathField")->setProperty("text", QStringLiteral("my backup"));
    QQuickItem* path_error = lookup(w, "wizDestS3PathError");
    CHECK(9045, wait_until([&] { return path_error && path_error->isVisible(); }, 2000), "no hint for a path with a space");
    CHECK_EQ(9046, path_error->property("text").toString(),
        loc(c.lang, "Use no spaces, quotes, #, ? or %, and no empty, . or .. parts.",
            "Verwenden Sie keine Leerzeichen, Anführungszeichen, #, ? oder % und keine leeren, «.»- oder «..»-Teile."),
        "path hint text");
    scroll_into_view(w, path_error);
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(600);
    if (!c.out_path.isEmpty()) {
        QString hint_shot = c.out_path;
        hint_shot.replace(QStringLiteral("wrong-key"), QStringLiteral("path-hint"));
        CHECK(5399, save_screenshot(w, hint_shot), "screenshot failed");
    }
    lookup(w, "wizDestS3PathField")->setProperty("text", QString());
    CHECK(9047, wait_until([&] { return !path_error->isVisible(); }, 2000), "path hint stays for an empty path");

    lookup(w, "wizDestNameField")->setProperty("text", QStringLiteral("Cloud"));
    lookup(w, "wizDestEndpointField")->setProperty("text", endpoint);
    lookup(w, "wizDestBucketField")->setProperty("text", QStringLiteral("kestic-") + c.lang);
    lookup(w, "wizDestAccessKeyField")->setProperty("text", access);
    // A wrong secret key: the right one reversed.
    QString wrong_secret = secret;
    std::reverse(wrong_secret.begin(), wrong_secret.end());
    if (const int rc = test_s3_connection(w, wrong_secret, 5420)) {
        return rc;
    }
    QQuickItem* banner = lookup(w, "wizDestFeedbackBanner");
    // AC-M5-02-3 (a)
    CHECK_EQ(5430, banner->property("text").toString(),
        loc(c.lang,
            "The destination rejected the login. Check the access data and test again.",
            "Das Ziel hat die Anmeldung abgelehnt. Prüfen Sie die Zugangsdaten und testen Sie erneut."),
        "AC-M5-02-3 (a): message with what to do");
    QObject* check_keys = lookup<QObject>(w, "wizDestCheckKeysAction");
    CHECK2(5431, check_keys != nullptr, check_keys->property("visible").toBool(), "AC-M5-02-3 (a): action «Check Keys» missing");
    CHECK(5432, !next->isEnabled(), "«Next» enabled after a failed test");
    CHECK(5396, count_visible(w, "wizDestKeysError") == 0, "key hint stays with both keys entered");
    // Like the other password fields of the form, the secret key field reports no accessible
    // name through Qt's interface, so it is not in this list.
    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestEndpointField"),
        QStringLiteral("wizDestBucketField"),
        QStringLiteral("wizDestS3PathField"),
        QStringLiteral("wizDestRegionField"),
        QStringLiteral("wizDestAccessKeyField"),
    });
    CHECK_EQ(5433, unnamed, QString(), "control without accessible name");
    // The messages fade in before their text is drawn.
    scroll_into_view(w, banner);
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(600);
    CHECK(5434, save_screenshot(w, c.out_path), "screenshot failed");
    QMetaObject::invokeMethod(check_keys, "trigger");
    CHECK(5435, wait_until([&] { return lookup(w, "wizDestSecretKeyField")->hasActiveFocus(); }, 2000),
        "AC-M5-02-3 (a): «Check Keys» does not lead to the secret key");

    if (const int rc = test_s3_connection(w, secret, 5440)) {
        return rc;
    }
    CHECK(5450, wait_until([&] { return next->isEnabled(); }, 2000), "«Next» disabled after a passed test");
    // The bucket of the scenario does not exist yet: the result names it.
    CHECK_EQ(5451, banner->property("text").toString(),
        loc(c.lang,
            "Reachable\nBucket 'kestic-en' does not exist yet. Kestic creates it when the destination is added.",
            "Erreichbar\nBucket «kestic-de» gibt es noch nicht; Kestic legt ihn beim Anlegen des Ziels an."),
        "result of the passed test names the missing bucket");
    CHECK(5452, count_visible(w, "wizDestNoteConfirmCheck") == 0, "the missing bucket needs no confirmation");
    scroll_into_view(w, banner);
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(600);
    if (!c.out_path.isEmpty()) {
        QString bucket_shot = c.out_path;
        bucket_shot.replace(QStringLiteral("wrong-key"), QStringLiteral("missing-bucket"));
        CHECK(5453, save_screenshot(w, bucket_shot), "screenshot failed");
    }
    return 0;
}

// Sets the text of the field `name`; returns false when it is missing.
bool set_text(QQuickWindow* w, const char* name, const QString& text) {
    QQuickItem* field = lookup(w, name);
    if (!field) {
        return false;
    }
    field->setProperty("text", text);
    // quiet window: the form validates the new text.
    wait_ms(50);
    return true;
}

QByteArray env_name(const char* field, int row) {
    return QByteArray("wizDestEnv") + field + "_" + QByteArray::number(row);
}

// Adds a variable row named `name` with `value`; returns the row's «Geheim» switch.
QQuickItem* add_variable(QQuickWindow* w, int row, const QString& name, const QString& value) {
    QQuickItem* add = lookup(w, "wizDestEnvAdd");
    if (!add) {
        return nullptr;
    }
    scroll_into_view(w, add);
    if (!click_ui_item(w, add, "Add Variable")) {
        return nullptr;
    }
    if (!wait_until([&] { return lookup(w, env_name("Name", row).constData()) != nullptr; }, 2000)) {
        return nullptr;
    }
    set_text(w, env_name("Name", row).constData(), name);
    set_text(w, env_name("Value", row).constData(), value);
    return lookup(w, env_name("Secret", row).constData());
}

// M5-S01 (GUI parts of AC-M5-01-1 and AC-M5-01-4 a): the card «NAS / Server via SSH» opens the
// SFTP form (host, port, user, path and the hint on the SSH key and ssh-agent). The connection
// test against an unknown server asks «Trust This Server?» with key type and SHA256 fingerprint;
// «Trust Server» tests again, the server answers and «Next» is enabled. Screenshots of the form
// with the dialog and of the passed test.
int scenario_sftp_form(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString port = qEnvironmentVariable("KESTIC_SMOKE_SFTP_PORT");
    const QString user = qEnvironmentVariable("KESTIC_SMOKE_SFTP_USER");
    const QString path = qEnvironmentVariable("KESTIC_SMOKE_SFTP_PATH");
    const QString fingerprint = qEnvironmentVariable("KESTIC_SMOKE_SFTP_FINGERPRINT");
    CHECK(5600, !port.isEmpty() && !user.isEmpty() && !path.isEmpty() && !fingerprint.isEmpty(), "KESTIC_SMOKE_SFTP_* not set");
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* sftp = card_item(w, "sftp");
    CHECK2(5601, sftp != nullptr, sftp->isVisible(), "sftp card missing");
    scroll_into_view(w, sftp);
    CHECK(5602, click_ui_item(w, sftp, "sftp"), "click on the sftp card failed");
    QQuickItem* next = lookup(w, "wizDestNextButton");
    CHECK(5603, wait_until([&] { return next && next->isEnabled(); }, 2000), "«Next» not enabled after choosing SFTP");
    CHECK(5604, click_ui_item(w, next, "Next"), "click on «Next» failed");
    CHECK(5605, wait_until([&] { return count_visible(w, "wizDestHostField") == 1; }, 2000), "SFTP form did not open");
    wait_until([&] {
        for (const char* field : {"wizDestPortField", "wizDestSftpUserField", "wizDestSftpPathField", "wizDestSshKeyHint"}) {
            if (count_visible(w, field) != 1) {
                return false;
            }
        }
        return count_visible(w, "wizDestLocationField") == 0;
    }, 150);
    int code = 5606;
    for (const char* field : {"wizDestPortField", "wizDestSftpUserField", "wizDestSftpPathField", "wizDestSshKeyHint"}) {
        CHECK(code, count_visible(w, field) == 1, field);
        ++code;
    }
    CHECK(9048, count_visible(w, "wizDestLocationField") == 0, "the local path field is shown for SFTP");
    CHECK_EQ(9049, lookup(w, "wizDestPortField")->property("text").toString(), QStringLiteral("22"), "default port");
    CHECK_EQ(9050, lookup(w, "wizDestSshKeyHint")->property("text").toString(),
        loc(c.lang,
            "With your SSH key or the ssh-agent. Copy your public key to the server first, for example with ssh-copy-id.",
            "Mit Ihrem SSH-Schlüssel oder dem ssh-agent. Kopieren Sie Ihren öffentlichen Schlüssel zuerst auf den Server, zum Beispiel mit ssh-copy-id."),
        "hint on the SSH key and ssh-agent");
    set_text(w, "wizDestNameField", QStringLiteral("NAS"));
    set_text(w, "wizDestHostField", QStringLiteral("127.0.0.1"));
    set_text(w, "wizDestPortField", port);
    set_text(w, "wizDestSftpPathField", path);
    // A user name with a character a shell reads is refused in the form, as in core.
    set_text(w, "wizDestSftpUserField", QStringLiteral("anna;id"));
    QQuickItem* user_error = lookup(w, "wizDestSftpUserError");
    CHECK(5631, wait_until([&] { return user_error && user_error->isVisible(); }, 2000), "no hint for the user name «anna;id»");
    CHECK_EQ(5632, user_error->property("text").toString(),
        loc(c.lang, "Enter the user name on the server: only letters, digits, '.', '_' and '-'.",
            "Geben Sie den Benutzernamen auf dem Server ein: nur Buchstaben, Ziffern, «.», «_» und «-»."),
        "hint for the user name");
    CHECK(5633, !lookup(w, "wizDestTestButton")->isEnabled(), "«Test Connection» enabled for «anna;id»");
    // The hint wraps within the form's width (the German text is longer than a narrow window).
    QQuickItem* user_field = lookup(w, "wizDestSftpUserField");
    const double hint_right = user_error->mapToScene(QPointF(user_error->width(), 0)).x();
    const double field_right = user_field->mapToScene(QPointF(user_field->width(), 0)).x();
    CHECK(5636, hint_right <= field_right + 1.0, "the user name hint is wider than the form");
    CHECK(5637, user_error->property("wrapMode").toInt() != 0, "the user name hint does not wrap");
    if (!c.out_path.isEmpty()) {
        scroll_into_view(w, user_error);
        // quiet window: the page draws its final state before the screenshot.
        wait_ms(600);
        QString hint_shot = c.out_path;
        hint_shot.replace(QStringLiteral("sftp-form"), QStringLiteral("sftp-user-hint"));
        CHECK(5635, save_screenshot(w, hint_shot), "screenshot failed");
    }
    set_text(w, "wizDestSftpUserField", QStringLiteral("anna€"));
    CHECK(5638, wait_until([&] { return user_error->isVisible(); }, 2000), "no hint for the user name «anna€»");
    set_text(w, "wizDestSftpUserField", QStringLiteral("jörg.müller"));
    CHECK(5639, wait_until([&] { return !user_error->isVisible(); }, 2000), "hint for the user name «jörg.müller»");
    set_text(w, "wizDestSftpUserField", QStringLiteral("a・b"));
    CHECK(5640, wait_until([&] { return user_error->isVisible(); }, 2000), "no hint for the user name «a・b»");
    set_text(w, "wizDestSftpUserField", QStringLiteral("علي"));
    CHECK(5641, wait_until([&] { return !user_error->isVisible(); }, 2000), "hint for the user name «علي»");
    set_text(w, "wizDestSftpUserField", user);
    CHECK(5634, wait_until([&] { return !user_error->isVisible(); }, 2000), "hint stays for a valid user name");
    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestHostField"),
        QStringLiteral("wizDestPortField"),
        QStringLiteral("wizDestSftpUserField"),
        QStringLiteral("wizDestSftpPathField"),
    });
    CHECK_EQ(5613, unnamed, QString(), "control without accessible name");

    QQuickItem* test = lookup(w, "wizDestTestButton");
    CHECK(5614, wait_until([&] { return test && test->isEnabled(); }, 2000), "«Test Connection» not enabled");
    scroll_into_view(w, test);
    CHECK(5615, click_ui_item(w, test, "Test Connection"), "click on «Test Connection» failed");
    QObject* dialog = lookup<QObject>(w, "wizDestHostKeyDialog");
    CHECK(5616, wait_until([&] { return dialog && dialog->property("opened").toBool(); }, 15000), "AC-M5-01-4 (a): «Trust This Server?» did not open");
    CHECK_EQ(5617, dialog->property("title").toString(), loc(c.lang, "Trust This Server?", "Diesem Server vertrauen?"), "dialog title");
    QObject* type = lookup<QObject>(w, "wizDestHostKeyType");
    QObject* print = lookup<QObject>(w, "wizDestHostKeyFingerprint");
    CHECK2(5618, type != nullptr, print != nullptr, "key type or fingerprint missing");
    CHECK_EQ(5619, type->property("text").toString(), QStringLiteral("ssh-ed25519"), "AC-M5-01-4 (a): key type");
    CHECK_EQ(5620, print->property("text").toString(), fingerprint, "AC-M5-01-4 (a): SHA256 fingerprint");
    CHECK(5621, !next->isEnabled(), "«Next» enabled before the server is trusted");
    // Cancelling keeps the shown key; the result line offers the dialog again.
    QMetaObject::invokeMethod(dialog, "close");
    CHECK(5628, wait_until([&] { return !dialog->property("opened").toBool(); }, 2000), "the dialog does not close");
    QObject* reopen = lookup<QObject>(w, "wizDestShowHostKeyAction");
    CHECK2(5629, reopen != nullptr, reopen->property("visible").toBool(), "«Trust Server…» missing after Cancel");
    QMetaObject::invokeMethod(reopen, "trigger");
    CHECK(5630, wait_until([&] { return dialog->property("opened").toBool(); }, 2000), "«Trust Server…» does not open the dialog");
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(600);
    if (!c.out_path.isEmpty()) {
        QString dialog_shot = c.out_path;
        dialog_shot.replace(QStringLiteral("sftp-form"), QStringLiteral("sftp-trust-dialog"));
        CHECK(5622, save_screenshot(w, dialog_shot), "screenshot failed");
    }
    QObject* trust = lookup<QObject>(w, "wizDestTrustHostKeyAction");
    CHECK(5623, trust != nullptr, "«Trust Server» missing");
    QMetaObject::invokeMethod(trust, "trigger");
    QQuickItem* banner = lookup(w, "wizDestFeedbackBanner");
    CHECK(5624, wait_until([&] { return next->isEnabled(); }, 15000), "«Next» not enabled after trusting the server");
    CHECK_EQ(5625, banner->property("text").toString(), loc(c.lang, "Reachable", "Erreichbar"), "result after trusting the server");
    CHECK(5626, !dialog->property("opened").toBool(), "the dialog stays open");
    scroll_into_view(w, banner);
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(600);
    return shot(c, 5627);
}

// M5-S03 (GUI): «Cloud Storage» offers Backblaze B2, Azure, Google Cloud and Swift beside S3;
// B2 opens the generic form with `b2:` filled in. An unknown scheme (AC-M5-03-1 c) and a reserved
// variable (AC-M5-03-7 b) show `core`'s message and keep «Test Connection», and so adding the
// destination, locked. «Geheim» starts on for `B2_ACCOUNT_KEY`, off for `RESTIC_HOST`, and can be
// switched (AC-M5-03-4). Screenshots of the filled form and of the refused variable.
int scenario_generic_form(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* cloud = card_item(w, "cloud");
    CHECK2(5500, cloud != nullptr, cloud->isVisible(), "cloud card missing");
    scroll_into_view(w, cloud);
    CHECK(5501, click_ui_item(w, cloud, "cloud"), "click on the cloud card failed");
    int code = 5502;
    for (const char* service : {"s3", "b2", "azure", "gs", "swift"}) {
        const QByteArray radio = QByteArray("wizDestCloudService_") + service;
        CHECK(code, wait_until([&] { return count_visible(w, radio.constData()) == 1; }, 2000), radio.constData());
        ++code;
    }
    QQuickItem* b2 = lookup(w, "wizDestCloudService_b2");
    CHECK_EQ(5507, b2->property("text").toString(), QStringLiteral("Backblaze B2"), "B2 choice text");
    // quiet window: the choices appear below the card; the page knows its new height only after a layout pass.
    wait_ms(300);
    scroll_into_view(w, b2);
    CHECK(5508, click_ui_item(w, b2, "Backblaze B2"), "click on «Backblaze B2» failed");
    QQuickItem* next = lookup(w, "wizDestNextButton");
    CHECK(9051, wait_until([&] { return next && next->isEnabled(); }, 2000), "«Next» not enabled after choosing B2");
    CHECK(9052, click_ui_item(w, next, "Next"), "click on «Next» failed");
    CHECK(9053, wait_until([&] { return count_visible(w, "wizDestUrlField") == 1; }, 2000), "generic form did not open");
    wait_until([&] { return lookup(w, "wizDestUrlField")->property("text").toString() == QLatin1String("b2:"); }, 150);
    QQuickItem* url = lookup(w, "wizDestUrlField");
    CHECK_EQ(9054, url->property("text").toString(), QStringLiteral("b2:"), "scheme filled in");
    CHECK(9055, count_visible(w, "wizDestEndpointField") == 0, "S3 fields shown for B2");
    CHECK(9056, count_visible(w, "wizDestLocationField") == 0, "path field shown for B2");

    QQuickItem* test = lookup(w, "wizDestTestButton");
    QQuickItem* error = lookup(w, "wizDestGenericError");
    set_text(w, "wizDestNameField", QStringLiteral("Cloud"));
    // AC-M5-03-1 (c)
    set_text(w, "wizDestUrlField", QStringLiteral("ftp:host:/x"));
    CHECK(5520, wait_until([&] { return error && error->isVisible(); }, 2000), "AC-M5-03-1 (c): no message for an unknown scheme");
    CHECK_EQ(5521, error->property("text").toString(),
        loc(c.lang,
            "The repository URL must start with b2:, azure:, gs:, swift: or rclone:.",
            "Die Repository-URL muss mit b2:, azure:, gs:, swift: oder rclone: beginnen."),
        "AC-M5-03-1 (c): message");
    CHECK(5522, !test->isEnabled(), "AC-M5-03-1 (c): «Test Connection» enabled with an unknown scheme");
    set_text(w, "wizDestUrlField", QStringLiteral("b2:kestic-bucket:laptop"));
    CHECK(5523, wait_until([&] { return !error->isVisible(); }, 2000), "message stays for a known scheme");

    // AC-M5-03-4
    QQuickItem* key_secret = add_variable(w, 0, QStringLiteral("B2_ACCOUNT_KEY"), QStringLiteral("K0012abcdefg"));
    CHECK(5530, key_secret != nullptr, "variable row 0 missing");
    CHECK(5531, key_secret->property("checked").toBool(), "AC-M5-03-4 (a): «Geheim» off for B2_ACCOUNT_KEY");
    QQuickItem* key_value = lookup(w, env_name("Value", 0).constData());
    CHECK(5532, key_value->property("echoMode").toInt() == 2, "the secret value is not hidden");
    QQuickItem* host_secret = add_variable(w, 1, QStringLiteral("RESTIC_HOST"), QStringLiteral("laptop"));
    CHECK(5533, host_secret != nullptr, "variable row 1 missing");
    CHECK(5534, !host_secret->property("checked").toBool(), "AC-M5-03-4 (b): «Geheim» on for RESTIC_HOST");
    scroll_into_view(w, host_secret);
    CHECK(5535, click_ui_item(w, host_secret, "Secret"), "click on «Geheim» failed");
    CHECK(5536, wait_until([&] { return host_secret->property("checked").toBool(); }, 2000), "AC-M5-03-4 (c): «Geheim» not switchable");
    CHECK(5537, click_ui_item(w, host_secret, "Secret"), "second click on «Geheim» failed");
    CHECK(5538, wait_until([&] { return !host_secret->property("checked").toBool(); }, 2000), "AC-M5-03-4 (c): «Geheim» not switchable back");
    CHECK(5539, wait_until([&] { return test->isEnabled(); }, 2000), "«Test Connection» disabled with valid fields");
    // A value typed before the name: a secret name still switches «Geheim» on.
    QQuickItem* add = lookup(w, "wizDestEnvAdd");
    scroll_into_view(w, add);
    CHECK(5542, click_ui_item(w, add, "Add Variable"), "click on «Add Variable» failed");
    CHECK(5543, wait_until([&] { return lookup(w, env_name("Name", 2).constData()) != nullptr; }, 2000), "variable row 2 missing");
    set_text(w, env_name("Value", 2).constData(), QStringLiteral("t0ken-value"));
    set_text(w, env_name("Name", 2).constData(), QStringLiteral("MY_TOKEN"));
    CHECK(5544, lookup(w, env_name("Secret", 2).constData())->property("checked").toBool(), "AC-M5-03-4 (a): «Geheim» off for MY_TOKEN typed after its value");
    QQuickItem* remove_token = lookup(w, env_name("Remove", 2).constData());
    scroll_into_view(w, remove_token);
    CHECK(5545, click_ui_item(w, remove_token, "Remove Variable"), "click on «Remove Variable» failed");
    CHECK(5546, wait_until([&] { return lookup(w, env_name("Name", 2).constData()) == nullptr; }, 2000), "variable row 2 not removed");
    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestUrlField"),
        env_name("Name", 0),
        env_name("Value", 1),
        env_name("Secret", 0),
        env_name("Remove", 0),
        QStringLiteral("wizDestEnvAdd"),
    });
    CHECK_EQ(5540, unnamed, QString(), "control without accessible name");
    scroll_into_view(w, url);
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(300);
    CHECK(5541, save_screenshot(w, c.out_path), "screenshot failed");

    // AC-M5-03-7 (b)
    QQuickItem* env_error = lookup(w, "wizDestEnvError");
    QQuickItem* preload = add_variable(w, 2, QStringLiteral("LD_PRELOAD"), QStringLiteral("/tmp/x.so"));
    CHECK(5550, preload != nullptr, "variable row 2 missing");
    CHECK(5551, wait_until([&] { return env_error && env_error->isVisible(); }, 2000), "AC-M5-03-7 (b): no message for LD_PRELOAD");
    CHECK(5552, env_error->property("text").toString().contains(QStringLiteral("LD_PRELOAD")), "AC-M5-03-7 (b): the message names the variable");
    CHECK(5557, !error->isVisible(), "the variable's message stands at the URL");
    CHECK(5553, !test->isEnabled(), "AC-M5-03-7 (b): «Test Connection» enabled with a reserved variable");
    if (!c.out_path.isEmpty()) {
        scroll_into_view(w, env_error);
        // quiet window: the page draws its final state before the screenshot.
        wait_ms(300);
        QString reserved_shot = c.out_path;
        reserved_shot.replace(QStringLiteral("generic-form"), QStringLiteral("generic-reserved"));
        CHECK(5554, save_screenshot(w, reserved_shot), "screenshot failed");
    }
    QQuickItem* remove = lookup(w, env_name("Remove", 2).constData());
    scroll_into_view(w, remove);
    CHECK(5555, click_ui_item(w, remove, "Remove Variable"), "click on «Remove Variable» failed");
    CHECK(5556, wait_until([&] { return !env_error->isVisible() && test->isEnabled(); }, 2000), "removing the variable does not unlock the test");

    // AC-M5-03-8 (a): an `azure:` or `gs:` destination without its credential variable keeps
    // «Test Connection», and so adding it, locked, and the message names the variables.
    set_text(w, "wizDestUrlField", QStringLiteral("azure:kestic-container:/laptop"));
    CHECK(5590, wait_until([&] { return env_error->isVisible() && !test->isEnabled(); }, 2000), "AC-M5-03-8 (a): azure: without a credential variable accepted");
    CHECK_EQ(5591, env_error->property("text").toString(),
        loc(c.lang,
            "Enter one of these variables in this destination: AZURE_ACCOUNT_KEY, AZURE_ACCOUNT_SAS, AZURE_FORCE_CLI_CREDENTIAL=true. Kestic uses no other access data.",
            "Tragen Sie in diesem Ziel eine dieser Variablen ein: AZURE_ACCOUNT_KEY, AZURE_ACCOUNT_SAS, AZURE_FORCE_CLI_CREDENTIAL=true. Kestic verwendet keine anderen Zugangsdaten."),
        "AC-M5-03-8 (a): message for azure:");
    if (!c.out_path.isEmpty()) {
        scroll_into_view(w, env_error);
        // quiet window: the page draws its final state before the screenshot.
        wait_ms(300);
        QString credentials_shot = c.out_path;
        credentials_shot.replace(QStringLiteral("generic-form"), QStringLiteral("generic-credentials"));
        CHECK(5592, save_screenshot(w, credentials_shot), "screenshot failed");
    }
    set_text(w, "wizDestUrlField", QStringLiteral("gs:kestic-bucket:/laptop"));
    CHECK(5593, wait_until([&] { return env_error->property("text").toString().contains(QStringLiteral("GOOGLE_ACCESS_TOKEN, GOOGLE_APPLICATION_CREDENTIALS")); }, 2000), "AC-M5-03-8 (a): the message for gs: names its variables");
    CHECK(5594, !test->isEnabled(), "AC-M5-03-8 (a): gs: without a credential variable accepted");
    set_text(w, "wizDestUrlField", QStringLiteral("azure:kestic-container:/laptop"));
    QQuickItem* cli_secret = add_variable(w, 2, QStringLiteral("AZURE_FORCE_CLI_CREDENTIAL"), QStringLiteral("true"));
    CHECK(5595, cli_secret != nullptr, "variable row 2 missing");
    CHECK(5596, wait_until([&] { return !env_error->isVisible() && test->isEnabled(); }, 2000), "AC-M5-03-8 (a): AZURE_FORCE_CLI_CREDENTIAL=true does not unlock the test");

    // AC-M5-03-8 (a), editing: an empty secret row keeps a saved value only when the edited
    // destination has one saved under its name, not when the row was added while editing.
    QQuickItem* wizard = lookup(w, "addDestinationWizard");
    QObject* draft = wizard ? qvariant_cast<QObject*>(wizard->property("draft")) : nullptr;
    CHECK(5597, draft != nullptr, "wizard draft missing");
    draft->setProperty("destinationId", QStringLiteral("smoke-edited"));
    set_text(w, env_name("Name", 2).constData(), QStringLiteral("AZURE_ACCOUNT_KEY"));
    set_text(w, env_name("Value", 2).constData(), QString());
    CHECK(5598, wait_until([&] { return env_error->isVisible() && !test->isEnabled(); }, 2000),
        "AC-M5-03-8 (a): an empty secret row added while editing counts as saved");
    draft->setProperty("storedSecretNames", QVariantList{QStringLiteral("AZURE_ACCOUNT_KEY")});
    CHECK(5599, wait_until([&] { return !env_error->isVisible(); }, 2000),
        "AC-M5-03-8 (a): a saved AZURE_ACCOUNT_KEY left empty while editing does not count");
    draft->setProperty("destinationId", QString());
    return 0;
}

// M5-S03 (GUI): with `rclone` installed the rclone card is ready and opens the generic form that
// asks for the rclone remote only; the repository URL becomes `rclone:<remote>`.
int scenario_rclone_form(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (const int rc = open_details_of(w, "rclone")) {
        return rc;
    }
    QQuickItem* url = lookup(w, "wizDestUrlField");
    CHECK2(5560, url != nullptr, url->isVisible(), "rclone form did not open");
    CHECK_EQ(5561, url->property("text").toString(), QString(), "remote empty, without the scheme");
    CHECK_EQ(5562, url->property("placeholderText").toString(), QStringLiteral("myremote:backups/laptop"), "remote placeholder");
    set_text(w, "wizDestNameField", QStringLiteral("Drive"));
    set_text(w, "wizDestUrlField", QStringLiteral("gdrive:backup"));
    QQuickItem* error = lookup(w, "wizDestGenericError");
    QQuickItem* test = lookup(w, "wizDestTestButton");
    // `gdrive:` is no scheme of its own: the form accepts it only as `rclone:gdrive:backup`.
    CHECK(5563, wait_until([&] { return !error->isVisible() && test->isEnabled(); }, 2000), "the remote is not put after rclone:");
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(300);
    return shot(c, 5564);
}

// M5-S09 (GUI parts of AC-M5-05-7): the edit page of a local destination whose path is changed
// to a folder with another repository. The connection test says «There is another backup
// destination at this address.», «Save» stays disabled (a, b), and «Add as New Destination…»
// opens the wizard «Add Destination» at its details with the entered type, path and name, without
// the edited destination's ID or any password (c). Screenshots of the message and of the wizard.
int scenario_edit_other_repository(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString dest_id = qEnvironmentVariable("KESTIC_SMOKE_DEST_ID");
    const QString dest_name = qEnvironmentVariable("KESTIC_SMOKE_DEST_NAME");
    const QString other_path = qEnvironmentVariable("KESTIC_SMOKE_OTHER_PATH");
    CHECK(5700, !dest_id.isEmpty() && !dest_name.isEmpty() && !other_path.isEmpty(), "KESTIC_SMOKE_* not set");
    QMetaObject::invokeMethod(w, "openDestinationEditPage", Q_ARG(QVariant, dest_id));
    CHECK(9057, wait_until([&] { return count_visible(w, "destEditLocationField") == 1; }, 3000), "edit page did not open");
    wait_ms(150); // quiet window: the page ends its transition
    set_text(w, "destEditLocationField", other_path);
    QQuickItem* test = lookup(w, "destEditTestButton");
    CHECK(9058, wait_until([&] { return test && test->isEnabled(); }, 2000), "«Test Connection» not enabled");
    // quiet window: the page knows its height only after a layout pass.
    wait_ms(300);
    scroll_into_view_in(w, "destinationEditPage", test);
    CHECK(9059, click_ui_item(w, test, "Test Connection"), "click on «Test Connection» failed");
    QQuickItem* banner = lookup(w, "destEditFeedbackBanner");
    const QString message = loc(c.lang, "There is another backup destination at this address.",
        "Unter dieser Adresse liegt ein anderes Backup-Ziel.");
    CHECK(9060, wait_until([&] { return banner && banner->property("text").toString() == message; }, 15000),
        "AC-M5-05-7 (b): no message «another backup destination»");
    QObject* save = lookup<QObject>(w, "saveDestinationAction");
    CHECK(9061, save != nullptr && !save->property("enabled").toBool(), "AC-M5-05-7 (a): «Save» enabled");
    QQuickItem* add_as_new = lookup(w, "destEditAddAsNewButton");
    CHECK2(9062, add_as_new != nullptr, add_as_new->isVisible(),
        "AC-M5-05-7 (c): «Add as New Destination…» missing");
    CHECK_EQ(9063, add_as_new->property("text").toString(),
        loc(c.lang, "Add as New Destination…", "Als neues Ziel hinzufügen…"), "action text");
    // The action has its own row below the message, which keeps its whole width for the text
    // (once the «Testing the connection…» line has gone and the message has opened).
    CHECK(9064, wait_until([&] { return scene_y(add_as_new) >= scene_y(banner) + banner->height(); }, 2000),
        "the action is not below the message");
    scroll_into_view_in(w, "destinationEditPage", banner);
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(600);
    CHECK(9065, save_screenshot(w, c.out_path), "screenshot failed");

    CHECK(9066, click_ui_item(w, add_as_new, "Add as New Destination"), "click on «Add as New Destination…» failed");
    CHECK(9067, wait_until([&] { return count_visible(w, "wizDestLocationField") == 1; }, 3000),
        "AC-M5-05-7 (c): the wizard did not open at its details");
    QQuickItem* wizard = lookup(w, "addDestinationWizard");
    CHECK(9068, wizard != nullptr, "wizard missing");
    QObject* draft = qvariant_cast<QObject*>(wizard->property("draft"));
    CHECK(9069, draft != nullptr, "wizard draft missing");
    CHECK_EQ(9070, lookup(w, "wizDestLocationField")->property("text").toString(), other_path, "AC-M5-05-7 (c): entered path");
    CHECK_EQ(9071, lookup(w, "wizDestNameField")->property("text").toString(), dest_name, "AC-M5-05-7 (c): entered name");
    CHECK_EQ(9072, draft->property("kind").toString(), QStringLiteral("local"), "AC-M5-05-7 (c): destination type");
    CHECK_EQ(9073, draft->property("destinationId").toString(), QString(), "AC-M5-05-7 (c): no ID of the edited destination");
    CHECK_EQ(9074, draft->property("repoPassword").toString(), QString(), "AC-M5-05-7 (c): no password carried over");
    CHECK(9075, wizard->property("step").toInt() == 2, "AC-M5-05-7 (c): the wizard is not at its details");
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(600);
    QString wizard_shot = c.out_path;
    wizard_shot.replace(QStringLiteral("edit-other-repository"), QStringLiteral("edit-add-as-new"));
    CHECK(9076, save_screenshot(w, wizard_shot), "screenshot failed");
    return 0;
}

// The settings window, opened and scrolled to its restic section.
QQuickWindow* open_restic_settings(QQuickWindow* w) {
    QMetaObject::invokeMethod(w, "openSettingsWindow");
    auto* sw = w->findChild<QQuickWindow*>(QStringLiteral("settingsWindow"));
    if (!sw || !wait_until([&] { return sw->isVisible(); }, 3000)) {
        return nullptr;
    }
    // quiet window: the page knows its height only after a layout pass.
    wait_ms(300);
    QMetaObject::invokeMethod(sw, "showResticSection");
    // quiet window: the page lays out its new height.
    wait_ms(200);
    return sw;
}

QString restic_install_state(const Ctx& c) {
    return c.backend->property("restic_install_state").toString();
}

// Screenshot of the settings window, scrolled to its restic section.
bool save_settings_screenshot(QQuickWindow* sw, const QString& out_path) {
    // quiet window: the page knows its new height only after a layout pass.
    wait_ms(200);
    QMetaObject::invokeMethod(sw, "showResticSection");
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(400);
    return save_screenshot(sw, out_path);
}

// AC-M5-06-12 (b): with restic too old and no installation started, (b1) the blocking bar
// offers «Download restic…» and (b2) the settings' restic section offers «Download restic»;
// nothing is installing. Screenshots of the window and of the settings.
int scenario_restic_download_offer(const Ctx& c) {
    QQuickWindow* w = c.window;
    CHECK(5800, wait_until([&] { return c.backend->property("blocking_kind").toString() == QLatin1String("restic_unavailable"); }, 5000),
        "precondition: restic is not usable");
    QObject* action = lookup<QObject>(w, "downloadResticAction");
    CHECK(5801, action != nullptr, "(b1) «Download restic…» missing in the blocking bar");
    CHECK(5802, action->property("visible").toBool(), "(b1) «Download restic…» hidden");
    CHECK(5803, action->property("enabled").toBool(), "(b1) «Download restic…» disabled");
    CHECK_EQ(5804, action->property("text").toString(), loc(c.lang, "Download restic…", "restic herunterladen…"), "(b1) action text");
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(400);
    CHECK(5805, save_screenshot(w, c.out_path), "screenshot failed");

    QQuickWindow* sw = open_restic_settings(w);
    CHECK(5806, sw != nullptr, "settings window did not open");
    QQuickItem* download = lookup(sw, "downloadResticButton");
    CHECK(5807, download != nullptr, "(b2) «Download restic» missing");
    CHECK(5808, download->isVisible(), "(b2) «Download restic» hidden");
    CHECK(5809, download->isEnabled(), "(b2) «Download restic» disabled");
    CHECK_EQ(9077, download->property("text").toString(), loc(c.lang, "Download restic", "restic herunterladen"), "(b2) button text");
    QQuickItem* update = lookup(sw, "updateResticButton");
    CHECK(9078, update != nullptr && !update->isVisible(), "«Update Now» shown for a restic Kestic does not manage");
    CHECK_EQ(9079, restic_install_state(c), QString(), "an installation started before the click");
    CHECK(9080, count_visible(sw, "resticInstallPanel") == 0, "installation panel shown before the click");
    // The settings window has one size; only the wide run saves it.
    const QString settings_shot = qEnvironmentVariable("KESTIC_SMOKE_SETTINGS_SHOT");
    if (!settings_shot.isEmpty()) {
        CHECK(5814, save_settings_screenshot(sw, settings_shot), "screenshot failed");
    }
    return 0;
}

// AC-M5-06-4, -5, -6 (a), GUI: a click on the button `KESTIC_SMOKE_RESTIC_BUTTON` starts the
// installation, which fails; the section shows `KESTIC_SMOKE_EXPECT_ERROR`, the raw detail as
// plain text under «Details», and offers the button again.
int scenario_restic_install_error(const Ctx& c) {
    const QByteArray button_name = qgetenv("KESTIC_SMOKE_RESTIC_BUTTON");
    const QString expected = qEnvironmentVariable("KESTIC_SMOKE_EXPECT_ERROR");
    CHECK(5820, !button_name.isEmpty() && !expected.isEmpty(), "KESTIC_SMOKE_* not set");
    QQuickWindow* sw = open_restic_settings(c.window);
    CHECK(5821, sw != nullptr, "settings window did not open");
    QQuickItem* button = lookup(sw, button_name.constData());
    CHECK(5822, button != nullptr, "restic button missing");
    CHECK(5823, click_ui_item(sw, button, "restic button"), "click on the restic button failed");
    CHECK(5824, wait_until([&] { return restic_install_state(c) == QLatin1String("error"); }, 60000),
        "the installation did not end with an error");
    QQuickItem* message = lookup(sw, "resticInstallErrorMessage");
    CHECK(5825, message != nullptr && message->isVisible(), "(a) error message hidden");
    CHECK_EQ(5826, message->property("text").toString(), expected, "(a) error message");
    QObject* details = lookup<QObject>(sw, "resticInstallDetailsAction");
    CHECK(5827, details != nullptr && details->property("visible").toBool(), "«Details» missing");
    trigger(details);
    QQuickItem* detail = lookup(sw, "resticInstallErrorDetail");
    CHECK(5828, wait_until([&] { return detail && detail->isVisible(); }, 2000), "raw detail hidden after «Details»");
    CHECK(5829, detail->property("textFormat").toInt() == Qt::PlainText, "raw detail not shown as plain text");
    CHECK(5830, !detail->property("text").toString().isEmpty(), "raw detail empty");
    CHECK(5831, wait_until([&] { return button->isVisible() && button->isEnabled(); }, 2000), "the button is not offered again");
    CHECK(5832, save_settings_screenshot(sw, c.out_path), "screenshot failed");
    return 0;
}

// A stalled download: the section shows the step «Downloading restic…», a progress bar and
// «Cancel»; after «Cancel» the daemon reports the installation cancelled, the progress is gone
// and «Download restic» is offered again.
int scenario_restic_install_cancel(const Ctx& c) {
    QQuickWindow* sw = open_restic_settings(c.window);
    CHECK(5840, sw != nullptr, "settings window did not open");
    QQuickItem* download = lookup(sw, "downloadResticButton");
    CHECK(5841, click_ui_item(sw, download, "Download restic"), "click on «Download restic» failed");
    QQuickItem* phase = lookup(sw, "resticInstallPhaseLabel");
    const QString downloading = loc(c.lang, "Downloading restic…", "restic wird heruntergeladen…");
    CHECK(5842, wait_until([&] { return phase && phase->isVisible() && phase->property("text").toString() == downloading; }, 30000),
        "the step «Downloading restic…» is not shown");
    QQuickItem* bar = lookup(sw, "resticInstallProgressBar");
    CHECK(5843, bar != nullptr && bar->isVisible(), "progress bar hidden");
    auto* bar_track = qvariant_cast<QQuickItem*>(bar->property("background"));
    CHECK(5851, bar_track != nullptr && bar_track->height() > 0 && bar_track->width() > 0, "progress bar drawn without height");
    QQuickItem* cancel = lookup(sw, "cancelResticInstallButton");
    CHECK(5844, cancel != nullptr && cancel->isVisible() && cancel->isEnabled(), "«Cancel» not offered while downloading");
    CHECK(5845, !download->isEnabled(), "«Download restic» enabled while installing");
    CHECK(5846, save_settings_screenshot(sw, c.out_path), "screenshot failed");
    CHECK(5847, click_ui_item(sw, cancel, "Cancel"), "click on «Cancel» failed");
    CHECK(5848, wait_until([&] { return restic_install_state(c) == QLatin1String("cancelled"); }, 5000),
        "the daemon did not report the installation cancelled");
    CHECK(5849, count_visible(sw, "resticInstallPanel") == 0, "installation panel still shown after the cancel");
    CHECK(5850, wait_until([&] { return download->isVisible() && download->isEnabled(); }, 2000), "«Download restic» not offered again");
    return 0;
}

// The blocking bar's «Download restic…» opens the settings at «Download restic» without
// starting anything; a click there starts the installation. The settings window is hidden at
// once and the installation still finishes: the blocking bar disappears, and the reopened
// settings show «restic <KESTIC_SMOKE_RELEASE> is installed.».
int scenario_restic_install_result(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString release = qEnvironmentVariable("KESTIC_SMOKE_RELEASE");
    CHECK(5860, !release.isEmpty(), "KESTIC_SMOKE_RELEASE not set");
    CHECK(5861, wait_until([&] { return c.backend->property("blocking_kind").toString() == QLatin1String("restic_unavailable"); }, 5000),
        "precondition: restic is not usable");
    QObject* action = lookup<QObject>(w, "downloadResticAction");
    CHECK(5862, action != nullptr && action->property("enabled").toBool(), "«Download restic…» not offered");
    trigger(action);
    auto* sw = w->findChild<QQuickWindow*>(QStringLiteral("settingsWindow"));
    CHECK(5863, sw != nullptr && wait_until([&] { return sw->isVisible(); }, 3000), "settings window did not open");
    QQuickItem* download = lookup(sw, "downloadResticButton");
    CHECK(5873, download != nullptr && wait_until([&] { return download->hasActiveFocus(); }, 2000),
        "«Download restic» has no focus");
    CHECK(5876, wait_until([&] { return download->mapToScene(QPointF(0, download->height())).y() <= sw->height(); }, 2000),
        "the settings did not scroll to «Download restic»");
    wait_ms(300); // quiet window: an installation that started before the click would show by now
    CHECK_EQ(5874, restic_install_state(c), QString(), "the installation started before the click in the settings");
    CHECK(5875, click_ui_item(sw, download, "Download restic"), "click on «Download restic» failed");
    CHECK(5864, wait_until([&] { return !restic_install_state(c).isEmpty(); }, 5000), "the installation did not start");
    sw->setVisible(false);
    CHECK(5865, wait_until([&] { return restic_install_state(c) == QLatin1String("done"); }, 60000),
        "the installation did not finish while the settings were closed");
    CHECK(5866, wait_until([&] { return c.backend->property("blocking_kind").toString().isEmpty(); }, 10000),
        "the blocking bar stayed after the installation");
    QQuickWindow* reopened = open_restic_settings(w);
    CHECK(5867, reopened == sw, "settings window did not open again");
    QQuickItem* result = lookup(sw, "resticInstallResultMessage");
    CHECK(5868, result != nullptr && result->isVisible(), "result message hidden");
    CHECK_EQ(5869, result->property("text").toString(),
        loc(c.lang, "restic %1 is installed.", "restic %1 ist installiert.").arg(release), "result message");
    CHECK(5870, count_visible(sw, "downloadResticButton") == 0, "«Download restic» still offered");
    CHECK(5871, count_visible(sw, "updateResticButton") == 1, "«Update Now» not offered for the installed restic");
    CHECK(5872, save_settings_screenshot(sw, c.out_path), "screenshot failed");
    return 0;
}


// The text of the label called `name` in the settings window `sw` ("" when it is hidden).
QString shown_text(QQuickWindow* sw, const char* name) {
    QQuickItem* item = lookup(sw, name);
    return item && item->isVisible() ? item->property("text").toString() : QString();
}

// Whether the inline message called `name` is shown.
bool message_shown(QQuickWindow* sw, const char* name) {
    QQuickItem* item = lookup(sw, name);
    return item && item->isVisible();
}

// AC-M5-07-3: «Automatic», system restic 0.16.4, no app-managed restic: the restic section
// (a) explains why 0.18.0 is needed and names 0.16.4 as too old, and (b) offers «Download
// restic»; nothing is installing.
int scenario_restic_too_old(const Ctx& c) {
    QQuickWindow* sw = open_restic_settings(c.window);
    CHECK(5900, sw != nullptr, "settings window did not open");
    CHECK(5901, wait_until([&] { return !shown_text(sw, "resticVersionLabel").isEmpty(); }, 5000), "no version row");
    // (a)
    CHECK_EQ(5902, shown_text(sw, "resticMinimumLabel"),
        loc(c.lang, "Kestic needs restic 0.18.0 or newer: older versions do not report errors and check results in a form Kestic can read.",
            "Kestic benötigt restic 0.18.0 oder neuer: Ältere Versionen melden Fehler und Prüfergebnisse nicht in einer Form, die Kestic auswerten kann."),
        "(a) the sentence why 0.18.0 is needed");
    CHECK_EQ(5903, shown_text(sw, "resticVersionLabel"),
        loc(c.lang, "0.16.4 · too old, at least 0.18.0", "0.16.4 · zu alt, mindestens 0.18.0"), "(a) version row");
    CHECK_EQ(5904, shown_text(sw, "resticSourceLabel"), loc(c.lang, "Automatic (System)", "Automatisch (System)"), "source row");
    // (b)
    QQuickItem* download = lookup(sw, "downloadResticButton");
    CHECK(5905, download != nullptr && download->isVisible() && download->isEnabled(), "(b) «Download restic» not offered");
    CHECK_EQ(9081, restic_install_state(c), QString(), "an installation started before the click");
    CHECK(9082, !message_shown(sw, "resticSwitchMessage") && !message_shown(sw, "resticRemoveMessage"),
        "switch or removal offered without an app-managed restic");
    CHECK(9083, save_settings_screenshot(sw, c.out_path), "screenshot failed");
    return 0;
}

// AC-M5-07-4: system restic 0.18.1 at KESTIC_SMOKE_SYSTEM_RESTIC and app-managed 0.19.1 under
// «Automatic»: (a) both versions in «Available:», (b) source, version and program each in a row
// of their own, (c) «recommended: 0.20.0» behind the version. AC-M5-07-5 (d): no switch hint,
// only the offer to delete. The source dialog has exactly one choice, «Automatic», and a new
// status does not reset a choice made in it.
int scenario_restic_versions(const Ctx& c) {
    const QString system = qEnvironmentVariable("KESTIC_SMOKE_SYSTEM_RESTIC");
    CHECK(9084, !system.isEmpty(), "KESTIC_SMOKE_SYSTEM_RESTIC not set");
    QQuickWindow* sw = open_restic_settings(c.window);
    CHECK(9085, sw != nullptr, "settings window did not open");
    CHECK(5912, wait_until([&] { return !shown_text(sw, "resticAvailableLabel").isEmpty(); }, 5000), "no row «Available:»");
    // (a)
    CHECK_EQ(5913, shown_text(sw, "resticAvailableLabel"),
        loc(c.lang, "System 0.18.1 · Managed by Kestic 0.19.1", "System 0.18.1 · Von Kestic verwaltet 0.19.1"), "(a) both versions");
    // (b) one row each, in this order
    CHECK_EQ(5914, shown_text(sw, "resticSourceLabel"), loc(c.lang, "Automatic (System)", "Automatisch (System)"), "(b) source row");
    CHECK_EQ(5915, shown_text(sw, "resticProgramLabel"), system, "(b) program row");
    QQuickItem* source = lookup(sw, "resticSourceLabel");
    QQuickItem* version = lookup(sw, "resticVersionLabel");
    QQuickItem* program = lookup(sw, "resticProgramLabel");
    QQuickItem* available = lookup(sw, "resticAvailableLabel");
    CHECK(5916, scene_y(source) < scene_y(version) && scene_y(version) < scene_y(program)
            && scene_y(program) < scene_y(available),
        "(b) source, version, program and available are not rows of their own");
    CHECK(5917, program->property("textFormat").toInt() == Qt::PlainText, "program path not shown as plain text");
    // (c)
    CHECK_EQ(5918, shown_text(sw, "resticVersionLabel"),
        loc(c.lang, "0.18.1 · recommended: 0.20.0", "0.18.1 · empfohlen: 0.20.0"), "(c) version row");
    CHECK(5919, shown_text(sw, "resticMinimumLabel").isEmpty(), "the sentence why 0.18.0 is needed without a too old version");
    // AC-M5-07-5 (d)
    CHECK(5920, !message_shown(sw, "resticSwitchMessage"), "AC-M5-07-5 (d): switch hint with «Automatic»");
    CHECK(5921, message_shown(sw, "resticRemoveMessage"), "AC-M5-07-5 (d): no offer to delete the app-managed restic");
    CHECK(5922, save_settings_screenshot(sw, c.out_path), "screenshot failed");

    QQuickItem* change = lookup(sw, "changeResticSourceButton");
    CHECK(5923, click_ui_item(sw, change, "Change restic Source"), "click on «Change restic Source…» failed");
    QQuickItem* automatic = lookup(sw, "resticSourceAutomaticRadio");
    CHECK(5924, wait_until([&] { return automatic && automatic->isVisible(); }, 3000), "source dialog did not open");
    const char* radios[] = {"resticSourceAutomaticRadio", "resticSourceSystemRadio", "resticSourceAppManagedRadio", "resticSourceCustomRadio"};
    const char* texts_en[] = {"Automatic", "System", "Managed by Kestic", "Custom path"};
    const char* texts_de[] = {"Automatisch", "System", "Von Kestic verwaltet", "Eigener Pfad"};
    int checked = 0;
    for (int i = 0; i < 4; ++i) {
        QQuickItem* radio = lookup(sw, radios[i]);
        CHECK(5925, radio != nullptr && radio->isVisible(), "a source is missing in the dialog");
        CHECK_EQ(5926, radio->property("text").toString(), loc(c.lang, texts_en[i], texts_de[i]), "source name");
        checked += radio->property("checked").toBool() ? 1 : 0;
    }
    CHECK(5927, checked == 1 && automatic->property("checked").toBool(), "not exactly «Automatic» checked");
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(400);
    QString dialog_shot = c.out_path;
    dialog_shot.replace(QStringLiteral("restic-versions"), QStringLiteral("restic-source-dialog"));
    CHECK(5928, save_screenshot(sw, dialog_shot), "screenshot failed");
    QQuickItem* custom = lookup(sw, "resticSourceCustomRadio");
    CHECK(5929, click_ui_item(sw, custom, "Custom path"), "click on «Custom path» failed");
    set_text(sw, "customResticPathField", QStringLiteral("/opt/restic/restic"));
    QMetaObject::invokeMethod(c.backend, "checkAvailability", Q_ARG(bool, true));
    // The check and the new state arrive off the main thread; this check shows that nothing
    // quiet window: changes, so it waits for them a fixed time.
    wait_ms(2000);
    CHECK(5930, custom->property("checked").toBool() && !automatic->property("checked").toBool(),
        "a new status reset the choice in the source dialog");
    CHECK_EQ(5931, lookup(sw, "customResticPathField")->property("text").toString(), QStringLiteral("/opt/restic/restic"),
        "a new status reset the entered path");
    QMetaObject::invokeMethod(sw, "handleEscape");
    CHECK(5932, wait_until([&] { return !automatic->isVisible(); }, 3000), "source dialog did not close");
    CHECK_EQ(5933, c.backend->property("restic_provider_kind").toString(), QStringLiteral("automatic"),
        "the source changed without «Apply»");
    return 0;
}

// AC-M5-07-5: «Managed by Kestic» and a system restic at KESTIC_SMOKE_SYSTEM_RESTIC that is
// recent enough now: (a) the hint «System restic is now recent enough – switch?»; (b) its action
// «Automatic» switches to the system restic; (c) then deleting the app-managed restic is offered
// and (d) no hint is shown. With KESTIC_SMOKE_EXPECT_REMOVE_FAILURE=1 the deletion fails: the
// reason is shown and the offer stays; otherwise the offer disappears.
int scenario_restic_switch(const Ctx& c) {
    const QString system = qEnvironmentVariable("KESTIC_SMOKE_SYSTEM_RESTIC");
    CHECK(5940, !system.isEmpty(), "KESTIC_SMOKE_SYSTEM_RESTIC not set");
    const bool expect_failure = qEnvironmentVariable("KESTIC_SMOKE_EXPECT_REMOVE_FAILURE") == QLatin1String("1");
    QQuickWindow* sw = open_restic_settings(c.window);
    CHECK(5941, sw != nullptr, "settings window did not open");
    // (a)
    CHECK(5942, wait_until([&] { return message_shown(sw, "resticSwitchMessage"); }, 5000), "(a) no switch hint");
    CHECK_EQ(5943, lookup(sw, "resticSwitchMessage")->property("text").toString(),
        loc(c.lang, "System restic is now recent enough – switch?", "System-restic ist jetzt aktuell genug – umstellen?"), "(a) hint");
    QObject* to_automatic = lookup<QObject>(sw, "switchResticToAutomaticAction");
    CHECK(5944, to_automatic != nullptr && to_automatic->property("enabled").toBool(), "(b) action missing");
    CHECK_EQ(5945, to_automatic->property("text").toString(), loc(c.lang, "Automatic", "Automatisch"), "(b) action text");
    CHECK_EQ(5946, shown_text(sw, "resticSourceLabel"), loc(c.lang, "Managed by Kestic", "Von Kestic verwaltet"), "source row");
    CHECK(5947, !message_shown(sw, "resticRemoveMessage"), "deletion offered while the app-managed restic is in use");
    CHECK(5948, save_settings_screenshot(sw, c.out_path), "screenshot failed");
    // (b)
    trigger(to_automatic);
    CHECK(5949, wait_until([&] { return c.backend->property("restic_provider_kind").toString() == QLatin1String("automatic"); }, 5000),
        "(b) the source did not switch to «Automatic»");
    CHECK_EQ(5950, shown_text(sw, "resticProgramLabel"), system, "(b) the system restic is not the one in use");
    CHECK_EQ(5951, shown_text(sw, "resticSourceLabel"), loc(c.lang, "Automatic (System)", "Automatisch (System)"), "(b) source row");
    QQuickItem* change = lookup(sw, "changeResticSourceButton");
    CHECK(5952, change != nullptr && change->hasActiveFocus(), "the focus is not on «Change restic Source…» after the switch");
    // (d), (c)
    CHECK(5953, wait_until([&] { return !message_shown(sw, "resticSwitchMessage"); }, 3000), "(d) hint after the switch");
    CHECK(5954, message_shown(sw, "resticRemoveMessage"), "(c) no offer to delete the app-managed restic");
    CHECK_EQ(5955, lookup(sw, "resticRemoveMessage")->property("text").toString(),
        loc(c.lang, "The restic managed by Kestic is no longer used.", "Das von Kestic verwaltete restic wird nicht mehr verwendet."),
        "(c) offer text");
    QObject* remove = lookup<QObject>(sw, "removeAppManagedResticAction");
    CHECK(5956, remove != nullptr && remove->property("enabled").toBool(), "(c) «Delete» missing");
    CHECK_EQ(5957, remove->property("text").toString(), loc(c.lang, "Delete", "Löschen"), "(c) action text");
    QString offer_shot = c.out_path;
    offer_shot.replace(QStringLiteral("restic-switch"), QStringLiteral("restic-remove-offer"));
    CHECK(5958, save_settings_screenshot(sw, offer_shot), "screenshot failed");
    trigger(remove);
    if (expect_failure) {
        CHECK(5959, wait_until([&] { return !shown_text(sw, "resticActionError").isEmpty(); }, 5000),
            "a failed deletion shows no reason");
        CHECK(5960, message_shown(sw, "resticRemoveMessage"), "the offer disappeared after a failed deletion");
        CHECK(5961, !shown_text(sw, "resticAvailableLabel").isEmpty(), "the app-managed restic is no longer listed");
        QString failed_shot = c.out_path;
        failed_shot.replace(QStringLiteral("restic-switch"), QStringLiteral("restic-remove-failed"));
        CHECK(5962, save_settings_screenshot(sw, failed_shot), "screenshot failed");
    } else {
        CHECK(5963, wait_until([&] { return !message_shown(sw, "resticRemoveMessage"); }, 5000), "(c) the offer stayed after deleting");
        CHECK(5964, shown_text(sw, "resticAvailableLabel").isEmpty(), "(c) the app-managed restic is still listed");
        CHECK(5965, shown_text(sw, "resticActionError").isEmpty(), "(c) an error after deleting");
    }
    CHECK(5966, change->hasActiveFocus(), "the focus is not on «Change restic Source…» after «Delete»");
    return 0;
}


// «Managed by Kestic» chosen in the source dialog while it is not installed: the dialog says that
// «Apply» downloads it first; «Apply» starts the installation, and once it finished the source
// is «Managed by Kestic».
int scenario_restic_source_download(const Ctx& c) {
    QQuickWindow* sw = open_restic_settings(c.window);
    CHECK(5970, sw != nullptr, "settings window did not open");
    QQuickItem* change = lookup(sw, "changeResticSourceButton");
    CHECK(5971, click_ui_item(sw, change, "Change restic Source"), "click on «Change restic Source…» failed");
    QQuickItem* app_managed = lookup(sw, "resticSourceAppManagedRadio");
    CHECK(5972, wait_until([&] { return app_managed && app_managed->isVisible(); }, 3000), "source dialog did not open");
    CHECK(5973, click_ui_item(sw, app_managed, "Managed by Kestic"), "click on «Managed by Kestic» failed");
    CHECK_EQ(5974, shown_text(sw, "resticSourceAppManagedHint"),
        loc(c.lang, "Not installed yet: Apply downloads and checks the official restic release first.",
            "Noch nicht installiert: «Übernehmen» lädt zuerst das offizielle restic-Release herunter und prüft es."),
        "hint for a restic that is not installed");
    // quiet window: the page draws its final state before the screenshot.
    wait_ms(300);
    CHECK(5975, save_screenshot(sw, c.out_path), "screenshot failed");
    CHECK_EQ(5976, restic_install_state(c), QString(), "an installation started before «Apply»");
    QObject* apply = lookup<QObject>(sw, "applyResticSourceAction");
    CHECK(5977, apply != nullptr && apply->property("enabled").toBool(), "«Apply» not enabled");
    trigger(apply);
    CHECK(5978, wait_until([&] { return !restic_install_state(c).isEmpty(); }, 5000), "«Apply» did not start the installation");
    CHECK(5979, wait_until([&] { return restic_install_state(c) == QLatin1String("done"); }, 60000),
        "the installation did not finish");
    CHECK(5980, wait_until([&] { return c.backend->property("restic_provider_kind").toString() == QLatin1String("app_managed"); }, 5000),
        "the source did not switch to «Managed by Kestic» after the installation");
    CHECK_EQ(5981, shown_text(sw, "resticSourceLabel"), loc(c.lang, "Managed by Kestic", "Von Kestic verwaltet"), "source row");
    return 0;
}


// A custom path at KESTIC_SMOKE_CUSTOM_PATH that does not exist: the section names the reason in
// the GUI language with the path as plain text, and shows the raw reason under «Details».
int scenario_restic_path_problem(const Ctx& c) {
    const QString path = qEnvironmentVariable("KESTIC_SMOKE_CUSTOM_PATH");
    CHECK(5990, !path.isEmpty(), "KESTIC_SMOKE_CUSTOM_PATH not set");
    QQuickWindow* sw = open_restic_settings(c.window);
    CHECK(5991, sw != nullptr, "settings window did not open");
    QQuickItem* message = lookup(sw, "resticProblemMessage");
    CHECK(5992, wait_until([&] { return message && message->isVisible(); }, 5000), "no reason shown");
    CHECK_EQ(5993, message->property("text").toString(),
        loc(c.lang, "Custom restic path is invalid: %1 does not exist", "Eigener restic-Pfad ist ungültig: %1 existiert nicht").arg(path),
        "translated reason");
    CHECK(5994, message->property("textFormat").toInt() == Qt::PlainText, "reason not shown as plain text");
    CHECK_EQ(5995, shown_text(sw, "resticSourceLabel"), loc(c.lang, "Custom path", "Eigener Pfad"), "source row");
    QQuickItem* details = lookup(sw, "resticProblemDetailsButton");
    CHECK(5996, details != nullptr && details->isVisible(), "«Details» missing");
    CHECK(5997, click_ui_item(sw, details, "Details"), "click on «Details» failed");
    QQuickItem* detail = lookup(sw, "resticProblemDetail");
    CHECK(5998, wait_until([&] { return detail && detail->isVisible(); }, 2000), "raw reason hidden after «Details»");
    CHECK(5999, detail->property("textFormat").toInt() == Qt::PlainText && !detail->property("text").toString().isEmpty(),
        "raw reason empty or not plain text");
    CHECK(5989, save_settings_screenshot(sw, c.out_path), "screenshot failed");
    return 0;
}

const Scenario kScenarios[] = {
    {"m5-destination-types", scenario_destination_types},
    {"m5-destination-types-check-again", scenario_check_again},
    {"m5-drive-list", scenario_drive_list},
    {"m5-drive-list-keys", scenario_drive_list_keys},
    {"m5-cifs-note", scenario_cifs_note},
    {"m5-s3-form", scenario_s3_form},
    {"m5-generic-form", scenario_generic_form},
    {"m5-rclone-form", scenario_rclone_form},
    {"m5-sftp-form", scenario_sftp_form},
    {"m5-edit-other-repository", scenario_edit_other_repository},
    {"m5-restic-download-offer", scenario_restic_download_offer},
    {"m5-restic-install-error", scenario_restic_install_error},
    {"m5-restic-install-cancel", scenario_restic_install_cancel},
    {"m5-restic-install-result", scenario_restic_install_result},
    {"m5-restic-too-old", scenario_restic_too_old},
    {"m5-restic-versions", scenario_restic_versions},
    {"m5-restic-switch", scenario_restic_switch},
    {"m5-restic-source-download", scenario_restic_source_download},
    {"m5-restic-path-problem", scenario_restic_path_problem},
};

} // namespace

void run_m5_smoke_scenario(
    QQmlApplicationEngine& engine,
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& scen,
    const QString& out_path,
    const QString& lang
) {
    (void)engine;
    const Ctx ctx { window, backend, out_path, lang };
    const ScenarioFn run = find_scenario(kScenarios, scen);
    exit_scenario("M5", scen, run ? run(ctx) : -1, 599, *warnings);
}

} // namespace kestic::smoke
