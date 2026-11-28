// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_m3_util.h"

#include <QUuid>
#include <qsignalspy.h>

namespace kestic::smoke {

namespace {

using namespace m3;

// The row of `id` (role `id_role`) in the model `model_name`, or -1.
int model_row(QQuickWindow* w, const char* model_name, const char* id_role, const QString& id) {
    QAbstractItemModel* model = model_named(w, model_name);
    for (int row = 0; model && row < model->rowCount(); ++row) {
        if (role_value(model, row, id_role).toString() == id) {
            return row;
        }
    }
    return -1;
}

// The revision of the backup or destination `id` in the window's state.
QString backend_revision(QObject* backend, const char* kind, const QString& id) {
    QString revision;
    QMetaObject::invokeMethod(backend, qstrcmp(kind, "destination") == 0 ? "destinationRevision" : "backupRevision",
        Q_RETURN_ARG(QString, revision), Q_ARG(QString, id));
    return revision;
}

QObject* dialog_action(QObject* dialog, const char* name) {
    return dialog ? dialog->findChild<QObject*>(QString::fromLatin1(name)) : nullptr;
}

/// The editors save and close through their page footer actions only.
/// Names each visible button Kirigami generates for an action `<x>Action` as `<x>ActionButton`,
/// so the checks below can click it and test its keyboard reachability and accessible name.
void name_action_buttons(QQuickWindow* w) {
    QList<QQuickItem*> items;
    collect_visual_descendants(w->contentItem(), items);
    for (QQuickItem* it : items) {
        if (!it->isVisible()) {
            continue;
        }
        QObject* act = it->property("action").value<QObject*>();
        if (act && act->objectName().endsWith(QLatin1String("Action"))) {
            it->setObjectName(act->objectName() + QStringLiteral("Button"));
        }
    }
}

QQuickItem* open_editor(QQuickWindow* w, const char* row, const char* page) {
    // The row of a page that just opened is placed by the layout in a later pass.
    wait_until([&] {
        QQuickItem* shown = find_visual_item(w, QString::fromLatin1(row));
        return shown && shown->isVisible() && shown->width() > 0.0 && shown->height() > 0.0;
    }, 3000);
    wait_ms(100); // quiet window: the layout places the row
    QQuickItem* r = find_visual_item(w, QString::fromLatin1(row));
    if (!r || !click_ui_item(w, r, row)) {
        return nullptr;
    }
    wait_until([&] { return lookup(w, page) != nullptr; }, 3000);
    wait_ms(200); // quiet window: the page ends its transition and creates its action buttons
    name_action_buttons(w);
    return lookup(w, page);
}

/// Asserts the «Save changes?» prompt `dialog` is open with the localized [Save] / [Discard]
/// buttons (AC-M3-14-9 (a)). Returns 0 when fine.
int check_unsaved_dialog_open(const Ctx& c, QObject* dialog, int base) {
    CHECK(base, wait_until([&] { return is_open(dialog); }, 3000), "unsaved-changes dialog did not open");
    QObject* save = dialog_action(dialog, "unsavedSaveAction");
    QObject* discard = dialog_action(dialog, "unsavedDiscardAction");
    CHECK2(base + 1, save, discard, "dialog must offer [Save] and [Discard]");
    CHECK_EQ(base + 2, save->property("text").toString(), loc(c.lang, "Save", "Speichern"), "Save button label");
    CHECK_EQ(base + 3, discard->property("text").toString(), loc(c.lang, "Discard", "Verwerfen"), "Discard button label");
    return 0;
}

int cancel_unsaved_dialog(QObject* dialog, int base) {
    QObject* cancel = dialog_action(dialog, "unsavedCancelAction");
    CHECK(base, cancel != nullptr, "dialog must offer [Cancel]");
    QMetaObject::invokeMethod(cancel, "trigger");
    CHECK(base + 1, wait_until([&] { return !is_open(dialog); }, 3000), "dialog did not close on Cancel");
    wait_ms(350); // quiet window: the modal overlay fades out before the next click
    return 0;
}

// AC-M3-14-1: Schedule editor of a new backup.
int scenario_schedule_defaults(const Ctx& c) {
    QQuickWindow* w = c.window;
    // (a) The new-backup wizard summary shows «Daily at 12:00» as default.
    QMetaObject::invokeMethod(w, "openNewBackupWizard", Q_ARG(QVariant, false));
    QObject* wiz = nullptr;
    wait_until([&] { return (wiz = lookup<QObject>(w, "newBackupWizard")) != nullptr; }, 250);
    CHECK(3301, wiz != nullptr, "newBackupWizard failed to open");
    wiz->setProperty("stepIndex", 2);
    CHECK(3302, wait_until([&] { return has_visible_text(w->contentItem(), loc(c.lang, "Daily at 12:00", "Täglich um 12:00")); }, 200), "wizard summary must show Daily at 12:00");
    QMetaObject::invokeMethod(w, "closeWizardLayer");
    wait_until([&] { return !w->property("inWizardLayer").toBool(); }, 200);

    const QString bid = backup_id_at(w, 0);
    CHECK(3303, !bid.isEmpty(), "no backup found");
    CHECK(4101, open_backup_detail(w, bid), "backupDetailPage did not open");
    QQuickItem* page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    CHECK(3304, page != nullptr, "backupScheduleEditPage did not open");

    QQuickItem* combo = lookup(w, "scheduleFrequencyCombo");
    CHECK(3305, combo != nullptr, "scheduleFrequencyCombo missing");
    // (a) «daily 12:00» preselected
    CHECK(3306, combo->property("currentIndex").toInt() == 0, "«Daily» must be preselected");
    CHECK_EQ(3307, combo->property("currentText").toString(), loc(c.lang, "Daily", "Täglich"), "preselected option label");
    CHECK_EQ(3308, spin_text(lookup(w, "scheduleHourSpinBox")), QStringLiteral("12"), "hour field");
    CHECK_EQ(3309, spin_text(lookup(w, "scheduleMinuteSpinBox")), QStringLiteral("00"), "minute field is two-digit");
    CHECK(3310, !page->property("dirty").toBool(), "untouched editor must not be dirty");

    // (b) selectable: «daily at», «on weekdays at», «every n days from date» (+ manual only)
    CHECK(3311, combo->property("count").toInt() == 4, "exactly 4 options (no Hourly)");
    CHECK_EQ(3312, combo_text(combo, 0), loc(c.lang, "Daily", "Täglich"), "option 0");
    CHECK_EQ(3313, combo_text(combo, 1), loc(c.lang, "On specific weekdays", "An bestimmten Wochentagen"), "option 1");
    CHECK_EQ(3314, combo_text(combo, 2), loc(c.lang, "Every n days", "Alle n Tage"), "option 2");
    CHECK_EQ(3315, combo_text(combo, 3), loc(c.lang, "Manual only", "Nur manuell"), "option 3");

    return shot(c, 3316);
}

// AC-M3-14-2 and AC-M3-14-6 (schedule editor).
int scenario_schedule_validation(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    CHECK(3401, !bid.isEmpty(), "no backup found");
    CHECK(4102, open_backup_detail(w, bid), "backupDetailPage did not open");
    QQuickItem* page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    CHECK(3402, page != nullptr, "backupScheduleEditPage did not open");
    QQuickItem* combo = lookup(w, "scheduleFrequencyCombo");
    QQuickItem* save_btn = lookup(w, "saveScheduleActionButton");
    QObject* save_action = lookup<QObject>(w, "saveScheduleAction");
    QQuickItem* banner = lookup(w, "scheduleValidationBanner");
    CHECK4(3403, combo, save_btn, save_action, banner, "schedule editor controls missing");

    // Case 1: weekdays without any weekday.
    combo->setProperty("currentIndex", 1);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 1));
    wait_until([&] { return lookup(w, "weekdayButton_monday") && lookup(w, "weekdayButton_tuesday") && lookup(w, "scheduleWeekdaysFrame"); }, 150);
    QQuickItem* monday = lookup(w, "weekdayButton_monday");
    QQuickItem* tuesday = lookup(w, "weekdayButton_tuesday");
    QQuickItem* frame = lookup(w, "scheduleWeekdaysFrame");
    CHECK3(3404, monday, tuesday, frame, "weekday controls missing");
    CHECK(3405, click_ui_item(w, tuesday, "weekdayButton_tuesday"), "failed to select Tuesday");
    CHECK(3406, wait_until([&] { return save_btn->isEnabled(); }, 150), "Save must be enabled for a valid weekday selection");
    CHECK(3407, click_ui_item(w, monday, "weekdayButton_monday"), "failed to deselect Monday");
    CHECK(3408, click_ui_item(w, tuesday, "weekdayButton_tuesday"), "failed to deselect Tuesday");
    wait_until([&] { return page->property("selectedWeekdays").toList().isEmpty() && !save_btn->isEnabled(); }, 200);
    CHECK(3409, page->property("selectedWeekdays").toList().isEmpty(), "no weekday may remain selected");
    CHECK(3410, !save_btn->isEnabled(), "(a) Save button must be disabled without a weekday");
    CHECK(3411, !save_action->property("enabled").toBool(), "(a) Save toolbar action must be disabled without a weekday");
    CHECK(3412, border_width(frame) == 2, "(b) weekday field must be marked");
    CHECK(3413, banner->isVisible(), "validation message must be shown");
    CHECK(3414, !banner->property("text").toString().isEmpty(), "validation message must not be empty");

