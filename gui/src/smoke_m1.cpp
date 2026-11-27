// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_helper.h"
#include "src/smoke_common.h"
#include <QColor>
#include <QElapsedTimer>
#include <functional>
#include <QFile>
#include <QLocale>

namespace kestic::smoke {

namespace {

// What the scripted main-window scenarios below work on.
struct Scene {
    QQuickWindow* window;
    QObject* backend;
    QQuickItem* top_nav;
    std::shared_ptr<QStringList> warnings;
    QString out_path;
};

// Walks through the empty app: Destinations -> Add Destination wizard -> Overview -> Set Up
// First Backup wizard (integrated destination flow) -> Backups -> History.
void scene_destinations_empty(const Scene& s) {
    SMOKE_CLICK("M1", 36, s.window, visible_button(s.top_nav, "Ziele|Destinations"), "Ziele tab");
    run_steps(s.window, {
        {250, [s] {
            QQuickWindow* w = s.window;
            SMOKE_CHECK("M1", 36, has_visible_text(w->contentItem(), QString::fromUtf8("Noch keine Ziele angelegt")),
                "DestinationsPage empty PlaceholderMessage not visible");
            QQuickItem* add_btn = visible_button(w->contentItem(), "Ziel hinzufügen|Add Destination");
            SMOKE_CHECK("M1", 36, add_btn, "visible Add Destination button not found");
            SMOKE_CHECK("M1", 10, save_screenshot(w, s.out_path), "could not save the screenshot");
            SMOKE_CLICK("M1", 37, w, add_btn, "Placeholder Add Destination");
        }},
        {350, [s] {
            QQuickWindow* w = s.window;
            QQuickItem* wiz = child_item(w, "addDestinationWizard");
            SMOKE_CHECK5("M1", 37, wiz, wiz->isVisible(), w->property("inWizardLayer").toBool(), !s.top_nav->isVisible(),
                wiz->width() >= w->width() - 20, "AddDestinationWizard must open full-width without tab bar");
            SMOKE_CLICK("M1", 37, w, visible_button(wiz, "Abbrechen|Cancel"), "Cancel AddDestinationWizard");
        }},
        {350, [s] { SMOKE_CLICK("M1", 38, s.window, visible_button(s.top_nav, "Übersicht|Overview"), "Overview tab"); }},
        {250, [s] { SMOKE_CLICK("M1", 38, s.window, child_item(s.window, "setupFirstBackupButton"), "Set Up First Backup"); }},
        {350, [s] {
            QQuickWindow* w = s.window;
            QQuickItem* nbw = child_item(w, "newBackupWizard");
            SMOKE_CHECK5("M1", 39, nbw, w->property("inWizardLayer").toBool(), !s.top_nav->isVisible(),
                nbw->property("integratedDestFlow").toBool(), nbw->width() >= w->width() - 20,
                "Setup First Backup must open full-width with integrated destination flow");
            SMOKE_CLICK("M1", 39, w, child_item(w, "wizBackupNextButton"), "First Backup Step 1 Next");
        }},
        {80, [s] {
            QQuickWindow* w = s.window;
            QQuickItem* nbw = child_item(w, "newBackupWizard");
            SMOKE_CHECK2("M1", 40, !child_item(w, "addDestinationWizard"), nbw->property("currentStage").toString() == QLatin1String("dest_type"),
                "expected inline dest_type stage without nested AddDestinationWizard");
            SMOKE_CLICK("M1", 40, w, child_item(w, "wizBackupNextButton"), "First Backup Step 2 Next");
        }},
        {80, [s] {
            QQuickWindow* w = s.window;
            QQuickItem* nbw = child_item(w, "newBackupWizard");
            QQuickItem* dest_name_field = child_item(w, "wizBackupDestNameField");
            SMOKE_CHECK3("M1", 40, nbw->property("currentStage").toString() == QLatin1String("dest_details"), dest_name_field,
                dest_name_field->isVisible(), "expected inline dest_details stage in First Backup wizard");
            SMOKE_CLICK("M1", 40, w, visible_button(nbw, "Abbrechen|Cancel"), "Cancel First Backup Wizard");
        }},
        {350, [s] { SMOKE_CLICK("M1", 41, s.window, visible_button(s.top_nav, "Backups"), "Backups tab"); }},
        {250, [s] {
            SMOKE_CHECK("M1", 41, has_visible_text(s.window->contentItem(), QString::fromUtf8("Noch keine Backups angelegt")),
                "BackupsPage empty PlaceholderMessage not visible");
            SMOKE_CLICK("M1", 42, s.window, visible_button(s.top_nav, "Verlauf|History"), "History tab");
        }},
        {250, [s] {
            SMOKE_CHECK("M1", 42, has_visible_text(s.window->contentItem(), QString::fromUtf8("Keine passenden Läufe gefunden")),
                "HistoryPage empty PlaceholderMessage not visible");
            smoke_finish("M1", s.warnings.get());
        }},
    });
}

// Adds a local destination with the wizard, a backup to it, runs the backup with real restic.
void scene_e2e_wizard_backup(const Scene& s) {
    const QString repo_dir = qEnvironmentVariable("KESTIC_SMOKE_REPO_DIR");
    const QString src_dir = qEnvironmentVariable("KESTIC_SMOKE_SRC_DIR");
    SMOKE_CHECK("E2E", 50, !repo_dir.isEmpty(), "KESTIC_SMOKE_REPO_DIR not set");
    SMOKE_CHECK("E2E", 50, !src_dir.isEmpty(), "KESTIC_SMOKE_SRC_DIR not set");
    const QString repo_password = qEnvironmentVariable("KESTIC_SMOKE_REPO_PASSWORD");
    SMOKE_CHECK("E2E", 50, !repo_password.isEmpty(), "KESTIC_SMOKE_REPO_PASSWORD not set");
    SMOKE_CLICK("E2E", 51, s.window, visible_button(s.top_nav, "Destinations"), "E2E Destinations tab");
    run_steps(s.window, {
        {250, [s] { SMOKE_CLICK("E2E", 52, s.window, visible_button(s.window->contentItem(), "Add Destination"), "E2E Add Destination"); }},
        {350, [s] { SMOKE_CLICK("E2E", 53, s.window, child_item(s.window, "wizDestNextButton"), "E2E Dest Step 1 Next"); }},
        {80, [s, repo_dir] {
            QQuickWindow* w = s.window;
            QQuickItem* next_btn = child_item(w, "wizDestNextButton");
            SMOKE_ENTER_TEXT("E2E", 54, w, child_item(w, "wizDestNameField"), QStringLiteral("E2E Local Repo"), "Dest Name");
            SMOKE_ENTER_TEXT("E2E", 54, w, child_item(w, "wizDestLocationField"), repo_dir, "Dest Location");
            SMOKE_CHECK("E2E", 55, !next_btn->isEnabled(), "Next must be disabled before connection test");
            SMOKE_CLICK("E2E", 56, w, child_item(w, "wizDestTestButton"), "E2E Test Connection");
            SMOKE_CHECK("E2E", 57, next_btn->isEnabled(), "Test Connection did not enable Next");
            SMOKE_CLICK("E2E", 57, w, next_btn, "E2E Dest Step 2 Next");
        }},
        {80, [s, repo_password] {
            QQuickWindow* w = s.window;
            SMOKE_ENTER_TEXT("E2E", 58, w, child_item(w, "wizDestPasswordField"), repo_password, "Repo Password");
            SMOKE_ENTER_TEXT("E2E", 58, w, child_item(w, "wizDestPasswordConfirmField"), repo_password, "Repeat Password");
            SMOKE_CLICK("E2E", 59, w, child_item(w, "wizDestNextButton"), "E2E Save Destination");
        }},
        {400, [s] {
            QQuickWindow* w = s.window;
            SMOKE_CHECK("E2E", 60, !child_object(w, "addDestinationWizard"), "AddDestinationWizard did not close after Save Destination");
            auto* dm = model_named(w, "destinationsModel");
            SMOKE_CHECK("E2E", 61, dm, "destinationsModel missing");
            SMOKE_CHECK("E2E", 61, dm->rowCount() == 1, "destinationsModel must hold exactly one destination");
            SMOKE_CHECK("E2E", 61, role_value(dm, 0, "name").toString() == QLatin1String("E2E Local Repo"),
                "destinationsModel does not contain E2E Local Repo");
            SMOKE_CLICK("E2E", 62, w, visible_button(s.top_nav, "Backups"), "E2E Backups tab");
        }},
        {250, [s] { SMOKE_CLICK("E2E", 63, s.window, visible_button(s.window->contentItem(), "New Backup"), "E2E New Backup"); }},
        {350, [s, src_dir] {
            QQuickWindow* w = s.window;
            SMOKE_ENTER_TEXT("E2E", 64, w, child_item(w, "wizBackupSourceField"), src_dir, "Backup Source");
            SMOKE_CLICK("E2E", 64, w, child_item(w, "wizBackupNextButton"), "E2E Backup Step 1 Next");
        }},
        {80, [s] {
            QQuickWindow* w = s.window;
            QQuickItem* nbw = child_item(w, "newBackupWizard");
            SMOKE_CHECK2("E2E", 65, nbw, has_visible_text(nbw, QStringLiteral("E2E Local Repo")), "E2E Local Repo not listed in NewBackupWizard");
            SMOKE_CLICK("E2E", 66, w, child_item(w, "wizBackupNextButton"), "E2E Backup Step 2 Next");
        }},
        {80, [s] {
            QQuickWindow* w = s.window;
            SMOKE_CLICK("E2E", 67, w, child_item(w, "wizBackupStartNowButton"), "E2E Back Up Now");
            poll_backup_run(w, "E2E", 68, 71, [s] {
                run_steps(s.window, {
                    {0, [s] { SMOKE_CLICK("E2E", 69, s.window, visible_button(s.top_nav, "Overview"), "E2E Overview tab"); }},
                    {250, [s] {
                        QQuickItem* ov = child_item(s.window, "overviewContent");
                        SMOKE_CHECK_VISIBLE("E2E", 70, ov, "overviewContent not visible after backup");
                        smoke_finish("E2E", s.warnings.get(), s.window, s.out_path);
                    }},
                });
            });
        }},
    });
}

// The first-backup wizard of the empty app with its integrated destination flow, then a real
// restic run.
void scene_e2e_first_backup(const Scene& s) {
    const QString repo_dir = qEnvironmentVariable("KESTIC_SMOKE_REPO_DIR");
    const QString src_dir = qEnvironmentVariable("KESTIC_SMOKE_SRC_DIR");
    SMOKE_CHECK("E2E first-backup", 80, !repo_dir.isEmpty(), "KESTIC_SMOKE_REPO_DIR not set");
    SMOKE_CHECK("E2E first-backup", 80, !src_dir.isEmpty(), "KESTIC_SMOKE_SRC_DIR not set");
    const QString repo_password = qEnvironmentVariable("KESTIC_SMOKE_REPO_PASSWORD");
    SMOKE_CHECK("E2E first-backup", 80, !repo_password.isEmpty(), "KESTIC_SMOKE_REPO_PASSWORD not set");
    SMOKE_CLICK("E2E first-backup", 81, s.window, child_item(s.window, "setupFirstBackupButton"), "E2E First Backup Welcome Button");
    run_steps(s.window, {
        {350, [s, src_dir] {
            QQuickWindow* w = s.window;
            QQuickItem* nbw = child_item(w, "newBackupWizard");
            SMOKE_CHECK2("E2E first-backup", 82, nbw, nbw->property("integratedDestFlow").toBool(), "wizard must use the integrated destination flow");
            SMOKE_ENTER_TEXT("E2E first-backup", 82, w, child_item(w, "wizBackupSourceField"), src_dir, "First Backup Source");
            SMOKE_CLICK("E2E first-backup", 82, w, child_item(w, "wizBackupNextButton"), "First Backup Step 1 Next");
        }},
        {80, [s] { SMOKE_CLICK("E2E first-backup", 83, s.window, child_item(s.window, "wizBackupNextButton"), "First Backup Step 2 Next"); }},
        {80, [s, repo_dir] {
            QQuickWindow* w = s.window;
            QQuickItem* next_btn = child_item(w, "wizBackupNextButton");
            SMOKE_ENTER_TEXT("E2E first-backup", 84, w, child_item(w, "wizBackupDestNameField"), QStringLiteral("First Local Repo"), "First Dest Name");
            SMOKE_ENTER_TEXT("E2E first-backup", 84, w, child_item(w, "wizBackupDestLocationField"), repo_dir, "First Dest Location");
            SMOKE_CLICK("E2E first-backup", 84, w, child_item(w, "wizBackupDestTestButton"), "First Dest Test Connection");
            SMOKE_CHECK("E2E first-backup", 84, next_btn->isEnabled(), "Test Connection did not enable Next");
            SMOKE_CLICK("E2E first-backup", 84, w, next_btn, "First Backup Step 3 Next");
        }},
        {80, [s, repo_password] {
            QQuickWindow* w = s.window;
            SMOKE_ENTER_TEXT("E2E first-backup", 85, w, child_item(w, "wizBackupDestPasswordField"), repo_password, "First Repo Password");
            SMOKE_ENTER_TEXT("E2E first-backup", 85, w, child_item(w, "wizBackupDestPasswordConfirmField"), repo_password, "First Repeat Password");
            SMOKE_CLICK("E2E first-backup", 85, w, child_item(w, "wizBackupNextButton"), "First Backup Step 4 Next");
        }},
        {80, [s] {
            QQuickWindow* w = s.window;
            SMOKE_CLICK("E2E first-backup", 86, w, child_item(w, "wizBackupStartNowButton"), "First Backup Back Up Now");
            poll_backup_run(w, "E2E first-backup", 87, 89, [s] {
                QQuickItem* ov = child_item(s.window, "overviewContent");
                SMOKE_CHECK_VISIBLE("E2E first-backup", 88, ov, "overviewContent not visible");
                smoke_finish("E2E first-backup", s.warnings.get(), s.window, s.out_path);
            });
        }},
    });
}

// Re-initialises a missing repository from the destination editor, then opens the nested
// Add Destination wizard from the New Backup wizard.
void scene_e2e_edit_reinit(const Scene& s) {
    QQuickWindow* w = s.window;
    QQuickItem* edit_page = child_item(w, "destinationEditPage");
    SMOKE_CHECK("E2E edit-reinit", 90, edit_page, "DestinationEditPage missing");
    SMOKE_CLICK("E2E edit-reinit", 90, w, child_item(w, "destEditTestButton"), "Edit Dest Test Connection");
    const QString kind = s.backend->property("connection_test_error_kind").toString();
    SMOKE_CHECK("E2E edit-reinit", 91, !s.backend->property("connection_test_ok").toBool(),
        "expected the connection test to fail for a missing repository");
    SMOKE_CHECK("E2E edit-reinit", 91, kind == QLatin1String("repository_missing"),
        QStringLiteral("expected repository_missing, got kind='%1'").arg(kind));
    QObject* switch_init_act = child_object(w, "destEditSwitchToInitAction");
    SMOKE_CHECK_VISIBLE("E2E edit-reinit", 92, switch_init_act, "destEditSwitchToInitAction missing or not visible");
    trigger(switch_init_act);
    QCoreApplication::processEvents();
    SMOKE_CHECK("E2E edit-reinit", 93, s.backend->property("connection_test_ok").toBool(), "switching to init did not pass connection test");
    SMOKE_CHECK("E2E edit-reinit", 93, edit_page->property("setupMode").toString() == QLatin1String("init"),
        "switching to init did not change setupMode to init");
    QObject* save_act = child_object(w, "saveDestinationAction");
    SMOKE_CHECK_ENABLED("E2E edit-reinit", 94, save_act, "saveDestinationAction missing or not enabled after init test");
    trigger(save_act);
    run_steps(w, {
        {350, [s] { QMetaObject::invokeMethod(s.window, "openNewBackupWizard", Q_ARG(QVariant, false)); }},
        {350, [s] {
            QQuickWindow* win = s.window;
            QQuickItem* nbw = child_item(win, "newBackupWizard");
            SMOKE_CHECK2("E2E nested-wizard", 95, nbw, !nbw->property("integratedDestFlow").toBool(), "wizard must not use the integrated destination flow");
            SMOKE_CLICK("E2E nested-wizard", 95, win, child_item(win, "wizBackupNextButton"), "Nested Wizard Step 1 Next");
        }},
        {120, [s] {
            QQuickWindow* win = s.window;
            QQuickItem* add = visible_button(child_item(win, "newBackupWizard"), "Add New Destination|Neues Ziel hinzufügen");
            SMOKE_CLICK("E2E nested-wizard", 96, win, add, "Nested Add New Destination");
        }},
        {350, [s] {
            QQuickWindow* win = s.window;
            QQuickItem* add_wiz = child_item(win, "addDestinationWizard");
            QQuickItem* dest_next = child_item(win, "wizDestNextButton");
            SMOKE_CHECK3("E2E nested-wizard", 97, add_wiz, add_wiz->isVisible(), dest_next, "nested AddDestinationWizard or wizDestNextButton missing");
            SMOKE_CHECK("E2E nested-wizard", 97, add_wiz->width() >= win->width() - 20,
                QStringLiteral("nested AddDestinationWizard not full-width (w=%1 win=%2)").arg(add_wiz->width()).arg(win->width()));
            const QPointF c = dest_next->mapToScene(QPointF(dest_next->width() / 2.0, dest_next->height() / 2.0));
            const QString clipped = QStringLiteral("wizDestNextButton clipped at (%1,%2) in (%3x%4)")
                                        .arg(c.x()).arg(c.y()).arg(win->width()).arg(win->height());
            SMOKE_CHECK4("E2E nested-wizard", 98, c.x() >= 0.0, c.x() <= win->width(), c.y() >= 0.0, c.y() <= win->height(), clipped);
            smoke_finish("E2E nested-wizard", s.warnings.get(), win, s.out_path);
        }},
    });
}

// Keyboard-only walk through every page, dialog and wizard (NFR-09, `AC-M0-11-*`).
void scene_keyboard_navigation(const Scene& s) {
    QQuickWindow* window = s.window;
    QObject* backend = s.backend;
    const auto& warnings = s.warnings;
    const QString& out_path = s.out_path;

    // 1. OverviewPage -> Tab reaches overviewBackupCard -> Return opens BackupDetailPage
    QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 0));
    QCoreApplication::processEvents();
    QQuickItem* card = tab_to(window, "overviewBackupCard", 20);
    SMOKE_CHECK("M1 keyboard", 101, card, "Tab did not reach overviewBackupCard on OverviewPage");
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK("M1 keyboard", 102, child_item(window, "backupDetailPage"),
        "Return on overviewBackupCard did not open BackupDetailPage");

