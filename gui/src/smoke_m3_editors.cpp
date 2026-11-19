// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_m3_util.h"

namespace kestic::smoke {

namespace {

using namespace m3;

QObject* dialog_action(QObject* dialog, const char* name) {
    return dialog ? dialog->findChild<QObject*>(QString::fromLatin1(name)) : nullptr;
}

void open_backup_detail(QQuickWindow* w, const QString& bid) {
    QMetaObject::invokeMethod(w, "openBackupDetailById", Q_ARG(QVariant, bid));
    wait_ms(250);
}

QQuickItem* open_editor(QQuickWindow* w, const char* row, const char* page) {
    QQuickItem* r = find_visual_item(w, QString::fromLatin1(row));
    if (!r || !click_ui_item(w, r, row)) {
        return nullptr;
    }
    wait_until([&] { return item_named(w, page) != nullptr; }, 3000);
    wait_ms(200);
    return item_named(w, page);
}

/// Asserts the «Save changes?» prompt `dialog` is open with the localized [Save] / [Discard]
/// buttons (AC-M3-14-9 (a)). Returns 0 when fine.
int check_unsaved_dialog_open(const Ctx& c, QObject* dialog, int base) {
    M3_CHECK(base, wait_until([&] { return is_open(dialog); }, 3000), "unsaved-changes dialog did not open");
    QObject* save = dialog_action(dialog, "unsavedSaveAction");
    QObject* discard = dialog_action(dialog, "unsavedDiscardAction");
    M3_CHECK(base + 1, save && discard, "dialog must offer [Save] and [Discard]");
    M3_CHECK_EQ(base + 2, save->property("text").toString(), loc(c.lang, "Save", "Speichern"), "Save button label");
    M3_CHECK_EQ(base + 3, discard->property("text").toString(), loc(c.lang, "Discard", "Verwerfen"), "Discard button label");
    return 0;
}

int cancel_unsaved_dialog(QObject* dialog, int base) {
    QObject* cancel = dialog_action(dialog, "unsavedCancelAction");
    M3_CHECK(base, cancel != nullptr, "dialog must offer [Cancel]");
    QMetaObject::invokeMethod(cancel, "trigger");
    M3_CHECK(base + 1, wait_until([&] { return !is_open(dialog); }, 3000), "dialog did not close on Cancel");
    wait_ms(350); // let the modal overlay fade out before the next click
    return 0;
}

// AC-M3-14-1: Schedule editor of a new backup.
int scenario_schedule_defaults(const Ctx& c) {
    QQuickWindow* w = c.window;
    // (a) The new-backup wizard summary shows «Daily at 12:00» as default (P35).
    QMetaObject::invokeMethod(w, "openNewBackupWizard", Q_ARG(QVariant, false));
    wait_ms(250);
    QObject* wiz = object_named(w, "newBackupWizard");
    M3_CHECK(3301, wiz != nullptr, "newBackupWizard failed to open");
    wiz->setProperty("stepIndex", 2);
    wait_ms(200);
    M3_CHECK(3302, has_visible_text(w->contentItem(), loc(c.lang, "Daily at 12:00", "Täglich um 12:00")), "wizard summary must show Daily at 12:00");
    QMetaObject::invokeMethod(w, "closeWizardLayer");
    wait_ms(200);

    const QString bid = backup_id_at(w, 0);
    M3_CHECK(3303, !bid.isEmpty(), "no backup found");
    open_backup_detail(w, bid);
    QQuickItem* page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    M3_CHECK(3304, page != nullptr, "backupScheduleEditPage did not open");

    QQuickItem* combo = item_named(w, "scheduleFrequencyCombo");
    M3_CHECK(3305, combo != nullptr, "scheduleFrequencyCombo missing");
    // (a) «daily 12:00» preselected
    M3_CHECK(3306, combo->property("currentIndex").toInt() == 0, "«Daily» must be preselected");
    M3_CHECK_EQ(3307, combo->property("currentText").toString(), loc(c.lang, "Daily", "Täglich"), "preselected option label");
    M3_CHECK_EQ(3308, spin_text(item_named(w, "scheduleHourSpinBox")), QStringLiteral("12"), "hour field");
    M3_CHECK_EQ(3309, spin_text(item_named(w, "scheduleMinuteSpinBox")), QStringLiteral("00"), "minute field is two-digit");
    M3_CHECK(3310, !page->property("dirty").toBool(), "untouched editor must not be dirty");

    // (b) selectable: «daily at», «on weekdays at», «every n days from date» (+ manual only)
    M3_CHECK(3311, combo->property("count").toInt() == 4, "exactly 4 options (no Hourly)");
    M3_CHECK_EQ(3312, combo_text(combo, 0), loc(c.lang, "Daily", "Täglich"), "option 0");
    M3_CHECK_EQ(3313, combo_text(combo, 1), loc(c.lang, "On specific weekdays", "An bestimmten Wochentagen"), "option 1");
    M3_CHECK_EQ(3314, combo_text(combo, 2), loc(c.lang, "Every n days", "Alle n Tage"), "option 2");
    M3_CHECK_EQ(3315, combo_text(combo, 3), loc(c.lang, "Manual only", "Nur manuell"), "option 3");

    M3_CHECK(3316, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// AC-M3-14-2 and AC-M3-14-6 (schedule editor).
int scenario_schedule_validation(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    M3_CHECK(3401, !bid.isEmpty(), "no backup found");
    open_backup_detail(w, bid);
    QQuickItem* page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    M3_CHECK(3402, page != nullptr, "backupScheduleEditPage did not open");
    QQuickItem* combo = item_named(w, "scheduleFrequencyCombo");
    QQuickItem* save_btn = item_named(w, "saveScheduleButton");
    QObject* save_action = object_named(w, "saveScheduleAction");
    QQuickItem* banner = item_named(w, "scheduleValidationBanner");
    M3_CHECK(3403, combo && save_btn && save_action && banner, "schedule editor controls missing");

    // Case 1: weekdays without any weekday.
    combo->setProperty("currentIndex", 1);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 1));
    wait_ms(150);
    QQuickItem* monday = item_named(w, "weekdayButton_monday");
    QQuickItem* tuesday = item_named(w, "weekdayButton_tuesday");
    QQuickItem* frame = item_named(w, "scheduleWeekdaysFrame");
    M3_CHECK(3404, monday && tuesday && frame, "weekday controls missing");
    M3_CHECK(3405, click_ui_item(w, tuesday, "weekdayButton_tuesday"), "failed to select Tuesday");
    wait_ms(150);
    M3_CHECK(3406, save_btn->isEnabled(), "Save must be enabled for a valid weekday selection");
    M3_CHECK(3407, click_ui_item(w, monday, "weekdayButton_monday"), "failed to deselect Monday");
    M3_CHECK(3408, click_ui_item(w, tuesday, "weekdayButton_tuesday"), "failed to deselect Tuesday");
    wait_ms(200);
    M3_CHECK(3409, page->property("selectedWeekdays").toList().isEmpty(), "no weekday may remain selected");
    M3_CHECK(3410, !save_btn->isEnabled(), "(a) Save button must be disabled without a weekday");
    M3_CHECK(3411, !save_action->property("enabled").toBool(), "(a) Save toolbar action must be disabled without a weekday");
    M3_CHECK(3412, border_width(frame) == 2, "(b) weekday field must be marked");
    M3_CHECK(3413, banner->isVisible(), "validation message must be shown");
    M3_CHECK(3414, !banner->property("text").toString().isEmpty(), "validation message must not be empty");

    // Case 2: every 0 days.
    combo->setProperty("currentIndex", 2);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 2));
    wait_ms(150);
    QQuickItem* interval = item_named(w, "scheduleIntervalSpinBox");
    QQuickItem* interval_border = item_named(w, "scheduleIntervalInvalidBorder");
    M3_CHECK(3420, interval && interval_border, "interval controls missing");
    interval->setProperty("value", 0);
    wait_ms(200);
    M3_CHECK(3421, !save_btn->isEnabled(), "(a) Save button must be disabled for «every 0 days»");
    M3_CHECK(3422, !save_action->property("enabled").toBool(), "(a) Save toolbar action must be disabled for «every 0 days»");
    M3_CHECK(3423, interval_border->isVisible(), "(b) interval field must be marked (visible)");
    M3_CHECK(3424, border_width(interval_border) == 2, "(b) interval marking must be a 2px border");
    M3_CHECK(3425, banner->isVisible(), "validation message must be shown for interval 0");