    // Case 2: every 0 days.
    combo->setProperty("currentIndex", 2);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 2));
    wait_until([&] { return lookup(w, "scheduleIntervalSpinBox") && lookup(w, "scheduleIntervalInvalidBorder"); }, 150);
    QQuickItem* interval = lookup(w, "scheduleIntervalSpinBox");
    QQuickItem* interval_border = lookup(w, "scheduleIntervalInvalidBorder");
    CHECK2(3420, interval, interval_border, "interval controls missing");
    // AC-M6-34-3 (a): the field starts at core's default and ends where core's range ends.
    CHECK(3426, interval->property("value").toInt() == 3, "«every n days» starts with core's default");
    CHECK(3427, interval->property("to").toInt() == 365, "the interval field ends at core's range end");
    interval->setProperty("value", 0);
    wait_until([&] { return !save_btn->isEnabled() && interval_border->isVisible(); }, 200);
    CHECK(3421, !save_btn->isEnabled(), "(a) Save button must be disabled for «every 0 days»");
    CHECK(3422, !save_action->property("enabled").toBool(), "(a) Save toolbar action must be disabled for «every 0 days»");
    CHECK(3423, interval_border->isVisible(), "(b) interval field must be marked (visible)");
    CHECK(3424, border_width(interval_border) == 2, "(b) interval marking must be a 2px border");
    CHECK(3425, banner->isVisible(), "validation message must be shown for interval 0");
    CHECK_EQ(3428, banner->property("text").toString(),
        loc(c.lang, "Interval must be between 2 and 365 days.", "Das Intervall muss zwischen 2 und 365 Tagen liegen."),
        "AC-M6-34-3 (a): the message names core's range");

    // The start date is chosen with the date picker (primary control), Escape closes it.
    QQuickItem* picker = lookup(w, "scheduleStartDatePickerButton");
    CHECK2(3430, picker, click_ui_item(w, picker, "scheduleStartDatePickerButton"), "date picker button failed to click");
    QObject* popup = lookup<QObject>(w, "scheduleStartDatePopup");
    CHECK(3431, wait_until([&] { return is_open(popup); }, 2000), "date picker popup did not open");
    send_key(w, Qt::Key_Escape);
    CHECK(3432, wait_until([&] { return !is_open(popup); }, 2000), "date picker popup did not close on Escape");

    // Valid again: AC-M3-14-6 keyboard reachability and accessible names (schedule editor).
    interval->setProperty("value", 4);
    wait_until([&] { return save_btn->isEnabled() && page->property("previewSlots").toList().size() == 3; }, 200);
    CHECK(3433, save_btn->isEnabled(), "Save must be enabled again for interval 4");
    CHECK(3434, page->property("previewSlots").toList().size() == 3, "3 preview slots expected");
    const QStringList controls {
        QStringLiteral("scheduleFrequencyCombo"),
        QStringLiteral("scheduleIntervalSpinBox"),
        QStringLiteral("scheduleStartDatePickerButton"),
        QStringLiteral("scheduleHourSpinBox"),
        QStringLiteral("scheduleMinuteSpinBox"),
        QStringLiteral("useGlobalOverdueCheck"),
        QStringLiteral("closeScheduleActionButton"),
        QStringLiteral("saveScheduleActionButton"),
    };
    const QString unnamed = first_unnamed(w, controls);
    CHECK(3435, unnamed.isEmpty(), qPrintable(QStringLiteral("(b) control without Accessible.name: ") + unnamed));
    combo->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    const QStringList missing = tab_missing(w, controls, 60);
    CHECK(3436, missing.isEmpty(), qPrintable(QStringLiteral("(a) not reachable by keyboard: ") + missing.join(QLatin1Char(','))));
    return shot(c, 3437);
}

// Leaves the schedule editor through `navigate` while it is dirty; asserts the prompt opens,
// the editor is kept, then cancels.
int guarded_navigation(const Ctx& c, const char* what, int base, const std::function<void()>& navigate) {
    QQuickWindow* w = c.window;
    QObject* dialog = lookup<QObject>(w, "unsavedTabSwitchDialog");
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
    wait_ms(150); // quiet window: an editor that Cancel closed would be gone by now
    QQuickItem* page = lookup(w, "backupScheduleEditPage");
    CHECK(base + 6, page != nullptr, "dirty editor must stay open after Cancel");
    CHECK(base + 7, page->property("dirty").toBool(), "edits must be kept after Cancel");
    CHECK(base + 8, w->property("activeTab").toInt() == tab_before, "active tab must not change");
    return 0;
}

