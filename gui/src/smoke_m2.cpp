// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_common.h"

#include <QRegularExpression>

namespace kestic::smoke {

namespace {

// What the scripted M2 scenarios below work on.
struct Scene {
    QQuickWindow* window;
    QObject* backend;
    std::shared_ptr<QStringList> warnings;
    QString out_path;
    QString lang;
};

// m2-excludes-simple-expert-lint
void scene_excludes_simple_expert_lint(const Scene& s) {
    QQuickWindow* window = s.window;
    QObject* backend = s.backend;
    const auto& warnings = s.warnings;
    const QString& out_path = s.out_path;
    const QString bid = backup_id_at(window, 0);
    SMOKE_CHECK("M2", 201, !bid.isEmpty(), "no backup found in backupsModel");
    QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
    wait_ms(200);
    QQuickItem* excl_row = child_item(window, "editExcludesRow");
    SMOKE_CHECK("M2", 201, excl_row, "editExcludesRow missing on BackupDetailPage");
    QMetaObject::invokeMethod(window, "focusItemInPageStack", Q_ARG(QVariant, QVariant::fromValue<QObject*>(excl_row)));
    wait_ms(80);
    press_and_wait(window, Qt::Key_Return, 200);

    QQuickItem* excl_page = child_item(window, "backupExcludesEditPage");
    QQuickItem* expert_sec = child_item(window, "excludeExpertSection");
    QQuickItem* readonly_info = child_item(window, "expertRulesReadonlyInfo");
    QQuickItem* add_btn = child_item(window, "addExcludeFolderButton");
    QQuickItem* cache_cb = child_item(window, "excludeCachesCheckbox");
    SMOKE_CHECK("M2", 202, excl_page, "backupExcludesEditPage missing");
    SMOKE_CHECK("M2", 202, expert_sec, "excludeExpertSection missing");
    SMOKE_CHECK("M2", 202, readonly_info, "expertRulesReadonlyInfo missing");
    SMOKE_CHECK("M2", 202, add_btn, "addExcludeFolderButton missing");
    SMOKE_CHECK("M2", 202, cache_cb, "excludeCachesCheckbox missing");
    SMOKE_CHECK("M2", 202, window->activeFocusItem() == add_btn,
        QStringLiteral("opening BackupExcludesEditPage must focus addExcludeFolderButton immediately, focus is on %1") .arg(describe_focus_item(window->activeFocusItem())));
    SMOKE_CHECK2("M2", 203, !expert_sec->isVisible(), readonly_info->isVisible(),
        "Simple Mode must hide excludeExpertSection and show expertRulesReadonlyInfo");
    SMOKE_CHECK("M2", 204, readonly_info->property("text").toString().contains(QStringLiteral("3")),
        "expertRulesReadonlyInfo must report 3 hidden expert rules");

    // AC-M2-02-7: dirty Escape opens unsavedExcludesDialog; clean Escape closes immediately and restores focus to editExcludesRow.
    cache_cb->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK2("M2", 205, excl_page->property("dirty").toBool(), window->activeFocusItem() == cache_cb,
        "toggling excludeCachesCheckbox did not mark dirty or lost focus");
    press_and_wait(window, Qt::Key_Escape, 120);
    QObject* unsaved_dlg = child_object(window, "unsavedExcludesDialog");
    SMOKE_CHECK_OPENED("M2", 206, unsaved_dlg, "Escape on dirty excludes page did not open unsavedExcludesDialog");
    send_key(window, Qt::Key_Escape);
    wait_until([&] { return is_closed(unsaved_dlg); }, 600);
    cache_cb->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK2("M2", 207, !excl_page->property("dirty").toBool(), window->activeFocusItem() == cache_cb,
        "toggling back did not restore clean state or lost focus");
    press_and_wait(window, Qt::Key_Escape, 150);
    SMOKE_CHECK("M2", 208, !child_item(window, "backupExcludesEditPage"),
        "clean Escape did not close BackupExcludesEditPage");
    SMOKE_CHECK("M2", 208, window->activeFocusItem() == excl_row,
        QStringLiteral("closing BackupExcludesEditPage with Escape did not restore focus to editExcludesRow, focus is on %1") .arg(describe_focus_item(window->activeFocusItem())));

    // Second cycle: pressing Return on editExcludesRow and Escape again must behave identically (no alternating columnView scroll/stack leak).
    press_and_wait(window, Qt::Key_Return, 200);
    add_btn = child_item(window, "addExcludeFolderButton");
    SMOKE_CHECK2("M2", 208, add_btn, window->activeFocusItem() == add_btn,
        "second Return on editExcludesRow did not focus addExcludeFolderButton");
    // Verify 2-column Shift-Left invariant (in wide mode: Lv1 disabled off-screen, Lv2 + Lv3 visible at ~50% width each) and accessible breadcrumb bar.
    for (const char* name : {"breadcrumbToolBar", "breadcrumbBackButton", "breadcrumbButton_0", "breadcrumbButton_1"}) {
        QQuickItem* crumb = find_visual_item(window, QString::fromLatin1(name));
        SMOKE_CHECK_VISIBLE("M2", 208, crumb, QStringLiteral("%1 missing or not visible").arg(QString::fromLatin1(name)));
    }
    for (const char* name : {"breadcrumbButton_0", "breadcrumbButton_1"}) {
        SMOKE_CHECK("M2", 208, !find_visual_item(window, QString::fromLatin1(name))->property("text").toString().isEmpty(),
            QStringLiteral("%1 has no text").arg(QString::fromLatin1(name)));
    }
    if (!window->property("narrowMode").toBool()) {
        QQuickItem* lv1_page = child_item(window, "backupsListPage");
        QQuickItem* lv2_page = child_item(window, "backupDetailPage");
        QQuickItem* lv3_page = child_item(window, "backupExcludesEditPage");
        SMOKE_CHECK5("M2", 208, lv1_page, !lv1_page->isEnabled(), lv2_page, lv2_page->isEnabled(), lv3_page,
            "wide mode at depth 3 must disable off-screen Lv1 and show Lv2 + Lv3 at 50% width each");
        SMOKE_CHECK3("M2", 208, lv3_page->isEnabled(), lv2_page->width() >= (window->width() / 2.0 - 10.0), lv3_page->width() >= (window->width() / 2.0 - 10.0),
            "wide mode at depth 3 must disable off-screen Lv1 and show Lv2 + Lv3 at 50% width each");
    }
    // Opening another Lv3 page (Source) while Excludes is open must REPLACE Lv3 at depth 3, never stack to depth 4.
    QMetaObject::invokeMethod(window, "openBackupSourcePage", Q_ARG(QVariant, bid));
    wait_ms(180);
    SMOKE_CHECK2("M2", 208, !child_item(window, "backupExcludesEditPage"), child_item(window, "backupSourceEditPage"),
        "opening BackupSourceEditPage from Lv2 did not replace BackupExcludesEditPage at depth 3");
    press_and_wait(window, Qt::Key_Escape, 150);
    SMOKE_CHECK2("M2", 208, !child_item(window, "backupSourceEditPage"), window->activeFocusItem() == excl_row,
        "Escape after Lv3 replacement did not close subpage or restore focus to editExcludesRow");

    // Reopen and enable Expert Mode via the inline notice dialog (AC-M2-02-2 / AC-M2-02-3).
    QMetaObject::invokeMethod(window, "openBackupExcludesPage", Q_ARG(QVariant, bid));
    wait_ms(200);
    excl_page = child_item(window, "backupExcludesEditPage");
    expert_sec = child_item(window, "excludeExpertSection");
    QObject* enable_act = child_object(window, "enableExpertModeAction");
    SMOKE_CHECK3("M2", 209, excl_page, expert_sec, enable_act, "failed to reopen BackupExcludesEditPage");
    trigger(enable_act);
    wait_ms(120);
    QObject* exp_dlg = child_object(window, "enableExpertModeDialog");
    QObject* confirm_exp = child_object(window, "confirmEnableExpertModeAction");
    SMOKE_CHECK3("M2", 210, exp_dlg, exp_dlg->property("visible").toBool(), confirm_exp, "enableExpertModeDialog did not open");
    trigger(confirm_exp);
    wait_ms(200);
    SMOKE_CHECK2("M2", 211, backend->property("expert_mode").toBool(), expert_sec->isVisible(),
        "confirming Enable Expert Mode did not unlock excludeExpertSection");

    // AC-M2-03-1: Lint warning on `dir` + `!dir/inner` and quick-fix to `dir/*`.
    QQuickItem* excl_area = child_item(window, "excludesTextArea");
    SMOKE_CHECK("M2", 212, excl_area, "excludesTextArea missing");
    excl_area->setProperty("text", QStringLiteral("*.tmp\ndir\n!dir/inner"));
    wait_ms(150);
    QQuickItem* lint_msg = find_visual_item(window, QStringLiteral("excludeLintWarningMessage"));
    QObject* apply_fix = child_object(window, "applyLintSuggestionAction");
    SMOKE_CHECK3("M2", 213, lint_msg, lint_msg->isVisible(), apply_fix,
        "lint warning message or quick-fix action not shown for dir + !dir/inner");
    SMOKE_CHECK("M2", 214, lint_msg->property("text").toString().contains(QStringLiteral("dir/*")),
        "lint warning text missing suggested fix dir/*");
    trigger(apply_fix);
    wait_ms(150);
    SMOKE_CHECK("M2", 215, excl_area->property("text").toString().contains(QStringLiteral("dir/*")),
        "applying quick-fix did not rewrite dir to dir/*");
    SMOKE_CHECK("M2", 216, !lint_msg->isVisible(), "lint warning remained visible after applying quick-fix");

    // Test Path evaluator
    QQuickItem* test_field = child_item(window, "testExcludePathField");
    QQuickItem* test_btn = child_item(window, "testExcludePathButton");
    SMOKE_CHECK2("M2", 217, test_field, test_btn, "testExcludePath controls missing");
    test_field->setProperty("text", QStringLiteral("dir/inner/file.txt"));
    QCoreApplication::processEvents();
    test_btn->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    press_and_wait(window, Qt::Key_Return, 120);
    QQuickItem* test_banner = child_item(window, "testPathResultBanner");
    SMOKE_CHECK_VISIBLE("M2", 219, test_banner, "testPathResultBanner not visible after Test Path");

    // Save valid changes first
    QObject* save_act = child_object(window, "saveExcludesAction");
    SMOKE_CHECK_ENABLED("M2", 220, save_act, "saveExcludesAction should be enabled");
    trigger(save_act);
    wait_ms(200);
    SMOKE_CHECK("M2", 221, !excl_page->property("dirty").toBool(), "BackupExcludesEditPage still dirty after valid save");

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
    SMOKE_CHECK2("M2", 222, !ok_invalid, !backend->property("last_action_error").toString().isEmpty(),
        "invalid updateBackupExcludes should fail with last_action_error");
    excl_page->setProperty("saveErrorText", backend->property("last_action_error"));
    wait_ms(80);
    QQuickItem* err_banner = child_item(window, "excludeSaveErrorBanner");
    SMOKE_CHECK3("M2", 223, err_banner, err_banner->isVisible(), excl_area->property("text").toString().contains(QStringLiteral("draft-preserved")),
        "error banner missing or draft lost after validation error");
    excl_page->setProperty("saveErrorText", QString());
    wait_ms(80);

    smoke_finish("M2", warnings.get(), window, out_path);
}

// m2-dry-run-preview
void scene_dry_run_preview(const Scene& s) {
    QQuickWindow* window = s.window;
    const auto& warnings = s.warnings;
    const QString& out_path = s.out_path;
    const QString bid = backup_id_at(window, 0);
    SMOKE_CHECK("M2", 230, !bid.isEmpty(), "no backup found");
    QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
    QMetaObject::invokeMethod(window, "openBackupDryRunPreviewPage", Q_ARG(QVariant, bid), Q_ARG(QVariant, QString()));
    wait_ms(100);

    QQuickItem* preview_page = child_item(window, "backupPreviewPage");
    SMOKE_CHECK("M2", 231, preview_page, "BackupPreviewPage did not open");
    wait_until([&] { return preview_page->property("dryRunState").toString() != QLatin1String("running"); }, 12000);
    SMOKE_CHECK("M2", 232, preview_page->property("dryRunState").toString() == QLatin1String("done"),
        "dry-run preview did not complete with state=done");
    QQuickItem* files_view = child_item(window, "dryRunFilesListView");
    QObject* cmd_act = child_object(window, "showDryRunCommandAction");
    SMOKE_CHECK5("M2", 233, files_view, files_view->isVisible(), files_view->property("count").toInt() >= 1, cmd_act, cmd_act->property("visible").toBool(),
        "dry-run files list or Show Command action missing");
    smoke_finish("M2", warnings.get(), window, out_path);
}

// m2-retention-editor-preview-confirm
void scene_retention_editor_preview_confirm(const Scene& s) {
    QQuickWindow* window = s.window;
    const auto& warnings = s.warnings;
    const QString& out_path = s.out_path;
    const QString bid = backup_id_at(window, 0);
    SMOKE_CHECK("M2", 240, !bid.isEmpty(), "no backup found");
    QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
    QMetaObject::invokeMethod(window, "openBackupRetentionPage", Q_ARG(QVariant, bid));
    wait_ms(200);

    QQuickItem* ret_page = child_item(window, "backupRetentionPage");
    QQuickItem* preset_combo = child_item(window, "retentionPresetCombo");
    QQuickItem* semantics_info = child_item(window, "retentionSemanticsInfo");
    SMOKE_CHECK4("M2", 241, ret_page, preset_combo, semantics_info, semantics_info->isVisible(), "BackupRetentionPage controls missing");

    // AC-M2-06-8: clean Escape closes; dirty Escape prompts.
    press_and_wait(window, Qt::Key_Escape, 150);
    SMOKE_CHECK("M2", 242, !child_item(window, "backupRetentionPage"),
        "clean Escape did not close BackupRetentionPage");
    QMetaObject::invokeMethod(window, "openBackupRetentionPage", Q_ARG(QVariant, bid));
    wait_ms(200);
    ret_page = child_item(window, "backupRetentionPage");
    SMOKE_CHECK("M2", 243, ret_page, "could not reopen BackupRetentionPage");

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
    SMOKE_CHECK("M2", 244, ret_page->property("dirty").toBool(), "BackupRetentionPage did not become dirty after changing rules");
    press_and_wait(window, Qt::Key_Escape, 120);
    QObject* unsaved_dlg = child_object(window, "unsavedRetentionDialog");
    SMOKE_CHECK_OPENED("M2", 245, unsaved_dlg, "dirty Escape did not open unsavedRetentionDialog");
    press_and_wait(window, Qt::Key_Escape, 200);

    // AC-M2-06-2: Open Retention Preview page.
    QObject* prev_act = child_object(window, "previewRetentionAction");
    SMOKE_CHECK("M2", 246, prev_act, "previewRetentionAction missing");
    trigger(prev_act);
    wait_ms(100);
    QQuickItem* prev_page = child_item(window, "backupPreviewPage");
    SMOKE_CHECK2("M2", 247, prev_page, prev_page->property("mode").toString() == QLatin1String("retention"), "retention preview page missing");
    wait_until([&] { return prev_page->property("retentionPreviewState").toString() != QLatin1String("running"); }, 6000);
    QQuickItem* ret_banner = child_item(window, "retentionPreviewSummaryBanner");
    SMOKE_CHECK3("M2", 247, prev_page->property("retentionPreviewState").toString() == QLatin1String("done"), ret_banner, ret_banner->isVisible(),
        "retention preview summary banner missing or preview did not complete");
    // AC-M2-06-2: (a) the summary counts kept and removed snapshots, (b) a kept snapshot names
    // its reason in the GUI language, (c) each snapshot shows its local date and time.
    const bool de = s.lang == QLatin1String("de");
    const QString summary = ret_banner->property("text").toString();
    SMOKE_CHECK("M2", 253, summary == (de ? QStringLiteral("1 Snapshot behalten · 2 Snapshots würden entfernt")
                                         : QStringLiteral("1 snapshot kept · 2 snapshots would be removed")),
        QStringLiteral("(a) unexpected retention preview summary '%1'").arg(summary));
    QQuickItem* kept_result = find_visual_item(window, QStringLiteral("retentionPreviewSnapshotResult"));
    SMOKE_CHECK("M2", 254, kept_result, "(b) retention preview rows missing");
    const QString kept_text = kept_result->property("text").toString();
    SMOKE_CHECK("M2", 254, kept_text == (de ? QStringLiteral("Behalten — jüngster") : QStringLiteral("Keep — latest snapshot")),
        QStringLiteral("(b) unexpected result of the kept snapshot '%1'").arg(kept_text));
    QQuickItem* kept_time = find_visual_item(window, QStringLiteral("retentionPreviewSnapshotTime"));
    SMOKE_CHECK("M2", 255, kept_time, "(c) retention preview time missing");
    const QString time_text = kept_time->property("text").toString();
    static const QRegularExpression local_time(QStringLiteral("^[0-9a-f]{8} · \\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}$"));
    SMOKE_CHECK("M2", 255, local_time.match(time_text).hasMatch(),
        QStringLiteral("(c) snapshot time is not local date and time: '%1'").arg(time_text));
    SMOKE_CHECK("M2", 10, save_screenshot(window, out_path), "could not save the screenshot");

    // Go back to BackupRetentionPage and save -> must show confirmation dialog (AC-M2-06-3).
    press_and_wait(window, Qt::Key_Escape, 200);
    QObject* save_act = child_object(window, "saveRetentionAction");
    SMOKE_CHECK("M2", 248, save_act, "saveRetentionAction missing");
    trigger(save_act);
    QObject* confirm_dlg = child_object(window, "confirmRetentionRemoveDialog");
    wait_until([&] { return confirm_dlg && confirm_dlg->property("visible").toBool(); }, 6000);
    QObject* confirm_act = child_object(window, "confirmRetentionSaveAction");
    SMOKE_CHECK4("M2", 249, confirm_dlg, confirm_dlg->property("visible").toBool(), confirm_act, ret_page->property("pendingRemoveCount").toInt() == 2,
        "confirmRetentionRemoveDialog did not open with pendingRemoveCount=2");
    trigger(confirm_act);
    wait_ms(200);
    SMOKE_CHECK("M2", 250, !ret_page->property("dirty").toBool(), "retention rules still dirty after confirming save");

    // AC-M2-06-5: Saving policy that removes 0 snapshots saves immediately without confirmation dialog.
    ret_page->setProperty("keepLastNum", 5);
    QMetaObject::invokeMethod(ret_page, "onFieldEdited");
    wait_ms(80);
    trigger(save_act);
    wait_until([&] { return !ret_page->property("saveBusy").toBool() && !ret_page->property("dirty").toBool(); }, 6000);
    SMOKE_CHECK_CLOSED("M2", 251, confirm_dlg, "confirmRetentionRemoveDialog must NOT open when 0 snapshots would be removed");
    SMOKE_CHECK("M2", 252, !ret_page->property("dirty").toBool(), "keep_last=5 did not save immediately");

    smoke_finish("M2", warnings.get());
}

// m2-keyboard-nav
void scene_keyboard_nav(const Scene& s) {
    QQuickWindow* window = s.window;
    const auto& warnings = s.warnings;
    const QString& out_path = s.out_path;
    const QString bid = backup_id_at(window, 0);
    SMOKE_CHECK("M2", 260, !bid.isEmpty(), "no backup found");
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
        SMOKE_CHECK("M2", 261, tab_until_object(window, obj, 30), QStringLiteral("Tab did not reach %1").arg(obj));
    }
    // Tab from excludesTextArea must escape without inserting '\t'
    QQuickItem* excl_area = tab_to(window, "excludesTextArea", 30);
    const QString before_txt = excl_area ? excl_area->property("text").toString() : QString();
    press_tab(window);
    SMOKE_CHECK3("M2", 262, excl_area, window->activeFocusItem() != excl_area, excl_area->property("text").toString() == before_txt,
        "Tab trapped or inserted character inside excludesTextArea");
    // Leave excludes page clean and open Retention page
    press_and_wait(window, Qt::Key_Escape, 150);

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
        SMOKE_CHECK("M2", 263, tab_until_object(window, obj, 40), QStringLiteral("Retention Tab did not reach %1").arg(obj));
    }
    // Toggle withinDailyCheck on with Return so withinDailySpin + withinDailyUnitCombo become keyboard focusable
    QQuickItem* wd_cb = window->activeFocusItem();
    SMOKE_CHECK("M2", 264, wd_cb, "active focus missing on withinDailyCheck");
    const bool wd0 = wd_cb->property("checked").toBool();
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK("M2", 265, wd_cb->property("checked").toBool() != wd0, "Return did not toggle withinDailyCheck");
    SMOKE_CHECK2("M2", 266, tab_to(window, "withinDailySpin", 10), tab_to(window, "withinDailyUnitCombo", 10),
        "Tab did not reach withinDailySpin / withinDailyUnitCombo after enabling withinDailyCheck");
    press_and_wait(window, Qt::Key_Escape, 120);
    QObject* unsaved_ret = child_object(window, "unsavedRetentionDialog");
    SMOKE_CHECK_OPENED("M2", 267, unsaved_ret, "Escape on dirty BackupRetentionPage did not open unsavedRetentionDialog");
    smoke_finish("M2", warnings.get(), window, out_path);
}