    // The start date is chosen with the date picker (primary control), Escape closes it.
    QQuickItem* picker = item_named(w, "scheduleStartDatePickerButton");
    M3_CHECK(3430, picker && click_ui_item(w, picker, "scheduleStartDatePickerButton"), "date picker button failed to click");
    QObject* popup = object_named(w, "scheduleStartDatePopup");
    M3_CHECK(3431, wait_until([&] { return is_open(popup); }, 2000), "date picker popup did not open");
    send_key(w, Qt::Key_Escape);
    M3_CHECK(3432, wait_until([&] { return !is_open(popup); }, 2000), "date picker popup did not close on Escape");

    // Valid again: AC-M3-14-6 keyboard reachability and accessible names (schedule editor).
    interval->setProperty("value", 4);
    wait_ms(200);
    M3_CHECK(3433, save_btn->isEnabled(), "Save must be enabled again for interval 4");
    M3_CHECK(3434, page->property("previewSlots").toList().size() == 3, "3 preview slots expected");
    const QStringList controls {
        QStringLiteral("scheduleFrequencyCombo"),
        QStringLiteral("scheduleIntervalSpinBox"),
        QStringLiteral("scheduleStartDatePickerButton"),
        QStringLiteral("scheduleHourSpinBox"),
        QStringLiteral("scheduleMinuteSpinBox"),
        QStringLiteral("useGlobalOverdueCheck"),
        QStringLiteral("discardScheduleButton"),
        QStringLiteral("saveScheduleButton"),
    };
    const QString unnamed = first_unnamed(w, controls);
    M3_CHECK(3435, unnamed.isEmpty(), qPrintable(QStringLiteral("(b) control without Accessible.name: ") + unnamed));
    combo->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    const QStringList missing = tab_missing(w, controls, 60);
    M3_CHECK(3436, missing.isEmpty(), qPrintable(QStringLiteral("(a) not reachable by keyboard: ") + missing.join(QLatin1Char(','))));
    M3_CHECK(3437, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// Leaves the schedule editor through `navigate` while it is dirty; asserts the prompt opens,
// the editor is kept, then cancels.
int guarded_navigation(const Ctx& c, const char* what, int base, const std::function<void()>& navigate) {
    QQuickWindow* w = c.window;
    QObject* dialog = object_named(w, "unsavedTabSwitchDialog");
    const int tab_before = w->property("activeTab").toInt();
    navigate();
    const int open_rc = check_unsaved_dialog_open(c, dialog, base);
    if (open_rc != 0) {
        std::fprintf(stderr, "navigation path: %s\n", what);
        return open_rc;
    }
    const int cancel_rc = cancel_unsaved_dialog(dialog, base + 4);
    if (cancel_rc != 0) {
        return cancel_rc;
    }
    wait_ms(150);
    QQuickItem* page = item_named(w, "backupScheduleEditPage");
    M3_CHECK(base + 6, page != nullptr, "dirty editor must stay open after Cancel");
    M3_CHECK(base + 7, page->property("dirty").toBool(), "edits must be kept after Cancel");
    M3_CHECK(base + 8, w->property("activeTab").toInt() == tab_before, "active tab must not change");
    return 0;
}

// AC-M3-14-8 (d) and AC-M3-14-9 for the schedule editor.
int scenario_schedule_unsaved(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    const QString did = destination_id_at(w, 0);
    M3_CHECK(3501, !bid.isEmpty() && !did.isEmpty(), "backup/destination missing");
    open_backup_detail(w, bid);
    QQuickItem* page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    M3_CHECK(3502, page != nullptr, "backupScheduleEditPage did not open");
    QQuickItem* combo = item_named(w, "scheduleFrequencyCombo");
    QQuickItem* interval = nullptr;
    const QByteArray before = read_config_bytes();
    M3_CHECK(3503, !before.isEmpty(), "config.toml must exist");

    combo->setProperty("currentIndex", 2);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 2));
    wait_ms(150);
    interval = item_named(w, "scheduleIntervalSpinBox");
    M3_CHECK(3504, interval != nullptr, "interval spin box missing");
    interval->setProperty("value", 4);
    wait_ms(200);
    M3_CHECK(3505, page->property("dirty").toBool(), "editor must be dirty after editing");
    // AC-M3-14-8 (d): config.toml is unchanged before [Save]; the draft lives only in the GUI.
    M3_CHECK(3506, read_config_bytes() == before, "(d) config.toml must be unchanged before Save");