// AC-M3-14-8 (d) and AC-M3-14-9 for the schedule editor.
int scenario_schedule_unsaved(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    const QString did = destination_id_at(w, 0);
    CHECK2(3501, !bid.isEmpty(), !did.isEmpty(), "backup/destination missing");
    CHECK(4103, open_backup_detail(w, bid), "backupDetailPage did not open");
    QQuickItem* page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    CHECK(3502, page != nullptr, "backupScheduleEditPage did not open");
    QQuickItem* combo = lookup(w, "scheduleFrequencyCombo");
    QQuickItem* interval = nullptr;
    const QByteArray before = read_config_bytes();
    CHECK(3503, !before.isEmpty(), "config.toml must exist");

    combo->setProperty("currentIndex", 2);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 2));
    wait_until([&] { return lookup(w, "scheduleIntervalSpinBox") != nullptr; }, 150);
    interval = lookup(w, "scheduleIntervalSpinBox");
    CHECK(3504, interval != nullptr, "interval spin box missing");
    interval->setProperty("value", 4);
    CHECK(3505, wait_until([&] { return page->property("dirty").toBool(); }, 200), "editor must be dirty after editing");
    // AC-M3-14-8 (d): config.toml is unchanged before [Save]; the draft lives only in the GUI.
    CHECK(3506, read_config_bytes() == before, "(d) config.toml must be unchanged before Save");

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
    wait_ms(200); // quiet window: the page stack ends its transition
    {
        QObject* stack = qvariant_cast<QObject*>(w->property("pageStack"));
        CHECK(10300, stack != nullptr, "pageStack missing");
        QObject* last = qvariant_cast<QObject*>(stack->property("lastItem"));
        CHECK(10301, last != nullptr, "pageStack has no last page");
        CHECK_EQ(3529, last->objectName(), QStringLiteral("backupScheduleEditPage"), "last page (conditions editor must not be pushed)");
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
    wait_until([&] { return !lookup(w, "saveScheduleActionButton")->isEnabled(); }, 200);
    QObject* dialog = lookup<QObject>(w, "unsavedTabSwitchDialog");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    CHECK(3560, wait_until([&] { return is_open(dialog); }, 3000), "prompt must open for invalid dirty editor");
    QMetaObject::invokeMethod(dialog_action(dialog, "unsavedSaveAction"), "trigger");
    wait_ms(300); // quiet window: a Save that went through would close the editor by now
    CHECK(3561, lookup(w, "backupScheduleEditPage") != nullptr, "failed Save must keep the editor open");
    CHECK(3562, w->property("activeTab").toInt() == 1, "failed Save must not switch tabs");
    CHECK(3563, w->property("pendingNavAction").isNull(), "failed Save must clear the pending navigation");
    QObject* overview_tab = lookup<QObject>(w, "overviewTabAction");
    QObject* backups_tab = lookup<QObject>(w, "backupsTabAction");
    CHECK2(3564, overview_tab, backups_tab, "tab actions missing");
    CHECK(3565, backups_tab->property("checked").toBool(), "tab bar must show the Backups tab as checked");
    CHECK(3566, !overview_tab->property("checked").toBool(), "tab bar must not show the Overview tab as checked");

    // (b) [Discard] leaves config.toml byte-identical.
    interval->setProperty("value", 4);
    wait_until([&] { return lookup(w, "saveScheduleActionButton")->isEnabled(); }, 200);
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    CHECK(3570, wait_until([&] { return is_open(dialog); }, 3000), "prompt must open before discarding");
    QMetaObject::invokeMethod(dialog_action(dialog, "unsavedDiscardAction"), "trigger");
    CHECK(3571, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "Discard must continue to the Overview");
    CHECK(3572, read_config_bytes() == before, "(b) Discard must leave config.toml byte-identical");

    // [Save] persists (checked on the test side): every 4 days.
    CHECK(4104, open_backup_detail(w, bid), "backupDetailPage did not open");
    page = open_editor(w, "editScheduleRow", "backupScheduleEditPage");
    CHECK(3580, page != nullptr, "schedule editor did not reopen");
    combo = lookup(w, "scheduleFrequencyCombo");
    combo->setProperty("currentIndex", 2);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, 2));
    wait_until([&] { return lookup(w, "scheduleIntervalSpinBox") != nullptr; }, 150);
    lookup(w, "scheduleIntervalSpinBox")->setProperty("value", 4);
    wait_until([&] { return lookup(w, "saveScheduleActionButton") && lookup(w, "saveScheduleActionButton")->isEnabled(); }, 200);
    QQuickItem* save_btn = lookup(w, "saveScheduleActionButton");
    CHECK2(3581, save_btn, click_ui_item(w, save_btn, "saveScheduleActionButton"), "failed to click Save");
    CHECK(3582, wait_until([&] { return read_config_bytes() != before; }, 5000), "Save must write config.toml");
    return shot(c, 3583);
}

const QStringList kConditionControls {
    QStringLiteral("requireAcCheck"),
    QStringLiteral("requireNetworkCheck"),
    QStringLiteral("skipMeteredCheck"),
    QStringLiteral("knownNetworkCheck_HomeWifi"),
    QStringLiteral("knownNetworkCheck_OfficeWifi"),
    QStringLiteral("newNetworkField"),
    QStringLiteral("addNetworkButton"),
    QStringLiteral("closeConditionsActionButton"),
    QStringLiteral("saveConditionsActionButton"),
};

// AC-M3-14-3 (new backup defaults) and AC-M3-14-6 (conditions editor).
int scenario_conditions_defaults(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    CHECK(3601, !bid.isEmpty(), "no backup found");
    CHECK(4105, open_backup_detail(w, bid), "backupDetailPage did not open");
    QQuickItem* page = open_editor(w, "editConditionsRow", "backupConditionsEditPage");
    CHECK(3602, page != nullptr, "backupConditionsEditPage did not open");
    // The known networks arrive from the daemon after the editor opened; the layout places
    // their rows in a later pass.
    wait_until([&] { return lookup(w, "knownNetworkCheck_HomeWifi") != nullptr; }, 10000);
    wait_ms(100); // quiet window: the layout places the rows of the networks
    QQuickItem* ac = lookup(w, "requireAcCheck");
    QQuickItem* net = lookup(w, "requireNetworkCheck");
    QQuickItem* metered = lookup(w, "skipMeteredCheck");
    QQuickItem* home = lookup(w, "knownNetworkCheck_HomeWifi");
    CHECK4(3603, ac, net, metered, home, "conditions controls missing");

    // (a) defaults: only on AC power + only with network
    CHECK(3604, ac->property("checked").toBool(), "(a) «only on AC power» must be active");
    CHECK(3605, net->property("checked").toBool(), "(a) «only with network» must be active");
    CHECK(3606, !metered->property("checked").toBool(), "metered option must start unchecked");
    CHECK(3607, !page->property("dirty").toBool(), "untouched editor must not be dirty");
    CHECK(3608, save_screenshot(w, c.out_path), "screenshot failed");

    // (b) selectable: «not on metered connection» and «only in network …» (known connections)
    CHECK(3610, metered->isEnabled(), "(b) metered option must be selectable");
    CHECK(3611, click_ui_item(w, metered, "skipMeteredCheck"), "failed to click metered option");
    CHECK(3612, metered->property("checked").toBool(), "(b) metered option must become checked");
    CHECK(3613, lookup(w, "knownNetworkCheck_OfficeWifi") != nullptr, "(b) known connection OfficeWifi must be offered");
    CHECK(3614, home->isEnabled(), "(b) known network option must be selectable");
    CHECK(3615, click_ui_item(w, home, "knownNetworkCheck_HomeWifi"), "failed to click HomeWifi");
    CHECK(3616, home->property("checked").toBool(), "(b) HomeWifi must become checked");
    CHECK_EQ(3617, accessible_name(home), loc(c.lang, "Allow network HomeWifi", "Netzwerk HomeWifi erlauben"), "known network accessible name");

    // AC-M3-14-6: accessible names + keyboard reachability
    const QString unnamed = first_unnamed(w, kConditionControls);
    CHECK(3620, unnamed.isEmpty(), qPrintable(QStringLiteral("(b) control without Accessible.name: ") + unnamed));
    // «Add» is only enabled (and focusable) once an SSID has been typed.
    lookup(w, "newNetworkField")->setProperty("text", QStringLiteral("GuestWifi"));
    wait_until([&] {
        QQuickItem* add = lookup(w, "addNetworkButton");
        return add && add->isEnabled();
    }, 100);
    ac->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    const QStringList missing = tab_missing(w, kConditionControls, 60);
    CHECK(3621, missing.isEmpty(), qPrintable(QStringLiteral("(a) not reachable by keyboard: ") + missing.join(QLatin1Char(','))));
    return 0;
}