// m2-exit3-unreadable-and-add-exclude
void scene_exit3_unreadable_and_add_exclude(const Scene& s) {
    QQuickWindow* window = s.window;
    const auto& warnings = s.warnings;
    const QString& out_path = s.out_path;
    QAbstractItemModel* rm = model_named(window, "runsModel");
    SMOKE_CHECK2("M2", 270, rm, rm->rowCount() >= 1, "no run found in runsModel");
    const QString rid = rm->data(rm->index(0, 0), 0).toString();
    QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 3));
    QMetaObject::invokeMethod(window, "openRunDetailById", Q_ARG(QVariant, rid));
    wait_ms(200);

    QQuickItem* add_btn = find_visual_item(window, QStringLiteral("addUnreadableExcludeButton"));
    SMOKE_CHECK_VISIBLE("M2", 271, add_btn, "addUnreadableExcludeButton missing or not visible on Exit 3 RunDetailPage");
    SMOKE_CHECK("M2", 272, click_ui_item(window, add_btn, "Add to Exclude Rules"), "failed to click addUnreadableExcludeButton");
    wait_ms(150);
    QObject* dlg = child_object(window, "addExcludePromptDialog");
    QObject* confirm_act = child_object(window, "confirmAddExcludeAction");
    SMOKE_CHECK3("M2", 273, dlg, dlg->property("visible").toBool(), confirm_act, "addExcludePromptDialog did not open");
    SMOKE_CHECK("M2", 10, save_screenshot(window, out_path), "could not save the screenshot");
    trigger(confirm_act);
    wait_until([&] { return is_closed(dlg); }, 600);
    SMOKE_CHECK_CLOSED("M2", 274, dlg, "addExcludePromptDialog did not close after confirmAddExcludeAction");
    smoke_finish("M2", warnings.get());
}