    // 2. BackupDetailPage -> Tab reaches all 5 rows -> Return on showSnapshotsButton opens BackupSnapshotsPage -> Esc closes
    const QStringList detail_rows = {
        QStringLiteral("showSnapshotsButton"),
        QStringLiteral("editSourceRow"),
        QStringLiteral("editExcludesRow"),
        QStringLiteral("editDestinationRow"),
        QStringLiteral("viewFullConfigRow"),
    };
    for (const QString& row_name : detail_rows) {
        SMOKE_CHECK("M1 keyboard", 103, tab_until_object(window, row_name, 25),
            QStringLiteral("BackupDetailPage row missing from Tab chain: %1").arg(row_name));
    }
    QQuickItem* snap_btn = tab_to(window, "showSnapshotsButton", 25);
    SMOKE_CHECK("M1 keyboard", 104, snap_btn, "Could not Tab back to showSnapshotsButton");
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK("M1 keyboard", 105, child_item(window, "backupSnapshotsPage"),
        "Return on showSnapshotsButton did not open BackupSnapshotsPage");
    press_and_wait(window, Qt::Key_Escape, 100);
    SMOKE_CHECK("M1 keyboard", 106, !child_item(window, "backupSnapshotsPage"),
        "Esc did not close BackupSnapshotsPage");