// AC-M3-14-9 for the run-conditions editor.
// A local destination offers no network conditions in the run-condition editor.
int scenario_conditions_local(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    CHECK(3780, !bid.isEmpty(), "no backup found");
    CHECK(4106, open_backup_detail(w, bid), "backupDetailPage did not open");
    QQuickItem* page = open_editor(w, "editConditionsRow", "backupConditionsEditPage");
    CHECK(3781, page != nullptr, "backupConditionsEditPage did not open");
    QQuickItem* hint = lookup(w, "localDestinationNetworkHint");
    QQuickItem* net = lookup(w, "requireNetworkCheck");
    QQuickItem* metered = lookup(w, "skipMeteredCheck");
    CHECK2(3782, hint, hint->isVisible(), "(a) local destination hint must be visible");
    CHECK4(3783, net, !net->isVisible(), metered, !metered->isVisible(), "(b) network conditions must be hidden");
    CHECK(3784, !page->property("dirty").toBool(), "(c) untouched editor must not be dirty");
    return shot(c, 3785);
}

int scenario_conditions_unsaved(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    CHECK(3701, !bid.isEmpty(), "no backup found");
    const QByteArray before = read_config_bytes();
    CHECK(4107, open_backup_detail(w, bid), "backupDetailPage did not open");
    QQuickItem* page = open_editor(w, "editConditionsRow", "backupConditionsEditPage");
    CHECK(3702, page != nullptr, "backupConditionsEditPage did not open");
    QQuickItem* metered = lookup(w, "skipMeteredCheck");
    CHECK(3703, click_ui_item(w, metered, "skipMeteredCheck"), "failed to click metered option");
    CHECK(3704, wait_until([&] { return page->property("dirty").toBool(); }, 150), "editor must be dirty");
    CHECK(3705, read_config_bytes() == before, "config.toml must be unchanged before Save");

    QObject* dialog = lookup<QObject>(w, "unsavedTabSwitchDialog");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    int rc = check_unsaved_dialog_open(c, dialog, 3710);
    if (rc != 0) {
        return rc;
    }
    rc = cancel_unsaved_dialog(dialog, 3720);
    if (rc != 0) {
        return rc;
    }
    CHECK(3722, lookup(w, "backupConditionsEditPage") != nullptr, "editor must stay after Cancel");
    CHECK(3723, page->property("dirty").toBool(), "edits must be kept after Cancel");

    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    CHECK(3730, wait_until([&] { return is_open(dialog); }, 3000), "prompt must open again");
    QMetaObject::invokeMethod(dialog_action(dialog, "unsavedDiscardAction"), "trigger");
    CHECK(3731, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "Discard must continue to the Overview");
    CHECK(3732, read_config_bytes() == before, "(b) Discard must leave config.toml byte-identical");

    // The editor's own prompt (back/close) offers the same localized buttons.
    CHECK(4108, open_backup_detail(w, bid), "backupDetailPage did not open");
    page = open_editor(w, "editConditionsRow", "backupConditionsEditPage");
    CHECK(3740, page != nullptr, "conditions editor did not reopen");
    metered = lookup(w, "skipMeteredCheck");
    CHECK(3741, click_ui_item(w, metered, "skipMeteredCheck"), "failed to click metered option");
    // The known networks arrive from the daemon after the editor opened; the layout places
    // their rows in a later pass.
    wait_until([&] { return lookup(w, "knownNetworkCheck_HomeWifi") != nullptr; }, 10000);
    wait_ms(100); // quiet window: the layout places the rows of the networks
    QQuickItem* home = lookup(w, "knownNetworkCheck_HomeWifi");
    CHECK(3742, click_ui_item(w, home, "knownNetworkCheck_HomeWifi"), "failed to click HomeWifi");
    wait_until([&] {
        QQuickItem* editor = lookup(w, "backupConditionsEditPage");
        return editor && editor->property("dirty").toBool();
    }, 150);
    QQuickItem* close_btn = lookup(w, "closeConditionsActionButton");
    CHECK2(3743, close_btn, click_ui_item(w, close_btn, "closeConditionsActionButton"), "failed to click Cancel button");
    QObject* own = lookup<QObject>(w, "unsavedConditionsDialog");
    rc = check_unsaved_dialog_open(c, own, 3750);
    if (rc != 0) {
        return rc;
    }
    rc = cancel_unsaved_dialog(own, 3760);
    if (rc != 0) {
        return rc;
    }
    CHECK(3762, page->property("dirty").toBool(), "edits must survive Cancel of the editor prompt");
    // [Save] persists (checked on the test side).
    QQuickItem* save_btn = lookup(w, "saveConditionsActionButton");
    CHECK2(3763, save_btn, click_ui_item(w, save_btn, "saveConditionsActionButton"), "failed to click Save");
    CHECK(3764, wait_until([&] { return read_config_bytes() != before; }, 5000), "Save must write config.toml");
    return shot(c, 3765);
}

// AC-M3-14-4 (per backup) and AC-M3-14-6 (pause menu as keyboard-only editor).
int scenario_pause_menu(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    CHECK(3801, !bid.isEmpty(), "no backup found");
    CHECK(4109, open_backup_detail(w, bid), "backupDetailPage did not open");

    struct Entry { const char* object; const char* en; const char* de; };
    const Entry entries[] = {
        { "pauseBackup1hItem", "1 Hour", "1 Stunde" },
        { "pauseBackup3hItem", "3 Hours", "3 Stunden" },
        { "pauseBackupTomorrowItem", "Until Tomorrow", "Bis morgen" },
        { "pauseBackupIndefiniteItem", "Until I Resume", "Bis ich fortsetze" },
    };
    auto check_entry = [&](const Entry& e, int code) {
        QObject* item = lookup<QObject>(w, e.object);
        CHECK(code, item != nullptr, "per-backup pause menu item missing");
        CHECK_EQ(code + 1, item->property("text").toString(), loc(c.lang, e.en, e.de), e.object);
        CHECK_EQ(code + 2, accessible_name(item), loc(c.lang, e.en, e.de), "pause item accessible name");
        return 0;
    };
    CHECK_RC(check_entry(entries[0], 3810));
    CHECK_RC(check_entry(entries[1], 3813));
    CHECK_RC(check_entry(entries[2], 3816));
    CHECK_RC(check_entry(entries[3], 3819));

    QObject* pause_act = lookup<QObject>(w, "backupDetailPauseAction");
    CHECK(3830, pause_act != nullptr, "backupDetailPauseAction missing");
    CHECK_EQ(3831, pause_act->property("text").toString(), loc(c.lang, "Pause…", "Pausieren…"), "pause action text");
    QMetaObject::invokeMethod(pause_act, "trigger");
    QObject* menu = lookup<QObject>(w, "backupPauseMenu");
    CHECK(3832, wait_until([&] { return is_open(menu); }, 2000), "pause menu did not open");
    CHECK(3833, save_screenshot(w, c.out_path), "screenshot failed");

    // Keyboard only: arrow keys walk through the four choices in visual order.
    auto check_focus = [&](const char* expected, int code) {
        QQuickItem* focus = w->activeFocusItem();
        CHECK(code, focus != nullptr, "no keyboard focus in the pause menu");
        CHECK_EQ(code + 1, focus->objectName(), QString::fromLatin1(expected), "focused pause menu item");
        return 0;
    };
    CHECK_RC(check_focus("pauseBackup1hItem", 3840));
    send_key(w, Qt::Key_Down);
    CHECK_RC(check_focus("pauseBackup3hItem", 3842));
    send_key(w, Qt::Key_Down);
    CHECK_RC(check_focus("pauseBackupTomorrowItem", 3844));
    send_key(w, Qt::Key_Down);
    CHECK_RC(check_focus("pauseBackupIndefiniteItem", 3846));
    send_key(w, Qt::Key_Return);
    QQuickItem* banner = lookup(w, "backupDetailPausedBanner");
    CHECK(3850, banner != nullptr, "backupDetailPausedBanner missing");
    CHECK(3851, wait_until([&] { return banner->isVisible(); }, 3000), "Enter on «Until I Resume» must pause the backup");
    CHECK_STARTS(3852, banner->property("text").toString(), loc(c.lang, "Paused since ", "Pausiert seit "), "paused banner language");
    CHECK_EQ(3853, pause_act->property("text").toString(), loc(c.lang, "Resume", "Fortsetzen"), "pause action while paused");
    QMetaObject::invokeMethod(pause_act, "trigger");
    CHECK(3854, wait_until([&] { return !banner->isVisible(); }, 3000), "action must resume the backup");
    return 0;
}

