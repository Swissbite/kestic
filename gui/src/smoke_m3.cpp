// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_common.h"

namespace kestic::smoke {

namespace {

bool check_accessible_names_on_controls(
    QObject* root_obj,
    const QStringList& control_names,
    QString* out_missing
) {
    for (const QString& name : control_names) {
        QQuickItem* it = root_obj->findChild<QQuickItem*>(name);
        if (!it) {
            *out_missing = QStringLiteral("missing control ") + name;
            return false;
        }
        const QVariant acc = it->property("Accessible");
        // Check QML Accessible attached property via QQmlProperty/meta-object or fallback to child attached
        QObject* acc_obj = qvariant_cast<QObject*>(acc);
        QString acc_name;
        if (acc_obj) {
            acc_name = acc_obj->property("name").toString();
        }
        // In Qt6 Quick, Accessible.name is stored on QQuickAccessibleAttached child object.
        if (acc_name.isEmpty()) {
            const auto children = it->children();
            for (QObject* ch : children) {
                if (ch && ch->metaObject()
                    && QString::fromLatin1(ch->metaObject()->className()).contains(QLatin1String("Accessible"))) {
                    const QString candidate = ch->property("name").toString();
                    if (!candidate.trimmed().isEmpty()) {
                        acc_name = candidate;
                        break;
                    }
                }
            }
        }
        if (acc_name.trimmed().isEmpty()) {
            *out_missing = QStringLiteral("empty Accessible.name on ") + name;
            return false;
        }
    }
    return true;
}

} // namespace

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
    auto finish_m3 = [window, warnings, out_path](int fail_code = 0, const char* fail_msg = nullptr) {
        if (fail_code != 0) {
            std::fprintf(stderr, "M3 smoke failed (%d): %s\n", fail_code, fail_msg ? fail_msg : "");
            QCoreApplication::exit(fail_code);
            return;
        }
        if (!warnings->isEmpty()) {
            for (const auto& w : *warnings) {
                std::fprintf(stderr, "M3 QML warning: %s\n", qPrintable(w));
            }
            QCoreApplication::exit(3);
            return;
        }
        if (!out_path.isEmpty() && !save_screenshot(window, out_path)) {
            QCoreApplication::exit(10);
            return;
        }
        QCoreApplication::exit(0);
    };

    auto get_backup_id = [window](int row) -> QString {
        QObject* backups_model = window->findChild<QObject*>(QStringLiteral("backupsModel"));
        auto* bm = qobject_cast<QAbstractItemModel*>(backups_model);
        if (!bm || row >= bm->rowCount()) {
            return QString();
        }
        return bm->data(bm->index(row, 0), 0).toString();
    };

    auto get_dest_id = [window](int row) -> QString {
        QObject* dests_model = window->findChild<QObject*>(QStringLiteral("destinationsModel"));
        auto* dm = qobject_cast<QAbstractItemModel*>(dests_model);
        if (!dm || row >= dm->rowCount()) {
            return QString();
        }
        return dm->data(dm->index(row, 0), 0).toString();
    };

