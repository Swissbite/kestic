// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_common.h"

namespace kestic::smoke {

void run_m2_smoke_scenario(
    QQmlApplicationEngine& engine,
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& scen,
    const QString& out_path,
    const QString& lang
) {
    (void)engine;
    auto finish_m2 = [window, warnings, out_path](int fail_code = 0, const char* fail_msg = nullptr) {
        if (fail_code != 0) {
            std::fprintf(stderr, "M2 smoke failed (%d): %s\n", fail_code, fail_msg ? fail_msg : "");
            QCoreApplication::exit(fail_code);
            return;
        }
        if (!warnings->isEmpty()) {
            for (const auto& w : *warnings) {
                std::fprintf(stderr, "M2 QML warning: %s\n", qPrintable(w));
            }
            QCoreApplication::exit(3);
            return;
        }
        if (!save_screenshot(window, out_path)) {
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

    if (scen == QLatin1String("m2-excludes-simple-expert-lint")) {
        const QString bid = get_backup_id(0);
        if (bid.isEmpty()) {
            finish_m2(201, "no backup found in backupsModel");
            return;
        }
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
        QMetaObject::invokeMethod(window, "openBackupExcludesPage", Q_ARG(QVariant, bid));
        wait_ms(200);

        QQuickItem* excl_page = window->findChild<QQuickItem*>(QStringLiteral("backupExcludesEditPage"));
        QQuickItem* expert_sec = window->findChild<QQuickItem*>(QStringLiteral("excludeExpertSection"));
        QQuickItem* readonly_info = window->findChild<QQuickItem*>(QStringLiteral("expertRulesReadonlyInfo"));
        QQuickItem* add_btn = window->findChild<QQuickItem*>(QStringLiteral("addExcludeFolderButton"));
        QQuickItem* cache_cb = window->findChild<QQuickItem*>(QStringLiteral("excludeCachesCheckbox"));
        if (!excl_page || !expert_sec || !readonly_info || !add_btn || !cache_cb) {
            finish_m2(202, "BackupExcludesEditPage controls missing");
            return;
        }
        if (expert_sec->isVisible() || !readonly_info->isVisible()) {
            finish_m2(203, "Simple Mode must hide excludeExpertSection and show expertRulesReadonlyInfo");
            return;
        }
        if (!readonly_info->property("text").toString().contains(QStringLiteral("3"))) {
            finish_m2(204, "expertRulesReadonlyInfo must report 3 hidden expert rules");
            return;
        }

        // AC-M2-02-7: dirty Escape opens unsavedExcludesDialog; clean Escape closes immediately.
        cache_cb->forceActiveFocus(Qt::TabFocusReason);
        QCoreApplication::processEvents();
        send_key(window, Qt::Key_Return);
        if (!excl_page->property("dirty").toBool()) {
            finish_m2(205, "toggling excludeCachesCheckbox did not mark dirty");
            return;
        }
        send_key(window, Qt::Key_Escape);
        wait_ms(120);
        QObject* unsaved_dlg = window->findChild<QObject*>(QStringLiteral("unsavedExcludesDialog"));
        if (!unsaved_dlg || (!unsaved_dlg->property("visible").toBool() && !unsaved_dlg->property("opened").toBool())) {
            finish_m2(206, "Escape on dirty excludes page did not open unsavedExcludesDialog");
            return;
        }
        send_key(window, Qt::Key_Escape);
        for (int i = 0; i < 10 && (unsaved_dlg->property("visible").toBool() || unsaved_dlg->property("opened").toBool()); ++i) {
            wait_ms(60);
        }
        send_key(window, Qt::Key_Return);
        if (excl_page->property("dirty").toBool()) {
            finish_m2(207, "toggling back did not restore clean state");
            return;
        }
        send_key(window, Qt::Key_Escape);
        wait_ms(150);
        if (window->findChild<QQuickItem*>(QStringLiteral("backupExcludesEditPage"))) {
            finish_m2(208, "clean Escape did not close BackupExcludesEditPage");
            return;
        }

        // Reopen and enable Expert Mode via the inline notice dialog (AC-M2-02-2 / AC-M2-02-3).
        QMetaObject::invokeMethod(window, "openBackupExcludesPage", Q_ARG(QVariant, bid));
        wait_ms(200);
        excl_page = window->findChild<QQuickItem*>(QStringLiteral("backupExcludesEditPage"));
        expert_sec = window->findChild<QQuickItem*>(QStringLiteral("excludeExpertSection"));
        QObject* enable_act = window->findChild<QObject*>(QStringLiteral("enableExpertModeAction"));
        if (!excl_page || !expert_sec || !enable_act) {
            finish_m2(209, "failed to reopen BackupExcludesEditPage");
            return;
        }
        QMetaObject::invokeMethod(enable_act, "trigger");
        wait_ms(120);
        QObject* exp_dlg = window->findChild<QObject*>(QStringLiteral("enableExpertModeDialog"));
        QObject* confirm_exp = window->findChild<QObject*>(QStringLiteral("confirmEnableExpertModeAction"));
        if (!exp_dlg || !exp_dlg->property("visible").toBool() || !confirm_exp) {
            finish_m2(210, "enableExpertModeDialog did not open");
            return;
        }
        QMetaObject::invokeMethod(confirm_exp, "trigger");
        wait_ms(200);
        if (!backend->property("expert_mode").toBool() || !expert_sec->isVisible()) {
            finish_m2(211, "confirming Enable Expert Mode did not unlock excludeExpertSection");
            return;
        }

        // AC-M2-03-1: Lint warning on `dir` + `!dir/inner` and quick-fix to `dir/*`.
        QQuickItem* excl_area = window->findChild<QQuickItem*>(QStringLiteral("excludesTextArea"));
        if (!excl_area) {
            finish_m2(212, "excludesTextArea missing");
            return;
        }
        excl_area->setProperty("text", QStringLiteral("*.tmp\ndir\n!dir/inner"));
        wait_ms(150);
        QQuickItem* lint_msg = find_visual_item(window, QStringLiteral("excludeLintWarningMessage"));
        QObject* apply_fix = window->findChild<QObject*>(QStringLiteral("applyLintSuggestionAction"));
        if (!lint_msg || !lint_msg->isVisible() || !apply_fix) {
            finish_m2(213, "lint warning message or quick-fix action not shown for dir + !dir/inner");
            return;
        }
        if (!lint_msg->property("text").toString().contains(QStringLiteral("dir/*"))) {
            finish_m2(214, "lint warning text missing suggested fix dir/*");
            return;
        }
        QMetaObject::invokeMethod(apply_fix, "trigger");
        wait_ms(150);
        if (!excl_area->property("text").toString().contains(QStringLiteral("dir/*"))) {
            finish_m2(215, "applying quick-fix did not rewrite dir to dir/*");
            return;
        }
        if (lint_msg->isVisible()) {
            finish_m2(216, "lint warning remained visible after applying quick-fix");
            return;
        }

        // Test Path evaluator
        QQuickItem* test_field = window->findChild<QQuickItem*>(QStringLiteral("testExcludePathField"));
        QQuickItem* test_btn = window->findChild<QQuickItem*>(QStringLiteral("testExcludePathButton"));
        if (!test_field || !test_btn) {
            finish_m2(217, "testExcludePath controls missing");
            return;
        }
        test_field->setProperty("text", QStringLiteral("dir/inner/file.txt"));
        QCoreApplication::processEvents();
        test_btn->forceActiveFocus(Qt::TabFocusReason);
        QCoreApplication::processEvents();
        send_key(window, Qt::Key_Return);
        wait_ms(120);
        QQuickItem* test_banner = window->findChild<QQuickItem*>(QStringLiteral("testPathResultBanner"));
        if (!test_banner || !test_banner->isVisible()) {
            finish_m2(219, "testPathResultBanner not visible after Test Path");
            return;
        }

        // Save valid changes first
        QObject* save_act = window->findChild<QObject*>(QStringLiteral("saveExcludesAction"));
        if (!save_act || !save_act->property("enabled").toBool()) {
            finish_m2(220, "saveExcludesAction should be enabled");
            return;
        }
        QMetaObject::invokeMethod(save_act, "trigger");
        wait_ms(200);
        if (excl_page->property("dirty").toBool()) {
            finish_m2(221, "BackupExcludesEditPage still dirty after valid save");
            return;
        }

        // AC-M2-02-6: Invalid D-Bus save preserves draft and shows error banner.
        excl_area->setProperty("text", QStringLiteral("*.tmp\ndraft-preserved\ndir/*\n!dir/inner"));
        wait_ms(80);
        bool ok_invalid = true;
        QMetaObject::invokeMethod(
            backend,
            "updateBackupExcludes",
            Qt::DirectConnection,
            Q_RETURN_ARG(bool, ok_invalid),
            Q_ARG(QString, bid),
            Q_ARG(QString, QStringLiteral("{invalid-json"))
        );
        if (ok_invalid || backend->property("last_action_error").toString().isEmpty()) {
            finish_m2(222, "invalid updateBackupExcludes should fail with last_action_error");
            return;
        }
        excl_page->setProperty("saveErrorText", backend->property("last_action_error"));
        wait_ms(80);
        QQuickItem* err_banner = window->findChild<QQuickItem*>(QStringLiteral("excludeSaveErrorBanner"));
        if (!err_banner || !err_banner->isVisible()
            || !excl_area->property("text").toString().contains(QStringLiteral("draft-preserved"))) {
            finish_m2(223, "error banner missing or draft lost after validation error");
            return;
        }
        excl_page->setProperty("saveErrorText", QString());
        wait_ms(80);

        finish_m2();
        return;
    }

    if (scen == QLatin1String("m2-dry-run-preview")) {
        const QString bid = get_backup_id(0);
        if (bid.isEmpty()) {
            finish_m2(230, "no backup found");
            return;
        }
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
        QMetaObject::invokeMethod(window, "openBackupDryRunPreviewPage", Q_ARG(QVariant, bid), Q_ARG(QVariant, QString()));
        wait_ms(100);

        QQuickItem* preview_page = window->findChild<QQuickItem*>(QStringLiteral("backupPreviewPage"));
        if (!preview_page) {
            finish_m2(231, "BackupPreviewPage did not open");
            return;
        }
        for (int i = 0; i < 80; ++i) {
            if (preview_page->property("dryRunState").toString() != QLatin1String("running")) {
                break;
            }
            wait_ms(150);
        }
        if (preview_page->property("dryRunState").toString() != QLatin1String("done")) {
            finish_m2(232, "dry-run preview did not complete with state=done");
            return;
        }
        QQuickItem* files_view = window->findChild<QQuickItem*>(QStringLiteral("dryRunFilesListView"));
        QObject* cmd_act = window->findChild<QObject*>(QStringLiteral("showDryRunCommandAction"));
        if (!files_view || !files_view->isVisible() || files_view->property("count").toInt() < 1
            || !cmd_act || !cmd_act->property("visible").toBool()) {
            finish_m2(233, "dry-run files list or Show Command action missing");
            return;
        }
        finish_m2();
        return;
    }

    if (scen == QLatin1String("m2-retention-editor-preview-confirm")) {
        const QString bid = get_backup_id(0);
        if (bid.isEmpty()) {
            finish_m2(240, "no backup found");
            return;
        }
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
        QMetaObject::invokeMethod(window, "openBackupRetentionPage", Q_ARG(QVariant, bid));
        wait_ms(200);

        QQuickItem* ret_page = window->findChild<QQuickItem*>(QStringLiteral("backupRetentionPage"));
        QQuickItem* preset_combo = window->findChild<QQuickItem*>(QStringLiteral("retentionPresetCombo"));
        QQuickItem* semantics_info = window->findChild<QQuickItem*>(QStringLiteral("retentionSemanticsInfo"));
        if (!ret_page || !preset_combo || !semantics_info || !semantics_info->isVisible()) {
            finish_m2(241, "BackupRetentionPage controls missing");
            return;
        }

        // AC-M2-06-8: clean Escape closes; dirty Escape prompts.
        send_key(window, Qt::Key_Escape);
        wait_ms(150);
        if (window->findChild<QQuickItem*>(QStringLiteral("backupRetentionPage"))) {
            finish_m2(242, "clean Escape did not close BackupRetentionPage");
            return;
        }
        QMetaObject::invokeMethod(window, "openBackupRetentionPage", Q_ARG(QVariant, bid));
        wait_ms(200);
        ret_page = window->findChild<QQuickItem*>(QStringLiteral("backupRetentionPage"));
        if (!ret_page) {
            finish_m2(243, "could not reopen BackupRetentionPage");
            return;
        }

        // Set keep_last = 1 (from standard policy across 3 snapshots -> removes 2).
        ret_page->setProperty("keepLastOn", true);
        ret_page->setProperty("keepLastNum", 1);
        ret_page->setProperty("keepLastAll", false);
        ret_page->setProperty("keepDailyOn", false);
        ret_page->setProperty("keepWeeklyOn", false);
        ret_page->setProperty("keepMonthlyOn", false);
        ret_page->setProperty("keepYearlyOn", false);
        QMetaObject::invokeMethod(ret_page, "onFieldEdited");
        wait_ms(100);
        if (!ret_page->property("dirty").toBool()) {
            finish_m2(244, "BackupRetentionPage did not become dirty after changing rules");
            return;
        }
        send_key(window, Qt::Key_Escape);
        wait_ms(120);
        QObject* unsaved_dlg = window->findChild<QObject*>(QStringLiteral("unsavedRetentionDialog"));
        if (!unsaved_dlg || (!unsaved_dlg->property("visible").toBool() && !unsaved_dlg->property("opened").toBool())) {
            finish_m2(245, "dirty Escape did not open unsavedRetentionDialog");
            return;
        }
        send_key(window, Qt::Key_Escape);
        wait_ms(200);

        // AC-M2-06-2: Open Retention Preview page.
        QObject* prev_act = window->findChild<QObject*>(QStringLiteral("previewRetentionAction"));
        if (!prev_act) {
            finish_m2(246, "previewRetentionAction missing");
            return;
        }
        QMetaObject::invokeMethod(prev_act, "trigger");
        wait_ms(250);
        QQuickItem* prev_page = window->findChild<QQuickItem*>(QStringLiteral("backupPreviewPage"));
        QQuickItem* ret_banner = window->findChild<QQuickItem*>(QStringLiteral("retentionPreviewSummaryBanner"));
        if (!prev_page || prev_page->property("mode").toString() != QLatin1String("retention")
            || !ret_banner || !ret_banner->isVisible()) {
            finish_m2(247, "retention preview page or summary banner missing");
            return;
        }
        if (!save_screenshot(window, out_path)) {
            QCoreApplication::exit(10);
            return;
        }

        // Go back to BackupRetentionPage and save -> must show confirmation dialog (AC-M2-06-3).
        send_key(window, Qt::Key_Escape);
        wait_ms(200);
        QObject* save_act = window->findChild<QObject*>(QStringLiteral("saveRetentionAction"));
        if (!save_act) {
            finish_m2(248, "saveRetentionAction missing");
            return;
        }
        QMetaObject::invokeMethod(save_act, "trigger");
        wait_ms(250);
        QObject* confirm_dlg = window->findChild<QObject*>(QStringLiteral("confirmRetentionRemoveDialog"));
        QObject* confirm_act = window->findChild<QObject*>(QStringLiteral("confirmRetentionSaveAction"));
        if (!confirm_dlg || !confirm_dlg->property("visible").toBool() || !confirm_act
            || ret_page->property("pendingRemoveCount").toInt() != 2) {
            finish_m2(249, "confirmRetentionRemoveDialog did not open with pendingRemoveCount=2");
            return;
        }
        QMetaObject::invokeMethod(confirm_act, "trigger");
        wait_ms(250);
        if (ret_page->property("dirty").toBool()) {
            finish_m2(250, "retention rules still dirty after confirming save");
            return;
        }

        // AC-M2-06-5: Saving policy that removes 0 snapshots saves immediately without confirmation dialog.
        ret_page->setProperty("keepLastNum", 5);
        QMetaObject::invokeMethod(ret_page, "onFieldEdited");
        wait_ms(80);
        QMetaObject::invokeMethod(save_act, "trigger");
        wait_ms(250);
        if (confirm_dlg->property("visible").toBool() || confirm_dlg->property("opened").toBool()) {
            finish_m2(251, "confirmRetentionRemoveDialog must NOT open when 0 snapshots would be removed");
            return;
        }
        if (ret_page->property("dirty").toBool()) {
            finish_m2(252, "keep_last=5 did not save immediately");
            return;
        }

        if (!warnings->isEmpty()) {
            for (const auto& w : *warnings) {
                std::fprintf(stderr, "M2 QML warning: %s\n", qPrintable(w));
            }
            QCoreApplication::exit(3);
            return;
        }
        QCoreApplication::exit(0);
        return;
    }

    if (scen == QLatin1String("m2-keyboard-nav")) {
        const QString bid = get_backup_id(0);
        if (bid.isEmpty()) {
            finish_m2(260, "no backup found");
            return;
        }
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
        QMetaObject::invokeMethod(window, "openBackupExcludesPage", Q_ARG(QVariant, bid));
        wait_ms(200);

        const QStringList excl_objs = {
            QStringLiteral("addExcludeFolderButton"),
            QStringLiteral("excludeCachesCheckbox"),
            QStringLiteral("excludeTrashCheckbox"),
            QStringLiteral("excludeLargerThanCheckbox"),
            QStringLiteral("excludesTextArea"),
            QStringLiteral("ignoreCaseCheckbox"),
            QStringLiteral("excludeIfPresentField"),
            QStringLiteral("testExcludePathField"),
            QStringLiteral("testExcludePathButton"),
        };
        for (const QString& obj : excl_objs) {
            if (!tab_until_object(window, obj, 30)) {
                std::fprintf(stderr, "M2 keyboard nav failed: Tab did not reach %s\n", qPrintable(obj));
                QCoreApplication::exit(261);
                return;
            }
        }
        // Tab from excludesTextArea must escape without inserting '\t'
        QQuickItem* excl_area = tab_until_object(window, QStringLiteral("excludesTextArea"), 30);
        const QString before_txt = excl_area ? excl_area->property("text").toString() : QString();
        send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
        if (!excl_area || window->activeFocusItem() == excl_area || excl_area->property("text").toString() != before_txt) {
            finish_m2(262, "Tab trapped or inserted character inside excludesTextArea");
            return;
        }
        // Leave excludes page clean and open Retention page
        send_key(window, Qt::Key_Escape);
        wait_ms(150);

        QMetaObject::invokeMethod(window, "openBackupRetentionPage", Q_ARG(QVariant, bid));
        wait_ms(200);
        const QStringList ret_objs = {
            QStringLiteral("retentionPresetCombo"),
            QStringLiteral("keepLastCheck"),
            QStringLiteral("keepDailyCheck"),
            QStringLiteral("keepDailySpin"),
            QStringLiteral("keepDailyUnlimitedCheck"),
            QStringLiteral("withinAllCheck"),
            QStringLiteral("withinDailyCheck"),
        };
        for (const QString& obj : ret_objs) {
            if (!tab_until_object(window, obj, 40)) {
                std::fprintf(stderr, "M2 keyboard nav failed: Retention Tab did not reach %s\n", qPrintable(obj));
                QCoreApplication::exit(263);
                return;
            }
        }
        // Toggle withinDailyCheck on with Return so withinDailySpin + withinDailyUnitCombo become keyboard focusable
        QQuickItem* wd_cb = window->activeFocusItem();
        if (!wd_cb) {
            finish_m2(264, "active focus missing on withinDailyCheck");
            return;
        }
        const bool wd0 = wd_cb->property("checked").toBool();
        send_key(window, Qt::Key_Return);
        if (wd_cb->property("checked").toBool() == wd0) {
            finish_m2(265, "Return did not toggle withinDailyCheck");
            return;
        }
        if (!tab_until_object(window, QStringLiteral("withinDailySpin"), 10)
            || !tab_until_object(window, QStringLiteral("withinDailyUnitCombo"), 10)) {
            finish_m2(266, "Tab did not reach withinDailySpin / withinDailyUnitCombo after enabling withinDailyCheck");
            return;
        }
        send_key(window, Qt::Key_Escape);
        wait_ms(120);
        QObject* unsaved_ret = window->findChild<QObject*>(QStringLiteral("unsavedRetentionDialog"));
        if (!unsaved_ret || (!unsaved_ret->property("visible").toBool() && !unsaved_ret->property("opened").toBool())) {
            finish_m2(267, "Escape on dirty BackupRetentionPage did not open unsavedRetentionDialog");
            return;
        }
        finish_m2();
        return;
    }

    if (scen == QLatin1String("m2-exit3-unreadable-and-add-exclude")) {
        QObject* runs_model = window->findChild<QObject*>(QStringLiteral("runsModel"));
        auto* rm = qobject_cast<QAbstractItemModel*>(runs_model);
        if (!rm || rm->rowCount() < 1) {
            finish_m2(270, "no run found in runsModel");
            return;
        }
        const QString rid = rm->data(rm->index(0, 0), 0).toString();
        QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 3));
        QMetaObject::invokeMethod(window, "openRunDetailById", Q_ARG(QVariant, rid));
        wait_ms(200);

        QQuickItem* add_btn = find_visual_item(window, QStringLiteral("addUnreadableExcludeButton"));
        if (!add_btn || !add_btn->isVisible()) {
            finish_m2(271, "addUnreadableExcludeButton missing or not visible on Exit 3 RunDetailPage");
            return;
        }
        if (!click_ui_item(window, add_btn, "Add to Exclude Rules")) {
            finish_m2(272, "failed to click addUnreadableExcludeButton");
            return;
        }
        wait_ms(150);
        QObject* dlg = window->findChild<QObject*>(QStringLiteral("addExcludePromptDialog"));
        QObject* confirm_act = window->findChild<QObject*>(QStringLiteral("confirmAddExcludeAction"));
        if (!dlg || !dlg->property("visible").toBool() || !confirm_act) {
            finish_m2(273, "addExcludePromptDialog did not open");
            return;
        }
        if (!save_screenshot(window, out_path)) {
            QCoreApplication::exit(10);
            return;
        }
        QMetaObject::invokeMethod(confirm_act, "trigger");
        for (int i = 0; i < 10 && (dlg->property("visible").toBool() || dlg->property("opened").toBool()); ++i) {
            wait_ms(60);
        }
        if (dlg->property("visible").toBool() || dlg->property("opened").toBool()) {
            finish_m2(274, "addExcludePromptDialog did not close after confirmAddExcludeAction");
            return;
        }
        if (!warnings->isEmpty()) {
            for (const auto& w : *warnings) {
                std::fprintf(stderr, "M2 QML warning: %s\n", qPrintable(w));
            }
            QCoreApplication::exit(3);
            return;
        }
        QCoreApplication::exit(0);
        return;
    }

    if (scen == QLatin1String("m2-drive-unmounted-nested-and-full-config")) {
        const QString drive_bid = get_backup_id(0);
        const QString nested_bid = get_backup_id(1);
        if (drive_bid.isEmpty() || nested_bid.isEmpty()) {
            finish_m2(280, "expected at least 2 backups for drive + nested smoke");
            return;
        }

        // AC-M2-09-1: Unmounted drive backup disables Back Up Now & displays reason
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, drive_bid));
        wait_ms(200);
        QObject* detail_act = window->findChild<QObject*>(QStringLiteral("detailBackupNowAction"));
        QQuickItem* detail_btn = window->findChild<QQuickItem*>(QStringLiteral("backupDetailBackUpNowButton"));
        QQuickItem* reason_lbl = window->findChild<QQuickItem*>(QStringLiteral("backupDetailUnavailableReason"));
        if (!detail_act || !detail_btn || !reason_lbl) {
            finish_m2(281, "BackupDetailPage controls missing for drive backup");
            return;
        }
        if (detail_act->property("enabled").toBool() || detail_btn->isEnabled() || !reason_lbl->isVisible()) {
            finish_m2(282, "Back Up Now must be disabled and unavailable reason visible for unmounted drive");
            return;
        }
        const QString reason_txt = reason_lbl->property("text").toString();
        const QString expected_reason = (lang == QLatin1String("de"))
            ? QStringLiteral("Laufwerk nicht eingehängt")
            : QStringLiteral("drive not mounted");
        if (!reason_txt.contains(expected_reason)) {
            std::fprintf(stderr, "M2 smoke failed: unexpected unmounted drive reason '%s'\n", qPrintable(reason_txt));
            QCoreApplication::exit(283);
            return;
        }

