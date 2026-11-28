// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// M4 GUI smoke scenarios of the «Restore» area: the backup and snapshot lists, the comparison
// and keyboard use of the lists.

#include "src/smoke_m4_util.h"

namespace kestic::smoke {

namespace {

using namespace m4;

// Records the requests of the snapshot list page: `last` is «browse:<id>»,
// «compare:<from>><to>» or «restore:<id>», `count` the number of requests.
QObject* request_spy(QQuickWindow* w, QObject* page) {
    QQmlEngine* engine = qmlEngine(w);
    if (!engine) {
        return nullptr;
    }
    QQmlComponent component(engine);
    component.setData(
        "import QtQml\n"
        "QtObject {\n"
        "    property string last: \"\"\n"
        "    property int count: 0\n"
        "    function browse(d, s) { last = \"browse:\" + s; count += 1 }\n"
        "    function compare(d, a, b) { last = \"compare:\" + a + \">\" + b; count += 1 }\n"
        "    function restore(d, s) { last = \"restore:\" + s; count += 1 }\n"
        "}\n",
        QUrl());
    QObject* spy = component.create();
    if (!spy) {
        return nullptr;
    }
    spy->setParent(page);
    auto link = [&](const char* signal, const char* method) {
        const QMetaObject* from = page->metaObject();
        const QMetaObject* to = spy->metaObject();
        const int si = from->indexOfSignal(QMetaObject::normalizedSignature(signal).constData());
        const int mi = to->indexOfMethod(QMetaObject::normalizedSignature(method).constData());
        return si >= 0 && mi >= 0
            && static_cast<bool>(QObject::connect(page, from->method(si), spy, to->method(mi)));
    };
    const bool linked = link("browseRequested(QVariant,QVariant)", "browse(QVariant,QVariant)")
        && link("compareRequested(QVariant,QVariant,QVariant)", "compare(QVariant,QVariant,QVariant)")
        && link("restoreRequested(QVariant,QVariant)", "restore(QVariant,QVariant)");
    return linked ? spy : nullptr;
}

// The delegate of the snapshot list that holds the keyboard focus, or nullptr.
QQuickItem* focused_snapshot_row(QQuickWindow* w) {
    for (QQuickItem* it = w->activeFocusItem(); it; it = it->parentItem()) {
        if (it->objectName() == QLatin1String("restoreSnapshotListItem")) {
            return it;
        }
    }
    return nullptr;
}

// Moves the highlight of the open context menu to `index` with the arrow keys.
bool menu_to(QQuickWindow* w, QObject* menu, int index) {
    for (int i = 0; i < 6 && menu->property("currentIndex").toInt() != index; ++i) {
        const int at = menu->property("currentIndex").toInt();
        press_until(w, Qt::Key_Down, [&] { return menu->property("currentIndex").toInt() != at; }, 60);
    }
    return menu->property("currentIndex").toInt() == index;
}

// The visible buttons of the window labelled `text` (mnemonic marks removed).
int visible_buttons_labelled(QQuickWindow* w, const QString& text) {
    QList<QQuickItem*> items;
    collect_visual_descendants(w->contentItem(), items);
    int n = 0;
    for (QQuickItem* it : items) {
        if (it->isVisible() && it->inherits("QQuickAbstractButton")
            && it->property("text").toString().remove(QLatin1Char('&')) == text) {
            ++n;
        }
    }
    return n;
}

/// AC-M4-01-6: the tab bar shows «Restore» between «Overview» and «Backups» (checked for every
/// scene in `smoke_m1.cpp`), and (b) the area opens the snapshot selection: the backups with
/// their number of snapshots and, next to it, the snapshots of the first backup.
int scenario_restore_tab(const Ctx& c) {
    QQuickWindow* w = c.window;
    QObject* tab = lookup<QObject>(w, "restoreTabAction");
    CHECK(5101, tab != nullptr, "restoreTabAction missing");
    CHECK_EQ(5102, tab->property("text").toString(), loc(c.lang, "Restore", "Wiederherstellen"),
        "(a) tab label");
    trigger(tab);
    CHECK(5103, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "(b) Restore tab active");
    CHECK(5104, wait_until([&] { return visible_item(w, "restorePage") != nullptr; }, 3000),
        "(b) restore page shown");
    CHECK(5105, wait_until([&] { return !c.backend->property("restore_overview_loading").toBool(); }, 20000),
        "(b) snapshot counts loaded");
    finish_incubation(w);
    CHECK(5106, model_rows(w, "restoreBackupsModel") == 2, "(b) both backups listed");
    QAbstractItemModel* backups = model_named(w, "restoreBackupsModel");
    CHECK_EQ(5107, role_value(backups, 0, "subtitle").toString(),
        loc(c.lang, "3 snapshots", "3 Snapshots"), "(b) number of snapshots of the first backup");
    CHECK_EQ(5115, role_value(backups, 1, "subtitle").toString(),
        loc(c.lang, "0 snapshots", "0 Snapshots"), "(b) number of snapshots of the second backup");
    const QString refresh = loc(c.lang, "Refresh", "Aktualisieren");
    if (w->property("narrowMode").toBool()) {
        // One column: the snapshot list opens from the backup row.
        CHECK(5116, visible_buttons_labelled(w, refresh) == 1, "the backup list can be refreshed");
        return shot(c, 5114);
    }
    CHECK(5108, wait_for_snapshots(w, c.backend, 3), "(b) snapshots of the first backup listed");
    finish_incubation(w);
    QQuickItem* snapshots_page = visible_item(w, "restoreSnapshotsPage");
    CHECK(5109, snapshots_page != nullptr, "(b) snapshot list shown next to the backups");
    CHECK_EQ(5110, snapshots_page->property("title").toString(), QStringLiteral("Home"),
        "(b) snapshot list titled with the backup");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    CHECK_EQ(5111, role_value(snaps, 0, "sectionName").toString(), loc(c.lang, "Today", "Heute"),
        "newest snapshot first, grouped under today");
    CHECK(5112, role_value(snaps, 0, "subtitle").toString().contains(loc(c.lang, " new · laptop", " neu · laptop")),
        "size and host of the newest snapshot");
    // stil-und-texte.md §13.3: no label twice in a window; the snapshot list refreshes both.
    CHECK(5117, wait_until([&] { return visible_buttons_labelled(w, refresh) == 1; }, 2000),
        qPrintable(QStringLiteral("one «Refresh» beside the snapshot list: %1").arg(visible_buttons_labelled(w, refresh))));
    return shot(c, 5113);
}

// Checks that every row of the snapshot list carries `hint` and that the hint is drawn.
int check_hints(QQuickWindow* w, const QString& hint, int code) {
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    CHECK(code, snaps != nullptr && snaps->rowCount() > 0, "snapshot rows listed");
    for (int row = 0; row < snaps->rowCount(); ++row) {
        CHECK_EQ(code + 1, role_value(snaps, row, "hint").toString(), hint, "(a) hint of every row");
    }
    finish_incubation(w);
    int drawn = 0;
    for (QObject* o : m3::objects_named(w, "restoreSnapshotHint")) {
        auto* label = qobject_cast<QQuickItem*>(o);
        if (label && label->isVisible() && label->property("text").toString() == hint) {
            ++drawn;
        }
    }
    CHECK(code + 2, drawn == snaps->rowCount(), "(a) the hint is shown on every row");
    return 0;
}

// Opens the further group `group` (titled `name`): in the wide layout with Tab and Return from
// the group row that holds the focus, in the narrow one (the list is hidden) directly.
bool open_next_group(const Ctx& c, const char* group, const QString& name) {
    QQuickWindow* w = c.window;
    if (w->property("narrowMode").toBool()) {
        return QMetaObject::invokeMethod(w, "openRestoreGroup",
            Q_ARG(QVariant, QString::fromLatin1(group)), Q_ARG(QVariant, name));
    }
    // The groups are one list: Down moves to the next one.
    auto focused_group_row = [w]() -> QQuickItem* {
        for (QQuickItem* it = w->activeFocusItem(); it; it = it->parentItem()) {
            if (it->objectName() == QLatin1String("restoreGroupListItem")) {
                return it;
            }
        }
        return nullptr;
    };
    QQuickItem* before = focused_group_row();
    press_until(w, Qt::Key_Down, [&] { return focused_group_row() != before; }, 80);
    QQuickItem* row = focused_group_row();
    if (!row || row->property("group").toString() != QLatin1String(group)) {
        return false;
    }
    send_key(w, Qt::Key_Return);
    return true;
}

// `out_path` with the group name `group` instead of «unassigned».
QString group_shot(const QString& out_path, const char* group) {
    QString path = out_path;
    return path.replace(QLatin1String("-unassigned-"), QStringLiteral("-%1-").arg(QLatin1String(group)));
}

/// AC-M4-07-8: foreign and orphaned snapshots in the list each carry the hint «Not managed by
/// Kestic»; the snapshots of a backup not configured here say so. The groups open from «Other
/// snapshots» of the restore area with the keyboard.
int scenario_foreign(const Ctx& c) {
    QQuickWindow* w = c.window;
    QObject* tab = lookup<QObject>(w, "restoreTabAction");
    CHECK(5701, tab != nullptr, "restoreTabAction missing");
    trigger(tab);
    CHECK(5702, wait_until([&] {
        return w->property("activeTab").toInt() == 4
            && !c.backend->property("restore_overview_loading").toBool()
            && model_rows(w, "restoreGroupsModel") == 3;
    }, 20000), "the restore area lists three further groups");
    finish_incubation(w);
    QAbstractItemModel* groups = model_named(w, "restoreGroupsModel");
    CHECK_EQ(5703, role_value(groups, 0, "name").toString(), loc(c.lang, "Unassigned", "Nicht zugeordnet"),
        "first group");
    CHECK_EQ(5704, role_value(groups, 0, "subtitle").toString(),
        loc(c.lang, "1 snapshot without a unique Kestic tag", "1 Snapshot ohne eindeutigen Kestic-Tag"), "unassigned count");
    CHECK_EQ(5705, role_value(groups, 1, "subtitle").toString(),
        loc(c.lang, "1 backup without description", "1 Backup ohne Beschreibung"), "orphaned count");
    CHECK_EQ(5706, role_value(groups, 2, "group").toString(), QStringLiteral("described"),
        "the backup not configured here is shown, not hidden");

    QQuickItem* group_row = tab_to(w, "restoreGroupListItem", 40);
    CHECK(5707, group_row != nullptr, "Tab reaches the further groups");
    send_key(w, Qt::Key_Return);
    const QString unassigned = loc(c.lang, "Unassigned", "Nicht zugeordnet");
    CHECK(5708, wait_for_group(w, c.backend, unassigned, 1), "Return opens the unassigned snapshots");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    CHECK_EQ(5709, role_value(snaps, 0, "sectionName").toString(),
        loc(c.lang, "Unassigned on 'NAS'\n1 snapshot", "Nicht zugeordnet auf «NAS»\n1 Snapshot"),
        "grouped by destination, with the number of snapshots");
    const QString not_managed = loc(c.lang, "Not managed by Kestic", "Von Kestic nicht verwaltet");
    if (int rc = check_hints(w, not_managed, 5710)) {
        return rc;
    }
    CHECK(8300, save_screenshot(w, c.out_path), "screenshot failed");

    const QString orphaned = loc(c.lang, "Orphaned", "Verwaist");
    CHECK(8301, open_next_group(c, "orphaned", orphaned), "open the orphaned group");
    CHECK(8302, wait_for_group(w, c.backend, orphaned, 1), "the orphaned snapshots are listed");
    CHECK(8303, role_value(snaps, 0, "sectionName").toString().startsWith(
        loc(c.lang, "Backup without description on 'NAS'\n1 snapshot · latest ",
            "Backup ohne Beschreibung auf «NAS»\n1 Snapshot · zuletzt ")),
        "orphaned backup with its number of snapshots and newest snapshot");
    if (int rc = check_hints(w, not_managed, 5717)) {
        return rc;
    }
    CHECK(8304, save_screenshot(w, group_shot(c.out_path, "orphaned")), "screenshot failed");

    const QString described = loc(c.lang, "Backups not configured here", "Hier nicht angelegte Backups");
    CHECK(8305, open_next_group(c, "described", described), "open the group of backups not configured here");
    CHECK(8306, wait_for_group(w, c.backend, described, 1), "the backup not configured here is listed");
    if (int rc = check_hints(w, loc(c.lang, "Backup not configured here", "Backup hier nicht angelegt"), 5723)) {
        return rc;
    }
    CHECK(5726, save_screenshot(w, group_shot(c.out_path, "described")), "screenshot failed");
    return 0;
}

// The compare page and its model.
struct Compare {
    QQuickItem* page = nullptr;
    QAbstractItemModel* model = nullptr;
};

Compare compare_page(QQuickWindow* w) {
    Compare cmp;
    cmp.page = visible_item(w, "restoreComparePage");
    cmp.model = cmp.page ? qobject_cast<QAbstractItemModel*>(
                               cmp.page->findChild<QObject*>(QStringLiteral("snapshotDiffModel")))
                         : nullptr;
    return cmp;
}

// Waits until the compare page shows the finished comparison.
bool wait_for_compare(QQuickWindow* w) {
    return wait_until([&] {
        Compare cmp = compare_page(w);
        return cmp.model && cmp.model->property("state").toString() == QLatin1String("done");
    }, 30000);
}

// Where the keyboard focus and the restore area stand, for a failed check.
QString restore_focus_state(QQuickWindow* w) {
    QQuickItem* f = w->activeFocusItem();
    QQuickItem* p = visible_item(w, "restoreSnapshotsPage");
    QQuickItem* list = lookup(w, "restoreBackupsListView");
    return QStringLiteral("focus=%1#%2 currentIndex=%3 title=%4")
        .arg(f ? f->objectName() : QStringLiteral("-"))
        .arg(f ? f->property("index").toInt() : -1)
        .arg(list ? list->property("currentIndex").toInt() : -1)
        .arg(p ? p->property("title").toString() : QStringLiteral("-"));
}

/// AC-M4-01-4: with the keyboard alone (a) every snapshot can be selected and (b) browsing,
/// comparing and restoring can be started.
int scenario_restore_keyboard(const Ctx& c) {
    QQuickWindow* w = c.window;
    QQuickItem* tab_button = tab_to_text(w, loc(c.lang, "Restore", "Wiederherstellen").toUtf8().constData(), 30);
    CHECK(5201, tab_button != nullptr, "Tab did not reach the Restore tab");
    send_key(w, Qt::Key_Return);
    CHECK(5202, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "Return on the Restore tab did not open the area");
    // The tab switch puts the focus on the selected backup; Down right after it moves on, and
    // nothing of the switch that runs later takes the row back.
    CHECK(5240, wait_until([&] {
        QQuickItem* f = w->activeFocusItem();
        return f && f->objectName() == QLatin1String("restoreBackupListItem");
    }, 3000), qPrintable(QStringLiteral("the switch focuses the selected backup: ")
        + restore_focus_state(w)));
    send_key(w, Qt::Key_Down);
    // quiet window: nothing of the switch that runs later may take the focus back.
    wait_ms(400);
    CHECK(5241, w->activeFocusItem() && w->activeFocusItem()->property("index").toInt() == 1,
        qPrintable(QStringLiteral("Down right after the switch stays on the second backup: ")
            + restore_focus_state(w)));
    send_key(w, Qt::Key_Up);
    CHECK(5203, wait_for_snapshots(w, c.backend, 3), "snapshots loaded");
    finish_incubation(w);

    QQuickItem* backup_row = tab_to(w, "restoreBackupListItem", 30);
    CHECK(5204, backup_row != nullptr, "Tab did not reach the backup row");
    send_key(w, Qt::Key_Down);
    CHECK(5242, wait_until([&] {
        QQuickItem* f = w->activeFocusItem();
        return f && f->objectName() == QLatin1String("restoreBackupListItem")
            && f->property("index").toInt() == 1;
    }, 3000), qPrintable(QStringLiteral("Down moves the focus to the second backup: ")
        + restore_focus_state(w)));
    send_key(w, Qt::Key_Return);
    CHECK(5205, wait_until([&] {
        QQuickItem* p = visible_item(w, "restoreSnapshotsPage");
        return p && p->property("title").toString() == QLatin1String("Photos")
            && !c.backend->property("restore_snapshots_loading").toBool();
    }, 20000), qPrintable(QStringLiteral("Down and Return on the second backup list its snapshots: ")
        + restore_focus_state(w)));
    CHECK(5225, model_rows(w, "restoreSnapshotsModel") == 0, "the second backup has no snapshots");
    QQuickItem* photos_row = tab_to(w, "restoreBackupListItem", 30);
    CHECK(5226, photos_row != nullptr, "Tab did not return to the backup list");
    press_until(w, Qt::Key_Up, [&] {
        QQuickItem* f = w->activeFocusItem();
        return f && f->property("index").toInt() == 0;
    }, 80);
    send_key(w, Qt::Key_Return);
    CHECK(5227, wait_for_snapshots(w, c.backend, 3), "Up and Return on the first backup list its snapshots");
    finish_incubation(w);

    QQuickItem* first = tab_to(w, "restoreSnapshotListItem", 30);
    CHECK(5206, first != nullptr, "Tab did not reach the snapshot list");
    auto* list = lookup(w, "restoreSnapshotsListView");
    CHECK(5207, list != nullptr, "restoreSnapshotsListView missing");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    for (int row = 0; row < 3; ++row) {
        if (row > 0) {
            press_until(w, Qt::Key_Down, [&] { return list->property("currentIndex").toInt() == row; }, 80);
        }
        QQuickItem* focused = focused_snapshot_row(w);
        CHECK(5208, focused != nullptr, "(a) a snapshot row holds the focus");
        CHECK(5209, focused->property("index").toInt() == row, "(a) arrow key selects the next snapshot");
        CHECK(5210, list->property("currentIndex").toInt() == row, "(a) the list follows the focus");
    }

    auto* page = lookup(w, "restoreSnapshotsPage");
    CHECK(5211, page != nullptr, "restoreSnapshotsPage missing");
    QObject* spy = request_spy(w, page);
    CHECK(5212, spy != nullptr, "request spy could not be connected");
    const QString oldest = role_value(snaps, 2, "snapshotId").toString();
    const QString middle = role_value(snaps, 1, "snapshotId").toString();

    send_key(w, Qt::Key_Return);
    CHECK_EQ(5213, spy->property("last").toString(), QStringLiteral("browse:") + oldest,
        "(b) Return on a snapshot starts browsing it");
    CHECK(5228, wait_until([&] { return visible_item(w, "restoreBrowsePage") != nullptr; }, 5000),
        "(b) the snapshot opens for browsing next to the list");

    // Back to the snapshot list with Tab; the arrow keys move from the current (oldest)
    // snapshot to the middle one.
    CHECK(5229, tab_to(w, "restoreSnapshotListItem", 60) != nullptr, "Tab returns to the snapshot list");
    QQuickItem* snapshot_list = lookup(w, "restoreSnapshotsListView");
    for (int i = 0; i < 4 && snapshot_list->property("currentIndex").toInt() != 1; ++i) {
        const int at = snapshot_list->property("currentIndex").toInt();
        press_until(w, at < 1 ? Qt::Key_Down : Qt::Key_Up,
            [&] { return snapshot_list->property("currentIndex").toInt() != at; }, 80);
    }
    CHECK(5230, focused_snapshot_row(w) && focused_snapshot_row(w)->property("index").toInt() == 1,
        "arrow keys reach the middle snapshot");
    QObject* menu = lookup<QObject>(w, "snapshotContextMenu");
    CHECK(5214, menu != nullptr, "snapshotContextMenu missing");
    send_key(w, Qt::Key_Menu);
    CHECK(5215, wait_until([&] { return is_open(menu); }, 2000), "(b) Menu key opens the context menu");
    CHECK(5216, menu_to(w, menu, 1), "(b) arrow keys reach «Compare with Previous»");
    send_key(w, Qt::Key_Return);
    CHECK_EQ(5217, spy->property("last").toString(),
        QStringLiteral("compare:") + oldest + QStringLiteral(">") + middle,
        "(b) the context menu compares the snapshot with the previous one");
    CHECK(5218, wait_until([&] { return is_closed(menu); }, 2000), "menu closes after the choice");
    // The comparison opens next to the list and compares exactly these two snapshots.
    CHECK(5243, wait_for_compare(w), "(b) «Compare with Previous» shows the finished comparison");
    Compare cmp = compare_page(w);
    CHECK_EQ(5244, cmp.page->property("fromSnapshotId").toString(), oldest,
        "(b) the comparison starts at the older snapshot");
    CHECK_EQ(5245, cmp.page->property("toSnapshotId").toString(), middle,
        "(b) the comparison ends at the newer snapshot");
    QObject* between = cmp.page->findChild<QObject*>(QStringLiteral("compareSnapshotsLabel"));
    CHECK(5246, between != nullptr, "compareSnapshotsLabel missing");
    CHECK_EQ(5247, between->property("text").toString(),
        loc(c.lang, "From 2026-11-10 12:00 to 2026-11-14 12:00", "Von 2026-11-10 12:00 bis 2026-11-14 12:00"),
        "(b) the comparison names the older and the newer snapshot");

    send_key(w, Qt::Key_F10, Qt::ShiftModifier);
    CHECK(5219, wait_until([&] { return is_open(menu); }, 2000), "(b) Shift+F10 opens the context menu");
    CHECK(5220, menu_to(w, menu, 2), "(b) arrow keys reach «Restore…»");
    send_key(w, Qt::Key_Return);
    CHECK_EQ(5221, spy->property("last").toString(), QStringLiteral("restore:") + middle,
        "(b) the context menu starts restoring the snapshot");
    CHECK(5222, spy->property("count").toInt() == 3, "one request per action");
    // «Restore…» opens the restore dialog, which takes the focus; Esc closes it again.
    Restore r = restore_dialog(w);
    CHECK(5231, r.dialog && wait_until([&] { return is_open(r.dialog); }, 3000),
        "(b) «Restore…» opens the restore dialog");
    finish_incubation(w);
    send_key(w, Qt::Key_Escape);
    CHECK(5232, wait_until([&] { return is_closed(r.dialog); }, 3000), "Esc closes the restore dialog");
    CHECK(5223, wait_until([&] { return focused_snapshot_row(w) != nullptr; }, 2000),
        "focus returns to the snapshot list");
    return shot(c, 5224);
}

// Row of the change whose path ends with `suffix`, or -1.
int change_row(QAbstractItemModel* m, const QString& suffix) {
    for (int r = 0; r < m->rowCount(); ++r) {
        if (role_value(m, r, "path").toString().endsWith(suffix)) {
            return r;
        }
    }
    return -1;
}

// Moves the keyboard focus in the snapshot list to row `target` with the arrow keys.
bool snapshot_to(QQuickWindow* w, int target) {
    for (int i = 0; i < 6; ++i) {
        QQuickItem* row = focused_snapshot_row(w);
        if (!row) {
            return false;
        }
        const int at = row->property("index").toInt();
        if (at == target) {
            return true;
        }
        press_until(w, at < target ? Qt::Key_Down : Qt::Key_Up, [&] {
            QQuickItem* now = focused_snapshot_row(w);
            return now && now->property("index").toInt() != at;
        }, 80);
    }
    return false;
}

/// AC-M4-02-1 and AC-M4-02-4 (smoke part): «Compare…», Space on two snapshots and «Show
/// Differences» compare the older with the newer one: (a) `a.txt` shows as changed, `b.txt` as
/// new and `c.txt` as removed, each with the text of its change; the chip «New · 3» leaves
/// exactly the 3 added files («3 of 9»).
int scenario_compare(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6001, 3)) {
        return rc;
    }
    QObject* compare = lookup<QObject>(w, "restoreCompareAction");
    CHECK(6006, compare && compare->property("enabled").toBool(), "«Compare…» enabled");
    trigger(compare);
    CHECK(6007, wait_until([&] { return visible_item(w, "cancelCompareButton") != nullptr; }, 3000),
        "«Compare…» shows the bar with «Cancel»");
    QQuickItem* row = tab_to(w, "restoreSnapshotListItem", 30);
    CHECK(6008, row != nullptr, "Tab reaches the snapshot list");
    CHECK(6009, snapshot_to(w, 1), "arrow keys reach the second snapshot");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    CHECK(6010, snapshot_to(w, 2), "arrow keys reach the oldest snapshot");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    QObject* chosen = lookup<QObject>(w, "compareChosenLabel");
    CHECK(6011, chosen != nullptr, "compareChosenLabel missing");
    CHECK_EQ(6012, chosen->property("text").toString(),
        loc(c.lang, "2 of 2 chosen", "2 von 2 gewählt"), "two snapshots chosen");
    QQuickItem* show = tab_to(w, "showDifferencesButton", 30);
    CHECK(6013, show != nullptr, "Tab reaches «Show Differences»");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    CHECK(6014, wait_for_compare(w), "«Show Differences» compares the two snapshots");
    finish_incubation(w);