    // 3. BackupExcludesEditPage -> Simple Mode CheckBox toggles once on Return, dirty Esc -> unsaved dialog -> Esc, then Expert Mode TextArea Tab escape
    QQuickItem* excl_row = tab_to(window, "editExcludesRow", 25);
    SMOKE_CHECK("M1 keyboard", 107, excl_row, "Could not Tab to editExcludesRow");
    send_key(window, Qt::Key_Return);
    QQuickItem* excl_page = child_item(window, "backupExcludesEditPage");
    SMOKE_CHECK("M1 keyboard", 108, excl_page, "Return on editExcludesRow did not open BackupExcludesEditPage");
    QQuickItem* cache_cb = tab_to(window, "excludeCachesCheckbox", 20);
    SMOKE_CHECK("M1 keyboard", 112, cache_cb, "Tab did not reach excludeCachesCheckbox");
    const bool cb0 = cache_cb->property("checked").toBool();
    send_key(window, Qt::Key_Return);
    const bool cb1 = cache_cb->property("checked").toBool();
    SMOKE_CHECK("M1 keyboard", 113, cb1 != cb0, "Return did not toggle excludeCachesCheckbox (or double-toggled)");
    SMOKE_CHECK("M1 keyboard", 114, excl_page->property("dirty").toBool(),
        "BackupExcludesEditPage did not mark dirty after checkbox toggle");
    press_and_wait(window, Qt::Key_Escape, 120);
    QObject* unsaved_dlg = child_object(window, "unsavedExcludesDialog");
    SMOKE_CHECK_OPENED("M1 keyboard", 115, unsaved_dlg, "Esc on dirty BackupExcludesEditPage did not open unsavedExcludesDialog");
    press_and_wait(window, Qt::Key_Escape, 300);
    SMOKE_CHECK_CLOSED("M1 keyboard", 116, unsaved_dlg, "Esc did not close unsavedExcludesDialog");
    // Restore checkbox to clean state
    cache_cb->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK2("M1 keyboard", 117, cache_cb->property("checked").toBool() == cb0, !excl_page->property("dirty").toBool(),
        "Second Return did not restore excludeCachesCheckbox to clean state");
    // Enable Expert Mode temporarily to verify excludesTextArea Tab-escape
    const bool prev_expert_mode = backend->property("expert_mode").toBool();
    if (!prev_expert_mode) {
        QMetaObject::invokeMethod(backend, "updateExpertMode", Q_ARG(bool, true));
        SMOKE_CHECK("M1 keyboard", 163, wait_until([&] { return backend->property("expert_mode").toBool(); }, 15000),
            "Expert mode did not switch on");
    }
    QQuickItem* excl_area = tab_to(window, "excludesTextArea", 20);
    SMOKE_CHECK("M1 keyboard", 109, excl_area, "Tab did not reach excludesTextArea");
    const QString excl_before = excl_area->property("text").toString();
    press_tab(window);
    SMOKE_CHECK("M1 keyboard", 110, window->activeFocusItem() != excl_area, "Tab remained trapped inside excludesTextArea");
    SMOKE_CHECK("M1 keyboard", 111, excl_area->property("text").toString() == excl_before,
        "Tab inserted a character into excludesTextArea");
    if (!prev_expert_mode) {
        QMetaObject::invokeMethod(backend, "updateExpertMode", Q_ARG(bool, false));
        SMOKE_CHECK("M1 keyboard", 164, wait_until([&] { return !backend->property("expert_mode").toBool(); }, 15000),
            "Expert mode did not switch off");
    }
    press_and_wait(window, Qt::Key_Escape, 120);
    SMOKE_CHECK("M1 keyboard", 118, !child_item(window, "backupExcludesEditPage"),
        "Clean Esc did not close BackupExcludesEditPage");