    // AC-M3-14-9 (a): every way of leaving the dirty editor asks first.
    int rc = guarded_navigation(c, "switchTab", 3510, [&] { QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0)); });
    if (rc != 0) {
        return rc;
    }
    rc = guarded_navigation(c, "settings row", 3520, [&] {
        QQuickItem* row = find_visual_item(w, QStringLiteral("editConditionsRow"));
        click_ui_item(w, row, "editConditionsRow");
    });
    if (rc != 0) {
        return rc;
    }
    wait_ms(200);
    {
        QObject* stack = qvariant_cast<QObject*>(w->property("pageStack"));
        M3_CHECK(3527, stack != nullptr, "pageStack missing");
        QObject* last = qvariant_cast<QObject*>(stack->property("lastItem"));
        M3_CHECK(3528, last != nullptr, "pageStack has no last page");
        M3_CHECK_EQ(3529, last->objectName(), QStringLiteral("backupScheduleEditPage"), "last page (conditions editor must not be pushed)");
    }
    rc = guarded_navigation(c, "openBackupDetailById", 3530, [&] { QMetaObject::invokeMethod(w, "openBackupDetailById", Q_ARG(QVariant, bid)); });
    if (rc != 0) {
        return rc;
    }
    rc = guarded_navigation(c, "openDestinationDetailById", 3540, [&] { QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did)); });
    if (rc != 0) {
        return rc;
    }
    rc = guarded_navigation(c, "openRunDetailById", 3550, [&] { QMetaObject::invokeMethod(w, "openRunDetailById", Q_ARG(QVariant, QStringLiteral("none"))); });
    if (rc != 0) {
        return rc;
    }

    // Failed Save (invalid input) keeps the editor, clears the pending navigation.
    interval->setProperty("value", 0);
    wait_ms(200);
    QObject* dialog = object_named(w, "unsavedTabSwitchDialog");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    M3_CHECK(3560, wait_until([&] { return is_open(dialog); }, 3000), "prompt must open for invalid dirty editor");
    QMetaObject::invokeMethod(dialog_action(dialog, "unsavedSaveAction"), "trigger");
    wait_ms(300);
    M3_CHECK(3561, item_named(w, "backupScheduleEditPage") != nullptr, "failed Save must keep the editor open");
    M3_CHECK(3562, w->property("activeTab").toInt() == 1, "failed Save must not switch tabs");
    M3_CHECK(3563, w->property("pendingNavAction").isNull(), "failed Save must clear the pending navigation");
    QObject* overview_tab = object_named(w, "overviewTabAction");
    QObject* backups_tab = object_named(w, "backupsTabAction");
    M3_CHECK(3564, overview_tab && backups_tab, "tab actions missing");
    M3_CHECK(3565, backups_tab->property("checked").toBool(), "tab bar must show the Backups tab as checked");
    M3_CHECK(3566, !overview_tab->property("checked").toBool(), "tab bar must not show the Overview tab as checked");

    // (b) [Discard] leaves config.toml byte-identical.
    interval->setProperty("value", 4);
    wait_ms(200);
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    M3_CHECK(3570, wait_until([&] { return is_open(dialog); }, 3000), "prompt must open before discarding");
    QMetaObject::invokeMethod(dialog_action(dialog, "unsavedDiscardAction"), "trigger");
    M3_CHECK(3571, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "Discard must continue to the Overview");
    M3_CHECK(3572, read_config_bytes() == before, "(b) Discard must leave config.toml byte-identical");

    // [Save] persists (checked on the test side): every 4 days.
    open_backup_detail(w, bid);
    page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    M3_CHECK(3580, page != nullptr, "schedule editor did not reopen");
    combo = item_named(w, "scheduleFrequencyCombo");
    combo->setProperty("currentIndex", 2);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 2));
    wait_ms(150);
    item_named(w, "scheduleIntervalSpinBox")->setProperty("value", 4);
    wait_ms(200);
    QQuickItem* save_btn = item_named(w, "saveScheduleButton");
    M3_CHECK(3581, save_btn && click_ui_item(w, save_btn, "saveScheduleButton"), "failed to click Save");
    M3_CHECK(3582, wait_until([&] { return read_config_bytes() != before; }, 5000), "Save must write config.toml");
    M3_CHECK(3583, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

const QStringList kConditionControls {
    QStringLiteral("requireAcCheck"),
    QStringLiteral("requireNetworkCheck"),
    QStringLiteral("skipMeteredCheck"),
    QStringLiteral("knownNetworkCheck_HomeWifi"),
    QStringLiteral("knownNetworkCheck_OfficeWifi"),
    QStringLiteral("newNetworkField"),
    QStringLiteral("addNetworkButton"),
    QStringLiteral("discardConditionsButton"),
    QStringLiteral("saveConditionsButton"),
};

// AC-M3-14-3 (new backup defaults) and AC-M3-14-6 (conditions editor).
int scenario_conditions_defaults(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    M3_CHECK(3601, !bid.isEmpty(), "no backup found");
    open_backup_detail(w, bid);
    QQuickItem* page = open_editor(w, "editConditionsRow", "backupConditionsEditPage");
    M3_CHECK(3602, page != nullptr, "backupConditionsEditPage did not open");
    QQuickItem* ac = item_named(w, "requireAcCheck");
    QQuickItem* net = item_named(w, "requireNetworkCheck");
    QQuickItem* metered = item_named(w, "skipMeteredCheck");
    QQuickItem* home = item_named(w, "knownNetworkCheck_HomeWifi");
    M3_CHECK(3603, ac && net && metered && home, "conditions controls missing");

    // (a) defaults: only on AC power + only with network
    M3_CHECK(3604, ac->property("checked").toBool(), "(a) «only on AC power» must be active");
    M3_CHECK(3605, net->property("checked").toBool(), "(a) «only with network» must be active");
    M3_CHECK(3606, !metered->property("checked").toBool(), "metered option must start unchecked");
    M3_CHECK(3607, !page->property("dirty").toBool(), "untouched editor must not be dirty");
    M3_CHECK(3608, save_screenshot(w, c.out_path), "screenshot failed");

    // (b) selectable: «not on metered connection» and «only in network …» (known connections)
    M3_CHECK(3610, metered->isEnabled(), "(b) metered option must be selectable");
    M3_CHECK(3611, click_ui_item(w, metered, "skipMeteredCheck"), "failed to click metered option");
    M3_CHECK(3612, metered->property("checked").toBool(), "(b) metered option must become checked");
    M3_CHECK(3613, item_named(w, "knownNetworkCheck_OfficeWifi") != nullptr, "(b) known connection OfficeWifi must be offered");
    M3_CHECK(3614, home->isEnabled(), "(b) known network option must be selectable");
    M3_CHECK(3615, click_ui_item(w, home, "knownNetworkCheck_HomeWifi"), "failed to click HomeWifi");
    M3_CHECK(3616, home->property("checked").toBool(), "(b) HomeWifi must become checked");
    M3_CHECK_EQ(3617, accessible_name(home), loc(c.lang, "Allow network HomeWifi", "Netzwerk HomeWifi erlauben"), "known network accessible name");

    // AC-M3-14-6: accessible names + keyboard reachability
    const QString unnamed = first_unnamed(w, kConditionControls);
    M3_CHECK(3620, unnamed.isEmpty(), qPrintable(QStringLiteral("(b) control without Accessible.name: ") + unnamed));
    // «Add» is only enabled (and focusable) once an SSID has been typed.
    item_named(w, "newNetworkField")->setProperty("text", QStringLiteral("GuestWifi"));
    wait_ms(100);
    ac->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    const QStringList missing = tab_missing(w, kConditionControls, 60);
    M3_CHECK(3621, missing.isEmpty(), qPrintable(QStringLiteral("(a) not reachable by keyboard: ") + missing.join(QLatin1Char(','))));
    return 0;
}

// AC-M3-14-9 for the run-conditions editor.
int scenario_conditions_unsaved(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    M3_CHECK(3701, !bid.isEmpty(), "no backup found");
    const QByteArray before = read_config_bytes();
    open_backup_detail(w, bid);
    QQuickItem* page = open_editor(w, "editConditionsRow", "backupConditionsEditPage");
    M3_CHECK(3702, page != nullptr, "backupConditionsEditPage did not open");
    QQuickItem* metered = item_named(w, "skipMeteredCheck");
    M3_CHECK(3703, click_ui_item(w, metered, "skipMeteredCheck"), "failed to click metered option");
    wait_ms(150);
    M3_CHECK(3704, page->property("dirty").toBool(), "editor must be dirty");
    M3_CHECK(3705, read_config_bytes() == before, "config.toml must be unchanged before Save");

    QObject* dialog = object_named(w, "unsavedTabSwitchDialog");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    int rc = check_unsaved_dialog_open(c, dialog, 3710);
    if (rc != 0) {
        return rc;
    }
    rc = cancel_unsaved_dialog(dialog, 3720);
    if (rc != 0) {
        return rc;
    }
    M3_CHECK(3722, item_named(w, "backupConditionsEditPage") != nullptr, "editor must stay after Cancel");
    M3_CHECK(3723, page->property("dirty").toBool(), "edits must be kept after Cancel");

    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    M3_CHECK(3730, wait_until([&] { return is_open(dialog); }, 3000), "prompt must open again");
    QMetaObject::invokeMethod(dialog_action(dialog, "unsavedDiscardAction"), "trigger");
    M3_CHECK(3731, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "Discard must continue to the Overview");
    M3_CHECK(3732, read_config_bytes() == before, "(b) Discard must leave config.toml byte-identical");

    // The editor's own prompt (back/close) offers the same localized buttons.
    open_backup_detail(w, bid);
    page = open_editor(w, "editConditionsRow", "backupConditionsEditPage");
    M3_CHECK(3740, page != nullptr, "conditions editor did not reopen");
    metered = item_named(w, "skipMeteredCheck");
    M3_CHECK(3741, click_ui_item(w, metered, "skipMeteredCheck"), "failed to click metered option");
    QQuickItem* home = item_named(w, "knownNetworkCheck_HomeWifi");
    M3_CHECK(3742, click_ui_item(w, home, "knownNetworkCheck_HomeWifi"), "failed to click HomeWifi");
    wait_ms(150);
    QQuickItem* close_btn = item_named(w, "discardConditionsButton");
    M3_CHECK(3743, close_btn && click_ui_item(w, close_btn, "discardConditionsButton"), "failed to click Cancel button");
    QObject* own = object_named(w, "unsavedConditionsDialog");
    rc = check_unsaved_dialog_open(c, own, 3750);
    if (rc != 0) {
        return rc;
    }
    rc = cancel_unsaved_dialog(own, 3760);
    if (rc != 0) {
        return rc;
    }
    M3_CHECK(3762, page->property("dirty").toBool(), "edits must survive Cancel of the editor prompt");
    // [Save] persists (checked on the test side).
    QQuickItem* save_btn = item_named(w, "saveConditionsButton");
    M3_CHECK(3763, save_btn && click_ui_item(w, save_btn, "saveConditionsButton"), "failed to click Save");
    M3_CHECK(3764, wait_until([&] { return read_config_bytes() != before; }, 5000), "Save must write config.toml");
    M3_CHECK(3765, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// AC-M3-14-4 (per backup) and AC-M3-14-6 (pause menu as keyboard-only editor).
int scenario_pause_menu(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    M3_CHECK(3801, !bid.isEmpty(), "no backup found");
    open_backup_detail(w, bid);

    struct Entry { const char* object; const char* en; const char* de; };
    const Entry entries[] = {
        { "pauseBackup1hItem", "1 Hour", "1 Stunde" },
        { "pauseBackup3hItem", "3 Hours", "3 Stunden" },
        { "pauseBackupTomorrowItem", "Until Tomorrow", "Bis morgen" },
        { "pauseBackupIndefiniteItem", "Until I Resume", "Bis ich fortsetze" },
    };
    int code = 3810;
    for (const Entry& e : entries) {
        QObject* item = object_named(w, e.object);
        M3_CHECK(code, item != nullptr, "per-backup pause menu item missing");
        M3_CHECK_EQ(code + 1, item->property("text").toString(), loc(c.lang, e.en, e.de), e.object);
        M3_CHECK_EQ(code + 2, accessible_name(item), loc(c.lang, e.en, e.de), "pause item accessible name");
        code += 3;
    }

    QObject* pause_act = object_named(w, "backupDetailPauseAction");
    M3_CHECK(3830, pause_act != nullptr, "backupDetailPauseAction missing");
    M3_CHECK_EQ(3831, pause_act->property("text").toString(), loc(c.lang, "Pause…", "Pausieren…"), "pause action text");
    QMetaObject::invokeMethod(pause_act, "trigger");
    QObject* menu = object_named(w, "backupPauseMenu");
    M3_CHECK(3832, wait_until([&] { return is_open(menu); }, 2000), "pause menu did not open");
    M3_CHECK(3833, save_screenshot(w, c.out_path), "screenshot failed");

    // Keyboard only: arrow keys walk through the four choices in visual order.
    const char* order[] = { "pauseBackup1hItem", "pauseBackup3hItem", "pauseBackupTomorrowItem", "pauseBackupIndefiniteItem" };
    code = 3840;
    bool first = true;
    for (const char* expected : order) {
        if (!first) {
            send_key(w, Qt::Key_Down);
        }
        first = false;
        QQuickItem* focus = w->activeFocusItem();
        M3_CHECK(code, focus != nullptr, "no keyboard focus in the pause menu");
        M3_CHECK_EQ(code + 1, focus->objectName(), QString::fromLatin1(expected), "focused pause menu item");
        code += 2;
    }
    send_key(w, Qt::Key_Return);
    QQuickItem* banner = item_named(w, "backupDetailPausedBanner");
    M3_CHECK(3850, banner != nullptr, "backupDetailPausedBanner missing");
    M3_CHECK(3851, wait_until([&] { return banner->isVisible(); }, 3000), "Enter on «Until I Resume» must pause the backup");
    M3_CHECK_STARTS(3852, banner->property("text").toString(), loc(c.lang, "Paused since ", "Pausiert seit "), "paused banner language");
    M3_CHECK_EQ(3853, pause_act->property("text").toString(), loc(c.lang, "Resume", "Fortsetzen"), "pause action while paused");
    QMetaObject::invokeMethod(pause_act, "trigger");
    M3_CHECK(3854, wait_until([&] { return !banner->isVisible(); }, 3000), "action must resume the backup");
    return 0;
}

// AC-M3-14-5, AC-M3-05-7 (a), AC-M3-14-6 (maintenance editor).
int scenario_maintenance(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString did = destination_id_at(w, 0);
    M3_CHECK(3901, !did.isEmpty(), "no destination found");
    QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
    wait_ms(250);

    // (a) defaults: «Clean up: Sundays 12:00» and «Verify: first Sunday of the month 12:00».
    QString summary;
    QMetaObject::invokeMethod(
        c.backend,
        "formatMaintenanceSummaryJson",
        Qt::DirectConnection,
        Q_RETURN_ARG(QString, summary),
        Q_ARG(QString, QStringLiteral("{\"prune\":{\"kind\":\"weekly\",\"weekday\":\"sunday\",\"hour\":12,\"minute\":0},\"check\":{\"kind\":\"monthly\",\"nth\":\"first\",\"weekday\":\"sunday\",\"hour\":12,\"minute\":0}}"))
    );
    M3_CHECK(3902, summary.contains(loc(c.lang, "Clean up: Sundays at 12:00", "Aufräumen: sonntags 12:00")), "(a) clean-up default summary");
    M3_CHECK(3903, summary.contains(loc(c.lang, "Verify: first Sunday of month at 12:00", "Prüfen: erster Sonntag im Monat 12:00")), "(a) verify default summary");
    QQuickItem* content = w->contentItem();
    M3_CHECK(3905, has_visible_text(content, loc(c.lang, "Next verify:", "Nächstes Prüfen:")), "detail must show «Next verify:»");

    QQuickItem* page = open_editor(w, "editMaintenanceRow", "destinationMaintenancePage");
    M3_CHECK(3906, page != nullptr, "destinationMaintenancePage did not open");
    const QStringList controls {
        QStringLiteral("pruneFrequencyCombo"),
        QStringLiteral("pruneHourSpin"),
        QStringLiteral("pruneMinuteSpin"),
        QStringLiteral("checkFrequencyCombo"),
        QStringLiteral("checkHourSpin"),
        QStringLiteral("checkMinuteSpin"),
        QStringLiteral("discardMaintenanceButton"),
        QStringLiteral("saveMaintenanceButton"),
    };
    M3_CHECK_EQ(3907, spin_text(item_named(w, "pruneHourSpin")), QStringLiteral("12"), "prune hour field");
    M3_CHECK_EQ(3908, spin_text(item_named(w, "pruneMinuteSpin")), QStringLiteral("00"), "prune minute field is two-digit");
    const QString unnamed = first_unnamed(w, controls);
    M3_CHECK(3909, unnamed.isEmpty(), qPrintable(QStringLiteral("(b) control without Accessible.name: ") + unnamed));

    QQuickItem* prune_combo = item_named(w, "pruneFrequencyCombo");
    prune_combo->setProperty("currentIndex", 2); // Daily
    QMetaObject::invokeMethod(prune_combo, "activated", Q_ARG(int, 2));
    wait_ms(150);
    prune_combo->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    const QStringList missing = tab_missing(w, controls, 80);
    M3_CHECK(3910, missing.isEmpty(), qPrintable(QStringLiteral("(a) not reachable by keyboard: ") + missing.join(QLatin1Char(','))));

    // AC-M3-14-5 (b) / AC-M3-05-7 (a): changing the clean-up schedule asks for confirmation
    // and shows the dry-run preview result before it can be confirmed.
    QQuickItem* save_btn = item_named(w, "saveMaintenanceButton");
    M3_CHECK(3911, save_btn && save_btn->isEnabled(), "Save must be enabled after changing the clean-up schedule");
    M3_CHECK(3912, click_ui_item(w, save_btn, "saveMaintenanceButton"), "failed to click Save");
    QObject* dlg = object_named(w, "pruneConfirmDialog");
    M3_CHECK(3913, wait_until([&] { return dlg && dlg->property("visible").toBool(); }, 3000), "pruneConfirmDialog did not open");
    QObject* confirm = dialog_action(dlg, "confirmPruneSaveAction");
    M3_CHECK(3914, confirm != nullptr, "confirmPruneSaveAction missing");
    if (!page->property("previewSettled").toBool()) {
        M3_CHECK(3915, !confirm->property("enabled").toBool(), "confirmation must stay disabled until the preview loaded");
    }
    M3_CHECK(3916, wait_until([&] { return page->property("previewSettled").toBool(); }, 20000), "preview did not load or fail");
    wait_ms(200);
    QQuickItem* result = item_named(w, "prunePreviewSummaryLabel");
    M3_CHECK(3917, result && result->isVisible(), "(a) preview result must be shown before confirming");
    M3_CHECK_EQ(
        3918,
        result->property("text").toString(),
        loc(c.lang, "Packs to remove: 0\nSpace to free: 0 B", "Zu entfernende Packs: 0\nFreizugebender Speicher: 0 B"),
        "preview result text"
    );
    M3_CHECK(3919, confirm->property("enabled").toBool(), "confirmation must be enabled once the preview is shown");
    M3_CHECK(3920, save_screenshot(w, c.out_path), "screenshot failed");
    QMetaObject::invokeMethod(confirm, "trigger");
    wait_ms(300);

    // A second, discarded edit leaves the persisted check schedule untouched.
    QQuickItem* check_combo = item_named(w, "checkFrequencyCombo");
    check_combo->setProperty("currentIndex", 3); // manual only
    QMetaObject::invokeMethod(check_combo, "activated", Q_ARG(int, 3));
    wait_ms(150);
    send_key(w, Qt::Key_Escape);
    QObject* unsaved = object_named(w, "unsavedMaintenanceDialog");
    M3_CHECK(3921, wait_until([&] { return is_open(unsaved); }, 3000), "unsavedMaintenanceDialog did not open on Escape");
    QMetaObject::invokeMethod(dialog_action(unsaved, "unsavedDiscardAction"), "trigger");
    wait_ms(250);
    return 0;
}

// Settings dialog: monitoring and notification controls persist.
int scenario_settings(const Ctx& c) {
    QQuickWindow* w = c.window;
    QMetaObject::invokeMethod(w, "openSettingsWindow");
    wait_ms(250);
    QObject* settings = object_named(w, "settingsWindow");
    M3_CHECK(4001, settings != nullptr, "settingsWindow not found");
    const QStringList controls {
        QStringLiteral("defaultOverdueSpinBox"),
        QStringLiteral("notifErrorSwitch"),
        QStringLiteral("notifWarningSwitch"),
        QStringLiteral("notifOverdueSwitch"),
        QStringLiteral("notifMaintenanceSwitch"),
        QStringLiteral("notifResticUpdateSwitch"),
    };
    for (const QString& name : controls) {
        QObject* ctl = settings->findChild<QObject*>(name);
        M3_CHECK(4002, ctl != nullptr, qPrintable(QStringLiteral("settings control missing: ") + name));
        M3_CHECK(4003, !accessible_name(ctl).trimmed().isEmpty(), qPrintable(QStringLiteral("settings control without Accessible.name: ") + name));
    }
    QObject* overdue_spin = settings->findChild<QObject*>(QStringLiteral("defaultOverdueSpinBox"));
    QObject* notif_overdue = settings->findChild<QObject*>(QStringLiteral("notifOverdueSwitch"));
    overdue_spin->setProperty("value", 4);
    QMetaObject::invokeMethod(overdue_spin, "valueModified");
    notif_overdue->setProperty("checked", false);
    QMetaObject::invokeMethod(notif_overdue, "toggled");
    wait_ms(300);
    M3_CHECK(4004, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

} // namespace

int run_m3_editor_scenario(const m3::Ctx& ctx, const QString& scen, bool* handled) {
    *handled = true;
    if (scen == QLatin1String("m3-schedule-editor-defaults")) return scenario_schedule_defaults(ctx);
    if (scen == QLatin1String("m3-schedule-editor-validation")) return scenario_schedule_validation(ctx);
    if (scen == QLatin1String("m3-schedule-editor-unsaved")) return scenario_schedule_unsaved(ctx);
    if (scen == QLatin1String("m3-conditions-editor-defaults")) return scenario_conditions_defaults(ctx);
    if (scen == QLatin1String("m3-conditions-editor-unsaved")) return scenario_conditions_unsaved(ctx);
    if (scen == QLatin1String("m3-pause-menu-keyboard")) return scenario_pause_menu(ctx);
    if (scen == QLatin1String("m3-maintenance-editor")) return scenario_maintenance(ctx);
    if (scen == QLatin1String("m3-settings-monitoring")) return scenario_settings(ctx);
    *handled = false;
    return 0;
}

} // namespace kestic::smoke