    Compare cmp = compare_page(w);
    CHECK_EQ(6015, cmp.page->property("title").toString(), loc(c.lang, "Compare", "Vergleichen"),
        "the breadcrumb names the comparison");
    QObject* between = lookup<QObject>(w, "compareSnapshotsLabel");
    CHECK(6016, between != nullptr, "compareSnapshotsLabel missing");
    CHECK_EQ(6017, between->property("text").toString(),
        loc(c.lang, "From 2026-11-13 12:00 to 2026-11-14 12:00", "Von 2026-11-13 12:00 bis 2026-11-14 12:00"),
        "from the older to the newer snapshot");
    CHECK(6018, cmp.model->rowCount() == 9, "nine changes listed");
    const int a = change_row(cmp.model, QStringLiteral("/docs/a.txt"));
    const int b = change_row(cmp.model, QStringLiteral("/docs/b.txt"));
    const int cc = change_row(cmp.model, QStringLiteral("/docs/c.txt"));
    CHECK(6019, a >= 0 && b >= 0 && cc >= 0, "(a) a.txt, b.txt and c.txt listed");
    CHECK_EQ(6020, role_value(cmp.model, a, "changeText").toString(), loc(c.lang, "Changed", "Geändert"),
        "(a) a.txt changed");
    CHECK_EQ(6021, role_value(cmp.model, b, "changeText").toString(), loc(c.lang, "New", "Neu"),
        "(a) b.txt new");
    CHECK_EQ(6022, role_value(cmp.model, cc, "changeText").toString(), loc(c.lang, "Removed", "Entfernt"),
        "(a) c.txt removed");
    QObject* count = lookup<QObject>(w, "compareCountLabel");
    CHECK(6023, count != nullptr, "compareCountLabel missing");
    CHECK_EQ(6024, count->property("text").toString(), loc(c.lang, "9 changes", "9 Änderungen"),
        "number of changes");
    QObject* added_chip = lookup<QObject>(w, "compareKindChip_added");
    CHECK(6025, added_chip != nullptr, "the chip of new files is shown");
    CHECK_EQ(6026, added_chip->property("text").toString(), loc(c.lang, "New · 3", "Neu · 3"),
        "the chip names the kind and its number");
    QObject* removed_chip = lookup<QObject>(w, "compareKindChip_removed");
    CHECK(6027, removed_chip != nullptr, "the chip of removed files is shown");
    CHECK_EQ(6028, removed_chip->property("text").toString(), loc(c.lang, "Removed · 2", "Entfernt · 2"),
        "the chip of removed files names their number");
    wait_ms(600); // quiet window: the page transition ends before the screenshot
    CHECK(6029, save_screenshot(w, c.out_path), "screenshot failed");