    // 4. NavigationTabButton Return + HistoryPage Issues Chip single toggle + RunDetailPage tabs & TextArea & Show Command Esc
    QQuickItem* hist_tab = tab_to_text(window, "History", 25);
    SMOKE_CHECK("M1 keyboard", 119, hist_tab, "Tab did not reach History NavigationTabButton");
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK("M1 keyboard", 120, window->property("activeTab").toInt() == 3,
        "Return on History NavigationTabButton did not switch activeTab to 3");
    QQuickItem* issues_chip = tab_to(window, "historyIssuesChip", 25);
    SMOKE_CHECK("M1 keyboard", 121, issues_chip, "Tab did not reach historyIssuesChip");
    const bool chip0 = issues_chip->property("checked").toBool();
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK("M1 keyboard", 122, issues_chip->property("checked").toBool() != chip0,
        "Return did not toggle historyIssuesChip (or double-toggled)");
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK("M1 keyboard", 123, issues_chip->property("checked").toBool() == chip0,
        "Second Return did not restore historyIssuesChip");
    QQuickItem* run_row = tab_to(window, "runListItem", 25);
    SMOKE_CHECK("M1 keyboard", 124, run_row, "Tab did not reach runListItem on HistoryPage");
    send_key(window, Qt::Key_Return);
    SMOKE_CHECK("M1 keyboard", 125, child_item(window, "runDetailPage"),
        "Return on runListItem did not open RunDetailPage");
    // The selected row must use the selection text colour, or it is unreadable on a
    // light selection in the dark theme.
    {
        QQuickItem* title = child_item(run_row, "runRowTitle");
        SMOKE_CHECK3("M1 keyboard", 155, title, run_row->property("highlighted").toBool(), title->property("color").value<QColor>() == run_row->property("selectedTextColor").value<QColor>(),
            "selected run row title does not use the selection text colour");
    }

    // RunDetailPage TabBar arrow navigation + Tab 1 Log controls + Tab 2 Commands TextArea escape + Show Command Esc
    QQuickItem* first_subtab = tab_to_text(window, "Result", 25);
    QObject* tab_bar = child_object(window, "runDetailTabBar");
    SMOKE_CHECK2("M1 keyboard", 126, first_subtab, tab_bar, "Tab did not reach Result TabButton on RunDetailPage");
    send_key(window, Qt::Key_Right);
    SMOKE_CHECK("M1 keyboard", 127, tab_bar->property("currentIndex").toInt() == 1,
        "Right arrow on RunDetail TabBar did not switch to Log tab (1)");
    SMOKE_CHECK4("M1 keyboard", 128, tab_to(window, "runLogSearchField", 10), tab_to(window, "runLogFilterCombo", 10), tab_to_text(window, "Copy Log", 10), tab_to_text(window, "Open Log File", 10),
        "Tab chain on RunDetail Log tab did not reach all toolbar controls");
    // Next Tab enters runLogsListView item
    QQuickItem* logs_view = child_item(window, "runLogsListView");
    press_tab(window);
    if (logs_view && logs_view->isVisible()) {
        QQuickItem* f = window->activeFocusItem();
        bool inside_logs = false;
        for (QQuickItem* p = f; p; p = p->parentItem()) {
            if (p == logs_view) {
                inside_logs = true;
                break;
            }
        }
        SMOKE_CHECK("M1 keyboard", 129, inside_logs, "Tab from Open Log File did not enter runLogsListView");
    }
    SMOKE_CHECK("M1 keyboard", 130, tab_to_text(window, "Log", 25), "Could not Tab back to Log TabButton");
    send_key(window, Qt::Key_Right);
    SMOKE_CHECK("M1 keyboard", 131, tab_bar->property("currentIndex").toInt() == 2,
        "Right arrow on RunDetail TabBar did not switch to Commands tab (2)");
    QQuickItem* run_cmd_area = tab_to(window, "runCommandTextArea", 10);
    SMOKE_CHECK("M1 keyboard", 132, run_cmd_area, "Tab did not reach runCommandTextArea on Commands tab");
    const QString cmd_before = run_cmd_area->property("text").toString();
    press_tab(window);
    SMOKE_CHECK2("M1 keyboard", 133, window->activeFocusItem() != run_cmd_area, run_cmd_area->property("text").toString() == cmd_before,
        "Tab was trapped or inserted \\t in runCommandTextArea");

    if (QObject* show_cmd = child_object(window, "runDetailShowCommandAction")) {
        trigger(show_cmd);
        wait_ms(120);
        QObject* cmd_dlg = child_object(window, "showCommandDialog");
        SMOKE_CHECK_VISIBLE("M1 keyboard", 134, cmd_dlg, "showCommandDialog failed to open");
        press_and_wait(window, Qt::Key_Escape, 300);
        SMOKE_CHECK_CLOSED("M1 keyboard", 135, cmd_dlg, "Esc did not close showCommandDialog");
    } else {
        SMOKE_FAIL("M1 keyboard", 136, "runDetailShowCommandAction missing");
    }

    // 5. Wizards: NewBackupWizard Step 3 TextArea Tab escape + Esc, AddDestinationWizard clean & dirty Esc
    QMetaObject::invokeMethod(window, "openNewBackupWizard", Q_ARG(QVariant, false));
    wait_ms(120);
    if (QObject* nbw = child_object(window, "newBackupWizard")) {
        nbw->setProperty("stepIndex", 2);
        QCoreApplication::processEvents();
    } else {
        SMOKE_FAIL("M1 keyboard", 137, "NewBackupWizard failed to open");
    }
    QQuickItem* wiz_excl = tab_to(window, "wizBackupExcludesArea", 15);
    SMOKE_CHECK("M1 keyboard", 138, wiz_excl, "Tab did not reach wizBackupExcludesArea in NewBackupWizard Step 3");
    press_tab(window);
    SMOKE_CHECK2("M1 keyboard", 139, window->activeFocusItem() != wiz_excl, !wiz_excl->property("text").toString().contains(QLatin1Char('\t')),
        "Tab trapped or inserted \\t in wizBackupExcludesArea");
    press_and_wait(window, Qt::Key_Escape, 150);
    SMOKE_CHECK("M1 keyboard", 140, !window->property("inWizardLayer").toBool(), "Esc did not close clean NewBackupWizard");

    QMetaObject::invokeMethod(window, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QVariant()));
    wait_ms(120);
    SMOKE_CHECK("M1 keyboard", 141, window->property("inWizardLayer").toBool(), "AddDestinationWizard did not open");
    press_and_wait(window, Qt::Key_Escape, 150);
    SMOKE_CHECK("M1 keyboard", 142, !window->property("inWizardLayer").toBool(), "Esc did not close clean AddDestinationWizard");
    QMetaObject::invokeMethod(window, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QVariant()));
    wait_ms(120);
    QObject* wiz = child_object(window, "addDestinationWizard");
    SMOKE_CHECK("M1 keyboard", 157, wiz, "addDestinationWizard missing");
    wiz->setProperty("destName", QStringLiteral("Unsaved Name"));
    QCoreApplication::processEvents();
    press_and_wait(window, Qt::Key_Escape, 120);
    QObject* discard_dlg = child_object(window, "discardDestDialog");
    SMOKE_CHECK_OPENED("M1 keyboard", 143, discard_dlg, "Esc on dirty AddDestinationWizard did not open discardDestDialog");
    press_and_wait(window, Qt::Key_Escape, 300);
    SMOKE_CHECK_CLOSED("M1 keyboard", 144, discard_dlg, "Esc did not close discardDestDialog");
    QMetaObject::invokeMethod(window, "closeWizardLayer");
    wait_ms(100);

    // 6. SettingsDialog: Tab -> expertModeSwitch single toggle on Return -> resticSourceDialog -> Esc closes dialog, second Esc closes SettingsWindow
    QMetaObject::invokeMethod(window, "openSettingsWindow");
    wait_ms(120);
    auto* sw = window->findChild<QQuickWindow*>(QStringLiteral("settingsWindow"));
    SMOKE_CHECK_VISIBLE("M1 keyboard", 145, sw, "SettingsDialog failed to open");
    const bool exp0 = backend->property("expert_mode").toBool();
    QQuickItem* exp_sw = tab_to(sw, "expertModeSwitch", 10);
    SMOKE_CHECK("M1 keyboard", 146, exp_sw, "Tab did not reach expertModeSwitch in SettingsDialog");
    send_key(sw, Qt::Key_Return);
    wait_until([&] { return backend->property("expert_mode").toBool() != exp0; }, 10000);
    SMOKE_CHECK("M1 keyboard", 147, backend->property("expert_mode").toBool() != exp0,
        "Return did not toggle expertModeSwitch in SettingsDialog (or double-toggled)");
    send_key(sw, Qt::Key_Return);
    wait_until([&] { return backend->property("expert_mode").toBool() == exp0; }, 10000);
    SMOKE_CHECK("M1 keyboard", 148, backend->property("expert_mode").toBool() == exp0,
        "Second Return did not restore expertModeSwitch in SettingsDialog");
    SMOKE_CHECK("M1 keyboard", 149, tab_to(sw, "changeResticSourceButton", 10),
        "Tab did not reach changeResticSourceButton in SettingsDialog");
    press_and_wait(sw, Qt::Key_Return, 120);
    QObject* restic_dlg = child_object(sw, "resticSourceDialog");
    SMOKE_CHECK_OPENED("M1 keyboard", 150, restic_dlg, "Return on changeResticSourceButton did not open resticSourceDialog");
    press_and_wait(sw, Qt::Key_Escape, 300);
    SMOKE_CHECK3("M1 keyboard", 151, !restic_dlg->property("visible").toBool(), !restic_dlg->property("opened").toBool(), sw->isVisible(),
        "First Esc in SettingsDialog did not close resticSourceDialog while keeping SettingsDialog open");
    press_and_wait(sw, Qt::Key_Escape, 150);
    SMOKE_CHECK("M1 keyboard", 152, !sw->isVisible(), "Second Esc in SettingsDialog did not close SettingsDialog window");

    // Return to Overview and focus overviewBackupCard so the artifact screenshot also captures the visible 2px focus ring
    window->requestActivate();
    QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 0));
    QCoreApplication::processEvents();
    QPointer<QQuickItem> final_card(tab_to(window, "overviewBackupCard", 20));
    SMOKE_CHECK("M1 keyboard", 153, !final_card.isNull(), "Could not re-focus overviewBackupCard on OverviewPage");
    wait_ms(120);
    SMOKE_CHECK2("M1 keyboard", 154, !final_card.isNull(), final_card->property("activeFocus").toBool(),
        "overviewBackupCard lost activeFocus across idle state sync");

    smoke_finish("M1 keyboard", warnings.get(), window, out_path);
}