const QStringList kMaintenanceControls {
    QStringLiteral("pruneFrequencyCombo"),
    QStringLiteral("pruneHourSpin"),
    QStringLiteral("pruneMinuteSpin"),
    QStringLiteral("checkFrequencyCombo"),
    QStringLiteral("checkHourSpin"),
    QStringLiteral("checkMinuteSpin"),
    QStringLiteral("closeMaintenanceActionButton"),
    QStringLiteral("saveMaintenanceActionButton"),
};

// Switches `combo` to `index` the way a user pick does.
void pick_combo_index(QQuickItem* combo, int index) {
    combo->setProperty("currentIndex", index);
    QMetaObject::invokeMethod(combo, "activated", Q_ARG(int, index));
    wait_ms(150); // quiet window: the page lays out the fields of the new choice
}

// AC-M3-14-5 (a) and (b), AC-M3-05-7 (a): defaults of the maintenance editor, then the prune
// preview that must be shown before a changed clean-up schedule can be confirmed.
int scenario_maintenance_prune_confirm(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString did = destination_id_at(w, 0);
    CHECK(3901, !did.isEmpty(), "no destination found");
    QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
    wait_until([&] { return lookup(w, "destinationDetailPage") != nullptr; }, 250);

    // (a) defaults: «Clean up: Sundays 12:00» and «Verify: first Sunday of the month 12:00».
    QString summary;
    QMetaObject::invokeMethod(
        c.backend,
        "formatMaintenanceSummaryJson",
        Qt::DirectConnection,
        Q_RETURN_ARG(QString, summary),
        Q_ARG(QString, QStringLiteral("{\"prune\":{\"kind\":\"weekly\",\"weekday\":\"sunday\",\"hour\":12,\"minute\":0},\"check\":{\"kind\":\"monthly\",\"nth\":\"first\",\"weekday\":\"sunday\",\"hour\":12,\"minute\":0}}"))
    );
    CHECK(3902, summary.contains(loc(c.lang, "Clean up: Sundays at 12:00", "Aufräumen: sonntags 12:00")), "(a) clean-up default summary");
    CHECK(3903, summary.contains(loc(c.lang, "Verify: first Sunday of the month at 12:00", "Prüfen: erster Sonntag im Monat 12:00")),
        "(a) verify default summary");
    QQuickItem* content = w->contentItem();
    CHECK(3905, has_visible_text(content, loc(c.lang, "Next verify:", "Nächstes Prüfen:")), "detail must show «Next verify:»");
    // One backup is named in the singular.
    CHECK(3940, has_visible_text(content, loc(c.lang, "1 configured backup", "1 konfiguriertes Backup")),
        "detail must show the singular «1 configured backup»");
    CHECK(3941, !has_visible_text(content, loc(c.lang, "1 configured backups", "1 konfigurierte Backups")),
        "detail must not use the plural for one backup");

    QQuickItem* page = open_editor(w, "editMaintenanceRow", "destinationMaintenancePage");
    CHECK(3906, page != nullptr, "destinationMaintenancePage did not open");
    CHECK_EQ(3907, spin_text(lookup(w, "pruneHourSpin")), QStringLiteral("12"), "prune hour field");
    CHECK_EQ(3908, spin_text(lookup(w, "pruneMinuteSpin")), QStringLiteral("00"), "prune minute field is two-digit");

    pick_combo_index(lookup(w, "pruneFrequencyCombo"), 2); // Daily

    // AC-M3-14-5 (b) / AC-M3-05-7 (a): changing the clean-up schedule asks for confirmation
    // and shows the dry-run preview result before it can be confirmed.
    QQuickItem* save_btn = lookup(w, "saveMaintenanceActionButton");
    CHECK2(3911, save_btn, save_btn->isEnabled(), "Save must be enabled after changing the clean-up schedule");
    CHECK(3912, click_ui_item(w, save_btn, "saveMaintenanceActionButton"), "failed to click Save");
    QObject* dlg = lookup<QObject>(w, "pruneConfirmDialog");
    CHECK(3913, wait_until([&] { return is_open(dlg); }, 3000), "pruneConfirmDialog did not open");
    QObject* confirm = dialog_action(dlg, "confirmPruneSaveAction");
    CHECK(3914, confirm != nullptr, "confirmPruneSaveAction missing");
    if (!page->property("previewSettled").toBool()) {
        CHECK(3915, !confirm->property("enabled").toBool(), "confirmation must stay disabled until the preview loaded");
    }
    CHECK(3916, wait_until([&] { return page->property("previewSettled").toBool(); }, 20000), "preview did not load or fail");
    wait_until([&] {
        QQuickItem* shown = lookup(w, "prunePreviewSummaryLabel");
        return shown && shown->isVisible();
    }, 200);
    QQuickItem* result = lookup(w, "prunePreviewSummaryLabel");
    CHECK2(3917, result, result->isVisible(), "(a) preview result must be shown before confirming");
    CHECK_EQ(
        3918,
        result->property("text").toString(),
        loc(c.lang, "Data blocks to remove: 0\nSpace to free: 0 B", "Zu entfernende Datenblöcke: 0\nFrei werdender Speicher: 0 B"),
        "preview result text"
    );
    CHECK(3919, confirm->property("enabled").toBool(), "confirmation must be enabled once the preview is shown");
    CHECK(3920, save_screenshot(w, c.out_path), "screenshot failed");
    QMetaObject::invokeMethod(confirm, "trigger");
    // The confirmed save runs before the scenario ends.
    wait_until([&] { return is_closed(dlg) && !page->property("saving").toBool(); }, 300);
    return 0;
}