    CHECK(6030, click_ui_item(w, qobject_cast<QQuickItem*>(added_chip), "chip New"), "chip could not be clicked");
    CHECK(6031, wait_until([&] { return cmp.model->rowCount() == 3; }, 5000),
        "AC-M4-02-4 (a) the chip leaves the 3 added files");
    for (int r = 0; r < 3; ++r) {
        CHECK_EQ(6032, role_value(cmp.model, r, "change").toString(), QStringLiteral("added"),
            "AC-M4-02-4 (a) only added files");
    }
    CHECK(6033, cmp.model->property("matched").toInt() == 3, "AC-M4-02-4 (b) matched is 3");
    CHECK_EQ(6034, count->property("text").toString(), loc(c.lang, "3 of 9", "3 von 9"),
        "the count names the filtered changes");
    CHECK_EQ(6035, removed_chip->property("text").toString(), loc(c.lang, "Removed · 2", "Entfernt · 2"),
        "AC-M4-02-4 (c) the numbers of the other kinds stay");
    return 0;
}

/// AC-M4-02-3 (smoke part): «Compare with Previous» on a snapshot equal to the one before
/// shows (a) «No differences» rather than an empty list.
int scenario_compare_identical(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6101, 3)) {
        return rc;
    }
    QQuickItem* row = tab_to(w, "restoreSnapshotListItem", 30);
    CHECK(6106, row != nullptr, "Tab reaches the snapshot list");
    CHECK(6107, snapshot_to(w, 0), "the newest snapshot holds the focus");
    QObject* menu = lookup<QObject>(w, "snapshotContextMenu");
    CHECK(6108, menu != nullptr, "snapshotContextMenu missing");
    send_key(w, Qt::Key_Menu);
    CHECK(6109, wait_until([&] { return is_open(menu); }, 2000), "Menu key opens the context menu");
    CHECK(6110, menu_to(w, menu, 1), "arrow keys reach «Compare with Previous»");
    send_key(w, Qt::Key_Return);
    CHECK(6111, wait_for_compare(w), "the snapshot is compared with the previous one");
    finish_incubation(w);
    Compare cmp = compare_page(w);
    CHECK(6112, cmp.model->property("identical").toBool(), "(a) the snapshots are identical");
    CHECK(6113, cmp.model->rowCount() == 0, "(a) no change listed");
    QQuickItem* message = visible_item(w, "compareNoDifferencesMessage");
    CHECK(6114, message != nullptr, "(a) «No differences» is shown");
    CHECK_EQ(6115, message->property("text").toString(),
        loc(c.lang, "No differences", "Keine Unterschiede"), "(a) text instead of an empty list");
    CHECK(6116, visible_item(w, "compareErrorMessage") == nullptr, "no error shown");
    wait_ms(600); // quiet window: the page transition ends before the screenshot
    return shot(c, 6117);
}