// Largest pause of the main thread (ms) between ticks of a 20 ms timer, until `stop` is set.
std::shared_ptr<qint64> watch_main_thread_gaps(QQuickWindow* window, const std::shared_ptr<bool>& stop) {
    auto largest = std::make_shared<qint64>(0);
    auto since_tick = std::make_shared<QElapsedTimer>();
    since_tick->start();
    auto* ticker = new QTimer(window);
    QObject::connect(ticker, &QTimer::timeout, window, [=]() {
        *largest = std::max(*largest, since_tick->restart());
        if (*stop) {
            ticker->stop();
        }
    });
    ticker->start(20);
    return largest;
}

// AC-M6-33-3: the daemon takes 6 s to start; the window is up long before and says so.
void run_daemon_starting_scene(
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& out_path,
    const QString& lang,
    qint64 since_load_ms
) {
    SMOKE_CHECK("M6", 1, since_load_ms < 3000,
        QStringLiteral("(a) the window answered %1 ms after loading: the main thread waited for the daemon").arg(since_load_ms));
    const QString kind = backend->property("blocking_kind").toString();
    SMOKE_CHECK("M6", 2, kind == QLatin1String("daemon_starting"),
        QStringLiteral("(b) expected blocking_kind 'daemon_starting', got '%1'").arg(kind));
    const QString expected = lang == QLatin1String("de")
        ? QStringLiteral("Hintergrunddienst startet…")
        : QStringLiteral("Starting the background service…");
    const QString shown = backend->property("blocking_message").toString();
    SMOKE_CHECK("M6", 3, shown == expected,
        QStringLiteral("(b) expected '%1', got '%2'").arg(expected, shown));
    QQuickItem* banner = child_item(window, "blockingStatusBanner");
    SMOKE_CHECK_VISIBLE("M6", 4, banner, "(b) the banner shows that the service starts");
    QQuickItem* welcome = child_item(window, "welcomeView");
    SMOKE_CHECK2("M6", 5, welcome, !welcome->property("visible").toBool(),
        "no welcome view before the daemon said whether a configuration exists");
    QQuickItem* placeholder = child_item(window, "stateLoadingPlaceholder");
    SMOKE_CHECK_VISIBLE("M6", 9, placeholder, "(b) the overview shows the loading state while the service starts");
    SMOKE_CHECK("M6", 6, save_screenshot(window, out_path), "screenshot of the starting state");

    auto stop = std::make_shared<bool>(false);
    auto largest_gap = watch_main_thread_gaps(window, stop);
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (backend->property("daemon_reachable").toBool()) {
            poll->stop();
            *stop = true;
            SMOKE_CHECK("M6", 7, *largest_gap < 1000,
                QStringLiteral("(a) the main thread paused %1 ms while the daemon started").arg(*largest_gap));
            smoke_finish("M6", warnings.get(), nullptr, out_path);
        } else if (waited->elapsed() > 30000) {
            poll->stop();
            smoke_fail("M6", 8, QStringLiteral("the daemon the GUI started never became reachable"));
        } else if (backend->property("blocking_kind").toString() != QLatin1String("daemon_starting")) {
            poll->stop();
            smoke_fail("M6", 10, QStringLiteral("(b) after %1 ms the window no longer says that the service starts: '%2'")
                .arg(waited->elapsed()).arg(backend->property("blocking_message").toString()));
        }
    });
    poll->start(100);
}

// AC-M6-33-5: building the shown command of a backup does not run `restic version` (held by
// the test for 8 s) in the main thread.
void run_transparent_command_scene(
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& out_path
) {
    const QString bid = backup_id_at(window, 0);
    SMOKE_CHECK("M6", 20, !bid.isEmpty(), "precondition: one backup");
    QString command;
    QElapsedTimer took;
    took.start();
    QMetaObject::invokeMethod(backend, "getBackupTransparentCommand",
        Q_RETURN_ARG(QString, command), Q_ARG(QString, bid));
    const qint64 ms = took.elapsed();
    SMOKE_CHECK("M6", 21, ms < 500,
        QStringLiteral("(a) getBackupTransparentCommand took %1 ms in the main thread").arg(ms));
    SMOKE_CHECK("M6", 22, command.contains(QStringLiteral("restic-wrapper.sh")),
        QStringLiteral("the command names the configured restic: %1").arg(command));
    smoke_finish("M6", warnings.get(), window, out_path);
}

// Calls `method` on the backend and fails the scene unless it returns within 100 ms (AC-M6-33-1 (a)).
bool returns_at_once(int code, const char* method, const std::function<bool()>& call) {
    QElapsedTimer took;
    took.start();
    const bool invoked = call();
    const qint64 ms = took.elapsed();
    if (!invoked) {
        smoke_fail("M6", code, QStringLiteral("could not call %1").arg(QLatin1String(method)));
        return false;
    }
    if (ms >= 100) {
        smoke_fail("M6", code, QStringLiteral("(a) %1 kept the main thread for %2 ms").arg(QLatin1String(method)).arg(ms));
        return false;
    }
    return true;
}