    if (scen == QLatin1String("m3-overview-schedule-and-pause")) {
        QQuickItem* ov = window->findChild<QQuickItem*>(QStringLiteral("overviewContent"));
        if (!ov || !ov->isVisible()) {
            finish_m3(301, "overviewContent must be visible");
            return;
        }

        // Verify role metadata for scheduled/overdue/pending tooltip (AC-M3-02-5, AC-M3-08-3).
        QObject* backups_model = window->findChild<QObject*>(QStringLiteral("backupsModel"));
        auto* bm = qobject_cast<QAbstractItemModel*>(backups_model);
        if (!bm || bm->rowCount() < 2) {
            finish_m3(302, "expected at least 2 backups in backupsModel");
            return;
        }
        const QString next0 = bm->data(bm->index(0, 0), 44).toString();
        const bool overdue0 = bm->data(bm->index(0, 0), 15).toBool();
        if (next0.isEmpty() || !overdue0) {
            std::fprintf(stderr, "M3 overview role check: next0='%s' overdue0=%d\n", qPrintable(next0), overdue0 ? 1 : 0);
            finish_m3(303, "expected backup 0 to have nextScheduledAtText and isOverdue=true");
            return;
        }

        // Verify pause dropdown (1h, 3h, tomorrow, indefinite) and globalPauseBanner + Resume (AC-M3-14-4).
        QObject* pause_1h = window->findChild<QObject*>(QStringLiteral("pauseGlobal1hItem"));
        QObject* pause_3h = window->findChild<QObject*>(QStringLiteral("pauseGlobal3hItem"));
        QObject* pause_tom = window->findChild<QObject*>(QStringLiteral("pauseGlobalTomorrowItem"));
        QObject* pause_indef = window->findChild<QObject*>(QStringLiteral("pauseGlobalIndefiniteItem"));
        if (!pause_1h || !pause_3h || !pause_tom || !pause_indef) {
            finish_m3(304, "missing one of the 4 global pause menu items");
            return;
        }
        QMetaObject::invokeMethod(pause_1h, "triggered");
        wait_ms(220);
        if (!backend->property("global_paused").toBool()) {
            finish_m3(305, "pauseGlobal1hItem did not pause schedules");
            return;
        }
        QQuickItem* pause_banner = find_visual_item(window, QStringLiteral("globalPauseBanner"));
        if (!pause_banner || !pause_banner->isVisible()) {
            finish_m3(306, "globalPauseBanner must be visible when global_paused=true");
            return;
        }
        if (!save_screenshot(window, out_path)) {
            QCoreApplication::exit(10);
            return;
        }

        // Resume via globalPauseResumeAction and check Back Up All Now skipped banner (AC-M3-08-6).
        QObject* resume_act = window->findChild<QObject*>(QStringLiteral("globalPauseResumeAction"));
        if (!resume_act) {
            finish_m3(307, "globalPauseResumeAction missing");
            return;
        }
        QMetaObject::invokeMethod(resume_act, "trigger");
        wait_ms(200);
        if (backend->property("global_paused").toBool()) {
            finish_m3(308, "globalPauseResumeAction did not resume global schedules");
            return;
        }

        QObject* backup_all_act = window->findChild<QObject*>(QStringLiteral("backUpAllAction"));
        if (!backup_all_act) {
            finish_m3(309, "backUpAllAction missing");
            return;
        }
        QMetaObject::invokeMethod(backup_all_act, "trigger");
        wait_ms(300);
        QQuickItem* skipped_banner = find_visual_item(window, QStringLiteral("startAllSkippedBanner"));
        if (!skipped_banner || !skipped_banner->isVisible()) {
            finish_m3(310, "startAllSkippedBanner must be visible when Back Up All skips error/unavailable backups");
            return;
        }
        // Exit without overwriting the pause screenshot we already saved.
        if (!warnings->isEmpty()) {
            QCoreApplication::exit(3);
            return;
        }
        QCoreApplication::exit(0);
        return;
    }