/// AC-M4-05-24 (c) in the GUI: started with `--show-restore` (the tray's «Restore Files…»
/// without a running GUI), the window opens on the tab «Restore». Running on another tab, the
/// GUI follows a click on the tray's «Restore Files…»: the scene writes the marker file
/// `KESTIC_SMOKE_TRAY_MARKER`; once the GUI is also subscribed to the daemon's signals (its
/// `RegisterGui` call passed the bus), the test clicks the entry in the daemon's tray menu, and
/// the daemon's signal switches the window to the tab «Restore».
int scenario_show_restore(const Ctx& c) {
    QQuickWindow* w = c.window;
    CHECK(6401, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 5000),
        "(c) the tab «Restore» is active");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    CHECK(6403, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "the overview is shown");
    // quiet window: nothing of the start on the tab «Restore» takes the window back there by itself.
    wait_ms(400);
    CHECK(6406, w->property("activeTab").toInt() == 0, "the overview stays shown");
    QFile marker(qEnvironmentVariable("KESTIC_SMOKE_TRAY_MARKER"));
    CHECK(6404, marker.open(QIODevice::WriteOnly), "tray marker could not be written");
    marker.close();
    CHECK(6405, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 5000),
        "(c) a running GUI switches to the tab «Restore» on a click on the tray's «Restore Files…»");
    CHECK(6407, wait_until([&] { return visible_item(w, "restorePage") != nullptr; }, 3000),
        "(c) the main window shows the restore area");
    finish_incubation(w);
    wait_ms(400); // quiet window: the tab switch settles before the screenshot
    return shot(c, 6402);
}

