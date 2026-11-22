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
// shows the package, «Open in Discover» with its `appstream://` link and `sudo apt install rclone`.
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
    QQuickItem* discover = item_named(w, "wizDestOpenInDiscoverButton");
    QQuickItem* command = item_named(w, "wizDestInstallCommandField");
    // AC-M5-04-3 (a)
    M3_CHECK2(5070, package != nullptr, package->isVisible(), "AC-M5-04-3 (a): package label missing");
    M3_CHECK_EQ(5071, package->property("text").toString(), loc(c.lang, "Package: rclone", "Paket: rclone"), "AC-M5-04-3 (a): package");
    // AC-M5-04-3 (b)
    M3_CHECK2(5072, discover != nullptr, discover->isVisible(), "AC-M5-04-3 (b): «Open in Discover» missing");
    M3_CHECK_EQ(5073, discover->property("text").toString(), loc(c.lang, "Open in Discover", "In Discover öffnen"), "AC-M5-04-3 (b): button text");
    M3_CHECK_EQ(5074, discover->property("url").toString(), QStringLiteral("appstream://rclone"), "AC-M5-04-3 (b): Discover link");
    // AC-M5-04-3 (c)
    M3_CHECK2(5075, command != nullptr, command->isVisible(), "AC-M5-04-3 (c): install command missing");
    M3_CHECK_EQ(5076, command->property("text").toString(), QStringLiteral("sudo apt install rclone"), "AC-M5-04-3 (c): command");
    QQuickItem* again = item_named(w, "wizDestCheckAgainButton");
    M3_CHECK2(5077, again != nullptr, again->isVisible(), "«Check Again» missing");

    const QString unnamed = first_unnamed(w, {
        QStringLiteral("wizDestTypeCard_folder"),
        QStringLiteral("wizDestTypeCard_rclone"),
        QStringLiteral("wizDestOpenInDiscoverButton"),
        QStringLiteral("wizDestInstallCommandField"),
        QStringLiteral("wizDestCopyInstallCommandButton"),
        QStringLiteral("wizDestCheckAgainButton"),
    });
    M3_CHECK_EQ(5079, unnamed, QString(), "control without accessible name");
    M3_CHECK_EQ(5080, accessible_name(discover), loc(c.lang, "Open in Discover", "In Discover öffnen"), "accessible name of «Open in Discover»");
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