// AC-M6-33-1: every daemon reply comes 5 s late. The window is up at once and shows that it
// loads; the actions return at once and their results arrive later; the main thread never
// pauses longer than 100 ms meanwhile, and the window switches tabs while calls are pending.
void run_slow_daemon_scene(
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& out_path,
    qint64 since_load_ms
) {
    SMOKE_CHECK("M6", 40, since_load_ms < 3000,
        QStringLiteral("(a) the window answered %1 ms after loading: the main thread waited for the daemon").arg(since_load_ms));
    SMOKE_CHECK("M6", 41, !backend->property("daemon_reachable").toBool(),
        "precondition: the first state is still on its way");
    const QString kind = backend->property("blocking_kind").toString();
    SMOKE_CHECK("M6", 42, kind.isEmpty(),
        QStringLiteral("(b) while the first state loads, no blocking state is shown: '%1'").arg(kind));
    QQuickItem* loading = child_item(window, "stateLoadingPlaceholder");
    SMOKE_CHECK_VISIBLE("M6", 43, loading, "(b) the overview shows that it loads");
    QString loading_shot = out_path;
    loading_shot.replace(QStringLiteral("m6-slow-daemon"), QStringLiteral("m6-slow-daemon-loading"));
    SMOKE_CHECK("M6", 53, save_screenshot(window, loading_shot), "screenshot of the loading state");
    // (b) no tab claims that nothing is configured while the state loads.
    const std::vector<std::pair<int, const char*>> empty_texts = {
        {1, "backupsEmptyPlaceholder"}, {2, "destinationsEmptyPlaceholder"}, {3, "historyEmptyPlaceholder"}};
    for (const auto& [tab, name] : empty_texts) {
        QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, tab));
        QCoreApplication::processEvents();
        QQuickItem* empty = child_item(window, name);
        SMOKE_CHECK2("M6", 52, empty, !empty->property("visible").toBool(),
            QStringLiteral("(b) %1 is shown while the state loads").arg(QLatin1String(name)));
    }
    QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 0));

    auto stop = std::make_shared<bool>(false);
    auto largest_gap = watch_main_thread_gaps(window, stop);
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto stage = std::make_shared<int>(0);
    auto expert_before = std::make_shared<bool>(false);
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (waited->elapsed() > 90000) {
            poll->stop();
            smoke_fail("M6", 44, QStringLiteral("timed out in stage %1").arg(*stage));
            return;
        }
        if (*stage == 0) {
            if (!backend->property("daemon_reachable").toBool()) {
                return;
            }
            *stage = 1;
            *expert_before = backend->property("expert_mode").toBool();
            const QString bid = backup_id_at(window, 0);
            const bool ok = !bid.isEmpty()
                && returns_at_once(45, "windowActive", [&]() {
                       return QMetaObject::invokeMethod(backend, "windowActive",
                           Q_ARG(bool, true), Q_ARG(QString, QStringLiteral("overview")));
                   })
                && returns_at_once(46, "checkAvailability", [&]() {
                       return QMetaObject::invokeMethod(backend, "checkAvailability", Q_ARG(bool, false));
                   })
                && returns_at_once(47, "pauseGlobal", [&]() {
                       return QMetaObject::invokeMethod(backend, "pauseGlobal", Q_ARG(QString, QStringLiteral("1h")));
                   })
                && returns_at_once(48, "updateExpertMode", [&]() {
                       return QMetaObject::invokeMethod(backend, "updateExpertMode", Q_ARG(bool, !*expert_before));
                   })
                && returns_at_once(49, "startBackup", [&]() {
                       return QMetaObject::invokeMethod(backend, "startBackup", Q_ARG(QString, bid));
                   });
            if (!ok) {
                poll->stop();
                return;
            }
            QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 2));
            const int tab = window->property("activeTab").toInt();
            if (tab != 2) {
                poll->stop();
                smoke_fail("M6", 50, QStringLiteral("(b) the window did not switch tabs while calls were pending: %1").arg(tab));
            }
            return;
        }
        if (backend->property("global_paused").toBool()
                && backend->property("expert_mode").toBool() != *expert_before) {
            poll->stop();
            *stop = true;
            SMOKE_CHECK("M6", 51, *largest_gap <= 100,
                QStringLiteral("(a) the main thread paused %1 ms while the daemon was slow").arg(*largest_gap));
            smoke_finish("M6", warnings.get(), window, out_path);
        }
    });
    poll->start(50);
}

// Polls `done` every 50 ms for at most `limit_ms`; then `finish` runs, or the scene fails with
// `code` and `what`.
void wait_then(QQuickWindow* window, int limit_ms, int code, const QString& what,
    const std::function<bool()>& done, const std::function<void()>& finish) {
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (done()) {
            poll->stop();
            finish();
        } else if (waited->elapsed() > limit_ms) {
            poll->stop();
            smoke_fail("M6", code, what);
        }
    });
    poll->start(50);
}

// AC-M6-33-1: the quit dialog opens before the first state arrived (tray «Quit» without a
// window); the state that arrives later names the running backup.
void run_quit_running_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    wait_then(window, 30000, 60, QStringLiteral("the quit dialog never named the running backup"),
        [=]() {
            return backend->property("quit_has_running_run").toBool()
                && backend->property("quit_running_backup_name").toString() == QLatin1String("Documents");
        },
        [=]() { smoke_finish("M6", warnings.get(), window, out_path); });
}

// AC-M6-33-1: the password prompt of the daemon (UnlockSecretStore, held back 30 s) does not hold
// back the user's other actions.
void run_prompt_alone_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    wait_then(window, 30000, 61, QStringLiteral("the state never arrived"),
        [=]() { return backend->property("daemon_reachable").toBool(); },
        [=]() {
            QMetaObject::invokeMethod(backend, "unlockSecretStore");
            QMetaObject::invokeMethod(backend, "pauseGlobal", Q_ARG(QString, QStringLiteral("1h")));
            wait_then(window, 5000, 62, QStringLiteral("pausing waited for the password prompt"),
                [=]() { return backend->property("global_paused").toBool(); },
                [=]() { smoke_finish("M6", warnings.get(), window, out_path); });
        });
}

// AC-M6-33-1: the window closes while a call is still on its way; the daemon still learns that
// the window closed (the test watches for WindowActive(false)).
void run_close_window_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    wait_then(window, 30000, 63, QStringLiteral("the state never arrived"),
        [=]() { return backend->property("daemon_reachable").toBool(); },
        [=]() {
            if (!warnings->isEmpty() || !save_screenshot(window, out_path)) {
                smoke_finish("M6", warnings.get(), window, out_path);
                return;
            }
            QMetaObject::invokeMethod(backend, "checkAvailability", Q_ARG(bool, false));
            window->close();
        });
}

// AC-M6-33-1: «Back Up Now» before the first state arrived, for a backup whose source is gone:
// the refusal names the backup and the reason the daemon gave.
void run_refused_start_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    const QString bid = qEnvironmentVariable("KESTIC_TEST_BACKUP_ID");
    SMOKE_CHECK("M6", 64, !bid.isEmpty(), "precondition: backup id");
    QMetaObject::invokeMethod(backend, "startBackup", Q_ARG(QString, bid));
    wait_then(window, 30000, 65, QStringLiteral("the refusal never arrived"),
        [=]() { return !backend->property("manual_run_blocked_message").toString().isEmpty(); },
        [=]() {
            const QString shown = backend->property("manual_run_blocked_message").toString();
            if (shown != QStringLiteral("Cannot back up 'Documents': source is unavailable.")) {
                smoke_fail("M6", 66, QStringLiteral("the refusal reads '%1'").arg(shown));
                return;
            }
            smoke_finish("M6", warnings.get(), window, out_path);
        });
}

// AC-M6-33-4 (a): the test changes the daemon state; the window shows it within 1 s, with no
// timer of its own. The scene writes `<marker>.ready` when it is up; the test writes
// `<marker>.go` and then changes expert mode over D-Bus.
void run_state_change_scene(
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& out_path
) {
    const QString marker = qEnvironmentVariable("KESTIC_TEST_STATE_CHANGE_MARKER");
    SMOKE_CHECK("M6", 30, !marker.isEmpty(), "precondition: marker path");
    const bool before = backend->property("expert_mode").toBool();
    QFile ready(marker + QStringLiteral(".ready"));
    SMOKE_CHECK("M6", 31, ready.open(QIODevice::WriteOnly), "write the ready marker");
    ready.close();
    auto since_go = std::make_shared<QElapsedTimer>();
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (!since_go->isValid()) {
            if (QFile::exists(marker + QStringLiteral(".go"))) {
                since_go->start();
            } else if (waited->elapsed() > 60000) {
                poll->stop();
                smoke_fail("M6", 32, QStringLiteral("the test never changed the daemon state"));
            }
            return;
        }
        if (backend->property("expert_mode").toBool() != before) {
            poll->stop();
            const qint64 ms = since_go->elapsed();
            SMOKE_CHECK("M6", 33, ms <= 1000,
                QStringLiteral("(a) the window showed the change after %1 ms").arg(ms));
            smoke_finish("M6", warnings.get(), window, out_path);
        } else if (since_go->elapsed() > 10000) {
            poll->stop();
            smoke_fail("M6", 34, QStringLiteral("(a) the change never reached the window"));
        }
    });
    poll->start(20);
}