/// AC-M4-05-22 (b, c): the setting «restic source» shows «· recommended: <tested version>»
/// after the version of restic 0.18.0 (`KESTIC_SMOKE_EXPECT_RECOMMENDED=1`) and no such note with
/// the newest tested version.
int scenario_restic_recommended(const Ctx& c) {
    QQuickWindow* w = c.window;
    CHECK(6301, wait_until([&] { return c.backend->property("restic_valid").toBool(); }, 10000), "restic usable");
    QMetaObject::invokeMethod(w, "openSettingsWindow");
    auto* settings = qobject_cast<QQuickWindow*>(lookup<QObject>(w, "settingsWindow"));
    CHECK(6302, settings != nullptr, "settingsWindow missing");
    CHECK(6303, wait_until([&] { return settings->isVisible(); }, 3000), "settings open");
    QObject* label = settings->findChild<QObject*>(QStringLiteral("resticSourceLabel"));
    CHECK(6304, label != nullptr, "resticSourceLabel missing");
    QObject* version_label = settings->findChild<QObject*>(QStringLiteral("resticVersionLabel"));
    CHECK(6310, version_label != nullptr, "resticVersionLabel missing");
    const QString text = label->property("text").toString();
    const QString version = version_label->property("text").toString();
    const QString note = loc(c.lang, "recommended:", "empfohlen:");
    for (const char* code : {"system", "app_managed", "custom"}) {
        CHECK(6308, !text.contains(QLatin1String(code)), "no raw provider code in the restic source");
    }
    CHECK_EQ(6309, text, loc(c.lang, "Custom path", "Eigener Pfad"),
        "the source is named like the choice in the source dialog");
    if (qEnvironmentVariable("KESTIC_SMOKE_EXPECT_RECOMMENDED") == QLatin1String("1")) {
        CHECK_EQ(6305, version, loc(c.lang, "0.18.0 · recommended: 0.20.0", "0.18.0 · empfohlen: 0.20.0"),
            "(b) «recommended» after 0.18.0");
    } else {
        CHECK(6306, !version.isEmpty() && !version.contains(note), "(c) no «recommended» with the newest version");
    }
    // Scroll the setting into view for the screenshot.
    auto* label_item = qobject_cast<QQuickItem*>(label);
    scroll_into_view(label_item);
    CHECK(6330, label_item && wait_until([&] { return shown_in_its_flickables(label_item); }, 400),
        "the restic source is scrolled into view for the screenshot");
    CHECK(6307, save_screenshot(settings, c.out_path), "screenshot failed");
    settings->close();
    return 0;
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

// AC-M4-01-7 (d) for `list`: no visible item in the list's content (rows, headings) is a Tab
// stop of its own.
int check_no_row_tab_stop(QQuickItem* list, int code) {
    auto* content = list->property("contentItem").value<QQuickItem*>();
    QList<QQuickItem*> items;
    collect_visual_descendants(content, items);
    for (QQuickItem* it : items) {
        CHECK(code, !(it->isVisible() && it->activeFocusOnTab()),
            qPrintable(QStringLiteral("(d) a row of %1 is a Tab stop: %2")
                .arg(list->objectName(), describe_focus_item(it))));
    }
    return 0;
}

// AC-M4-01-7 (a, c, d) for the visible list `name` with its current row at `row`: Tab enters the
// list on that row, the next Tab leaves it, Shift+Tab comes back to the row and the next
// Shift+Tab leaves it to the control before it, from where Tab enters it again.
int check_one_tab_stop(QQuickWindow* w, const char* name, int row, int code) {
    QQuickItem* list = visible_item(w, name);
    CHECK(code, list != nullptr, name);
    list->setProperty("currentIndex", row);
    CHECK(code + 1, tab_into(w, list, 80), qPrintable(QStringLiteral("Tab reaches %1").arg(name)));
    CHECK(code + 2, focused_row_of(w, list) == row,
        qPrintable(QStringLiteral("(a) Tab enters %1 on its current row %2: %3").arg(name).arg(row).arg(focus_name(w))));
    send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    QQuickItem* next = w->activeFocusItem();
    CHECK(code + 3, next && !lies_in(next, list),
        qPrintable(QStringLiteral("(c) the next Tab leaves %1: %2").arg(name, focus_name(w))));
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
    CHECK(code + 4, focused_row_of(w, list) == row,
        qPrintable(QStringLiteral("(c) Shift+Tab returns to row %1 of %2: %3").arg(row).arg(name, focus_name(w))));
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
    QQuickItem* before = w->activeFocusItem();
    CHECK(code + 5, before && !lies_in(before, list) && before != next,
        qPrintable(QStringLiteral("(c) the next Shift+Tab leaves %1 to the control before it: %2").arg(name, focus_name(w))));
    send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    CHECK(code + 6, focused_row_of(w, list) == row,
        qPrintable(QStringLiteral("(c) from the control before %1, Tab enters it again: %2").arg(name, focus_name(w))));
    return check_no_row_tab_stop(list, code + 7);
}

// AC-M4-01-7 (d) for every list of the window built so far; the number of lists checked.
int check_all_lists(QQuickWindow* w, int code, int* checked) {
    QList<QQuickItem*> items;
    collect_visual_descendants(w->contentItem(), items);
    for (QQuickItem* it : items) {
        if (QByteArray(it->metaObject()->className()).startsWith("KeyboardList")) {
            if (int rc = check_no_row_tab_stop(it, code)) {
                return rc;
            }
            *checked += 1;
        }
    }
    return 0;
}

/// AC-M4-01-7: every list is one Tab stop. (a) Tab enters it on its current row, the first one
/// at first; (b) Up/Down, Home/End and Page Up/Page Down move within it; (c) the next Tab or
/// Shift+Tab leaves it, also with 1 000 rows; (d) no row of a list is a Tab stop of its own.
int scenario_list_tab_stop(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7001)) {
        return rc;
    }
    if (int rc = check_one_tab_stop(w, "restoreBackupsListView", 0, 7010)) {
        return rc;
    }
    if (int rc = check_one_tab_stop(w, "restoreSnapshotsListView", 1, 7020)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 1, 7030)) {
        return rc;
    }
    Browse b = browse_page(w);
    const QString top = b.model->property("dir").toString();
    CHECK(7033, wait_for_dir(w, top), "the folder with 1 000 entries is listed");
    finish_incubation(w);
    QQuickItem* list = visible_item(w, "browseListView");
    CHECK(7034, list != nullptr, "browseListView missing");
    CHECK(7035, tab_into(w, list, 80), "Tab reaches the folder list");
    CHECK(7036, wait_until([&] { return focused_row_of(w, list) == 0; }, 2000),
        qPrintable(QStringLiteral("(a) the first time on the first row: %1 current=%2 item=%3")
            .arg(focus_name(w)).arg(list->property("currentIndex").toInt())
            .arg(describe_focus_item(list->property("currentItem").value<QQuickItem*>()))));
    send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    CHECK(7037, !lies_in(w->activeFocusItem(), list),
        qPrintable(QStringLiteral("(c) one Tab leaves a list of 1 000 entries: ") + focus_name(w)));
    if (int rc = check_one_tab_stop(w, "browseListView", 150, 7040)) {
        return rc;
    }

    // (b) Home/End and Page Up/Page Down with the focus in the list.
    CHECK(7050, tab_into(w, list, 80), "Tab reaches the folder list again");
    CHECK(7059, b.model->rowCount() < 1000, "not every entry is loaded yet");
    send_key(w, Qt::Key_End);
    // The list moves to the last row once its pages have arrived.
    wait_until([&] { return list->property("currentIndex").toInt() == 999 && focused_row_of(w, list) == 999; }, 10000);
    CHECK(7051, list->property("currentIndex").toInt() == 999 && focused_row_of(w, list) == 999,
        qPrintable(QStringLiteral("(b) one End press moves to the last of 1 000 entries (%1 loaded): %2")
            .arg(b.model->rowCount()).arg(focus_name(w))));
    // Up from the first visible row keeps the new current row below the column headings.
    press_until(w, Qt::Key_PageUp, [&] { return list->property("currentIndex").toInt() < 999; }, 120);
    const int before_page_up = list->property("currentIndex").toInt();
    QQuickItem* heading = list->property("headerItem").value<QQuickItem*>();
    QQuickItem* up_row = nullptr;
    auto up_row_below_headings = [&] {
        up_row = list->property("currentItem").value<QQuickItem*>();
        return heading && up_row && list->property("currentIndex").toInt() == before_page_up - 1
            && up_row->mapToItem(list, QPointF(0, 0)).y() >= heading->height() - 1;
    };
    CHECK(7064, press_until(w, Qt::Key_Up, up_row_below_headings, 80),
        qPrintable(QStringLiteral("(b) Up keeps the current row in view below the headings: row y=%1, headings %2")
            .arg(up_row ? up_row->mapToItem(list, QPointF(0, 0)).y() : -1).arg(heading ? heading->height() : -1)));
    CHECK(7052, press_until(w, Qt::Key_Home, [&] { return focused_row_of(w, list) == 0; }, 120), qPrintable(QStringLiteral("(b) Home moves to the first row: ") + focus_name(w)));
    press_until(w, Qt::Key_PageDown, [&] { return focused_row_of(w, list) >= 2; }, 120);
    const int page = focused_row_of(w, list);
    CHECK(7053, page >= 2, qPrintable(QStringLiteral("(b) Page Down moves a page down: ") + focus_name(w)));
    CHECK(7054, press_until(w, Qt::Key_Down, [&] { return focused_row_of(w, list) == page + 1; }, 80),
        "(b) Down moves one row");
    CHECK(7055, press_until(w, Qt::Key_PageUp, [&] { return focused_row_of(w, list) < page + 1; }, 120),
        "(b) Page Up moves a page up");
    const int before_up = focused_row_of(w, list);
    CHECK(7058, press_until(w, Qt::Key_Up, [&] { return focused_row_of(w, list) == before_up - 1; }, 80),
        "(b) Up moves one row");
    CHECK(7056, press_until(w, Qt::Key_Home, [&] { return focused_row_of(w, list) == 0; }, 120),
        "(b) Home again on the first row");
    CHECK(7057, save_screenshot(w, c.out_path), "screenshot failed");

    // (d) every list of every tab.
    int checked = 0;
    for (int tab = 0; tab <= 4; ++tab) {
        QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, tab));
        CHECK(7060, wait_until([&] { return w->property("activeTab").toInt() == tab; }, 3000), "tab switched");
        wait_ms(600); // quiet window: the lists of the tab are created after the switch
        finish_incubation(w);
        if (int rc = check_all_lists(w, 7061, &checked)) {
            return rc;
        }
    }
    CHECK(7062, checked >= 5, qPrintable(QStringLiteral("(d) lists checked on the tabs: %1").arg(checked)));
    return 0;
}