// AC-M3-14-6 (maintenance editor): every control has an accessible name and is reachable by
// keyboard; a discarded check-frequency change leaves the persisted schedule untouched.
int scenario_maintenance_discard(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString did = destination_id_at(w, 0);
    CHECK(3942, !did.isEmpty(), "no destination found");
    QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
    wait_until([&] { return lookup(w, "destinationDetailPage") != nullptr; }, 250);
    QQuickItem* page = open_editor(w, "editMaintenanceRow", "destinationMaintenancePage");
    CHECK(3943, page != nullptr, "destinationMaintenancePage did not open");

    const QString unnamed = first_unnamed(w, kMaintenanceControls);
    CHECK(3909, unnamed.isEmpty(), qPrintable(QStringLiteral("(b) control without Accessible.name: ") + unnamed));

    QQuickItem* prune_combo = lookup(w, "pruneFrequencyCombo");
    pick_combo_index(prune_combo, 2); // Daily
    prune_combo->forceActiveFocus(Qt::TabFocusReason);
    QCoreApplication::processEvents();
    const QStringList missing = tab_missing(w, kMaintenanceControls, 80);
    CHECK(3910, missing.isEmpty(), qPrintable(QStringLiteral("(a) not reachable by keyboard: ") + missing.join(QLatin1Char(','))));

    pick_combo_index(lookup(w, "checkFrequencyCombo"), 3); // manual only
    send_key(w, Qt::Key_Escape);
    QObject* unsaved = lookup<QObject>(w, "unsavedMaintenanceDialog");
    CHECK(3921, wait_until([&] { return is_open(unsaved); }, 3000), "unsavedMaintenanceDialog did not open on Escape");
    CHECK(3944, save_screenshot(w, c.out_path), "screenshot failed");
    QMetaObject::invokeMethod(dialog_action(unsaved, "unsavedDiscardAction"), "trigger");
    wait_until([&] { return is_closed(unsaved); }, 250);
    return 0;
}

// Settings dialog: monitoring and notification controls persist.
int scenario_settings(const Ctx& c) {
    QQuickWindow* w = c.window;
    QMetaObject::invokeMethod(w, "openSettingsWindow");
    wait_until([&] {
        auto* shown = qobject_cast<QQuickWindow*>(lookup<QObject>(w, "settingsWindow"));
        return shown && shown->isVisible();
    }, 250);
    QObject* settings = lookup<QObject>(w, "settingsWindow");
    CHECK(4001, settings != nullptr, "settingsWindow not found");
    const QStringList controls {
        QStringLiteral("defaultOverdueSpinBox"),
        QStringLiteral("notifErrorSwitch"),
        QStringLiteral("notifWarningSwitch"),
        QStringLiteral("notifOverdueSwitch"),
        QStringLiteral("notifMaintenanceSwitch"),
    };
    for (const QString& name : controls) {
        QObject* ctl = settings->findChild<QObject*>(name);
        CHECK(4002, ctl != nullptr, qPrintable(QStringLiteral("settings control missing: ") + name));
        CHECK(4003, !accessible_name(ctl).trimmed().isEmpty(), qPrintable(QStringLiteral("settings control without Accessible.name: ") + name));
    }
    QObject* overdue_spin = settings->findChild<QObject*>(QStringLiteral("defaultOverdueSpinBox"));
    QObject* notif_overdue = settings->findChild<QObject*>(QStringLiteral("notifOverdueSwitch"));
    overdue_spin->setProperty("value", 4);
    QMetaObject::invokeMethod(overdue_spin, "valueModified");
    notif_overdue->setProperty("checked", false);
    QMetaObject::invokeMethod(notif_overdue, "toggled");
    wait_ms(300); // quiet window: the settings save the switch before the screenshot
    return shot(c, 4004);
}

// Every way of leaving a dirty editor asks first, also when a page sits on top of the editor
// (the dry-run preview of the exclude rules) and when the window is closed.
int scenario_editor_guard_paths(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    CHECK(4201, !bid.isEmpty(), "no backup found");
    CHECK(4202, open_backup_detail(w, bid), "backupDetailPage did not open");
    QQuickItem* editor = open_editor(w, "editExcludesRow", "backupExcludesEditPage");
    CHECK(4203, editor != nullptr, "backupExcludesEditPage did not open");
    const QByteArray before = read_config_bytes();
    CHECK(4204, !before.isEmpty(), "config.toml must exist");
    editor->setProperty("excludeCaches", true);
    CHECK(4205, wait_until([&] { return editor->property("dirty").toBool(); }, 150), "editor must be dirty after editing");

    QVariant excludes_json;
    QMetaObject::invokeMethod(editor, "currentExcludesJson", Q_RETURN_ARG(QVariant, excludes_json));
    QMetaObject::invokeMethod(w, "openBackupDryRunPreviewPage", Q_ARG(QVariant, bid), Q_ARG(QVariant, excludes_json));
    CHECK(4206, wait_until([&] { return lookup(w, "backupPreviewPage") != nullptr; }, 3000), "preview page did not open");
    wait_ms(200); // quiet window: the page ends its transition
    QObject* dialog = lookup<QObject>(w, "unsavedTabSwitchDialog");
    CHECK(4207, dialog != nullptr, "unsavedTabSwitchDialog missing");

    // (a) the breadcrumb back to the backup detail asks, and Cancel keeps editor and preview.
    QMetaObject::invokeMethod(w, "navigateToBreadcrumbIndex", Q_ARG(QVariant, 1));
    CHECK(4210, wait_until([&] { return is_open(dialog); }, 3000), "(a) breadcrumb under the preview must ask first");
    int rc = cancel_unsaved_dialog(dialog, 4211);
    if (rc != 0) {
        return rc;
    }
    CHECK(4213, lookup(w, "backupExcludesEditPage") != nullptr, "(a) Cancel must keep the dirty editor");
    CHECK(4214, editor->property("dirty").toBool(), "(a) Cancel must keep the edits");
    CHECK(4215, lookup(w, "backupPreviewPage") != nullptr, "(a) Cancel must keep the preview");

    // (b) [Save] in the prompt saves the editor below the preview and continues.
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    CHECK(4220, wait_until([&] { return is_open(dialog); }, 3000), "(b) tab switch must ask first");
    QMetaObject::invokeMethod(dialog_action(dialog, "unsavedSaveAction"), "trigger");
    CHECK(4221, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "(b) Save must continue to the Overview");
    CHECK(4222, read_config_bytes() != before, "(b) Save must persist the edited exclude rules");

    // (c) closing the window with a dirty editor asks first; Cancel keeps the window.
    CHECK(4230, open_backup_detail(w, bid), "backupDetailPage did not reopen");
    editor = open_editor(w, "editExcludesRow", "backupExcludesEditPage");
    CHECK(4231, editor != nullptr, "backupExcludesEditPage did not reopen");
    editor->setProperty("excludeCaches", false);
    CHECK(4232, wait_until([&] { return editor->property("dirty").toBool(); }, 150), "(c) editor must be dirty after editing");
    w->close();
    CHECK(4233, wait_until([&] { return is_open(dialog); }, 3000), "(c) closing the window must ask first");
    CHECK(4234, w->isVisible(), "(c) the window must stay open while asking");
    rc = cancel_unsaved_dialog(dialog, 4235);
    if (rc != 0) {
        return rc;
    }
    CHECK(4237, w->isVisible(), "(c) Cancel must keep the window open");
    CHECK(4238, editor->property("dirty").toBool(), "(c) Cancel must keep the edits");
    return shot(c, 4239);
}

