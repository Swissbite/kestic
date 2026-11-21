// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_m3_util.h"

namespace kestic::smoke {

namespace {

using namespace m3;

// AC-M3-08-3: three backups (ok / waiting / overdue) with status, reason and next slot.
// The screenshot is taken while schedules are NOT paused.
int scenario_status_rows(const Ctx& c) {
    QQuickWindow* w = c.window;
    QQuickItem* ov = item_named(w, "overviewContent");
    M3_CHECK2(3001, ov, ov->isVisible(), "overviewContent must be visible");
    QAbstractItemModel* bm = model_named(w, "backupsModel");
    M3_CHECK2(3002, bm, bm->rowCount() == 3, "backupsModel must hold exactly 3 backups");
    M3_CHECK(3003, !c.backend->property("global_paused").toBool(), "schedules must not be paused");
    QQuickItem* banner = item_named(w, "globalPauseBanner");
    M3_CHECK2(3004, banner, !banner->isVisible(), "globalPauseBanner must be hidden while not paused");

    const int ok = row_by_name(bm, QStringLiteral("Docs OK"));
    const int pending = row_by_name(bm, QStringLiteral("Docs Pending"));
    const int overdue = row_by_name(bm, QStringLiteral("Docs Overdue"));
    M3_CHECK3(3005, ok >= 0, pending >= 0, overdue >= 0, "rows Docs OK / Docs Pending / Docs Overdue missing");

    const QString next_prefix = loc(c.lang, "Next: ", "Nächster Lauf: ");

    // (a) status of every row
    const QString ok_status = role_value(bm, ok, "statusText").toString();
    const QString pending_status = role_value(bm, pending, "statusText").toString();
    const QString overdue_status = role_value(bm, overdue, "statusText").toString();
    M3_CHECK_STARTS(3010, ok_status, loc(c.lang, "Backed up · ", "Gesichert · "), "ok row status");
    M3_CHECK_EQ(3011, pending_status, loc(c.lang, "Waiting: AC power", "Wartet: Netzstrom"), "pending row status");
    M3_CHECK_STARTS(3012, overdue_status, loc(c.lang, "Overdue (", "Überfällig ("), "overdue row status");

    // (b) reason: waiting row names its reason, ok row has none, overdue row names missed slots
    M3_CHECK_EQ(
        3013,
        role_value(bm, pending, "pendingPrimaryReason").toString(),
        loc(c.lang, "Waiting: AC power", "Wartet: Netzstrom"),
        "pending row reason"
    );
    M3_CHECK(3014, role_value(bm, ok, "pendingPrimaryReason").toString().isEmpty(), "ok row must have no waiting reason");
    M3_CHECK(3015, role_value(bm, overdue, "isOverdue").toBool(), "overdue row must be flagged overdue");
    M3_CHECK(3016, !role_value(bm, ok, "isOverdue").toBool(), "ok row must not be overdue");

    // (c) next slot of every row
    M3_CHECK_STARTS(3017, role_value(bm, ok, "nextScheduledAtText").toString(), next_prefix, "ok row next slot");
    M3_CHECK_STARTS(3018, role_value(bm, pending, "nextScheduledAtText").toString(), next_prefix, "pending row next slot");
    M3_CHECK_STARTS(3019, role_value(bm, overdue, "nextScheduledAtText").toString(), next_prefix, "overdue row next slot");

    // All three are really rendered on the cards.
    QQuickItem* content = w->contentItem();
    M3_CHECK(3020, has_visible_text(content, ok_status), "ok status not rendered on its card");
    M3_CHECK(3021, has_visible_text(content, pending_status), "pending status not rendered on its card");
    M3_CHECK(3022, has_visible_text(content, overdue_status), "overdue status not rendered on its card");

    // «Back Up All Now»: hint BEFORE the click that not all backups can be backed up now.
    QQuickItem* hint = item_named(w, "backUpAllHintLabel");
    M3_CHECK2(3030, hint, hint->isVisible(), "backUpAllHintLabel must be visible");
    M3_CHECK_EQ(
        3031,
        hint->property("text").toString(),
        loc(c.lang, "Only 2 of 3 backups can start now.", "Nur 2 von 3 Backups können jetzt starten."),
        "back up all hint"
    );
    // Pause button while NOT paused offers pausing.
    QObject* pause_act = object_named(w, "pauseSchedulesAction");
    M3_CHECK(3032, pause_act != nullptr, "pauseSchedulesAction missing");
    M3_CHECK_EQ(
        3033,
        pause_act->property("text").toString(),
        loc(c.lang, "Pause Schedules", "Zeitpläne pausieren"),
        "pause button text"
    );

    M3_CHECK(3040, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// AC-M3-14-4 (global), AC-M3-08-6, pause line format and pause/resume button.
int scenario_pause(const Ctx& c) {
    QQuickWindow* w = c.window;
    QQuickItem* ov = item_named(w, "overviewContent");
    M3_CHECK2(3101, ov, ov->isVisible(), "overviewContent must be visible");

    // AC-M3-14-4 (a)+(b global): four pause choices with their labels.
    struct Entry { const char* object; const char* en; const char* de; };
    const Entry entries[] = {
        { "pauseGlobal1hItem", "1 Hour", "1 Stunde" },
        { "pauseGlobal3hItem", "3 Hours", "3 Stunden" },
        { "pauseGlobalTomorrowItem", "Until Tomorrow", "Bis morgen" },
        { "pauseGlobalIndefiniteItem", "Until I Resume", "Bis ich fortsetze" },
    };
    int code = 3110;
    for (const Entry& e : entries) {
        QObject* item = object_named(w, e.object);
        M3_CHECK(code, item != nullptr, "global pause menu item missing");
        M3_CHECK_EQ(code + 1, item->property("text").toString(), loc(c.lang, e.en, e.de), e.object);
        code += 2;
    }

    QObject* pause_1h = object_named(w, "pauseGlobal1hItem");
    QMetaObject::invokeMethod(pause_1h, "triggered");
    M3_CHECK(3120, wait_until([&] { return c.backend->property("global_paused").toBool(); }, 3000), "pauseGlobal1hItem did not pause");
    wait_ms(200);

    QQuickItem* banner = item_named(w, "globalPauseBanner");
    M3_CHECK2(3121, banner, banner->isVisible(), "globalPauseBanner must be visible while paused");
    const QString line = c.backend->property("global_paused_line").toString();
    M3_CHECK_STARTS(3122, line, loc(c.lang, "Paused since ", "Pausiert seit "), "pause line language");
    M3_CHECK_EQ(3123, banner->property("text").toString(), line, "banner text equals pause line");
    M3_CHECK(3124, line.contains(QLatin1String("(")), "pause line must name the end of the pause");

    // Headline must be in the GUI language (daemon text is never passed through).
    const QString headline = c.backend->property("overall_headline").toString();
    M3_CHECK_EQ(
        3125,
        headline,
        loc(c.lang, "1 backup failed", "1 Backup fehlgeschlagen"),
        "overview headline language"
    );

    // Pause button turns into «Resume Schedules».
    QObject* pause_act = object_named(w, "pauseSchedulesAction");
    M3_CHECK(3126, pause_act != nullptr, "pauseSchedulesAction missing");
    M3_CHECK_EQ(
        3127,
        pause_act->property("text").toString(),
        loc(c.lang, "Resume Schedules", "Zeitpläne fortsetzen"),
        "pause button text while paused"
    );
    M3_CHECK(3128, save_screenshot(w, c.out_path), "screenshot failed");

    // Resume through the same button.
    QMetaObject::invokeMethod(pause_act, "trigger");
    M3_CHECK(3130, wait_until([&] { return !c.backend->property("global_paused").toBool(); }, 3000), "pause button did not resume");
    wait_ms(150);
    M3_CHECK(3131, !banner->isVisible(), "globalPauseBanner must disappear after resume");
    M3_CHECK_EQ(
        3132,
        pause_act->property("text").toString(),
        loc(c.lang, "Pause Schedules", "Zeitpläne pausieren"),
        "pause button text after resume"
    );

    return 0;
}

// AC-M3-08-6 (b): «Back Up All Now» names both skipped backups with their reason.
int scenario_start_all(const Ctx& c) {
    QQuickWindow* w = c.window;
    QObject* back_up_all = object_named(w, "backUpAllAction");
    M3_CHECK(3140, back_up_all != nullptr, "backUpAllAction missing");
    QMetaObject::invokeMethod(back_up_all, "trigger");
    M3_CHECK(3141, wait_until([&] { return !c.backend->property("start_all_skipped_message").toString().isEmpty(); }, 5000), "no start-all message");
    wait_ms(200);
    QQuickItem* skipped = item_named(w, "startAllSkippedBanner");
    M3_CHECK2(3142, skipped, skipped->isVisible(), "startAllSkippedBanner must be visible");
    const QString msg = skipped->property("text").toString();
    M3_CHECK_STARTS(3143, msg, loc(c.lang, "2 backups skipped: ", "2 Backups übersprungen: "), "message must count 2 skipped backups");
    M3_CHECK(3144, msg.contains(QStringLiteral("Offline Media (") + loc(c.lang, "Source unavailable", "Quelle nicht verfügbar") + QLatin1Char(')')),
        "message must name Offline Media with reason");
    M3_CHECK(3145, msg.contains(QStringLiteral("Locked Docs (") + loc(c.lang, "Permanent error, needs fixing", "Dauerhafter Fehler, muss behoben werden") + QLatin1Char(')')),
        "message must name Locked Docs with reason");
    M3_CHECK(3146, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// AC-M3-08-5 (overview + detail) and manual override of unmet run conditions.
int scenario_exit12_and_override(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid_err = backup_id_at(w, 0);
    const QString bid_cond = backup_id_at(w, 1);
    M3_CHECK2(3201, !bid_err.isEmpty(), !bid_cond.isEmpty(), "expected 2 backups");

    // Overview: (a) «Update Saved Password…» replaces «Back Up Now» on the Exit-12 card.
    M3_CHECK(3202, count_visible(w, "overviewUpdatePasswordButton") == 1, "exactly one card must show «Update Saved Password…»");
    M3_CHECK(3203, count_visible(w, "overviewBackUpNowButton") == 1, "exactly one card must keep «Back Up Now»");
    for (QObject* o : objects_named(w, "overviewUpdatePasswordButton")) {
        auto* it = qobject_cast<QQuickItem*>(o);
        if (it && it->isVisible()) {
            M3_CHECK_EQ(
                3204,
                it->property("text").toString(),
                loc(c.lang, "Update Saved Password…", "Gespeichertes Passwort aktualisieren…"),
                "overview update password button text"
            );
        }
    }
    // Overview: (b) «Back Up Now Anyway» in the ⋮ menu of the Exit-12 card only.
    int anyway = 0;
    int plain = 0;
    const QString anyway_text = loc(c.lang, "Back Up Now Anyway", "Trotzdem jetzt sichern");
    const QString plain_text = loc(c.lang, "Back Up Now", "Jetzt sichern");
    for (QObject* mi : objects_named(w, "overviewCardBackUpMenuItem")) {
        const QString t = mi->property("text").toString();
        anyway += (t == anyway_text) ? 1 : 0;
        plain += (t == plain_text) ? 1 : 0;
    }
    M3_CHECK(3205, anyway == 1, "exactly one ⋮ menu must offer «Back Up Now Anyway»");
    M3_CHECK(3206, plain == 1, "the other ⋮ menu must offer plain «Back Up Now»");

    // Detail: (a)+(b)
    M3_CHECK(3209, open_backup_detail(w, bid_err), "backupDetailPage of the failed backup did not open");
    QQuickItem* inline_run = item_named(w, "backupDetailBackUpNowButton");
    QQuickItem* inline_pwd = item_named(w, "backupDetailUpdatePasswordButton");
    QObject* anyway_act = object_named(w, "backupDetailBackUpAnywayAction");
    M3_CHECK3(3210, inline_run, inline_pwd, anyway_act, "BackupDetailPage Exit 12 controls missing");
    M3_CHECK(3211, !inline_run->isVisible(), "detail: «Back Up Now» must be replaced");
    M3_CHECK(3212, inline_pwd->isVisible(), "detail: «Update Saved Password…» must be shown");
    M3_CHECK(3213, anyway_act->property("visible").toBool(), "detail: «Back Up Now Anyway» must be in the ⋮ menu");
    M3_CHECK_EQ(3214, anyway_act->property("text").toString(), anyway_text, "detail anyway action text");

    // Unmet run conditions -> confirmation dialog (AC-M3-02-5, AC-M3-03-7).
    M3_CHECK(3219, open_backup_detail(w, bid_cond), "backupDetailPage of the conditional backup did not open");
    QAbstractItemModel* bm = model_named(w, "backupsModel");
    const QString code_text = role_value(bm, 1, "pendingPrimaryCode").toString();
    M3_CHECK_EQ(3220, code_text, QStringLiteral("on_battery"), "pending code of battery backup");
    M3_CHECK(3221, !role_value(bm, 1, "pendingReasonsTooltip").toString().isEmpty(), "pending reasons tooltip must not be empty");
    QQuickItem* run_btn = find_visual_item(w, QStringLiteral("backupDetailBackUpNowButton"));
    M3_CHECK3(3222, run_btn, run_btn->isVisible(), run_btn->isEnabled(), "Back Up Now must be enabled to allow override");
    M3_CHECK(3223, click_ui_item(w, run_btn, "Back Up Now (on battery)"), "failed to click Back Up Now");
    QObject* override_dlg = object_named(w, "conditionOverrideDialog");
    M3_CHECK(3224, wait_until([&] { return is_open(override_dlg); }, 3000), "conditionOverrideDialog did not open");
    QObject* override_act = object_named(w, "confirmConditionOverrideAction");
    M3_CHECK(3225, override_act != nullptr, "confirmConditionOverrideAction missing");
    M3_CHECK(3226, save_screenshot(w, c.out_path), "screenshot failed");
    QMetaObject::invokeMethod(override_act, "trigger");
    M3_CHECK(3227, wait_until([&] { return is_closed(override_dlg); }, 3000), "dialog did not close after Run Anyway");
    return 0;
}

// A running backup «Docs» on the overview: while restic still finds files the card shows a bar
// without end point and the amount looked through; afterwards percentage and time left.
int scenario_backup_progress(const Ctx& c, bool scanning) {
    QQuickWindow* w = c.window;
    QAbstractItemModel* bm = model_named(w, "backupsModel");
    M3_CHECK(3860, bm != nullptr, "backupsModel missing");
    M3_CHECK(3878, bm->rowCount() == 1, "backupsModel must hold exactly 1 backup");
    M3_CHECK(3861, role_value(bm, 0, "isRunning").toBool(), "backup must be running");

    const QString status = role_value(bm, 0, "statusText").toString();
    const QString line = role_value(bm, 0, "activeProgressText").toString();
    const QString headline = c.backend->property("overall_headline").toString();
    QQuickItem* bar = item_named(w, "backupCardProgressBar");
    M3_CHECK(3862, bar != nullptr, "card progress bar missing");
    M3_CHECK(3879, bar->isVisible(), "card progress bar must be visible");

    if (scanning) {
        // (a) status line, (b) amount looked through, (c) headline, (d) bar without end point
        M3_CHECK_EQ(3863, status, loc(c.lang, "Backing up · looking through files", "Sichert · Dateien werden durchgesehen"), "status while scanning");
        M3_CHECK_STARTS(3864, line, QStringLiteral("4.9 GB · "), "amount looked through");
        M3_CHECK(3865, line.endsWith(loc(c.lang, " files so far", " Dateien bisher")), "number of files looked through");
        M3_CHECK(3866, !line.contains(QLatin1Char('%')), "no percentage while scanning");
        M3_CHECK_EQ(3867, headline, loc(c.lang, "Docs: backing up · looking through files", "Docs: sichert · Dateien werden durchgesehen"), "headline while scanning");
        M3_CHECK(3868, bar->property("indeterminate").toBool(), "bar must have no end point while scanning");
        M3_CHECK(3869, role_value(bm, 0, "activeScanning").toBool(), "activeScanning role");
    } else {
        M3_CHECK_EQ(3870, status, loc(c.lang, "Backing up · 25%", "Sichert · 25%"), "status after the scan");
        M3_CHECK_EQ(3871, line, loc(c.lang, "25% · 1.0 MB of 4.2 MB · about 30 s left", "25% · 1.0 MB von 4.2 MB · noch etwa 30 s"), "percentage, amount and time left");
        M3_CHECK_EQ(3872, headline, loc(c.lang, "Docs: backing up · 25%", "Docs: sichert · 25%"), "headline after the scan");
        M3_CHECK(3873, !bar->property("indeterminate").toBool(), "bar must have an end point after the scan");
        M3_CHECK(3874, !role_value(bm, 0, "activeScanning").toBool(), "activeScanning role");
    }
    QQuickItem* content = w->contentItem();
    M3_CHECK(3875, has_visible_text(content, status), "status not rendered on the card");
    M3_CHECK(3876, has_visible_text(content, line), "progress line not rendered on the card");
    M3_CHECK(3877, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

} // namespace

int run_m3_editor_scenario(const m3::Ctx& ctx, const QString& scen, bool* handled);
int run_m3_pages_scenario(const m3::Ctx& ctx, const QString& scen, bool* handled);
int run_main_menu_scenario(const m3::Ctx& ctx, const QString& scen, bool* handled);

void run_m3_smoke_scenario(
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
    bool handled = true;
    if (scen == QLatin1String("m3-overview-status-rows")) {
        rc = scenario_status_rows(ctx);
    } else if (scen == QLatin1String("m3-overview-pause")) {
        rc = scenario_pause(ctx);
    } else if (scen == QLatin1String("m3-overview-start-all")) {
        rc = scenario_start_all(ctx);
    } else if (scen == QLatin1String("m3-overview-progress-scanning")) {
        rc = scenario_backup_progress(ctx, true);
    } else if (scen == QLatin1String("m3-overview-progress-percent")) {
        rc = scenario_backup_progress(ctx, false);
    } else if (scen == QLatin1String("m3-exit12-and-condition-override")) {
        rc = scenario_exit12_and_override(ctx);
    } else {
        rc = run_m3_pages_scenario(ctx, scen, &handled);
        if (!handled) {
            rc = run_m3_editor_scenario(ctx, scen, &handled);
        }
        if (!handled) {
            rc = run_main_menu_scenario(ctx, scen, &handled);
        }
    }
    if (!handled) {
        std::fprintf(stderr, "M3 smoke failed (399): unknown m3 smoke scenario %s\n", qPrintable(scen));
        QCoreApplication::exit(399);
        return;
    }
    if (rc != 0) {
        std::fprintf(stderr, "M3 smoke `%s` failed with code %d\n", qPrintable(scen), rc);
        QCoreApplication::exit(rc);
        return;
    }
    if (!warnings->isEmpty()) {
        for (const auto& w : *warnings) {
            std::fprintf(stderr, "M3 QML warning: %s\n", qPrintable(w));
        }
        QCoreApplication::exit(3);
        return;
    }
    QCoreApplication::exit(0);
}

} // namespace kestic::smoke