// The visible row of the snapshot list at `index` (nullptr when it is not created).
QQuickItem* row_at(QQuickWindow* w, int index) {
    for (QObject* o : m3::objects_named(w, "restoreSnapshotListItem")) {
        auto* it = qobject_cast<QQuickItem*>(o);
        if (it && it->isVisible() && it->property("index").toInt() == index) {
            return it;
        }
    }
    return nullptr;
}

// Where the open popup `menu` stands in the window.
QPointF popup_scene_pos(QObject* menu) {
    auto* parent = menu->property("parent").value<QQuickItem*>();
    const QPointF local(menu->property("x").toReal(), menu->property("y").toReal());
    return parent ? parent->mapToScene(local) : local;
}

/// AC-M4-01-8: a right click on a snapshot opens its context menu (a) on the press, also when
/// the button is held long or the pointer moves before the release, (b) at the pointer, and
/// (c) on every row, every time; (d) Esc closes it, also while the pointer rests on an entry.
int scenario_snapshot_right_click(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7101, 3)) {
        return rc;
    }
    // quiet window: the tab switch moves the focus to the selected backup a moment later.
    wait_ms(1000);
    QQuickItem* list = visible_item(w, "restoreSnapshotsListView");
    QObject* menu = lookup<QObject>(w, "snapshotContextMenu");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    CHECK(7106, list && menu && snaps, "snapshot list and menu exist");
    auto snapshot_of_menu = [&] { return menu->property("snapshotId").toString(); };

    // (a) Held for a second before the release.
    QQuickItem* first = row_at(w, 0);
    CHECK(7107, first != nullptr, "first row created");
    const QPointF at = first->mapToScene(QPointF(first->width() * 0.4, first->height() * 0.5));
    send_mouse(w, at, Qt::NoButton, QEvent::MouseMove, 20);
    send_mouse(w, at, Qt::RightButton, QEvent::MouseButtonPress, 20);
    CHECK(7108, wait_until([&] { return is_open(menu); }, 2000),
        "(a) the menu opens on the press, while the button is still down");
    wait_ms(1000); // quiet window: the button is held down for a second
    send_mouse(w, at, Qt::RightButton, QEvent::MouseButtonRelease, 1200);
    wait_ms(200); // quiet window: a release that closed the menu would show by now
    CHECK(7109, is_open(menu), "(a) the menu stays open after a long press");
    CHECK_EQ(7110, snapshot_of_menu(), role_value(snaps, 0, "snapshotId").toString(),
        "(a) the menu belongs to the row clicked");
    const QPointF pos = popup_scene_pos(menu);
    CHECK(7111, qAbs(pos.x() - at.x()) <= 2.0 && qAbs(pos.y() - at.y()) <= 2.0,
        qPrintable(QStringLiteral("(b) the menu opens at the pointer (%1,%2), not at (%3,%4)")
            .arg(at.x()).arg(at.y()).arg(pos.x()).arg(pos.y())));
    CHECK(7112, save_screenshot(w, c.out_path), "screenshot failed");
    send_key(w, Qt::Key_Escape);
    CHECK(7113, wait_until([&] { return is_closed(menu); }, 2000),
        qPrintable(QStringLiteral("(d) Esc closes the menu, also with the pointer on its first entry: opened=%1 focus=%2")
            .arg(menu->property("opened").toBool()).arg(focus_name(w))));

    // (a) The pointer moves while the button is down.
    QQuickItem* second = row_at(w, 1);
    CHECK(7114, second != nullptr, "second row created");
    const QPointF from = second->mapToScene(QPointF(second->width() * 0.3, second->height() * 0.3));
    const QPointF to = from + QPointF(14.0, 12.0);
    send_mouse(w, from, Qt::NoButton, QEvent::MouseMove, 20);
    send_mouse(w, from, Qt::RightButton, QEvent::MouseButtonPress, 20);
    send_mouse(w, to, Qt::NoButton, QEvent::MouseMove, 40);
    send_mouse(w, to, Qt::RightButton, QEvent::MouseButtonRelease, 40);
    CHECK(7115, wait_until([&] { return is_open(menu); }, 2000), "(a) the menu opens when the pointer moves");
    CHECK_EQ(7116, snapshot_of_menu(), role_value(snaps, 1, "snapshotId").toString(),
        "(a) the menu belongs to the second row");
    send_key(w, Qt::Key_Escape);
    CHECK(7117, wait_until([&] { return is_closed(menu); }, 2000), "Esc closes the menu");

    // (c) Short clicks on every row, twice each.
    for (int round = 0; round < 2; ++round) {
        for (int r = 0; r < 3; ++r) {
            QQuickItem* row = row_at(w, r);
            CHECK(7118, row != nullptr, "row created");
            const QPointF p = row->mapToScene(QPointF(row->width() * 0.6, row->height() * 0.5));
            send_mouse(w, p, Qt::NoButton, QEvent::MouseMove, 300);
            send_mouse(w, p, Qt::RightButton, QEvent::MouseButtonPress, 20);
            send_mouse(w, p, Qt::RightButton, QEvent::MouseButtonRelease, 60);
            CHECK(7119, wait_until([&] { return is_open(menu); }, 2000),
                qPrintable(QStringLiteral("(c) the menu opens on row %1, round %2").arg(r).arg(round)));
            CHECK_EQ(7120, snapshot_of_menu(), role_value(snaps, r, "snapshotId").toString(),
                "(c) the menu belongs to the row clicked");
            send_key(w, Qt::Key_Escape);
            CHECK(7121, wait_until([&] { return is_closed(menu); }, 2000), "Esc closes the menu");
        }
    }
    return 0;
}

