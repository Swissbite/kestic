// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Offscreen GUI smoke scenarios for Milestone M5. Each scenario returns 0 on success and a unique
// non-zero code on the first failed check.

#include "src/smoke_m3_util.h"

#include <QFile>
#include <algorithm>
#include <QPointF>

namespace kestic::smoke {

namespace {

using namespace m3;

QByteArray card_name(const char* card) {
    return QByteArray("wizDestTypeCard_") + card;
}

QQuickItem* card_item(QQuickWindow* w, const char* card) {
    return item_named(w, card_name(card).constData());
}

double scene_y(QQuickItem* item) {
    return item->mapToScene(QPointF(0, 0)).y();
}

// Scrolls the wizard page so that `item` is in view (the narrow window does not show all cards).
void scroll_into_view(QQuickWindow* w, QQuickItem* item) {
    QQuickItem* page = item_named(w, "addDestinationWizard");
    auto* flick = page ? qvariant_cast<QQuickItem*>(page->property("flickable")) : nullptr;
    auto* content = flick ? qvariant_cast<QQuickItem*>(flick->property("contentItem")) : nullptr;
    if (!content) {
        return;
    }
    const double top = item->mapToItem(content, QPointF(0, 0)).y();
    const double max_y = std::max(0.0, flick->property("contentHeight").toDouble() - flick->height());
    flick->setProperty("contentY", std::clamp(top - 20.0, 0.0, max_y));
    wait_ms(100);
}

// Opens «Add Destination» on its first step, the destination type.
int open_destination_types(QQuickWindow* w) {
    QMetaObject::invokeMethod(w, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QStringLiteral("init")));
    M3_CHECK(5001, wait_until([&] { return item_named(w, "addDestinationWizard") != nullptr; }, 3000), "addDestinationWizard did not open");
    M3_CHECK(5002, wait_until([&] { return card_item(w, "drive") != nullptr; }, 3000), "destination type cards missing");
    wait_ms(250);
    finish_incubation(w);
    return 0;
}

// Clicks the rclone entry and waits for its install panel.
int open_rclone_entry(QQuickWindow* w) {
    QQuickItem* rclone = card_item(w, "rclone");
    M3_CHECK(5010, rclone != nullptr, "rclone card missing");
    scroll_into_view(w, rclone);
    M3_CHECK(5011, click_ui_item(w, rclone, "rclone card"), "click on the rclone card failed");
    M3_CHECK(5012, wait_until([&] { return count_visible(w, "wizDestInstallPanel") == 1; }, 2000), "install panel did not open");
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

    QQuickItem* ready_header = item_named(w, "wizDestReadySection");
    QQuickItem* needs_header = item_named(w, "wizDestNeedsInstallationSection");
    M3_CHECK2(5020, ready_header != nullptr, ready_header->isVisible(), "«Ready» section missing");
    M3_CHECK2(5021, needs_header != nullptr, needs_header->isVisible(), "«Needs Installation» section missing");
    M3_CHECK_EQ(5022, ready_header->property("text").toString(), loc(c.lang, "Ready", "Bereit"), "ready section title");
    M3_CHECK_EQ(5023, needs_header->property("text").toString(), loc(c.lang, "Needs Installation", "Braucht Installation"), "needs installation section title");

    QQuickItem* rclone = card_item(w, "rclone");
    M3_CHECK2(5024, rclone != nullptr, rclone->isVisible(), "rclone card missing");
    // (b) rclone sits under «Needs Installation».
    M3_CHECK(5025, scene_y(rclone) > scene_y(needs_header), "AC-M5-04-1 (b): rclone card is not under «Needs Installation»");
    M3_CHECK(5026, !rclone->property("checkable").toBool(), "AC-M5-04-1 (b): rclone card is a ready card");
    int code = 5030;
    for (const char* ready : {"drive", "folder", "sftp", "rest", "cloud"}) {
        QQuickItem* card = card_item(w, ready);
        M3_CHECK2(code, card != nullptr, card->isVisible(), card_name(ready).constData());
        // (a) under «Ready».
        M3_CHECK(code + 1, scene_y(card) > scene_y(ready_header), "AC-M5-04-1 (a): ready card above the «Ready» section");
        M3_CHECK(code + 2, card->property("checkable").toBool(), "AC-M5-04-1 (a): card is not a ready card");
        // (c) before every card that needs installation.
        M3_CHECK(code + 3, scene_y(card) < scene_y(needs_header), "AC-M5-04-1 (c): ready card after «Needs Installation»");
        code += 4;
    }
    M3_CHECK_EQ(5060, card_item(w, "sftp")->property("text").toString(), loc(c.lang, "NAS / Server via SSH", "NAS / Server per SSH"), "sftp card title");
    M3_CHECK_EQ(5061, rclone->property("text").toString(), loc(c.lang, "Cloud Services via rclone", "Cloud-Dienste über rclone"), "rclone card title");

    if (const int rc = open_rclone_entry(w)) {
        return rc;
    }
    QQuickItem* package = item_named(w, "wizDestInstallPackageLabel");
    QQuickItem* command = item_named(w, "wizDestInstallCommandField");
    // AC-M5-04-3 (a)
    M3_CHECK2(5070, package != nullptr, package->isVisible(), "AC-M5-04-3 (a): package label missing");
    M3_CHECK_EQ(5071, package->property("text").toString(), loc(c.lang, "Package: rclone", "Paket: rclone"), "AC-M5-04-3 (a): package");
    // AC-M5-04-3 (b)
    // The same lookup finds the button «Check Again» of the panel, so it would find a Discover one.
    M3_CHECK(5073, visible_button(w->contentItem(), "Check Again|Erneut prüfen") != nullptr, "button lookup finds «Check Again»");
    M3_CHECK(5072, visible_button(w->contentItem(), "Open in Discover|In Discover öffnen") == nullptr,
        "AC-M5-04-3 (b): rclone has no AppStream component, but «Open in Discover» is shown");
    // AC-M5-04-3 (c)
    M3_CHECK2(5075, command != nullptr, command->isVisible(), "AC-M5-04-3 (c): install command missing");
    M3_CHECK_EQ(5076, command->property("text").toString(), QStringLiteral("sudo apt install rclone"), "AC-M5-04-3 (c): command");
    QQuickItem* again = item_named(w, "wizDestCheckAgainButton");
    M3_CHECK2(5077, again != nullptr, again->isVisible(), "«Check Again» missing");

    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestTypeCard_folder"),
        QStringLiteral("wizDestTypeCard_rclone"),
        QStringLiteral("wizDestInstallCommandField"),
        QStringLiteral("wizDestCopyInstallCommandButton"),
        QStringLiteral("wizDestCheckAgainButton"),
    });
    M3_CHECK_EQ(5079, unnamed, QString(), "control without accessible name");
    M3_CHECK_EQ(5080, accessible_name(item_named(w, "wizDestCopyInstallCommandButton")), loc(c.lang, "Copy", "Kopieren"), "accessible name of «Copy»");
    M3_CHECK(5078, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// AC-M5-04-4 (GUI): rclone is put into the daemon's PATH while its entry is open; «Check Again»
// moves it under «Ready».
int scenario_check_again(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* rclone_card = card_item(w, "rclone");
    M3_CHECK2(5100, rclone_card != nullptr, !rclone_card->property("checkable").toBool(), "precondition: rclone needs installation");
    if (const int rc = open_rclone_entry(w)) {
        return rc;
    }

    const QString dir = qEnvironmentVariable("KESTIC_SMOKE_PROGRAM_DIR");
    M3_CHECK(5101, !dir.isEmpty(), "KESTIC_SMOKE_PROGRAM_DIR not set");
    QFile rclone_file(dir + QStringLiteral("/rclone"));
    M3_CHECK(5102, rclone_file.open(QIODevice::WriteOnly), "cannot write the fake rclone");
    rclone_file.write("#!/bin/sh\nexit 0\n");
    rclone_file.close();
    M3_CHECK(5103, rclone_file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner), "cannot make the fake rclone executable");

    QQuickItem* again = item_named(w, "wizDestCheckAgainButton");
    M3_CHECK(5110, again != nullptr, "«Check Again» missing");
    scroll_into_view(w, again);
    M3_CHECK(5104, click_ui_item(w, again, "Check Again"), "click on «Check Again» failed");
    // (a) rclone is ready now: a ready card, below «Ready», and no section «Needs Installation».
    M3_CHECK(5105, wait_until([&] {
        QQuickItem* card = card_item(w, "rclone");
        return card && card->property("checkable").toBool();
    }, 3000), "AC-M5-04-4 (a): rclone did not become ready");
    wait_ms(200);
    QQuickItem* needs_header = item_named(w, "wizDestNeedsInstallationSection");
    QQuickItem* ready_header = item_named(w, "wizDestReadySection");
    M3_CHECK2(5111, needs_header != nullptr, ready_header != nullptr, "section headers missing");
    M3_CHECK(5106, !needs_header->isVisible(), "AC-M5-04-4 (a): «Needs Installation» still shown");
    M3_CHECK(5107, scene_y(card_item(w, "rclone")) > scene_y(ready_header), "AC-M5-04-4 (a): rclone not under «Ready»");
    M3_CHECK(5108, count_visible(w, "wizDestInstallPanel") == 0, "AC-M5-04-4 (a): install panel still open");

    M3_CHECK(5109, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// Chooses the ready card `card` and moves on to the step «destination details».
int open_details_of(QQuickWindow* w, const char* card) {
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* item = card_item(w, card);
    M3_CHECK2(5200, item != nullptr, item->isVisible(), card_name(card).constData());
    scroll_into_view(w, item);
    M3_CHECK(5201, click_ui_item(w, item, card), "click on the card failed");
    QQuickItem* next = item_named(w, "wizDestNextButton");
    M3_CHECK(5202, wait_until([&] { return next && next->isEnabled(); }, 2000), "«Next» not enabled after choosing the card");
    M3_CHECK(5203, click_ui_item(w, next, "Next"), "click on «Next» failed");
    M3_CHECK(5204, wait_until([&] {
        QQuickItem* name = item_named(w, "wizDestNameField");
        return name && name->isVisible();
    }, 2000), "step «destination details» did not open");
    wait_ms(150);
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
    M3_CHECK(5210, wait_until([&] { return count_visible(w, "wizDestDrive_0") == 1; }, 3000), "AC-M5-05-1 (a): the drive is not offered");
    QQuickItem* drive = item_named(w, "wizDestDrive_0");
    M3_CHECK_EQ(5211, drive->property("text").toString(), QStringLiteral("USB-Disk"), "AC-M5-05-1 (a): name");
    M3_CHECK_EQ(5212, drive->property("subtitle").toString(), QStringLiteral("/media/user/USB-Disk"), "AC-M5-05-1 (a): mount point");
    M3_CHECK(5213, count_visible(w, "wizDestDrive_1") == 0, "only the removable drive is offered");
    M3_CHECK(5214, count_visible(w, "wizDestNoDriveMessage") == 0, "«no drive» message shown next to a drive");
    M3_CHECK(5215, click_ui_item(w, drive, "drive"), "click on the drive failed");
    QQuickItem* location = item_named(w, "wizDestLocationField");
    M3_CHECK(5216, wait_until([&] { return location && location->property("text").toString() == QLatin1String("/media/user/USB-Disk"); }, 2000),
        "choosing the drive did not set the path");
    M3_CHECK(5217, drive->property("checked").toBool(), "chosen drive is not marked");
    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestDrive_0"),
        QStringLiteral("wizDestDrivesCheckAgainButton"),
    });
    M3_CHECK_EQ(5218, unnamed, QString(), "control without accessible name");
    M3_CHECK(5219, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// AC-M5-05-2 (GUI): a folder on a CIFS mount (mountinfo fake); after «Test Connection» Kestic
// (a) warns that restic advises against repositories on CIFS/SMB shares, «Next» stays disabled
// until the warning is confirmed, and (b) the destination can be saved after the confirmation.
int scenario_cifs_note(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString dir = qEnvironmentVariable("KESTIC_SMOKE_CIFS_DIR");
    M3_CHECK(5300, !dir.isEmpty(), "KESTIC_SMOKE_CIFS_DIR not set");
    if (const int rc = open_details_of(w, "folder")) {
        return rc;
    }
    item_named(w, "wizDestNameField")->setProperty("text", QStringLiteral("NAS share"));
    item_named(w, "wizDestLocationField")->setProperty("text", dir);
    QQuickItem* test = item_named(w, "wizDestTestButton");
    M3_CHECK(5301, wait_until([&] { return test && test->isEnabled(); }, 2000), "«Test Connection» not enabled");
    M3_CHECK(5302, click_ui_item(w, test, "Test Connection"), "click on «Test Connection» failed");
    QQuickItem* note = item_named(w, "wizDestNoteBanner");
    QQuickItem* confirm = item_named(w, "wizDestNoteConfirmCheck");
    QQuickItem* next = item_named(w, "wizDestNextButton");
    // (a)
    M3_CHECK(5303, wait_until([&] { return note && note->isVisible(); }, 5000), "AC-M5-05-2 (a): no warning after the connection test");
    M3_CHECK_EQ(5304, note->property("text").toString(),
        loc(c.lang,
            "This folder is on a CIFS/SMB network share. restic advises against repositories on such shares.",
            "Dieser Ordner liegt auf einer CIFS/SMB-Netzwerkfreigabe. restic rät von Repositorys auf solchen Freigaben ab."),
        "AC-M5-05-2 (a): warning text");
    M3_CHECK2(5305, confirm != nullptr, confirm->isVisible(), "AC-M5-05-2 (b): confirmation missing");
    M3_CHECK(5306, !next->isEnabled(), "AC-M5-05-2: «Next» enabled before the confirmation");
    M3_CHECK_EQ(5307, accessible_name(confirm), loc(c.lang, "Use this folder anyway", "Diesen Ordner trotzdem verwenden"), "confirmation text");
    // The messages fade in before their text is drawn.
    wait_ms(600);
    M3_CHECK(5308, save_screenshot(w, c.out_path), "screenshot failed");
    // (b)
    M3_CHECK(5309, click_ui_item(w, confirm, "confirm"), "click on the confirmation failed");
    M3_CHECK(5310, wait_until([&] { return next->isEnabled(); }, 2000), "AC-M5-05-2 (b): «Next» disabled after the confirmation");
    M3_CHECK(5311, click_ui_item(w, next, "Next"), "click on «Next» failed");
    QQuickItem* pw = item_named(w, "wizDestPasswordField");
    M3_CHECK(5312, wait_until([&] { return pw && pw->isVisible(); }, 2000), "step «password» did not open");
    pw->setProperty("text", QStringLiteral("correct horse battery"));
    item_named(w, "wizDestPasswordConfirmField")->setProperty("text", QStringLiteral("correct horse battery"));
    M3_CHECK(5313, wait_until([&] { return next->isEnabled(); }, 2000), "«Save Destination» not enabled");
    M3_CHECK(5314, click_ui_item(w, next, "Save Destination"), "click on «Save Destination» failed");
    M3_CHECK(5315, wait_until([&] { return count_visible(w, "addDestinationWizard") == 0; }, 15000),
        "AC-M5-05-2 (b): the wizard did not close after saving");
    return 0;
}

// Fills the S3 form with the server of KESTIC_SMOKE_S3_* and the secret key `secret`, then
// runs «Test Connection» and waits for its banner.
int test_s3_connection(QQuickWindow* w, const QString& secret, int code) {
    item_named(w, "wizDestSecretKeyField")->setProperty("text", secret);
    QQuickItem* test = item_named(w, "wizDestTestButton");
    M3_CHECK(code, wait_until([&] { return test && test->isEnabled(); }, 2000), "«Test Connection» not enabled");
    M3_CHECK(code + 1, click_ui_item(w, test, "Test Connection"), "click on «Test Connection» failed");
    QQuickItem* banner = item_named(w, "wizDestFeedbackBanner");
    M3_CHECK(code + 2, wait_until([&] { return banner && banner->isVisible(); }, 15000), "no result after the connection test");
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
    M3_CHECK(5400, !endpoint.isEmpty() && !access.isEmpty() && !secret.isEmpty(), "KESTIC_SMOKE_S3_* not set");
    if (const int rc = open_destination_types(w)) {
        return rc;
    }
    QQuickItem* cloud = card_item(w, "cloud");
    M3_CHECK2(5401, cloud != nullptr, cloud->isVisible(), "cloud card missing");
    scroll_into_view(w, cloud);
    M3_CHECK(5402, click_ui_item(w, cloud, "cloud"), "click on the cloud card failed");
    QQuickItem* s3_choice = item_named(w, "wizDestCloudService_s3");
    M3_CHECK(5403, wait_until([&] { return s3_choice && s3_choice->isVisible(); }, 2000), "«S3-Compatible Storage» not offered");
    M3_CHECK(5404, s3_choice->property("checked").toBool(), "«S3-Compatible Storage» not chosen");
    M3_CHECK_EQ(5405, s3_choice->property("text").toString(), loc(c.lang, "S3-Compatible Storage", "S3-kompatibler Speicher"), "S3 choice text");
    QQuickItem* next = item_named(w, "wizDestNextButton");
    M3_CHECK(5406, wait_until([&] { return next && next->isEnabled(); }, 2000), "«Next» not enabled after choosing S3");
    M3_CHECK(5407, click_ui_item(w, next, "Next"), "click on «Next» failed");
    M3_CHECK(5408, wait_until([&] { return count_visible(w, "wizDestEndpointField") == 1; }, 2000), "S3 form did not open");
    wait_ms(150);
    int code = 5410;
    for (const char* field : {"wizDestBucketField", "wizDestS3PathField", "wizDestRegionField", "wizDestAccessKeyField", "wizDestSecretKeyField"}) {
        M3_CHECK(code, count_visible(w, field) == 1, field);
        ++code;
    }
    M3_CHECK(5416, count_visible(w, "wizDestLocationField") == 0, "the path field is shown for S3");

    item_named(w, "wizDestNameField")->setProperty("text", QStringLiteral("Cloud"));
    item_named(w, "wizDestEndpointField")->setProperty("text", endpoint);
    item_named(w, "wizDestBucketField")->setProperty("text", QStringLiteral("kestic-") + c.lang);
    item_named(w, "wizDestAccessKeyField")->setProperty("text", access);
    if (const int rc = test_s3_connection(w, QStringLiteral("wrong-secret-key"), 5420)) {
        return rc;
    }
    QQuickItem* banner = item_named(w, "wizDestFeedbackBanner");
    // AC-M5-02-3 (a)
    M3_CHECK_EQ(5430, banner->property("text").toString(),
        loc(c.lang,
            "The destination rejected the login. Check the access data and test again.",
            "Das Ziel hat die Anmeldung abgelehnt. Prüfen Sie die Zugangsdaten und testen Sie erneut."),
        "AC-M5-02-3 (a): message with what to do");
    QObject* check_keys = object_named(w, "wizDestCheckKeysAction");
    M3_CHECK2(5431, check_keys != nullptr, check_keys->property("visible").toBool(), "AC-M5-02-3 (a): action «Check Keys» missing");
    M3_CHECK(5432, !next->isEnabled(), "«Next» enabled after a failed test");
    // Like the other password fields of the form, the secret key field reports no accessible
    // name through Qt's interface, so it is not in this list.
    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestEndpointField"),
        QStringLiteral("wizDestBucketField"),
        QStringLiteral("wizDestS3PathField"),
        QStringLiteral("wizDestRegionField"),
        QStringLiteral("wizDestAccessKeyField"),
    });
    M3_CHECK_EQ(5433, unnamed, QString(), "control without accessible name");
    // The messages fade in before their text is drawn.
    wait_ms(600);
    M3_CHECK(5434, save_screenshot(w, c.out_path), "screenshot failed");
    QMetaObject::invokeMethod(check_keys, "trigger");
    M3_CHECK(5435, wait_until([&] { return item_named(w, "wizDestSecretKeyField")->hasActiveFocus(); }, 2000),
        "AC-M5-02-3 (a): «Check Keys» does not lead to the secret key");

    if (const int rc = test_s3_connection(w, secret, 5440)) {
        return rc;
    }
    M3_CHECK(5450, wait_until([&] { return next->isEnabled(); }, 2000), "«Next» disabled after a passed test");
    // The bucket of the scenario does not exist yet: the result names it.
    M3_CHECK_EQ(5451, banner->property("text").toString(),
        loc(c.lang,
            "Reachable\nBucket 'kestic-en' does not exist yet. Kestic creates it when the destination is added.",
            "Erreichbar\nBucket «kestic-de» gibt es noch nicht; Kestic legt ihn beim Anlegen des Ziels an."),
        "result of the passed test names the missing bucket");
    M3_CHECK(5452, count_visible(w, "wizDestNoteConfirmCheck") == 0, "the missing bucket needs no confirmation");
    wait_ms(600);
    if (!c.out_path.isEmpty()) {
        QString bucket_shot = c.out_path;
        bucket_shot.replace(QStringLiteral("wrong-key"), QStringLiteral("missing-bucket"));
        M3_CHECK(5453, save_screenshot(w, bucket_shot), "screenshot failed");
    }
    return 0;
}

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
    int rc = 0;
    if (scen == QLatin1String("m5-destination-types")) {
        rc = scenario_destination_types(ctx);
    } else if (scen == QLatin1String("m5-destination-types-check-again")) {
        rc = scenario_check_again(ctx);
    } else if (scen == QLatin1String("m5-drive-list")) {
        rc = scenario_drive_list(ctx);
    } else if (scen == QLatin1String("m5-cifs-note")) {
        rc = scenario_cifs_note(ctx);
    } else if (scen == QLatin1String("m5-s3-form")) {
        rc = scenario_s3_form(ctx);
    } else {
        std::fprintf(stderr, "M5 smoke failed (599): unknown m5 smoke scenario %s\n", qPrintable(scen));
        QCoreApplication::exit(599);
        return;
    }
    if (rc != 0) {
        std::fprintf(stderr, "M5 smoke `%s` failed with code %d\n", qPrintable(scen), rc);
        QCoreApplication::exit(rc);
        return;
    }
    if (!warnings->isEmpty()) {
        for (const auto& w : *warnings) {
            std::fprintf(stderr, "M5 QML warning: %s\n", qPrintable(w));
        }
        QCoreApplication::exit(3);
        return;
    }
    QCoreApplication::exit(0);
}

} // namespace kestic::smoke