void run_m6_smoke_scenario(
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& scen,
    const QString& out_path,
    const QString& lang,
    qint64 since_load_ms
) {
    if (scen == QLatin1String("m6-daemon-starting")) {
        run_daemon_starting_scene(window, backend, warnings, out_path, lang, since_load_ms);
    } else if (scen == QLatin1String("m6-transparent-command")) {
        run_transparent_command_scene(window, backend, warnings, out_path);
    } else if (scen == QLatin1String("m6-state-change")) {
        run_state_change_scene(window, backend, warnings, out_path);
    } else if (scen == QLatin1String("m6-slow-daemon")) {
        run_slow_daemon_scene(window, backend, warnings, out_path, since_load_ms);
    } else if (scen == QLatin1String("m6-quit-running")) {
        run_quit_running_scene(window, backend, warnings, out_path);
    } else if (scen == QLatin1String("m6-prompt-alone")) {
        run_prompt_alone_scene(window, backend, warnings, out_path);
    } else if (scen == QLatin1String("m6-close-window")) {
        run_close_window_scene(window, backend, warnings, out_path);
    } else if (scen == QLatin1String("m6-refused-start")) {
        run_refused_start_scene(window, backend, warnings, out_path);
    } else {
        smoke_fail("M6", 99, QStringLiteral("unknown scenario %1").arg(scen));
    }
}

} // namespace