// The narrowest width of the window that is not narrow, found by halving between `lo` (narrow)
// and `hi` (wide).
int first_wide_width(QQuickWindow* w, int lo, int hi) {
    while (hi - lo > 1) {
        const int mid = (lo + hi) / 2;
        w->resize(mid, w->height());
        QCoreApplication::processEvents();
        if (w->property("narrowMode").toBool()) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return hi;
}

/// AC-M4-01-9: one breakpoint. (a) Just below it the window shows one column with the tab bar
/// at the bottom, at it two columns and the tab bar at the top; (b) at that width the folder
/// list shows short names in full.
int scenario_breakpoint(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7501)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7506)) {
        return rc;
    }
    auto* stack = w->property("pageStack").value<QObject*>();
    QObject* bottom = lookup<QObject>(w, "bottomNavBar");
    CHECK(7509, stack && bottom, "page stack and bottom tab bar exist");
    w->resize(1400, w->height());
    QCoreApplication::processEvents();
    CHECK(7510, !w->property("narrowMode").toBool(), "1400 px is wide");
    const int wide = first_wide_width(w, 300, 1400);
    w->resize(wide - 1, w->height());
    CHECK(7511, wait_until([&] { return w->property("narrowMode").toBool(); }, 300), "one pixel less is narrow");
    CHECK(7512, wait_until([&] { return bottom->property("visible").toBool(); }, 300),
        "(a) narrow: the tab bar is at the bottom");
    CHECK(7513, wait_until([&] { return !stack->property("wideMode").toBool(); }, 300),
        qPrintable(QStringLiteral("(a) narrow at %1 px: one column").arg(wide - 1)));
    w->resize(wide, w->height());
    CHECK(7514, wait_until([&] { return !bottom->property("visible").toBool(); }, 300),
        "(a) wide: the tab bar is at the top");
    CHECK(7515, wait_until([&] { return stack->property("wideMode").toBool(); }, 300),
        qPrintable(QStringLiteral("(a) wide at %1 px: two columns").arg(wide)));

    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(7516, wait_for_dir(w, top), "the folder is listed");
    finish_incubation(w);
    QQuickItem* page = nullptr;
    CHECK(7517, wait_until([&] {
        page = visible_item(w, "restoreBrowsePage");
        return page && page->width() < wide * 0.6;
    }, 300), "the folder list is one of two columns");
    QList<QQuickItem*> items;
    collect_visual_descendants(page, items);
    int names = 0;
    for (QQuickItem* it : items) {
        if (it->objectName() == QLatin1String("browseEntryName") && it->isVisible()) {
            names += 1;
            CHECK(7518, !it->property("truncated").toBool(),
                qPrintable(QStringLiteral("(b) «%1» is shown in full in a column of %2 px")
                    .arg(it->property("text").toString()).arg(page->width())));
        }
    }
    CHECK(7519, names == 4, "(b) the four names of the folder checked");
    return shot(c, 7520);
}

// Where the Tab stop `it` lies: the header with the navigation, a page's content, or the action
// bar of a page at the bottom of the page row.
QString tab_region(QQuickWindow* w, QQuickItem* it) {
    if (!it) {
        return QStringLiteral("none");
    }
    if (lies_in(it, lookup(w, "mainHeaderArea"))) {
        return QStringLiteral("navigation");
    }
    for (QQuickItem* p = it; p; p = p->parentItem()) {
        if (p->inherits("QQuickPage")) {
            return QStringLiteral("content");
        }
    }
    if (lies_in(it, w->property("pageStack").value<QQuickItem*>())) {
        return QStringLiteral("action bar");
    }
    return QStringLiteral("other: ") + describe_focus_item(it);
}

// The regions of one full Tab round from the focus item, in order, with runs of the same region
// merged, starting with the navigation: a round is a cycle, and one that begins in the middle of
// the navigation still lists it once.
QStringList tab_round_regions(QQuickWindow* w, int max_steps) {
    QQuickItem* start = w->activeFocusItem();
    QStringList stops;
    for (int i = 0; i < max_steps; ++i) {
        stops << tab_region(w, w->activeFocusItem());
        press_tab(w);
        if (w->activeFocusItem() == start) {
            break;
        }
    }
    int first = 0;
    for (int i = 0; i < stops.size(); ++i) {
        const QString& prev = stops.at((i + stops.size() - 1) % stops.size());
        if (stops.at(i) == QLatin1String("navigation") && prev != QLatin1String("navigation")) {
            first = i;
            break;
        }
    }
    QStringList runs;
    for (int i = 0; i < stops.size(); ++i) {
        const QString& region = stops.at((first + i) % stops.size());
        if (runs.isEmpty() || runs.constLast() != region) {
            runs << region;
        }
    }
    return runs;
}

/// AC-M4-01-4 (b): the Tab order follows the visible order. On «Restore» one Tab from the
/// snapshot list reaches «Compare…» in the action bar, Shift+Tab from the backup list reaches the
/// navigation, and a full Tab round runs navigation, content, action bar; on «Backups» the
/// content is followed by the action bar and then the navigation.
int scenario_tab_order(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7801, 3)) {
        return rc;
    }
    CHECK(7810, tab_to(w, "restoreSnapshotListItem", 40) != nullptr, "Tab reaches the snapshot list");
    press_tab(w);
    QObject* compare = lookup<QObject>(w, "restoreCompareAction");
    CHECK(7811, compare != nullptr, "restoreCompareAction missing");
    CHECK(7812, tab_region(w, w->activeFocusItem()) == QLatin1String("action bar"),
        qPrintable(QStringLiteral("one Tab from the snapshot list reaches the action bar: ") + focus_name(w)));
    CHECK(7813, w->activeFocusItem()->property("text").toString() == compare->property("text").toString(),
        qPrintable(QStringLiteral("one Tab from the snapshot list reaches «Compare…»: ") + focus_name(w)));

    CHECK(7820, tab_to(w, "restoreBackupListItem", 40) != nullptr, "Tab reaches the backup list");
    press_backtab(w);
    CHECK(7821, tab_region(w, w->activeFocusItem()) == QLatin1String("navigation"),
        qPrintable(QStringLiteral("Shift+Tab from the backup list reaches the navigation: ") + focus_name(w)));

    const QStringList round = tab_round_regions(w, 60);
    const QStringList expected { QStringLiteral("navigation"), QStringLiteral("content"), QStringLiteral("action bar") };
    CHECK(7830, round == expected,
        qPrintable(QStringLiteral("a full Tab round on «Restore» runs navigation, content, action bar: ")
            + round.join(QStringLiteral(", "))));
    CHECK(7831, save_screenshot(w, c.out_path), "screenshot failed");

    QObject* backups_tab = lookup<QObject>(w, "backupsTabAction");
    CHECK(7840, backups_tab != nullptr, "backupsTabAction missing");
    trigger(backups_tab);
    CHECK(7841, wait_until([&] {
        QQuickItem* list = visible_item(w, "backupsListPage");
        return w->property("activeTab").toInt() == 1 && list && visible_item(w, "backupDetailPage")
            && lies_in(w->activeFocusItem(), list);
    }, 5000), qPrintable(QStringLiteral("«Backups» shows the list with the focus and the detail page: ") + focus_name(w)));
    finish_incubation(w);
    QString before = tab_region(w, w->activeFocusItem());
    for (int i = 0; i < 60 && tab_region(w, w->activeFocusItem()) != QLatin1String("action bar"); ++i) {
        before = tab_region(w, w->activeFocusItem());
        press_tab(w);
    }
    CHECK(7842, tab_region(w, w->activeFocusItem()) == QLatin1String("action bar"),
        qPrintable(QStringLiteral("Tab reaches the action bar of «Backups»: ") + focus_name(w)));
    CHECK(7843, before == QLatin1String("content"),
        qPrintable(QStringLiteral("on «Backups» the action bar follows the content, not ") + before));
    for (int i = 0; i < 10 && tab_region(w, w->activeFocusItem()) == QLatin1String("action bar"); ++i) {
        press_tab(w);
    }
    CHECK(7844, tab_region(w, w->activeFocusItem()) == QLatin1String("navigation"),
        qPrintable(QStringLiteral("on «Backups» the navigation follows the action bar: ") + focus_name(w)));
    return 0;
}