        // AC-M2-10-1: Nested source warning visible on BackupDetailPage and BackupSourceEditPage
        QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, nested_bid));
        wait_ms(200);
        QQuickItem* nested_banner = find_visual_item(window, QStringLiteral("nestedSourceWarningBanner"));
        if (!nested_banner || !nested_banner->isVisible()) {
            finish_m2(284, "nestedSourceWarningBanner not visible on BackupDetailPage");
            return;
        }
        QMetaObject::invokeMethod(window, "openBackupSourcePage", Q_ARG(QVariant, nested_bid));
        wait_ms(200);
        QQuickItem* src_nested_banner = find_visual_item(window, QStringLiteral("sourceNestedWarningBanner"));
        if (!src_nested_banner || !src_nested_banner->isVisible()) {
            finish_m2(285, "sourceNestedWarningBanner not visible on BackupSourceEditPage");
            return;
        }

        // AC-M2-11-1 & AC-M2-11-2: Full Configuration page shows all details and NO plaintext secrets
        send_key(window, Qt::Key_Escape);
        wait_ms(120);
        QMetaObject::invokeMethod(window, "openBackupFullConfigPage", Q_ARG(QVariant, nested_bid));
        wait_ms(200);
        QQuickItem* full_page = window->findChild<QQuickItem*>(QStringLiteral("backupFullConfigPage"));
        QQuickItem* dest_row = window->findChild<QQuickItem*>(QStringLiteral("fullConfigDestinationRow"));
        QQuickItem* excl_row = window->findChild<QQuickItem*>(QStringLiteral("fullConfigExcludesRow"));
        QQuickItem* ret_row = window->findChild<QQuickItem*>(QStringLiteral("fullConfigRetentionRow"));
        if (!full_page || !dest_row || !excl_row || !ret_row) {
            finish_m2(286, "BackupFullConfigPage summary rows missing");
            return;
        }
        const QString dest_val = dest_row->property("rowValue").toString();
        const QString excl_val = excl_row->property("rowValue").toString();
        const QString ret_val = ret_row->property("rowValue").toString();
        if ((!dest_val.contains(QStringLiteral("KWallet")) && !dest_val.contains(QStringLiteral("Secret Service")))
            || !excl_val.contains(QStringLiteral("*.tmp"))
            || ret_val.isEmpty()) {
            finish_m2(287, "Full Config page missing KWallet hint, expert rules, or retention summary");
            return;
        }
        // Verify no plaintext password appears in any visible child label on Full Config page
        QList<QQuickItem*> full_items;
        collect_visual_descendants(full_page, full_items);
        for (QQuickItem* it : full_items) {
            if (it->property("text").toString().contains(QStringLiteral("m2-smoke-pw"))) {
                finish_m2(288, "plaintext password leaked on BackupFullConfigPage");
                return;
            }
        }
        finish_m2();
        return;
    }

    finish_m2(299, "unknown m2 smoke scenario");
}

} // namespace kestic::smoke