// Asks the test to change the backup or destination elsewhere (step `step`) and waits until the
// window has the new revision of `id` (`kind` «backup» or «destination»).
bool request_change_elsewhere(QObject* backend, const char* kind, const QString& id, int step) {
    const QString before = backend_revision(backend, kind, id);
    QFile marker(qEnvironmentVariable("KESTIC_SMOKE_SYNC_DIR") + QStringLiteral("/change-%1").arg(step));
    if (!marker.open(QIODevice::WriteOnly)) {
        return false;
    }
    marker.close();
    return wait_until([&] { return backend_revision(backend, kind, id) != before; }, 10000);
}

// Opens the editor `page` of the backup `bid` with the window function `opener`.
QQuickItem* open_backup_editor(QQuickWindow* w, const QString& bid, const char* opener, const char* page) {
    // The page open before must be complete when it closes now.
    finish_incubation(w);
    if (!open_backup_detail(w, bid)) {
        return nullptr;
    }
    // The schedule opens through its row on the detail page, like a user would.
    if (qstrcmp(page, "backupScheduleEditPage") == 0) {
        return open_editor(w, "editScheduleRow", page);
    }
    QMetaObject::invokeMethod(w, opener, Q_ARG(QVariant, bid));
    wait_until([&] { return lookup(w, page) != nullptr; }, 3000);
    wait_ms(200); // quiet window: the page ends its transition
    return lookup(w, page);
}

// Saves `page` through its save function and waits for the answer (`saving` back to false).
void save_page(QQuickItem* page, const char* method, const QVariantList& args) {
    switch (args.size()) {
    case 0:
        QMetaObject::invokeMethod(page, method, Q_ARG(QVariant, QVariant()));
        break;
    case 1:
        QMetaObject::invokeMethod(page, method, Q_ARG(QVariant, args[0]), Q_ARG(QVariant, QVariant()));
        break;
    default:
        break;
    }
    wait_until([&] { return !page->property("saving").toBool(); }, 15000);
    // The page reloads after the answer: it shows the stored state again.
    wait_until([&] { return !page->property("dirty").toBool(); }, 200);
    // A reload after the answer may create delegates again; they finish before the page closes.
    finish_incubation(page->window());
}

struct ConflictEditor {
    const char* opener;
    const char* page;
    const char* save;
    // The property the edit changes, and its new value.
    const char* field;
    QVariant value;
};

// AC-M6-36-10 (a)-(c): an editor page opened before a change made elsewhere refuses to overwrite
// it: the save sends the page's revision, the daemon answers `Conflict`, the page says so,
// shows the current state and takes its revision.
int check_conflict(QObject* backend, QQuickItem* page, const char* kind, const QString& id, int step, int base) {
    CHECK(base, page != nullptr, "editor page did not open");
    const QString opened_with = page->property("revision").toString();
    CHECK(base + 1, opened_with == backend_revision(backend, kind, id), "(a) the page keeps the revision it opened with");
    CHECK(base + 2, request_change_elsewhere(backend, kind, id, step), "the change made elsewhere did not arrive");
    CHECK(base + 3, page->property("revision").toString() == opened_with,
        "(a) the page must not take the new revision from the window's state");
    return 0;
}

// The message of AC-M6-36-10 (b) for `kind` in `lang`.
QString changed_elsewhere_text(const char* kind, const QString& lang) {
    const bool dest = qstrcmp(kind, "destination") == 0;
    if (lang == QLatin1String("de")) {
        return dest ? QStringLiteral("Dieses Ziel wurde inzwischen an anderer Stelle geändert. Der aktuelle Stand ist "
                                     "geladen; Ihre Änderung ist nicht gespeichert.")
                    : QStringLiteral("Dieses Backup wurde inzwischen an anderer Stelle geändert. Der aktuelle Stand ist "
                                     "geladen; Ihre Änderung ist nicht gespeichert.");
    }
    return dest ? QStringLiteral("This destination was changed elsewhere in the meantime. The current version is loaded; "
                                 "your change was not saved.")
                : QStringLiteral("This backup was changed elsewhere in the meantime. The current version is loaded; "
                                 "your change was not saved.");
}

int check_reloaded(QObject* backend, QQuickItem* page, const char* kind, const QString& id, const QString& lang,
    int base) {
    QQuickItem* message = find_visual_item(page->window(), QStringLiteral("editorChangedElsewhereMessage"));
    CHECK(base, message && message->isVisible(), "(b) the page must say the backup or destination changed");
    CHECK(base + 1, message->property("text").toString() == changed_elsewhere_text(kind, lang),
        "(b) the message must be the text of AC-M6-36-10 (b)");
    // Kirigami.MessageType.Warning
    CHECK(base + 8, message->property("type").toInt() == 2, "(b) the message is a warning");
    CHECK(base + 2, !page->property("dirty").toBool(), "(c) the page must show the current state again");
    CHECK(base + 3, page->property("revision").toString() == backend_revision(backend, kind, id),
        "(c) the page must hold the current revision");
    return 0;
}