// The nearest flickable around `item`.
QQuickItem* outer_flickable(QQuickItem* item) {
    for (QQuickItem* p = item ? item->parentItem() : nullptr; p; p = p->parentItem()) {
        if (p->inherits("QQuickFlickable")) {
            return p;
        }
    }
    return nullptr;
}

// Whether `row` lies wholly in the visible area of `flick`.
bool row_in_view(QQuickItem* row, QQuickItem* flick) {
    const QRectF r = row->mapRectToItem(flick, QRectF(0, 0, row->width(), row->height()));
    return r.top() >= -0.5 && r.bottom() <= flick->height() + 0.5;
}

QString row_view_state(QQuickItem* row, QQuickItem* flick) {
    if (!row || !flick) {
        return QStringLiteral("row or flickable missing");
    }
    const QRectF r = row->mapRectToItem(flick, QRectF(0, 0, row->width(), row->height()));
    return QStringLiteral("row %1 at %2..%3, visible height %4")
        .arg(row->property("index").toInt()).arg(r.top()).arg(r.bottom()).arg(flick->height());
}

/// AC-M4-01-7 (b): in the snapshot list with more rows than fit, the current row stays in the
/// visible area of the page after Down to the last row, and Page Down from the first row moves
/// by about one visible page, not to the last row.
int scenario_list_page_keys(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7901, 30)) {
        return rc;
    }
    QQuickItem* list = visible_item(w, "restoreSnapshotsListView");
    CHECK(7910, list != nullptr, "restoreSnapshotsListView missing");
    const int count = list->property("count").toInt();
    QQuickItem* flick = outer_flickable(list);
    CHECK(7911, flick != nullptr, "no flickable around the snapshot list");
    // The list grows to the height of its rows once the layout has run.
    CHECK(7912, wait_until([&] { return list->height() > flick->height() * 2; }, 5000),
        qPrintable(QStringLiteral("the snapshot list is taller than two visible pages: %1 rows, %2 of %3")
            .arg(count).arg(list->height()).arg(flick->height())));
    CHECK(7913, tab_into(w, list, 40), "Tab reaches the snapshot list");
    CHECK(7914, press_until(w, Qt::Key_Home, [&] { return list->property("currentIndex").toInt() == 0; }, 80),
        "Home selects the first snapshot");

    for (int i = 1; i < count; ++i) {
        QQuickItem* row = nullptr;
        CHECK(7920, press_until(w, Qt::Key_Down, [&] {
            row = focused_snapshot_row(w);
            return row != nullptr && row->property("index").toInt() == i && row_in_view(row, flick);
        }, 30),
            qPrintable(QStringLiteral("(b) after Down the current row is in view: ") + row_view_state(row, flick)));
    }
    CHECK(7921, list->property("currentIndex").toInt() == count - 1, "(b) Down reaches the last snapshot");
    CHECK(7922, save_screenshot(w, c.out_path), "screenshot failed");

    QQuickItem* first = nullptr;
    CHECK(7930, press_until(w, Qt::Key_Home, [&] {
        first = focused_snapshot_row(w);
        return first != nullptr && first->property("index").toInt() == 0 && row_in_view(first, flick);
    }, 80),
        qPrintable(QStringLiteral("(b) Home shows the first snapshot: ") + row_view_state(first, flick)));
    const int page = static_cast<int>(flick->height() / first->height());
    QQuickItem* paged = nullptr;
    press_until(w, Qt::Key_PageDown, [&] {
        paged = focused_snapshot_row(w);
        return paged != nullptr && paged->property("index").toInt() > 0 && row_in_view(paged, flick);
    }, 80);
    const int after_page_down = list->property("currentIndex").toInt();
    CHECK(7931, after_page_down < count - 1,
        qPrintable(QStringLiteral("(b) Page Down from the first row does not jump to the last row: %1 of %2")
            .arg(after_page_down).arg(count)));
    CHECK(7932, std::abs(after_page_down - page) <= 1,
        qPrintable(QStringLiteral("(b) Page Down moves by about one visible page (%1 rows): to row %2")
            .arg(page).arg(after_page_down)));
    CHECK(7933, paged != nullptr && row_in_view(paged, flick),
        qPrintable(QStringLiteral("(b) after Page Down the current row is in view: ") + row_view_state(paged, flick)));
    QQuickItem* back = nullptr;
    CHECK(7934, press_until(w, Qt::Key_PageUp, [&] {
        back = focused_snapshot_row(w);
        return list->property("currentIndex").toInt() == 0 && back != nullptr && row_in_view(back, flick);
    }, 80),
        qPrintable(QStringLiteral("(b) Page Up goes back one page to the first row, in view: ") + row_view_state(back, flick)));
    return 0;
}

/// AC-M4-01-4 (b), any page: a control scrolled out of view that gets the focus by Tab is
/// scrolled into the visible area. The backup detail page in a short window, scrolled to its
/// top; Tab walks down to its last row.
int scenario_focus_in_view(const Ctx& c) {
    QQuickWindow* w = c.window;
    w->resize(w->width(), 420);
    wait_ms(300); // quiet window: the layout follows the new window height
    QObject* backups_tab = lookup<QObject>(w, "backupsTabAction");
    CHECK(8201, backups_tab != nullptr, "backupsTabAction missing");
    trigger(backups_tab);
    CHECK(8202, wait_until([&] { return visible_item(w, "backupDetailPage") != nullptr; }, 5000),
        "«Backups» shows the detail of a backup");
    finish_incubation(w);
    QQuickItem* page = nullptr;
    QQuickItem* row = nullptr;
    CHECK(8203, wait_until([&] {
        page = visible_item(w, "backupDetailPage");
        row = visible_item(w, "viewFullConfigRow");
        return page && row;
    }, 600), "detail page or its last row missing");
    auto* flick = page->property("flickable").value<QQuickItem*>();
    CHECK(8204, flick != nullptr, "the detail page has no flickable");
    flick->setProperty("contentY", flick->property("originY").toReal() - flick->property("topMargin").toReal());
    CHECK(8205, wait_until([&] { return !shown_in_its_flickables(row); }, 200), "the last row starts out of view");
    CHECK(8206, tab_to(w, "viewFullConfigRow", 60) != nullptr,
        qPrintable(QStringLiteral("Tab reaches the last row: ") + focus_name(w)));
    CHECK(8207, wait_until([&] { return shown_in_its_flickables(row) && inside_window(w, row); }, 200),
        "a control that gets the focus by Tab is scrolled into view");
    return shot(c, 8208);
}

const Scenario kScenarios[] = {
    {"m4-restore-tab", scenario_restore_tab},
    {"m4-restore-keyboard", scenario_restore_keyboard},
    {"m4-compare", scenario_compare},
    {"m4-compare-identical", scenario_compare_identical},
    {"m4-restic-recommended", scenario_restic_recommended},
    {"m4-foreign", scenario_foreign},
    {"m4-show-restore", scenario_show_restore},
    {"m4-list-tab-stop", scenario_list_tab_stop},
    {"m4-snapshot-right-click", scenario_snapshot_right_click},
    {"m4-tab-order", scenario_tab_order},
    {"m4-list-page-keys", scenario_list_page_keys},
    {"m4-focus-in-view", scenario_focus_in_view},
    {"m4-breakpoint", scenario_breakpoint},
};

} // namespace

m3::ScenarioFn find_m4_lists_scenario(const QString& scen) {
    return find_scenario(kScenarios, scen);
}

} // namespace kestic::smoke