void install_m1_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str scenario,
    rust::Str screenshot_path,
    rust::Str layout_mode,
    rust::Str expected_lang
) {
    const QString scen = QString::fromUtf8(scenario.data(), static_cast<qsizetype>(scenario.size()));
    const QString out_path = QString::fromUtf8(screenshot_path.data(), static_cast<qsizetype>(screenshot_path.size()));
    const QString mode = QString::fromUtf8(layout_mode.data(), static_cast<qsizetype>(layout_mode.size()));
    const QString lang = QString::fromUtf8(expected_lang.data(), static_cast<qsizetype>(expected_lang.size()));
    // Numbers and sizes follow the formats locale; the smoke stands in for a system whose formats
    // match its language, because the test image has only the en_US glibc locale.
    QLocale::setDefault(QLocale(lang));
    auto warnings = std::make_shared<QStringList>();
    // Time since before the window loaded: a main thread that waits while loading shows here.
    auto since_load = std::make_shared<QElapsedTimer>();
    since_load->start();
    finish_incubation_on_quit(engine);

    QObject::connect(
        &engine,
        &QQmlEngine::warnings,
        &engine,
        [warnings](const QList<QQmlError>& qml_warnings) {
            for (const auto& err : qml_warnings) {
                warnings->append(err.toString());
            }
        }
    );

    QTimer::singleShot(250, &engine, [&engine, warnings, scen, out_path, mode, lang, since_load]() {
        const auto roots = engine.rootObjects();
        SMOKE_CHECK("M1", 2, !roots.isEmpty(), "no root objects loaded");
        auto* window = qobject_cast<QQuickWindow*>(roots.first());
        SMOKE_CHECK("M1", 4, window, "root object is not a QQuickWindow");
        QObject* backend = child_object(window, "appBackend");
        SMOKE_CHECK("M1", 5, backend, "appBackend not found");

        if (mode == QLatin1String("narrow")) {
            QMetaObject::invokeMethod(window, "resizeToMinWidthForSmoke");
        }

        // Perform scenario navigation setup
        if (scen == QLatin1String("backups-detail-show-cmd")) {
            QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 1));
        } else if (scen == QLatin1String("destinations-reassign")) {
            const QString bid = backup_id_at(window, 0);
            if (!bid.isEmpty()) {
                QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
                QMetaObject::invokeMethod(window, "openBackupReassignPage", Q_ARG(QVariant, bid));
            }
        } else if (scen == QLatin1String("snapshots-previous")) {
            QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 1));
            const QString bid = backup_id_at(window, 0);
            if (!bid.isEmpty()) {
                const QString bname = role_value(model_named(window, "backupsModel"), 0, "name").toString();
                QMetaObject::invokeMethod(window, "openBackupSnapshotsPage", Q_ARG(QVariant, bid), Q_ARG(QVariant, bname));
            }
        } else if (scen == QLatin1String("history-run-detail")) {
            QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 3));
            const QString rid = run_id_at(window, 0);
            if (!rid.isEmpty()) {
                QMetaObject::invokeMethod(window, "openRunDetailById", Q_ARG(QVariant, rid));
            }
        } else if (scen == QLatin1String("manual-run-blocked")) {
            const QString bid = backup_id_at(window, 0);
            if (!bid.isEmpty()) {
                QMetaObject::invokeMethod(backend, "startBackup", Q_ARG(QString, bid));
            }
        } else if (scen == QLatin1String("wizard-validation")) {
            QMetaObject::invokeMethod(window, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QVariant()));
        } else if (scen == QLatin1String("e2e-edit-reinit-and-nested-wizard")) {
            const QString did = destination_id_at(window, 0);
            if (!did.isEmpty()) {
                QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 2));
                QMetaObject::invokeMethod(window, "openDestinationEditPage", Q_ARG(QVariant, did));
            }
        } else if (scen == QLatin1String("settings")) {
            QMetaObject::invokeMethod(window, "openSettingsWindow");
        }

        // Allow async QML creations / wizard advance / snapshots load
        QTimer::singleShot(400, &engine, [&engine, window, backend, warnings, scen, out_path, mode, lang, since_load]() {
            if (!warnings->isEmpty()) {
                smoke_finish("M1", warnings.get());
                return;
            }

            SMOKE_CHECK("M1", 11, window->title() == QLatin1String("Kestic"),
                QStringLiteral("expected title 'Kestic', got '%1'").arg(window->title()));

            // Common M1 invariants across main window scenarios
            auto* top_nav = child_item(window, "topNavBar");
            auto* bottom_nav = child_item(window, "bottomNavBar");
            SMOKE_CHECK2("M1", 12, top_nav, bottom_nav, "NavigationTabBars missing");

            const bool de = lang == QLatin1String("de");
            const QString overview_label = de ? QStringLiteral("Übersicht") : QStringLiteral("Overview");
            const QString restore_label = de ? QStringLiteral("Wiederherstellen") : QStringLiteral("Restore");
            const QStringList required_tabs = de
                ? QStringList {
                    overview_label,
                    restore_label,
                    QStringLiteral("Backups"),
                    QStringLiteral("Ziele"),
                    QStringLiteral("Verlauf"),
                }
                : QStringList {
                    overview_label,
                    restore_label,
                    QStringLiteral("Backups"),
                    QStringLiteral("Destinations"),
                    QStringLiteral("History"),
                };

            const bool in_wizard = window->property("inWizardLayer").toBool();
            if (in_wizard) {
                SMOKE_CHECK2("M1", 13, !top_nav->isVisible(), !bottom_nav->isVisible(), "NavigationTabBar must be hidden inside wizard layer");
            } else if (mode == QLatin1String("narrow")) {
                SMOKE_CHECK2("M1", 13, !top_nav->isVisible(), bottom_nav->isVisible(), "expected bottomNavBar visible in narrow mode");
                SMOKE_CHECK("M1", 14, verify_tab_bar_labels(bottom_nav, 5, required_tabs), "bottomNavBar tab labels missing or truncated");
            } else {
                SMOKE_CHECK("M1", 15, verify_tab_bar_labels(top_nav, 5, required_tabs), "topNavBar tab labels missing or truncated");
            }

            // AC-M4-01-6 (a): «Restore» sits between «Overview» and «Backups» in the tab bar.
            if (!in_wizard) {
                QList<QQuickItem*> nav_items;
                collect_visual_descendants(mode == QLatin1String("narrow") ? bottom_nav : top_nav, nav_items);
                auto tab_x = [&](const QString& label) -> qreal {
                    for (QQuickItem* it : nav_items) {
                        if (it->isVisible() && it->property("truncated").isValid()
                                && it->property("text").toString() == label) {
                            return it->mapToScene(QPointF(0, 0)).x();
                        }
                    }
                    return -1;
                };
                const qreal overview_x = tab_x(overview_label);
                const qreal restore_x = tab_x(restore_label);
                const qreal backups_x = tab_x(QStringLiteral("Backups"));
                SMOKE_CHECK("M1", 16, overview_x >= 0, "Overview tab missing");
                SMOKE_CHECK("M1", 19, restore_x > overview_x, "Restore tab must follow Overview");
                SMOKE_CHECK("M1", 18, backups_x > restore_x, "Restore tab must come before Backups");
            }

            {
                QObject* hamburger = child_object(window, "hamburgerMenu");
                SMOKE_CHECK("M1", 156, hamburger, "hamburgerMenu missing");
                QList<QObject*> objs;
                collect_all_qobjects(hamburger, objs);
                const QStringList forbidden_items {
                    QStringLiteral("New Backup"),
                    QStringLiteral("Neues Backup"),
                    QStringLiteral("Add Destination"),
                    QStringLiteral("Ziel hinzufügen"),
                    QStringLiteral("Quit Kestic"),
                    QStringLiteral("Kestic beenden"),
                };
                for (QObject* o : objs) {
                    const QString txt = o->property("text").toString();
                    for (const QString& forbidden : forbidden_items) {
                        SMOKE_CHECK("M1", 17, !txt.contains(forbidden), QStringLiteral("forbidden hamburger item '%1'").arg(txt));
                    }
                }
            }

            QQuickWindow* screenshot_target = window;

            if (scen.startsWith(QLatin1String("m2-"))) {
                run_m2_smoke_scenario(engine, window, backend, warnings, scen, out_path, lang);
                return;
            } else if (scen.startsWith(QLatin1String("m3-"))) {
                run_m3_smoke_scenario(engine, window, backend, warnings, scen, out_path, lang);
                return;
            } else if (scen.startsWith(QLatin1String("m4-"))) {
                run_m4_smoke_scenario(engine, window, backend, warnings, scen, out_path, lang);
                return;
            } else if (scen.startsWith(QLatin1String("m5-"))) {
                run_m5_smoke_scenario(engine, window, backend, warnings, scen, out_path, lang);
                return;
            } else if (scen.startsWith(QLatin1String("m6-"))) {
                run_m6_smoke_scenario(window, backend, warnings, scen, out_path, lang, since_load->elapsed());
                return;
            } else if (scen == QLatin1String("welcome")) {
                QQuickItem* wv = child_item(window, "welcomeView");
                SMOKE_CHECK_VISIBLE("M1", 20, wv, "welcomeView should be visible when unconfigured");
            } else if (scen == QLatin1String("overview")) {
                QQuickItem* ov = child_item(window, "overviewContent");
                SMOKE_CHECK_VISIBLE("M1", 21, ov, "overviewContent should be visible");
            } else if (scen.startsWith(QLatin1String("blocking-"))) {
                const QString expected_kind = scen.mid(QStringLiteral("blocking-").size());
                const QString actual_kind = backend->property("blocking_kind").toString();
                SMOKE_CHECK("M1", 22, actual_kind == expected_kind,
                    QStringLiteral("expected blocking_kind='%1', got '%2'").arg(expected_kind, actual_kind));
                if (expected_kind == QLatin1String("restic_unavailable")) {
                    QObject* dl_action = child_object(window, "downloadResticAction");
                    SMOKE_CHECK_ENABLED("M1", 23, dl_action, "Download restic action must be offered while restic is unusable");
                }
            } else if (scen == QLatin1String("backups-detail-show-cmd")) {
                QObject* show_cmd = child_object(window, "backupDetailShowCommandAction");
                SMOKE_CHECK_ENABLED("M1", 24, show_cmd, "Show Command action on BackupDetailPage must be enabled even when expert_mode=false");
                trigger(show_cmd);
                QTimer::singleShot(250, &engine, [window, screenshot_target, out_path]() {
                    QObject* cmd_area = child_object(window, "showCommandTextArea");
                    SMOKE_CHECK2("M1", 25, cmd_area, cmd_area->property("text").toString().contains(QStringLiteral("RESTIC_PASSWORD_COMMAND")),
                        "Show Command dialog missing transparent command");
                    smoke_finish("M1", nullptr, screenshot_target, out_path);
                });
                return;
            } else if (scen == QLatin1String("destinations-reassign")) {
                QObject* switch_act = child_object(window, "switchDestinationAction");
                SMOKE_CHECK_DISABLED("M1", 26, switch_act, "Switch Destination must be disabled before selecting a destination");
            } else if (scen == QLatin1String("snapshots-previous")) {
                QQuickItem* snap_list = child_item(window, "snapshotsListView");
                SMOKE_CHECK("M1", 27, snap_list, "snapshotsListView not found");
                QList<QQuickItem*> snap_items;
                collect_visual_descendants(snap_list, snap_items);
                const QString earlier_header_prefix = (lang == QLatin1String("de"))
                    ? QString::fromUtf8("Frühere Snapshots auf '")
                    : QStringLiteral("Earlier snapshots on '");
                bool found_prev_header = false;
                for (QQuickItem* it : snap_items) {
                    if (it->objectName() == QLatin1String("snapshotSectionHeader")) {
                        const QString hdr = it->property("text").toString();
                        if (hdr.contains(earlier_header_prefix)) {
                            found_prev_header = true;
                            break;
                        }
                    }
                }
                SMOKE_CHECK("M1", 28, found_prev_header, "previous destination snapshots section header not found");
            } else if (scen == QLatin1String("history-run-detail")) {
                QObject* tab_bar = child_object(window, "runDetailTabBar");
                QObject* show_cmd = child_object(window, "runDetailShowCommandAction");
                SMOKE_CHECK4("M1", 29, tab_bar, tab_bar->property("count").toInt() == 3, show_cmd, show_cmd->property("enabled").toBool(),
                    "RunDetailPage 3 tabs or Show Command missing");
            } else if (scen == QLatin1String("manual-run-blocked")) {
                QQuickItem* banner = child_item(window, "manualRunBlockedBanner");
                // The refusal arrives once the daemon answered.
                wait_until([&] { return banner && banner->isVisible(); }, 10000);
                SMOKE_CHECK3("M1", 30, banner, banner->isVisible(), !backend->property("manual_run_blocked_message").toString().isEmpty(),
                    "ManualRunBlocked inline banner not shown");
            } else if (scen == QLatin1String("wizard-validation")) {
                QObject* wiz = child_object(window, "addDestinationWizard");
                QObject* next_btn = child_object(window, "wizDestNextButton");
                SMOKE_CHECK2("M1", 31, wiz, next_btn, "AddDestinationWizard not opened");
                wiz->setProperty("step", 2);
                QCoreApplication::processEvents();
                QQuickItem* name_err = child_item(window, "wizDestNameError");
                QQuickItem* loc_err = child_item(window, "wizDestLocationError");
                SMOKE_CHECK5("M1", 32, name_err, name_err->isVisible(), loc_err, loc_err->isVisible(), !next_btn->property("enabled").toBool(),
                    "wizard validation errors / disabled Next not enforced");
            } else if (scen == QLatin1String("quit-dialog")) {
                QObject* dlg = child_object(window, "quitConfirmDialog");
                SMOKE_CHECK_VISIBLE("M1", 33, dlg, "quitConfirmDialog must open when --confirm-quit is passed");
            } else if (scen == QLatin1String("settings")) {
                auto* sw = window->findChild<QQuickWindow*>(QStringLiteral("settingsWindow"));
                SMOKE_CHECK_VISIBLE("M1", 34, sw, "settingsWindow not visible");
                QObject* dl_btn = child_object(sw, "downloadResticButton");
                SMOKE_CHECK_VISIBLE("M1", 35, dl_btn, "Download restic button must be offered while restic is unusable");
                SMOKE_CHECK_ENABLED("M1", 157, dl_btn, "Download restic button must be enabled while restic is unusable");
                screenshot_target = sw;
            } else if (scen == QLatin1String("destinations-empty")) {
                scene_destinations_empty({window, backend, top_nav, warnings, out_path});
                return;
            } else if (scen == QLatin1String("e2e-wizard-backup")) {
                scene_e2e_wizard_backup({window, backend, top_nav, warnings, out_path});
                return;
            } else if (scen == QLatin1String("e2e-first-backup")) {
                scene_e2e_first_backup({window, backend, top_nav, warnings, out_path});
                return;
            } else if (scen == QLatin1String("e2e-edit-reinit-and-nested-wizard")) {
                scene_e2e_edit_reinit({window, backend, top_nav, warnings, out_path});
                return;
            } else if (scen == QLatin1String("keyboard-navigation")) {
                scene_keyboard_navigation({window, backend, top_nav, warnings, out_path});
                return;
            }

            smoke_finish("M1", nullptr, screenshot_target, out_path);
        });
    });
}

} // namespace kestic::smoke