// m2-drive-unmounted-nested-and-full-config
void scene_drive_unmounted_nested_and_full_config(const Scene& s) {
    QQuickWindow* window = s.window;
    QObject* backend = s.backend;
    const auto& warnings = s.warnings;
    const QString& out_path = s.out_path;
    const QString& lang = s.lang;
    const QString drive_bid = backup_id_at(window, 0);
    const QString nested_bid = backup_id_at(window, 1);
    SMOKE_CHECK2("M2", 280, !drive_bid.isEmpty(), !nested_bid.isEmpty(), "expected at least 2 backups for drive + nested smoke");

    // AC-M2-09-1: Unmounted drive backup disables Back Up Now & displays reason
    QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, drive_bid));
    wait_ms(200);
    QObject* detail_act = child_object(window, "detailBackupNowAction");
    QQuickItem* detail_btn = child_item(window, "backupDetailBackUpNowButton");
    QQuickItem* reason_lbl = child_item(window, "backupDetailUnavailableReason");
    SMOKE_CHECK3("M2", 281, detail_act, detail_btn, reason_lbl, "BackupDetailPage controls missing for drive backup");
    SMOKE_CHECK3("M2", 282, !detail_act->property("enabled").toBool(), !detail_btn->isEnabled(), reason_lbl->isVisible(),
        "Back Up Now must be disabled and unavailable reason visible for unmounted drive");
    const QString reason_txt = reason_lbl->property("text").toString();
    const QString expected_reason = (lang == QLatin1String("de"))
        ? QStringLiteral("Laufwerk nicht eingehängt")
        : QStringLiteral("drive not mounted");
    SMOKE_CHECK("M2", 283, reason_txt.contains(expected_reason), QStringLiteral("unexpected unmounted drive reason '%1'").arg(reason_txt));

    // D-Bus errors are shown by their error name in the GUI language, whatever language the
    // daemon runs in: removing an unknown backup fails with `NotFound`.
    {
        bool removed = true;
        QMetaObject::invokeMethod(
            backend,
            "removeBackup",
            Qt::DirectConnection,
            Q_RETURN_ARG(bool, removed),
            Q_ARG(QString, QStringLiteral("aaaaaaaa"))
        );
        const QString not_found_text = backend->property("last_action_error").toString();
        const QString expected_not_found = (lang == QLatin1String("de"))
            ? QStringLiteral("Der Eintrag existiert nicht mehr.")
            : QStringLiteral("The item no longer exists.");
        SMOKE_CHECK("M2", 289, !removed, "removing an unknown backup must fail");
        SMOKE_CHECK("M2", 289, not_found_text == expected_not_found, QStringLiteral("unexpected NotFound error text '%1'").arg(not_found_text));
    }

    // AC-M2-10-1: Nested source warning visible on BackupDetailPage and BackupSourceEditPage
    QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, nested_bid));
    wait_ms(200);
    QQuickItem* nested_banner = find_visual_item(window, QStringLiteral("nestedSourceWarningBanner"));
    SMOKE_CHECK_VISIBLE("M2", 284, nested_banner, "nestedSourceWarningBanner not visible on BackupDetailPage");
    QMetaObject::invokeMethod(window, "openBackupSourcePage", Q_ARG(QVariant, nested_bid));
    wait_ms(200);
    QQuickItem* src_nested_banner = find_visual_item(window, QStringLiteral("sourceNestedWarningBanner"));
    SMOKE_CHECK_VISIBLE("M2", 285, src_nested_banner, "sourceNestedWarningBanner not visible on BackupSourceEditPage");

    // AC-M2-11-1 & AC-M2-11-2: Full Configuration page shows all details and NO plaintext secrets
    press_and_wait(window, Qt::Key_Escape, 120);
    QMetaObject::invokeMethod(window, "openBackupFullConfigPage", Q_ARG(QVariant, nested_bid));
    wait_ms(200);
    QQuickItem* full_page = child_item(window, "backupFullConfigPage");
    QQuickItem* dest_row = child_item(window, "fullConfigDestinationRow");
    QQuickItem* excl_row = child_item(window, "fullConfigExcludesRow");
    QQuickItem* ret_row = child_item(window, "fullConfigRetentionRow");
    SMOKE_CHECK4("M2", 286, full_page, dest_row, excl_row, ret_row, "BackupFullConfigPage summary rows missing");
    const QString dest_val = dest_row->property("rowValue").toString();
    const QString excl_val = excl_row->property("rowValue").toString();
    const QString ret_val = ret_row->property("rowValue").toString();
    SMOKE_CHECK("M2", 287, dest_val.contains(QStringLiteral("KWallet")),
        "Full Config page missing KWallet hint, expert rules, or retention summary");
    SMOKE_CHECK("M2", 287, dest_val.contains(QStringLiteral("Secret Service")), "Full Config page missing Secret Service hint");
    SMOKE_CHECK2("M2", 287, excl_val.contains(QStringLiteral("*.tmp")), !ret_val.isEmpty(),
        "Full Config page missing KWallet hint, expert rules, or retention summary");
    // Verify no plaintext password appears in any visible child label on Full Config page
    const QString repo_password = qEnvironmentVariable("KESTIC_SMOKE_REPO_PASSWORD");
    SMOKE_CHECK("M2", 288, !repo_password.isEmpty(), "KESTIC_SMOKE_REPO_PASSWORD not set");
    QList<QQuickItem*> full_items;
    collect_visual_descendants(full_page, full_items);
    for (QQuickItem* it : full_items) {
        SMOKE_CHECK("M2", 288, !it->property("text").toString().contains(repo_password),
            "plaintext password leaked on BackupFullConfigPage");
    }
    smoke_finish("M2", warnings.get(), window, out_path);
}

} // namespace

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
    const Scene scene {window, backend, warnings, out_path, lang};
    if (scen == QLatin1String("m2-excludes-simple-expert-lint")) {
        scene_excludes_simple_expert_lint(scene);
        return;
    }
    if (scen == QLatin1String("m2-dry-run-preview")) {
        scene_dry_run_preview(scene);
        return;
    }
    if (scen == QLatin1String("m2-retention-editor-preview-confirm")) {
        scene_retention_editor_preview_confirm(scene);
        return;
    }
    if (scen == QLatin1String("m2-keyboard-nav")) {
        scene_keyboard_nav(scene);
        return;
    }
    if (scen == QLatin1String("m2-exit3-unreadable-and-add-exclude")) {
        scene_exit3_unreadable_and_add_exclude(scene);
        return;
    }
    if (scen == QLatin1String("m2-drive-unmounted-nested-and-full-config")) {
        scene_drive_unmounted_nested_and_full_config(scene);
        return;
    }

    smoke_fail("M2", 299, QStringLiteral("unknown m2 smoke scenario"));
}

} // namespace kestic::smoke