int scenario_revision_conflict(const Ctx& c) {
    QQuickWindow* w = c.window;
    QObject* backend = c.backend;
    const QString bid = backup_id_at(w, 0);
    const QString did = destination_id_at(w, 0);
    CHECK(6100, !bid.isEmpty() && !did.isEmpty(), "no backup or destination found");
    const ConflictEditor editors[] = {
        { "openBackupConditionsPage", "backupConditionsEditPage", "saveForGuard", "requireAcPower", false },
        { "openBackupSchedulePage", "backupScheduleEditPage", "saveForGuard", "hour", 13 },
        { "openBackupExcludesPage", "backupExcludesEditPage", "saveForGuard", "excludeCaches", true },
        { "openBackupRetentionPage", "backupRetentionPage", "commitSave", "keepDailyNum", 9 },
        { "openBackupSourcePage", "backupSourceEditPage", "saveForGuard", "sourcePath", QStringLiteral("/tmp") },
    };
    auto run_editor = [&](const ConflictEditor& e, int step, int base) {
        QQuickItem* page = open_backup_editor(w, bid, e.opener, e.page);
        CHECK_RC(check_conflict(backend, page, "backup", bid, step, base));
        if (e.field) {
            // A switch flips, so the page is dirty also when an earlier run left it at the value.
            const QVariant current = page->property(e.field);
            page->setProperty(e.field, current.typeId() == QMetaType::Bool ? QVariant(!current.toBool()) : e.value);
            wait_until([&] { return page->property("dirty").toBool(); }, 100);
        }
        QVariantList args;
        if (QLatin1String(e.save) == QLatin1String("commitSave")) {
            QVariant json;
            QMetaObject::invokeMethod(page, "currentPolicyJson", Q_RETURN_ARG(QVariant, json));
            args << json;
        }
        save_page(page, e.save, args);
        CHECK_RC(check_reloaded(backend, page, "backup", bid, c.lang, base + 4));
        // The first change elsewhere also switched «skip on metered networks» on: the page shows it.
        if (step == 1) {
            CHECK(6129, page->property("skipOnMetered").toBool(), "(c) the page must show the change made elsewhere");
        }
        return 0;
    };
    CHECK_RC(run_editor(editors[0], 1, 10302));
    CHECK_RC(run_editor(editors[1], 2, 6130));
    CHECK_RC(run_editor(editors[2], 3, 6150));
    CHECK_RC(run_editor(editors[3], 4, 6170));
    CHECK_RC(run_editor(editors[4], 5, 6190));

    // The destination form: a new name, while the maintenance schedule was changed elsewhere.
    finish_incubation(w);
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 2));
    wait_until([&] { return w->property("activeTab").toInt() == 2; }, 150);
    QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
    wait_until([&] { return lookup(w, "destinationDetailPage") != nullptr; }, 200);
    QMetaObject::invokeMethod(w, "openDestinationEditPage", Q_ARG(QVariant, did));
    wait_until([&] { return lookup(w, "destinationEditPage") != nullptr; }, 3000);
    wait_ms(200); // quiet window: the page ends its transition
    QQuickItem* dest_page = lookup(w, "destinationEditPage");
    int rc = check_conflict(backend, dest_page, "destination", did, 6, 10315);
    if (rc != 0) {
        return rc;
    }
    QObject* draft = nullptr;
    for (QObject* child : dest_page->findChildren<QObject*>()) {
        if (child->metaObject()->indexOfProperty("connectionAccepted") >= 0) {
            draft = child;
            break;
        }
    }
    CHECK(6232, draft != nullptr, "destination draft missing");
    const QString typed_name = draft->property("name").toString() + QStringLiteral(" typed");
    draft->setProperty("name", typed_name);
    // A typed password makes the save check the connection; the page treats it as tested.
    draft->setProperty("repoPassword", QUuid::createUuid().toString(QUuid::WithoutBraces));
    draft->setProperty("connectionVerified", true);
    CHECK(6230, wait_until([&] { return dest_page->property("canSave").toBool(); }, 100), "precondition: the destination form can save");
    save_page(dest_page, "saveForGuard", {});
    rc = check_reloaded(backend, dest_page, "destination", did, c.lang, 10319);
    if (rc != 0) {
        return rc;
    }
    const QString daemon_name =
        role_value(model_named(w, "destinationsModel"), model_row(w, "destinationsModel", "destinationId", did), "name")
            .toString();
    CHECK(6231, draft->property("repoPassword").toString().isEmpty(), "(c) the typed password must be cleared");
    // The message after the refused save, for the UI review.
    CHECK(6234, save_screenshot(w, QString(c.out_path).replace(QStringLiteral(".png"), QStringLiteral("-conflict.png"))),
        "conflict screenshot failed");
    CHECK(6233, draft->property("name").toString() == daemon_name && daemon_name != typed_name,
        "(c) the form must show the name the destination got elsewhere");

    // AC-M6-36-10 (d): two saves right after another on one page, and (e) a second page opened
    // right after a save: no `Conflict` with the window's own saves.
    QQuickItem* cond = open_backup_editor(w, bid, "openBackupConditionsPage", "backupConditionsEditPage");
    CHECK(6240, cond != nullptr, "conditions page did not reopen");
    cond->setProperty("requireAcPower", !cond->property("requireAcPower").toBool());
    wait_until([&] { return cond->property("dirty").toBool(); }, 100);
    QMetaObject::invokeMethod(cond, "saveForGuard", Q_ARG(QVariant, QVariant()));
    QMetaObject::invokeMethod(cond, "saveForGuard", Q_ARG(QVariant, QVariant()));
    wait_until([&] { return !cond->property("saving").toBool(); }, 15000);
    wait_ms(200); // quiet window: a conflict of the second save would show by now
    CHECK(6241, !cond->property("changedElsewhere").toBool(), "(d) two quick saves must not conflict");
    cond->setProperty("requireAcPower", !cond->property("requireAcPower").toBool());
    wait_until([&] { return cond->property("dirty").toBool(); }, 100);
    save_page(cond, "saveForGuard", {});
    CHECK(6242, !cond->property("changedElsewhere").toBool(), "(d) the next save takes the returned revision");
    CHECK(6243, !cond->property("dirty").toBool(), "(d) the save went through");
    // A save of another page queued while the first is on its way, with the revision from before
    // the first: it only missed the window's own save, so it must not conflict.
    const QString before_both = backend_revision(backend, "backup", bid);
    const QString schedule_json =
        role_value(model_named(w, "backupsModel"), model_row(w, "backupsModel", "backupId", bid), "scheduleJson").toString();
    QSignalSpy answers(backend, SIGNAL(daemonCallDone(int, bool, QString)));
    cond->setProperty("requireAcPower", !cond->property("requireAcPower").toBool());
    wait_until([&] { return cond->property("dirty").toBool(); }, 100);
    QMetaObject::invokeMethod(cond, "saveForGuard", Q_ARG(QVariant, QVariant()));
    int queued = 0;
    QMetaObject::invokeMethod(backend, "updateBackupSchedule", Q_RETURN_ARG(int, queued), Q_ARG(QString, bid),
        Q_ARG(QString, schedule_json), Q_ARG(int, 0), Q_ARG(QString, before_both));
    CHECK(10329, queued > 0, "the queued save was not sent");
    bool queued_ok = false;
    CHECK(6251, wait_until([&] {
        for (const QList<QVariant>& answer : answers) {
            if (answer.at(0).toInt() == queued) {
                queued_ok = answer.at(1).toBool();
                return true;
            }
        }
        return false;
    }, 15000), "the queued save got no answer");
    CHECK(6252, queued_ok, "(d) a save queued behind the window's own save must not conflict");
    wait_until([&] { return !cond->property("saving").toBool(); }, 15000);
    // The page reloads after the answer: it shows the stored state again.
    wait_until([&] { return !cond->property("dirty").toBool(); }, 200);

    QQuickItem* sched = open_backup_editor(w, bid, "openBackupSchedulePage", "backupScheduleEditPage");
    CHECK(6244, sched != nullptr, "schedule page did not open");
    sched->setProperty("hour", sched->property("hour").toInt() == 14 ? 15 : 14);
    wait_until([&] { return sched->property("dirty").toBool(); }, 100);
    save_page(sched, "saveForGuard", {});
    CHECK(6245, !sched->property("changedElsewhere").toBool(), "(e) a page opened after a save must not conflict");
    CHECK(6246, !sched->property("dirty").toBool(), "(e) the save went through");
    return shot(c, 10328);
}

const Scenario kScenarios[] = {
    {"m3-schedule-editor-defaults", scenario_schedule_defaults},
    {"m3-schedule-editor-validation", scenario_schedule_validation},
    {"m3-schedule-editor-unsaved", scenario_schedule_unsaved},
    {"m3-conditions-editor-defaults", scenario_conditions_defaults},
    {"m3-conditions-editor-unsaved", scenario_conditions_unsaved},
    {"m3-conditions-editor-local", scenario_conditions_local},
    {"m3-pause-menu-keyboard", scenario_pause_menu},
    {"m3-maintenance-editor-prune-confirm", scenario_maintenance_prune_confirm},
    {"m3-maintenance-editor-discard", scenario_maintenance_discard},
    {"m3-settings-monitoring", scenario_settings},
    {"m3-editor-guard-paths", scenario_editor_guard_paths},
    {"m3-revision-conflict", scenario_revision_conflict},
};

} // namespace

m3::ScenarioFn find_m3_editor_scenario(const QString& scen) {
    return find_scenario(kScenarios, scen);
}

} // namespace kestic::smoke