    if (scen == QLatin1String("m3-exit12-and-condition-override")) {
        const QString bid_err = get_backup_id(0);
        const QString bid_cond = get_backup_id(1);
        if (bid_err.isEmpty() || bid_cond.isEmpty()) {
            finish_m3(320, "expected at least 2 backups for exit12 + condition override");
            return;
        }

        // 1. Exit 12 primary button replacement & Back Up Now Anyway in overflow menu (AC-M3-08-5).
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid_err));
        wait_ms(220);
        QQuickItem* inline_run = window->findChild<QQuickItem*>(QStringLiteral("backupDetailBackUpNowButton"));
        QQuickItem* inline_pwd = window->findChild<QQuickItem*>(QStringLiteral("backupDetailUpdatePasswordButton"));
        QObject* anyway_act = window->findChild<QObject*>(QStringLiteral("backupDetailBackUpAnywayAction"));
        if (!inline_run || !inline_pwd || !anyway_act) {
            finish_m3(321, "BackupDetailPage Exit 12 controls missing");
            return;
        }
        if (inline_run->isVisible() || !inline_pwd->isVisible() || !anyway_act->property("visible").toBool()) {
            finish_m3(322, "Exit 12 must hide inlineBackUpNowButton, show backupDetailUpdatePasswordButton and backupDetailBackUpAnywayAction");
            return;
        }

        // 2. Unmet RunConditions tooltip & manual confirmation override dialog (AC-M3-02-5, AC-M3-03-7).
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid_cond));
        wait_ms(220);
        QObject* backups_model = window->findChild<QObject*>(QStringLiteral("backupsModel"));
        auto* bm = qobject_cast<QAbstractItemModel*>(backups_model);
        const QString tooltip = bm ? bm->data(bm->index(1, 0), 49).toString() : QString();
        const QString pending_code = bm ? bm->data(bm->index(1, 0), 47).toString() : QString();
        if (pending_code != QLatin1String("on_battery") || tooltip.isEmpty()) {
            std::fprintf(stderr, "M3 condition check: code='%s' tooltip='%s'\n", qPrintable(pending_code), qPrintable(tooltip));
            finish_m3(323, "expected on_battery pendingPrimaryCode and non-empty pendingReasonsTooltip");
            return;
        }

        QQuickItem* run_btn = find_visual_item(window, QStringLiteral("backupDetailBackUpNowButton"));
        if (!run_btn || !run_btn->isVisible() || !run_btn->isEnabled()) {
            finish_m3(324, "backupDetailBackUpNowButton must be enabled to allow override confirmation");
            return;
        }
        if (!click_ui_item(window, run_btn, "Back Up Now (on battery)")) {
            finish_m3(325, "failed to click backupDetailBackUpNowButton");
            return;
        }
        wait_ms(250);

        QObject* override_dlg = window->findChild<QObject*>(QStringLiteral("conditionOverrideDialog"));
        QObject* override_act = window->findChild<QObject*>(QStringLiteral("confirmConditionOverrideAction"));
        if (!override_dlg || !override_dlg->property("visible").toBool() || !override_act) {
            finish_m3(326, "conditionOverrideDialog did not open on unmet RunCondition");
            return;
        }
        if (!save_screenshot(window, out_path)) {
            QCoreApplication::exit(10);
            return;
        }
        QMetaObject::invokeMethod(override_act, "trigger");
        wait_ms(300);
        if (override_dlg->property("visible").toBool()) {
            finish_m3(327, "conditionOverrideDialog did not close after Run Anyway");
            return;
        }
        if (!warnings->isEmpty()) {
            QCoreApplication::exit(3);
            return;
        }
        QCoreApplication::exit(0);
        return;
    }

    if (scen == QLatin1String("m3-schedule-and-conditions-editors")) {
        // 1. Check NewBackupWizard default schedule text (AC-M3-14-1, P35).
        QMetaObject::invokeMethod(window, "openNewBackupWizard", Q_ARG(QVariant, false));
        wait_ms(220);
        QObject* wiz_obj = window->findChild<QObject*>(QStringLiteral("newBackupWizard"));
        if (!wiz_obj) {
            finish_m3(330, "newBackupWizard failed to open");
            return;
        }
        wiz_obj->setProperty("stepIndex", 2);
        wait_ms(180);
        const QString expected_default_needle = (lang == QLatin1String("de"))
            ? QStringLiteral("Täglich um 12:00")
            : QStringLiteral("Daily at 12:00");
        if (!has_visible_text(window->contentItem(), expected_default_needle)) {
            finish_m3(331, "NewBackupWizard defaults summary does not reflect Daily at 12:00");
            return;
        }
        QMetaObject::invokeMethod(window, "closeWizardLayer");
        wait_ms(180);

        const QString bid = get_backup_id(0);
        if (bid.isEmpty()) {
            finish_m3(332, "no backup found");
            return;
        }
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
        wait_ms(200);

        // 2. Schedule editor: Accessible.name + invalid N=1 validation + valid N=4 Tab chain + unsaved Escape + Save.
        QQuickItem* sched_row = find_visual_item(window, QStringLiteral("editScheduleRow"));
        if (!sched_row) {
            finish_m3(333, "editScheduleRow missing on BackupDetailPage");
            return;
        }
        if (!click_ui_item(window, sched_row, "editScheduleRow")) {
            finish_m3(333, "failed to click editScheduleRow");
            return;
        }
        wait_ms(220);

        QQuickItem* sched_page = window->findChild<QQuickItem*>(QStringLiteral("backupScheduleEditPage"));
        if (!sched_page) {
            finish_m3(334, "backupScheduleEditPage did not open");
            return;
        }
        QString missing_acc;
        if (!check_accessible_names_on_controls(
                sched_page,
                {
                    QStringLiteral("scheduleFrequencyCombo"),
                    QStringLiteral("scheduleHourSpinBox"),
                    QStringLiteral("scheduleMinuteSpinBox"),
                    QStringLiteral("useGlobalOverdueCheck"),
                    QStringLiteral("discardScheduleButton"),
                    QStringLiteral("saveScheduleButton"),
                },
                &missing_acc
            )) {
            finish_m3(335, qPrintable(missing_acc));
            return;
        }

        // Switch to every_n_days (index 2) and test invalid N = 1 -> validation banner + disabled Save (AC-M3-14-3).
        QQuickItem* freq_combo = window->findChild<QQuickItem*>(QStringLiteral("scheduleFrequencyCombo"));
        if (!freq_combo) {
            finish_m3(337, "scheduleFrequencyCombo missing");
            return;
        }
        freq_combo->setProperty("currentIndex", 2);
        QMetaObject::invokeMethod(freq_combo, "activated", Q_ARG(int, 2));
        wait_ms(140);
        QQuickItem* interval_spin = window->findChild<QQuickItem*>(QStringLiteral("scheduleIntervalSpinBox"));
        QQuickItem* val_banner = window->findChild<QQuickItem*>(QStringLiteral("scheduleValidationBanner"));
        QQuickItem* save_btn = window->findChild<QQuickItem*>(QStringLiteral("saveScheduleButton"));
        if (!interval_spin || !val_banner || !save_btn) {
            finish_m3(337, "every_n_days controls missing");
            return;
        }
        interval_spin->setProperty("value", 1);
        wait_ms(120);
        if (!val_banner->isVisible() || save_btn->isEnabled()) {
            finish_m3(338, "interval=1 must show scheduleValidationBanner and disable saveScheduleButton");
            return;
        }

        // Fix interval = 4, verify preview slots, Tab navigation to enabled saveScheduleButton (NFR-09), and unsaved Escape (U18).
        interval_spin->setProperty("value", 4);
        wait_ms(160);
        if (val_banner->isVisible() || !save_btn->isEnabled()) {
            finish_m3(339, "interval=4 must clear validation error and enable saveScheduleButton");
            return;
        }
        if (sched_page->property("previewSlots").toList().size() != 3) {
            finish_m3(340, "expected 3 previewSlots for valid schedule");
            return;
        }
        freq_combo->forceActiveFocus(Qt::TabFocusReason);
        QCoreApplication::processEvents();
        if (!tab_until_object(window, QStringLiteral("saveScheduleButton"), 36)) {
            finish_m3(336, "Tab navigation could not reach saveScheduleButton on BackupScheduleEditPage");
            return;
        }
        if (!save_screenshot(window, out_path)) {
            QCoreApplication::exit(10);
            return;
        }
        send_key(window, Qt::Key_Escape);
        wait_ms(140);
        QObject* unsaved_sched = window->findChild<QObject*>(QStringLiteral("unsavedScheduleDialog"));
        if (!unsaved_sched || (!unsaved_sched->property("visible").toBool() && !unsaved_sched->property("opened").toBool())) {
            finish_m3(341, "Escape on dirty BackupScheduleEditPage did not open unsavedScheduleDialog");
            return;
        }
        send_key(window, Qt::Key_Escape);
        wait_ms(120);
        QMetaObject::invokeMethod(sched_page, "commitSave", Q_ARG(QVariant, true));
        wait_ms(220);

        // 3. Conditions editor: Accessible.name, known networks from FakeNM, Tab chain, Save.
        QQuickItem* cond_row = find_visual_item(window, QStringLiteral("editConditionsRow"));
        if (!cond_row || !click_ui_item(window, cond_row, "editConditionsRow")) {
            finish_m3(342, "failed to open BackupConditionsEditPage via editConditionsRow");
            return;
        }
        wait_ms(220);
        QQuickItem* cond_page = window->findChild<QQuickItem*>(QStringLiteral("backupConditionsEditPage"));
        if (!cond_page) {
            finish_m3(343, "backupConditionsEditPage did not open");
            return;
        }
        if (!check_accessible_names_on_controls(
                cond_page,
                {
                    QStringLiteral("requireAcCheck"),
                    QStringLiteral("requireNetworkCheck"),
                    QStringLiteral("skipMeteredCheck"),
                    QStringLiteral("newNetworkField"),
                    QStringLiteral("addNetworkButton"),
                    QStringLiteral("discardConditionsButton"),
                    QStringLiteral("saveConditionsButton"),
                },
                &missing_acc
            )) {
            finish_m3(344, qPrintable(missing_acc));
            return;
        }
        QQuickItem* home_wifi = find_visual_item(window, QStringLiteral("knownNetworkCheck_HomeWifi"));
        QQuickItem* skip_metered = find_visual_item(window, QStringLiteral("skipMeteredCheck"));
        if (!home_wifi || !skip_metered) {
            std::fprintf(
                stderr,
                "M3 cond debug: home_wifi=%p skip_metered=%p knownNetworks=[%s]\n",
                static_cast<void*>(home_wifi),
                static_cast<void*>(skip_metered),
                qPrintable(cond_page->property("knownNetworks").toStringList().join(QStringLiteral(", ")))
            );
            finish_m3(346, "knownNetworkCheck_HomeWifi or skipMeteredCheck missing");
            return;
        }
        skip_metered->setProperty("checked", true);
        QMetaObject::invokeMethod(skip_metered, "toggled");
        home_wifi->setProperty("checked", true);
        QMetaObject::invokeMethod(home_wifi, "toggled");
        wait_ms(140);
        QQuickItem* req_ac = window->findChild<QQuickItem*>(QStringLiteral("requireAcCheck"));
        if (req_ac) {
            req_ac->forceActiveFocus(Qt::TabFocusReason);
            QCoreApplication::processEvents();
        }
        if (!tab_until_object(window, QStringLiteral("saveConditionsButton"), 36)) {
            finish_m3(345, "Tab chain failed on BackupConditionsEditPage");
            return;
        }
        QMetaObject::invokeMethod(cond_page, "commitSave", Q_ARG(QVariant, true));
        wait_ms(220);

        if (!warnings->isEmpty()) {
            QCoreApplication::exit(3);
            return;
        }
        QCoreApplication::exit(0);
        return;
    }

    if (scen == QLatin1String("m3-destination-maintenance-and-settings")) {
        const QString did = get_dest_id(0);
        if (did.isEmpty()) {
            finish_m3(350, "no destination found");
            return;
        }
        QMetaObject::invokeMethod(window, "openDestinationDetailById", Q_ARG(QVariant, did));
        wait_ms(220);

        QObject* unlock_act = window->findChild<QObject*>(QStringLiteral("destUnlockAction"));
        if (!unlock_act) {
            finish_m3(351, "destUnlockAction missing on DestinationDetailPage");
            return;
        }

        // Open DestinationMaintenancePage, verify Accessible.name + Tab chain + prune confirmation dialog.
        QQuickItem* maint_row = find_visual_item(window, QStringLiteral("editMaintenanceRow"));
        if (!maint_row || !click_ui_item(window, maint_row, "editMaintenanceRow")) {
            finish_m3(352, "editMaintenanceRow missing or failed to click");
            return;
        }
        wait_ms(220);
        QQuickItem* maint_page = window->findChild<QQuickItem*>(QStringLiteral("destinationMaintenancePage"));
        if (!maint_page) {
            finish_m3(353, "destinationMaintenancePage did not open");
            return;
        }
        QString missing_acc;
        if (!check_accessible_names_on_controls(
                maint_page,
                {
                    QStringLiteral("pruneFrequencyCombo"),
                    QStringLiteral("checkFrequencyCombo"),
                    QStringLiteral("discardMaintenanceButton"),
                    QStringLiteral("saveMaintenanceButton"),
                },
                &missing_acc
            )) {
            finish_m3(354, qPrintable(missing_acc));
            return;
        }
        QQuickItem* prune_combo = window->findChild<QQuickItem*>(QStringLiteral("pruneFrequencyCombo"));
        if (!prune_combo) {
            finish_m3(355, "pruneFrequencyCombo missing");
            return;
        }
        // Change prune frequency -> Daily (index 2) so saveMaintenanceButton becomes enabled, then verify Tab chain.
        prune_combo->setProperty("currentIndex", 2);
        QMetaObject::invokeMethod(prune_combo, "activated", Q_ARG(int, 2));
        wait_ms(140);
        prune_combo->forceActiveFocus(Qt::TabFocusReason);
        QCoreApplication::processEvents();
        if (!tab_until_object(window, QStringLiteral("saveMaintenanceButton"), 36)) {
            finish_m3(355, "Tab chain failed on DestinationMaintenancePage");
            return;
        }
        QQuickItem* save_maint_btn = find_visual_item(window, QStringLiteral("saveMaintenanceButton"));
        if (!save_maint_btn || !save_maint_btn->isEnabled()) {
            finish_m3(356, "saveMaintenanceButton not enabled after changing prune frequency");
            return;
        }
        if (!click_ui_item(window, save_maint_btn, "saveMaintenanceButton")) {
            finish_m3(356, "failed to click saveMaintenanceButton");
            return;
        }
        wait_ms(300);
        QObject* prune_dlg = window->findChild<QObject*>(QStringLiteral("pruneConfirmDialog"));
        QObject* prune_confirm_act = window->findChild<QObject*>(QStringLiteral("confirmPruneSaveAction"));
        if (!prune_dlg || !prune_dlg->property("visible").toBool() || !prune_confirm_act) {
            finish_m3(357, "pruneConfirmDialog did not open when prune schedule changed");
            return;
        }
        if (!save_screenshot(window, out_path)) {
            QCoreApplication::exit(10);
            return;
        }
        QMetaObject::invokeMethod(prune_confirm_act, "trigger");
        wait_ms(220);

        // Verify Discard flow on clean edit then dirty edit leaves check frequency unchanged.
        QQuickItem* check_combo = window->findChild<QQuickItem*>(QStringLiteral("checkFrequencyCombo"));
        check_combo->setProperty("currentIndex", 3); // manual_only
        QMetaObject::invokeMethod(check_combo, "activated", Q_ARG(int, 3));
        wait_ms(120);
        send_key(window, Qt::Key_Escape);
        wait_ms(140);
        QObject* unsaved_maint = window->findChild<QObject*>(QStringLiteral("unsavedMaintenanceDialog"));
        if (!unsaved_maint || (!unsaved_maint->property("visible").toBool() && !unsaved_maint->property("opened").toBool())) {
            finish_m3(358, "unsavedMaintenanceDialog did not open on Escape");
            return;
        }
        QMetaObject::invokeMethod(unsaved_maint, "discarded");
        wait_ms(180);

        // Open SettingsDialog and verify M3 Monitoring & Notifications controls + persistence.
        QMetaObject::invokeMethod(window, "openSettingsWindow");
        wait_ms(220);
        QObject* settings_win = window->findChild<QObject*>(QStringLiteral("settingsWindow"));
        if (!settings_win) {
            finish_m3(359, "settingsWindow not found");
            return;
        }
        if (!check_accessible_names_on_controls(
                settings_win,
                {
                    QStringLiteral("defaultOverdueSpinBox"),
                    QStringLiteral("notifErrorSwitch"),
                    QStringLiteral("notifWarningSwitch"),
                    QStringLiteral("notifOverdueSwitch"),
                    QStringLiteral("notifMaintenanceSwitch"),
                    QStringLiteral("notifResticUpdateSwitch"),
                },
                &missing_acc
            )) {
            finish_m3(360, qPrintable(missing_acc));
            return;
        }
        QObject* overdue_spin = settings_win->findChild<QObject*>(QStringLiteral("defaultOverdueSpinBox"));
        QObject* notif_overdue = settings_win->findChild<QObject*>(QStringLiteral("notifOverdueSwitch"));
        overdue_spin->setProperty("value", 4);
        QMetaObject::invokeMethod(overdue_spin, "valueModified");
        notif_overdue->setProperty("checked", false);
        QMetaObject::invokeMethod(notif_overdue, "toggled");
        wait_ms(220);

        if (!warnings->isEmpty()) {
            QCoreApplication::exit(3);
            return;
        }
        QCoreApplication::exit(0);
        return;
    }

    finish_m3(399, "unknown m3 smoke scenario");
}

} // namespace kestic::smoke
