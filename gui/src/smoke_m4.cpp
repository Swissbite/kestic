// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// M4 GUI smoke scenarios: the «Restore» area (backup list and snapshot list).
// Every scenario returns 0 on success and a unique non-zero code on the first failed
// assertion; one precise condition per assertion (no `||` fallbacks).

#include <cstdlib>
#include "src/smoke_m3_util.h"

#include <algorithm>

#include <QFile>
#include <QImage>
#include <QLocale>
#include <QMetaMethod>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQmlProperty>
#include <QSet>
#include <QUrl>

namespace kestic::smoke {

namespace {

using m3::accessible_name;
using m3::Ctx;
using m3::item_named;
using m3::loc;
using m3::object_named;

#define M4_CHECK(code, cond, msg)                                                      \
    do {                                                                               \
        if (!(cond)) {                                                                 \
            std::fprintf(stderr, "M4 check failed (%d): %s [%s]\n", (code), (msg), #cond); \
            return (code);                                                             \
        }                                                                              \
    } while (0)

#define M4_CHECK_EQ(code, actual, expected, what)                                      \
    do {                                                                               \
        const QString a_ = (actual);                                                   \
        const QString e_ = (expected);                                                 \
        if (a_ != e_) {                                                                \
            std::fprintf(stderr, "M4 check failed (%d): %s: expected '%s' got '%s'\n", \
                (code), (what), qPrintable(e_), qPrintable(a_));                       \
            return (code);                                                             \
        }                                                                              \
    } while (0)

// The visible item called `name` (nullptr when there is none or it is hidden).
QQuickItem* visible_item(QQuickWindow* w, const char* name) {
    for (QObject* o : m3::objects_named(w, name)) {
        auto* it = qobject_cast<QQuickItem*>(o);
        if (it && it->isVisible()) {
            return it;
        }
    }
    return nullptr;
}

int model_rows(QQuickWindow* w, const char* model) {
    QAbstractItemModel* m = model_named(w, model);
    return m ? m->rowCount() : -1;
}

// Waits until the restore area shows the snapshots of its first backup (`expected` rows).
bool wait_for_snapshots(QQuickWindow* w, QObject* backend, int expected) {
    return wait_until([&] {
        return !backend->property("restore_snapshots_loading").toBool()
            && model_rows(w, "restoreSnapshotsModel") == expected;
    }, 20000);
}

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
        press_and_wait(w, Qt::Key_Down, 60);
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
    QObject* tab = object_named(w, "restoreTabAction");
    M4_CHECK(5101, tab != nullptr, "restoreTabAction missing");
    M4_CHECK_EQ(5102, tab->property("text").toString(), loc(c.lang, "Restore", "Wiederherstellen"),
        "(a) tab label");
    trigger(tab);
    M4_CHECK(5103, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "(b) Restore tab active");
    M4_CHECK(5104, wait_until([&] { return visible_item(w, "restorePage") != nullptr; }, 3000),
        "(b) restore page shown");
    M4_CHECK(5105, wait_until([&] { return !c.backend->property("restore_overview_loading").toBool(); }, 20000),
        "(b) snapshot counts loaded");
    finish_incubation(w);
    M4_CHECK(5106, model_rows(w, "restoreBackupsModel") == 2, "(b) both backups listed");
    QAbstractItemModel* backups = model_named(w, "restoreBackupsModel");
    M4_CHECK_EQ(5107, role_value(backups, 0, "subtitle").toString(),
        loc(c.lang, "3 snapshots", "3 Snapshots"), "(b) number of snapshots of the first backup");
    M4_CHECK_EQ(5115, role_value(backups, 1, "subtitle").toString(),
        loc(c.lang, "0 snapshots", "0 Snapshots"), "(b) number of snapshots of the second backup");
    const QString refresh = loc(c.lang, "Refresh", "Aktualisieren");
    if (w->property("narrowMode").toBool()) {
        // One column: the snapshot list opens from the backup row.
        M4_CHECK(5116, visible_buttons_labelled(w, refresh) == 1, "the backup list can be refreshed");
        M4_CHECK(5114, save_screenshot(w, c.out_path), "screenshot failed");
        return 0;
    }
    M4_CHECK(5108, wait_for_snapshots(w, c.backend, 3), "(b) snapshots of the first backup listed");
    finish_incubation(w);
    QQuickItem* snapshots_page = visible_item(w, "restoreSnapshotsPage");
    M4_CHECK(5109, snapshots_page != nullptr, "(b) snapshot list shown next to the backups");
    M4_CHECK_EQ(5110, snapshots_page->property("title").toString(), QStringLiteral("Home"),
        "(b) snapshot list titled with the backup");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    M4_CHECK_EQ(5111, role_value(snaps, 0, "sectionName").toString(), loc(c.lang, "Today", "Heute"),
        "newest snapshot first, grouped under today");
    M4_CHECK(5112, role_value(snaps, 0, "subtitle").toString().contains(loc(c.lang, " new · laptop", " neu · laptop")),
        "size and host of the newest snapshot");
    // stil-und-texte.md §13.3: no label twice in a window; the snapshot list refreshes both.
    M4_CHECK(5117, wait_until([&] { return visible_buttons_labelled(w, refresh) == 1; }, 2000),
        qPrintable(QStringLiteral("one «Refresh» beside the snapshot list: %1").arg(visible_buttons_labelled(w, refresh))));
    M4_CHECK(5113, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// The snapshot list shows the further group `title` with `rows` snapshots.
bool wait_for_group(QQuickWindow* w, QObject* backend, const QString& title, int rows) {
    return wait_until([&] {
        QQuickItem* p = visible_item(w, "restoreSnapshotsPage");
        return p && p->property("title").toString() == title
            && !backend->property("restore_overview_loading").toBool()
            && model_rows(w, "restoreSnapshotsModel") == rows;
    }, 20000);
}

// Checks that every row of the snapshot list carries `hint` and that the hint is drawn.
int check_hints(QQuickWindow* w, const QString& hint, int code) {
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    M4_CHECK(code, snaps != nullptr && snaps->rowCount() > 0, "snapshot rows listed");
    for (int row = 0; row < snaps->rowCount(); ++row) {
        M4_CHECK_EQ(code + 1, role_value(snaps, row, "hint").toString(), hint, "(a) hint of every row");
    }
    finish_incubation(w);
    int drawn = 0;
    for (QObject* o : m3::objects_named(w, "restoreSnapshotHint")) {
        auto* label = qobject_cast<QQuickItem*>(o);
        if (label && label->isVisible() && label->property("text").toString() == hint) {
            ++drawn;
        }
    }
    M4_CHECK(code + 2, drawn == snaps->rowCount(), "(a) the hint is shown on every row");
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
    press_and_wait(w, Qt::Key_Down, 80);
    QQuickItem* row = nullptr;
    for (QQuickItem* it = w->activeFocusItem(); it; it = it->parentItem()) {
        if (it->objectName() == QLatin1String("restoreGroupListItem")) {
            row = it;
            break;
        }
    }
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
    QObject* tab = object_named(w, "restoreTabAction");
    M4_CHECK(5701, tab != nullptr, "restoreTabAction missing");
    trigger(tab);
    M4_CHECK(5702, wait_until([&] {
        return w->property("activeTab").toInt() == 4
            && !c.backend->property("restore_overview_loading").toBool()
            && model_rows(w, "restoreGroupsModel") == 3;
    }, 20000), "the restore area lists three further groups");
    finish_incubation(w);
    QAbstractItemModel* groups = model_named(w, "restoreGroupsModel");
    M4_CHECK_EQ(5703, role_value(groups, 0, "name").toString(), loc(c.lang, "Unassigned", "Nicht zugeordnet"),
        "first group");
    M4_CHECK_EQ(5704, role_value(groups, 0, "subtitle").toString(),
        loc(c.lang, "1 snapshot without a unique Kestic tag", "1 Snapshot ohne eindeutigen Kestic-Tag"), "unassigned count");
    M4_CHECK_EQ(5705, role_value(groups, 1, "subtitle").toString(),
        loc(c.lang, "1 backup without description", "1 Backup ohne Beschreibung"), "orphaned count");
    M4_CHECK_EQ(5706, role_value(groups, 2, "group").toString(), QStringLiteral("described"),
        "the backup not configured here is shown, not hidden");

    QQuickItem* group_row = tab_to(w, "restoreGroupListItem", 40);
    M4_CHECK(5707, group_row != nullptr, "Tab reaches the further groups");
    send_key(w, Qt::Key_Return);
    const QString unassigned = loc(c.lang, "Unassigned", "Nicht zugeordnet");
    M4_CHECK(5708, wait_for_group(w, c.backend, unassigned, 1), "Return opens the unassigned snapshots");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    M4_CHECK_EQ(5709, role_value(snaps, 0, "sectionName").toString(),
        loc(c.lang, "Unassigned on 'NAS'\n1 snapshot", "Nicht zugeordnet auf «NAS»\n1 Snapshot"),
        "grouped by destination, with the number of snapshots");
    const QString not_managed = loc(c.lang, "Not managed by Kestic", "Von Kestic nicht verwaltet");
    if (int rc = check_hints(w, not_managed, 5710)) {
        return rc;
    }
    M4_CHECK(5713, save_screenshot(w, c.out_path), "screenshot failed");

    const QString orphaned = loc(c.lang, "Orphaned", "Verwaist");
    M4_CHECK(5714, open_next_group(c, "orphaned", orphaned), "open the orphaned group");
    M4_CHECK(5715, wait_for_group(w, c.backend, orphaned, 1), "the orphaned snapshots are listed");
    M4_CHECK(5716, role_value(snaps, 0, "sectionName").toString().startsWith(
        loc(c.lang, "Backup without description on 'NAS'\n1 snapshot · latest ",
            "Backup ohne Beschreibung auf «NAS»\n1 Snapshot · zuletzt ")),
        "orphaned backup with its number of snapshots and newest snapshot");
    if (int rc = check_hints(w, not_managed, 5717)) {
        return rc;
    }
    M4_CHECK(5720, save_screenshot(w, group_shot(c.out_path, "orphaned")), "screenshot failed");

    const QString described = loc(c.lang, "Backups not configured here", "Hier nicht angelegte Backups");
    M4_CHECK(5721, open_next_group(c, "described", described), "open the group of backups not configured here");
    M4_CHECK(5722, wait_for_group(w, c.backend, described, 1), "the backup not configured here is listed");
    if (int rc = check_hints(w, loc(c.lang, "Backup not configured here", "Backup hier nicht angelegt"), 5723)) {
        return rc;
    }
    M4_CHECK(5726, save_screenshot(w, group_shot(c.out_path, "described")), "screenshot failed");
    return 0;
}

// The restore dialog's model and the dialog itself.
struct Restore {
    QObject* dialog = nullptr;
    QObject* model = nullptr;
};

Restore restore_dialog(QQuickWindow* w) {
    return Restore { object_named(w, "restoreDialog"), object_named(w, "restorePreviewModel") };
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
    QQuickItem* list = item_named(w, "restoreBackupsListView");
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
    M4_CHECK(5201, tab_button != nullptr, "Tab did not reach the Restore tab");
    send_key(w, Qt::Key_Return);
    M4_CHECK(5202, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "Return on the Restore tab did not open the area");
    // The tab switch puts the focus on the selected backup; Down right after it moves on, and
    // nothing of the switch that runs later takes the row back.
    M4_CHECK(5240, wait_until([&] {
        QQuickItem* f = w->activeFocusItem();
        return f && f->objectName() == QLatin1String("restoreBackupListItem");
    }, 3000), qPrintable(QStringLiteral("the switch focuses the selected backup: ")
        + restore_focus_state(w)));
    send_key(w, Qt::Key_Down);
    wait_ms(400);
    M4_CHECK(5241, w->activeFocusItem() && w->activeFocusItem()->property("index").toInt() == 1,
        qPrintable(QStringLiteral("Down right after the switch stays on the second backup: ")
            + restore_focus_state(w)));
    press_and_wait(w, Qt::Key_Up, 80);
    M4_CHECK(5203, wait_for_snapshots(w, c.backend, 3), "snapshots loaded");
    finish_incubation(w);

    QQuickItem* backup_row = tab_to(w, "restoreBackupListItem", 30);
    M4_CHECK(5204, backup_row != nullptr, "Tab did not reach the backup row");
    press_and_wait(w, Qt::Key_Down, 80);
    M4_CHECK(5242, wait_until([&] {
        QQuickItem* f = w->activeFocusItem();
        return f && f->objectName() == QLatin1String("restoreBackupListItem")
            && f->property("index").toInt() == 1;
    }, 3000), qPrintable(QStringLiteral("Down moves the focus to the second backup: ")
        + restore_focus_state(w)));
    send_key(w, Qt::Key_Return);
    M4_CHECK(5205, wait_until([&] {
        QQuickItem* p = visible_item(w, "restoreSnapshotsPage");
        return p && p->property("title").toString() == QLatin1String("Photos")
            && !c.backend->property("restore_snapshots_loading").toBool();
    }, 20000), qPrintable(QStringLiteral("Down and Return on the second backup list its snapshots: ")
        + restore_focus_state(w)));
    M4_CHECK(5225, model_rows(w, "restoreSnapshotsModel") == 0, "the second backup has no snapshots");
    QQuickItem* photos_row = tab_to(w, "restoreBackupListItem", 30);
    M4_CHECK(5226, photos_row != nullptr, "Tab did not return to the backup list");
    press_and_wait(w, Qt::Key_Up, 80);
    send_key(w, Qt::Key_Return);
    M4_CHECK(5227, wait_for_snapshots(w, c.backend, 3), "Up and Return on the first backup list its snapshots");
    finish_incubation(w);

    QQuickItem* first = tab_to(w, "restoreSnapshotListItem", 30);
    M4_CHECK(5206, first != nullptr, "Tab did not reach the snapshot list");
    auto* list = item_named(w, "restoreSnapshotsListView");
    M4_CHECK(5207, list != nullptr, "restoreSnapshotsListView missing");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    for (int row = 0; row < 3; ++row) {
        if (row > 0) {
            press_and_wait(w, Qt::Key_Down, 80);
        }
        QQuickItem* focused = focused_snapshot_row(w);
        M4_CHECK(5208, focused != nullptr, "(a) a snapshot row holds the focus");
        M4_CHECK(5209, focused->property("index").toInt() == row, "(a) arrow key selects the next snapshot");
        M4_CHECK(5210, list->property("currentIndex").toInt() == row, "(a) the list follows the focus");
    }

    auto* page = item_named(w, "restoreSnapshotsPage");
    M4_CHECK(5211, page != nullptr, "restoreSnapshotsPage missing");
    QObject* spy = request_spy(w, page);
    M4_CHECK(5212, spy != nullptr, "request spy could not be connected");
    const QString oldest = role_value(snaps, 2, "snapshotId").toString();
    const QString middle = role_value(snaps, 1, "snapshotId").toString();

    send_key(w, Qt::Key_Return);
    M4_CHECK_EQ(5213, spy->property("last").toString(), QStringLiteral("browse:") + oldest,
        "(b) Return on a snapshot starts browsing it");
    M4_CHECK(5228, wait_until([&] { return visible_item(w, "restoreBrowsePage") != nullptr; }, 5000),
        "(b) the snapshot opens for browsing next to the list");

    // Back to the snapshot list with Tab; the arrow keys move from the current (oldest)
    // snapshot to the middle one.
    M4_CHECK(5229, tab_to(w, "restoreSnapshotListItem", 60) != nullptr, "Tab returns to the snapshot list");
    QQuickItem* snapshot_list = item_named(w, "restoreSnapshotsListView");
    for (int i = 0; i < 4 && snapshot_list->property("currentIndex").toInt() != 1; ++i) {
        const int at = snapshot_list->property("currentIndex").toInt();
        press_and_wait(w, at < 1 ? Qt::Key_Down : Qt::Key_Up, 80);
    }
    M4_CHECK(5230, focused_snapshot_row(w) && focused_snapshot_row(w)->property("index").toInt() == 1,
        "arrow keys reach the middle snapshot");
    QObject* menu = object_named(w, "snapshotContextMenu");
    M4_CHECK(5214, menu != nullptr, "snapshotContextMenu missing");
    send_key(w, Qt::Key_Menu);
    M4_CHECK(5215, wait_until([&] { return is_open(menu); }, 2000), "(b) Menu key opens the context menu");
    M4_CHECK(5216, menu_to(w, menu, 1), "(b) arrow keys reach «Compare with Previous»");
    send_key(w, Qt::Key_Return);
    M4_CHECK_EQ(5217, spy->property("last").toString(),
        QStringLiteral("compare:") + oldest + QStringLiteral(">") + middle,
        "(b) the context menu compares the snapshot with the previous one");
    M4_CHECK(5218, wait_until([&] { return is_closed(menu); }, 2000), "menu closes after the choice");
    // The comparison opens next to the list and compares exactly these two snapshots.
    M4_CHECK(5243, wait_for_compare(w), "(b) «Compare with Previous» shows the finished comparison");
    Compare cmp = compare_page(w);
    M4_CHECK_EQ(5244, cmp.page->property("fromSnapshotId").toString(), oldest,
        "(b) the comparison starts at the older snapshot");
    M4_CHECK_EQ(5245, cmp.page->property("toSnapshotId").toString(), middle,
        "(b) the comparison ends at the newer snapshot");
    QObject* between = cmp.page->findChild<QObject*>(QStringLiteral("compareSnapshotsLabel"));
    M4_CHECK(5246, between != nullptr, "compareSnapshotsLabel missing");
    M4_CHECK_EQ(5247, between->property("text").toString(),
        loc(c.lang, "From 2026-11-10 12:00 to 2026-11-14 12:00", "Von 2026-11-10 12:00 bis 2026-11-14 12:00"),
        "(b) the comparison names the older and the newer snapshot");

    send_key(w, Qt::Key_F10, Qt::ShiftModifier);
    M4_CHECK(5219, wait_until([&] { return is_open(menu); }, 2000), "(b) Shift+F10 opens the context menu");
    M4_CHECK(5220, menu_to(w, menu, 2), "(b) arrow keys reach «Restore…»");
    send_key(w, Qt::Key_Return);
    M4_CHECK_EQ(5221, spy->property("last").toString(), QStringLiteral("restore:") + middle,
        "(b) the context menu starts restoring the snapshot");
    M4_CHECK(5222, spy->property("count").toInt() == 3, "one request per action");
    // «Restore…» opens the restore dialog, which takes the focus; Esc closes it again.
    Restore r = restore_dialog(w);
    M4_CHECK(5231, r.dialog && wait_until([&] { return is_open(r.dialog); }, 3000),
        "(b) «Restore…» opens the restore dialog");
    finish_incubation(w);
    send_key(w, Qt::Key_Escape);
    M4_CHECK(5232, wait_until([&] { return is_closed(r.dialog); }, 3000), "Esc closes the restore dialog");
    M4_CHECK(5223, wait_until([&] { return focused_snapshot_row(w) != nullptr; }, 2000),
        "focus returns to the snapshot list");
    M4_CHECK(5224, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// The browse page and its folder model.
struct Browse {
    QQuickItem* page = nullptr;
    QAbstractItemModel* model = nullptr;
};

Browse browse_page(QQuickWindow* w) {
    Browse b;
    b.page = visible_item(w, "restoreBrowsePage");
    b.model = b.page ? qobject_cast<QAbstractItemModel*>(
                           b.page->findChild<QObject*>(QStringLiteral("snapshotBrowseModel")))
                     : nullptr;
    return b;
}

// Waits until the browse page lists `dir` (state «done»).
bool wait_for_dir(QQuickWindow* w, const QString& dir) {
    return wait_until([&] {
        Browse b = browse_page(w);
        return b.model && b.model->property("state").toString() == QLatin1String("done")
            && b.model->property("dir").toString() == dir;
    }, 30000);
}

QStringList browse_names(QAbstractItemModel* m) {
    QStringList out;
    for (int r = 0; r < m->rowCount(); ++r) {
        out << role_value(m, r, "name").toString();
    }
    return out;
}

// The delegate of the folder list that holds the keyboard focus, or nullptr.
QQuickItem* focused_entry_row(QQuickWindow* w) {
    for (QQuickItem* it = w->activeFocusItem(); it; it = it->parentItem()) {
        if (it->objectName() == QLatin1String("browseEntryRow")) {
            return it;
        }
    }
    return nullptr;
}

// Moves the keyboard focus in the folder list to the row named `name` with the arrow keys.
bool entry_to(QQuickWindow* w, QAbstractItemModel* m, const QString& name) {
    const QStringList names = browse_names(m);
    const int target = names.indexOf(name);
    if (target < 0) {
        return false;
    }
    for (int i = 0; i < names.size() + 2; ++i) {
        QQuickItem* row = focused_entry_row(w);
        if (!row) {
            return false;
        }
        const int at = row->property("index").toInt();
        if (at == target) {
            return true;
        }
        press_and_wait(w, at < target ? Qt::Key_Down : Qt::Key_Up, 60);
    }
    return false;
}

// Opens the restore area and, in the narrow layout, the snapshot list of the first backup,
// which holds `snapshots` snapshots.
int open_restore_snapshots(const Ctx& c, int code, int snapshots = 2) {
    QQuickWindow* w = c.window;
    QObject* tab = object_named(w, "restoreTabAction");
    M4_CHECK(code, tab != nullptr, "restoreTabAction missing");
    trigger(tab);
    M4_CHECK(code + 1, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "Restore tab active");
    M4_CHECK(code + 2, wait_until([&] { return !c.backend->property("restore_overview_loading").toBool(); }, 20000),
        "snapshot counts loaded");
    finish_incubation(w);
    if (w->property("narrowMode").toBool()) {
        QQuickItem* backup_row = tab_to(w, "restoreBackupListItem", 30);
        M4_CHECK(code + 3, backup_row != nullptr, "Tab did not reach the backup row");
        send_key(w, Qt::Key_Return);
    }
    M4_CHECK(code + 4, wait_for_snapshots(w, c.backend, snapshots), "the snapshots of «Home» listed");
    finish_incubation(w);
    return 0;
}

// Opens snapshot `row` of the snapshot list for browsing (not by keyboard).
int open_browser_at(const Ctx& c, int row, int code) {
    QQuickWindow* w = c.window;
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    M4_CHECK(code, snaps != nullptr, "restoreSnapshotsModel missing");
    const QVariant dest = role_value(snaps, row, "destinationId");
    const QVariant snap = role_value(snaps, row, "snapshotId");
    M4_CHECK(code + 1, QMetaObject::invokeMethod(w, "openSnapshotBrowser", Q_ARG(QVariant, dest), Q_ARG(QVariant, snap)),
        "openSnapshotBrowser failed");
    M4_CHECK(code + 2, wait_until([&] { return browse_page(w).model != nullptr; }, 5000),
        "browse page shown");
    finish_incubation(w);
    return 0;
}

// The item or object below `root` that belongs to a tree view (a TreeView or one of its
// delegates), or nullptr.
QObject* tree_view_part(QQuickItem* root) {
    QList<QQuickItem*> items;
    collect_visual_descendants(root, items);
    QList<QObject*> objects;
    collect_all_qobjects(root, objects);
    for (QQuickItem* it : items) {
        objects << it;
    }
    for (QObject* o : objects) {
        if (o->inherits("QQuickTreeView") || QByteArray(o->metaObject()->className()).contains("TreeView")) {
            return o;
        }
    }
    return nullptr;
}

// The name of a property of `o` that would make it a node to expand or collapse: one of the
// properties of a tree view delegate, or any whose name contains «expand» or «collapse» in any
// case (empty when it has none).
QString expand_property(const QObject* o) {
    static const char* const names[] = { "hasChildren", "isTreeNode", "depth", "treeView" };
    const QMetaObject* meta = o->metaObject();
    for (int i = 0; i < meta->propertyCount(); ++i) {
        const QString name = QString::fromLatin1(meta->property(i).name());
        if (name.contains(QLatin1String("expand"), Qt::CaseInsensitive)
            || name.contains(QLatin1String("collapse"), Qt::CaseInsensitive)) {
            return name;
        }
        for (const char* tree_name : names) {
            if (name == QLatin1String(tree_name)) {
                return name;
            }
        }
    }
    return QString();
}

// The list, repeater or table/tree view nested inside the delegate `row` (its class name), or an
// empty string: a delegate that holds one could show child entries below itself.
QString nested_view(QQuickItem* row) {
    QList<QQuickItem*> items;
    collect_visual_descendants(row, items);
    QList<QObject*> objects;
    collect_all_qobjects(row, objects);
    for (QQuickItem* it : items) {
        objects << it;
    }
    for (QObject* o : objects) {
        if (o == row) {
            continue;
        }
        if (o->inherits("QQuickItemView") || o->inherits("QQuickRepeater") || o->inherits("QQuickTableView")) {
            return QString::fromLatin1(o->metaObject()->className());
        }
    }
    return QString();
}

/// AC-M4-03-3 (b), «no tree view to expand and collapse»: the browse page holds no TreeView
/// (nor any part of one), no entry of its folder list can be expanded (no property to expand or
/// collapse it, no list, repeater or tree view inside it), and its model is a flat list without
/// child rows.
int check_no_tree(const Browse& b) {
    QObject* tree = tree_view_part(b.page);
    M4_CHECK(5334, tree == nullptr,
        qPrintable(QStringLiteral("AC-M4-03-3 (b) the browse page holds no tree view: ")
            + QLatin1String(tree ? tree->metaObject()->className() : "-")));
    finish_incubation(b.page->window());
    QList<QQuickItem*> items;
    collect_visual_descendants(b.page, items);
    int rows = 0;
    for (QQuickItem* it : items) {
        if (it->objectName() != QLatin1String("browseEntryRow")) {
            continue;
        }
        ++rows;
        const QString prop = expand_property(it);
        M4_CHECK(5335, prop.isEmpty(),
            qPrintable(QStringLiteral("AC-M4-03-3 (b) no folder list entry can be expanded: ") + prop));
        const QString view = nested_view(it);
        M4_CHECK(5338, view.isEmpty(),
            qPrintable(QStringLiteral("AC-M4-03-3 (b) no folder list entry holds a list of its own: ") + view));
    }
    M4_CHECK(5336, rows > 0, "AC-M4-03-3 (b) folder list entries checked");
    for (int r = 0; r < b.model->rowCount(); ++r) {
        M4_CHECK(5337, !b.model->hasChildren(b.model->index(r, 0)),
            "AC-M4-03-3 (b) the folder list has no rows below its entries");
    }
    return 0;
}

/// AC-M4-03-3 (smoke part) and AC-M4-03-1 (a) on screen: with the keyboard alone (a) Return
/// opens sub-folders, (b) the path row leads back to every folder above, and (c) Space marks
/// entries for restore; the selection stays when the folder changes.
int scenario_browse_keyboard(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 5301)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(5306, top.endsWith(QLatin1String("/trees/src")), "newest snapshot opens in its folder");
    QQuickItem* first = tab_to(w, "restoreSnapshotListItem", 40);
    M4_CHECK(5307, first != nullptr, "Tab did not reach the snapshot list");
    M4_CHECK(5308, first->property("index").toInt() == 0, "newest snapshot first");
    send_key(w, Qt::Key_Return);
    M4_CHECK(5309, wait_for_dir(w, top), "Return opens the snapshot in its folder");
    finish_incubation(w);
    Browse b = browse_page(w);
    M4_CHECK_EQ(5310, b.page->property("title").toString(), QStringLiteral("2026-11-15 12:00"),
        "breadcrumb names the snapshot by date and time");
    M4_CHECK_EQ(5311, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("a.txt,b.bin,docs,empty"),
        "AC-M4-03-1 (a) folders and files of the top folder");
    M4_CHECK_EQ(5312, role_value(b.model, 0, "sizeText").toString(), QStringLiteral("5 B"),
        "AC-M4-03-1 (a) size of a file");
    M4_CHECK_EQ(5313, role_value(b.model, 2, "sizeText").toString(), QStringLiteral("–"),
        "AC-M4-03-1 (a) folders have no size");
    M4_CHECK(5314, role_value(b.model, 0, "modifiedText").toString().length() == 10,
        "AC-M4-03-1 (a) modification date shown");

    QQuickItem* row = tab_to(w, "browseEntryRow", 40);
    M4_CHECK(5315, row != nullptr, "Tab did not reach the folder list");
    M4_CHECK(5316, entry_to(w, b.model, QStringLiteral("docs")), "arrow keys reach «docs»");
    send_key(w, Qt::Key_Return);
    M4_CHECK(5317, wait_for_dir(w, top + QStringLiteral("/docs")), "(a) Return opens «docs»");
    finish_incubation(w);
    M4_CHECK_EQ(5318, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("deep,x.txt"),
        "(a) entries of «docs»");
    M4_CHECK(5319, tab_to(w, "browseEntryRow", 40) != nullptr, "Tab reaches the list of «docs»");
    M4_CHECK(5320, entry_to(w, b.model, QStringLiteral("deep")), "arrow keys reach «deep»");
    send_key(w, Qt::Key_Return);
    M4_CHECK(5321, wait_for_dir(w, top + QStringLiteral("/docs/deep")), "(a) Return opens «deep»");
    finish_incubation(w);

    M4_CHECK(5322, tab_to(w, "browseEntryRow", 40) != nullptr, "Tab reaches the list of «deep»");
    M4_CHECK(5323, entry_to(w, b.model, QStringLiteral("y.txt")), "arrow keys reach «y.txt»");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    M4_CHECK(5324, wait_until([&] { return role_value(b.model, 0, "selected").toBool(); }, 2000),
        "(c) Space marks the entry");
    M4_CHECK(5325, b.model->property("selection_count").toInt() == 1, "(c) one entry marked");
    QObject* restore_action = object_named(w, "restoreSelectionAction");
    M4_CHECK(5326, restore_action && restore_action->property("enabled").toBool(),
        "(c) «Restore Selection…» enabled with a selection");

    // (b) Back up through every folder above with the path row, down to the top of the snapshot.
    QStringList paths;
    QMetaObject::invokeMethod(b.model, "segmentPaths", Q_RETURN_ARG(QStringList, paths),
        Q_ARG(QString, top + QStringLiteral("/docs/deep")));
    M4_CHECK(5327, paths.size() >= 4, "path row from the top down to «deep»");
    for (int i = paths.size() - 2; i >= 0; --i) {
        const QByteArray name = QStringLiteral("browsePathButton_%1").arg(i).toUtf8();
        QQuickItem* button = tab_to(w, name.constData(), 60);
        M4_CHECK(5328, button != nullptr, "(b) Tab reaches the path row button of a folder above");
        send_key(w, Qt::Key_Return);
        M4_CHECK(5329, wait_for_dir(w, paths.at(i)), "(b) the path row opens the folder above");
        finish_incubation(w);
        M4_CHECK(5330, b.model->property("selection_count").toInt() == 1,
            "(c) the selection stays when the folder changes");
        if (paths.at(i) == top) {
            M4_CHECK_EQ(5331, b.model->property("selection_text").toString(),
                loc(c.lang, "1 item selected (0 here, 1 in ‹deep›)", "1 Element ausgewählt (0 hier, 1 in ‹deep›)"),
                "(c) the selection line names where the marked entry is");
            M4_CHECK(5332, save_screenshot(w, c.out_path), "screenshot failed");
        }
    }
    M4_CHECK(5333, item_named(w, "browseListView") != nullptr, "folder list, no tree");
    return check_no_tree(b);
}

// The part of the window that `item` covers, as drawn right now.
QImage drawn_under(QQuickWindow* w, QQuickItem* item) {
    const QImage all = w->grabWindow().convertToFormat(QImage::Format_ARGB32);
    const qreal dpr = w->effectiveDevicePixelRatio();
    const QRectF r = item->mapRectToScene(QRectF(0.0, 0.0, item->width(), item->height()));
    return all.copy(QRectF(r.topLeft() * dpr, r.size() * dpr).toAlignedRect());
}

// The visible Text item below `root` that shows `text` (nullptr when there is none).
QQuickItem* text_item_showing(QQuickItem* root, const QString& text) {
    QList<QQuickItem*> items;
    collect_visual_descendants(root, items);
    for (QQuickItem* it : items) {
        if (it->isVisible() && it->inherits("QQuickText") && it->property("text").toString() == text) {
            return it;
        }
    }
    return nullptr;
}

// Checks that every folder `names` of the path row of the open browse page is drawn exactly
// once (AC-M4-03-1 (a), breadcrumb): with its content hidden, a button that is neither hovered
// nor focused looks like no button at all, so the style draws no second name or icon beneath the
// content. Folder names are plain text, and the top folder keeps its accessible name.
int check_path_row_drawn_once(const Ctx& c, const QStringList& names) {
    QQuickWindow* w = c.window;
    QQuickItem* row = visible_item(w, "browsePathRow");
    M4_CHECK(6212, row != nullptr, "browsePathRow missing");
    // The page transition has ended once two grabs in a row are the same.
    QImage last;
    M4_CHECK(6213, wait_until([&] {
        const QImage now = drawn_under(w, row);
        const bool still = !last.isNull() && now == last;
        last = now;
        return still;
    }, 5000), "the path row stops moving");
    QObject* flick = object_named(w, "browsePathFlick");
    M4_CHECK(6227, flick != nullptr, "browsePathFlick missing");
    for (int i = 0; i < names.size(); ++i) {
        const QByteArray name = QStringLiteral("browsePathButton_%1").arg(i).toUtf8();
        QQuickItem* button = visible_item(w, name.constData());
        M4_CHECK(6214, button != nullptr, "path row button missing");
        // A path longer than the row scrolls sideways: bring the folder into view first.
        QMetaObject::invokeMethod(flick, "ensureVisible",
            Q_ARG(QVariant, QVariant::fromValue(button->parentItem())));
        M4_CHECK(6215, !button->property("hovered").toBool(), "button not hovered");
        M4_CHECK(6216, !button->hasActiveFocus(), "button without focus");
        auto* content = button->property("contentItem").value<QQuickItem*>();
        M4_CHECK(6217, content != nullptr, "button without content");

        const QImage drawn = drawn_under(w, button);
        button->setOpacity(0.0);
        const QImage without_button = drawn_under(w, button);
        button->setOpacity(1.0);
        content->setOpacity(0.0);
        const QImage without_content = drawn_under(w, button);
        content->setOpacity(1.0);
        M4_CHECK(6218, drawn != without_button, "the button shows its folder");
        M4_CHECK(6219, drawn != without_content, "hiding the content changes the picture");
        M4_CHECK(6220, without_content == without_button,
            qPrintable(QStringLiteral("path row button %1 is drawn a second time beneath its content").arg(i)));

        if (i == 0) {
            M4_CHECK_EQ(6221, accessible_name(button), loc(c.lang, "Top folder", "Oberster Ordner"),
                "accessible name of the top folder");
        } else {
            M4_CHECK_EQ(6222, accessible_name(button), names.at(i), "accessible name of a folder");
            QQuickItem* label = text_item_showing(button, names.at(i));
            M4_CHECK(6223, label != nullptr, "the folder name is shown");
            M4_CHECK(6224, label->property("textFormat").toInt() == 0, "the folder name is plain text");
        }
    }
    return 0;
}

/// AC-M4-03-1 (a), breadcrumb: every folder of the path row is drawn exactly once. With its
/// content hidden, a button that is neither hovered nor focused looks like no button at all: the
/// style draws no second name or icon beneath the content. Folder names are shown literally as
/// plain text (the path holds «R&D <b>x<b>»), and the top folder keeps its accessible name.
int scenario_browse_path_row(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6201)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6206)) {
        return rc;
    }
    Browse b = browse_page(w);
    M4_CHECK(6209, wait_until([&] { return b.model->property("state").toString() == QLatin1String("done"); }, 30000),
        "the folder is listed");
    finish_incubation(w);
    const QString dir = b.model->property("dir").toString();
    const QString odd_name = QStringLiteral("R&D <b>x<b>");
    M4_CHECK(6210, dir.endsWith(QLatin1Char('/') + odd_name + QStringLiteral("/src")),
        "the snapshot opens in its folder");
    QStringList names;
    QMetaObject::invokeMethod(b.model, "segmentNames", Q_RETURN_ARG(QStringList, names), Q_ARG(QString, dir));
    M4_CHECK(6211, names.size() >= 4, "path row from the top down to «src»");
    M4_CHECK(6226, names.contains(odd_name), "the path holds «R&D <b>x<b>»");
    if (int rc = check_path_row_drawn_once(c, names)) {
        return rc;
    }
    QMetaObject::invokeMethod(object_named(w, "browsePathFlick"), "showEnd");
    M4_CHECK(6225, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// AC-M4-03-2 (c) and AC-M4-03-5: while the top folder with 1 000 entries loads, the view keeps
/// reacting to input; it names 1 000 before scrolling, then fetches pages of at most 200 at the
/// end of the list until every entry is there exactly once.
int scenario_browse_paging(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 5401)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 1, 5406)) {
        return rc;
    }
    Browse b = browse_page(w);
    M4_CHECK(5409, b.model->property("state").toString() == QLatin1String("running"),
        "the folder is still loading (restic ls held)");
    QQuickItem* filter = tab_to(w, "browseFilterField", 40);
    M4_CHECK(5410, filter != nullptr, "AC-M4-03-2 (c) Tab moves the focus while the folder loads");
    send_key(w, Qt::Key_X, Qt::NoModifier, QStringLiteral("x"));
    M4_CHECK_EQ(5411, filter->property("text").toString(), QStringLiteral("x"),
        "AC-M4-03-2 (c) typing reaches the filter field while the folder loads");
    send_key(w, Qt::Key_Backspace);
    M4_CHECK_EQ(5412, filter->property("text").toString(), QString(), "Backspace empties the filter field");
    M4_CHECK(5413, b.model->property("state").toString() == QLatin1String("running"),
        "the input happened while the folder loaded");
    QFile marker(qEnvironmentVariable("KESTIC_SMOKE_TYPED_MARKER"));
    M4_CHECK(5414, marker.open(QIODevice::WriteOnly), "typed marker could not be written");
    marker.close();

    const QString top = b.model->property("dir").toString();
    M4_CHECK(5415, wait_for_dir(w, top), "the folder loads after restic ls is released");
    finish_incubation(w);
    M4_CHECK(5416, b.model->property("total").toInt() == 1000, "(c) total of 1 000 known");
    M4_CHECK(5417, b.model->rowCount() == 200, "(a) the first page holds 200 entries");
    QObject* count = object_named(w, "browseCountLabel");
    M4_CHECK(5418, count != nullptr, "browseCountLabel missing");
    M4_CHECK_EQ(5419, count->property("text").toString(),
        QLocale().toString(1000) + loc(c.lang, " entries", " Einträge"),
        "(c) the view names 1 000 before scrolling");
    M4_CHECK(5420, save_screenshot(w, c.out_path), "screenshot failed");

    QQuickItem* list = item_named(w, "browseListView");
    M4_CHECK(5421, list != nullptr, "browseListView missing");
    int rows = b.model->rowCount();
    for (int step = 0; step < 20 && rows < 1000; ++step) {
        QMetaObject::invokeMethod(list, "positionViewAtEnd");
        wait_until([&] { return b.model->rowCount() > rows; }, 3000);
        const int now = b.model->rowCount();
        M4_CHECK(5422, now - rows <= 200, "(a) at most 200 entries per page");
        M4_CHECK(5423, now > rows, "(a) scrolling to the end fetches the next page");
        rows = now;
    }
    M4_CHECK(5424, rows == 1000, "(b) every entry fetched");
    QSet<QString> paths;
    for (int r = 0; r < rows; ++r) {
        paths.insert(role_value(b.model, r, "path").toString());
    }
    M4_CHECK(5425, paths.size() == 1000, "(b) each entry exactly once");
    M4_CHECK(5426, !b.model->canFetchMore(QModelIndex()), "nothing beyond the last entry");
    return 0;
}

/// AC-M4-03-7 (c): a folder list over the memory limit shows «Too many entries to show»
/// instead of a partial list.
int scenario_browse_too_large(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 5501)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 1, 5506)) {
        return rc;
    }
    Browse b = browse_page(w);
    const QString top = b.model->property("dir").toString();
    M4_CHECK(5509, wait_for_dir(w, top), "the folder query finishes");
    finish_incubation(w);
    M4_CHECK(5510, b.model->property("too_large").toBool(), "the list is too large");
    M4_CHECK(5511, b.model->rowCount() == 0, "(c) no partial list");
    QQuickItem* message = visible_item(w, "browseTooLargeMessage");
    M4_CHECK(5512, message != nullptr, "(c) the too-large message is shown");
    M4_CHECK_EQ(5513, message->property("text").toString(),
        loc(c.lang, "Too many entries to show (%1). Choose a subfolder or narrow the search.",
            "Zu viele Einträge für die Anzeige (%1). Einen Unterordner wählen oder enger suchen.")
            .arg(QLocale().toString(1000)),
        "(c) text names the number of entries");
    M4_CHECK(5514, visible_item(w, "browseEmptyMessage") == nullptr, "not shown as an empty folder");
    M4_CHECK(5515, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// M4-S03: going back from a folder that still loads (restic ls held) cancels its query; the
/// Rust test checks that restic got SIGTERM.
int scenario_browse_leave(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 5601)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 1, 5606)) {
        return rc;
    }
    Browse b = browse_page(w);
    M4_CHECK(5609, b.model->property("state").toString() == QLatin1String("running"),
        "the folder is still loading (restic ls held)");
    wait_ms(1500);
    QQuickItem* back = visible_item(w, "breadcrumbBackButton");
    M4_CHECK(5610, back != nullptr, "breadcrumbBackButton missing");
    M4_CHECK(5611, click_ui_item(w, back, "breadcrumb back"), "back button could not be clicked");
    M4_CHECK(5612, wait_until([&] { return visible_item(w, "restoreBrowsePage") == nullptr; }, 3000),
        "the browse page closes");
    M4_CHECK(5614, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// Types `text` into the item with the keyboard focus, one key per character.
void type_text(QQuickWindow* w, const QString& text) {
    for (const QChar ch : text) {
        send_key(w, ch.toUpper().unicode(), Qt::NoModifier, QString(ch));
    }
}

// The search page and its model.
struct Search {
    QQuickItem* page = nullptr;
    QAbstractItemModel* model = nullptr;
};

Search search_page(QQuickWindow* w) {
    Search s;
    s.page = visible_item(w, "restoreSearchPage");
    s.model = s.page ? qobject_cast<QAbstractItemModel*>(
                           s.page->findChild<QObject*>(QStringLiteral("snapshotSearchModel")))
                     : nullptr;
    return s;
}

// Index of the hit row that holds the keyboard focus, or -1.
int focused_hit_index(QQuickWindow* w) {
    for (QQuickItem* it = w->activeFocusItem(); it; it = it->parentItem()) {
        if (it->objectName() == QLatin1String("searchHitRow")) {
            return it->property("index").toInt();
        }
    }
    return -1;
}

// Ctrl+F in the snapshot list, `text` and Return: waits until the search page lists its hits.
int search_with_keyboard(const Ctx& c, const QString& text, int code) {
    QQuickWindow* w = c.window;
    w->requestActivate();
    QCoreApplication::processEvents();
    send_key(w, Qt::Key_F, Qt::ControlModifier);
    M4_CHECK(code, wait_until([&] {
        QQuickItem* field = item_named(w, "snapshotSearchField");
        return field && field->hasActiveFocus();
    }, 2000), "Ctrl+F moves the focus to the search field of the snapshot list");
    type_text(w, text);
    send_key(w, Qt::Key_Return);
    M4_CHECK(code + 1, wait_until([&] {
        Search s = search_page(w);
        return s.model && s.model->property("state").toString() == QLatin1String("done");
    }, 30000), "Return searches the snapshots and shows the hits next to the list");
    finish_incubation(w);
    Search s = search_page(w);
    M4_CHECK_EQ(code + 2, s.page->property("title").toString(), loc(c.lang, "Search", "Suche"),
        "the breadcrumb names the search");
    M4_CHECK_EQ(code + 3, s.model->property("search_text").toString(), text, "searched text");
    return 0;
}

/// AC-M4-04-1 (smoke part): searching «bericht.pdf» lists (a) two hits with path and snapshot
/// date, newest snapshot first, and (b) Return on a hit opens its folder in the folder list of
/// its snapshot, with the hit selected and the breadcrumb named after the snapshot.
int scenario_search(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 5701, 3)) {
        return rc;
    }
    if (int rc = search_with_keyboard(c, QStringLiteral("bericht.pdf"), 5706)) {
        return rc;
    }
    Search s = search_page(w);
    M4_CHECK(5710, s.model->rowCount() == 2, "(a) two hits");
    M4_CHECK(5711, role_value(s.model, 0, "path").toString().endsWith(QLatin1String("/t2/src/Archiv/2026/bericht.pdf")),
        "(a) path of the hit in the newest snapshot first");
    M4_CHECK(5712, role_value(s.model, 1, "path").toString().endsWith(QLatin1String("/t1/src/Dokumente/bericht.pdf")),
        "(a) path of the hit in the older snapshot");
    M4_CHECK(5713, role_value(s.model, 0, "subtitle").toString().startsWith(QLatin1String("2026-11-14 12:00")),
        "(a) the hit names the date of its snapshot");
    M4_CHECK(5714, role_value(s.model, 1, "subtitle").toString().startsWith(QLatin1String("2026-11-13 12:00")),
        "(a) the other hit names the date of its snapshot");
    QObject* count = object_named(w, "searchCountLabel");
    M4_CHECK(5715, count != nullptr, "searchCountLabel missing");
    M4_CHECK_EQ(5716, count->property("text").toString(),
        loc(c.lang, "2 hits for ‹bericht.pdf› in 2 snapshots", "2 Treffer für ‹bericht.pdf› in 2 Snapshots"),
        "(a) number of hits and snapshots");
    wait_ms(600); // let the page transition end before the screenshot
    M4_CHECK(5717, save_screenshot(w, c.out_path), "screenshot failed");

    QQuickItem* row = tab_to(w, "searchHitRow", 40);
    M4_CHECK(5718, row != nullptr, "Tab reaches the hits");
    M4_CHECK(5719, row->property("index").toInt() == 0, "the first hit holds the focus");
    for (int i = 0; i < 4 && focused_hit_index(w) != 1; ++i) {
        press_and_wait(w, Qt::Key_Down, 80);
    }
    M4_CHECK(5727, focused_hit_index(w) == 1, "the arrow keys reach the second hit");
    const QString path = role_value(s.model, 1, "path").toString();
    const QString folder = role_value(s.model, 1, "folder").toString();
    M4_CHECK_EQ(5720, folder + QStringLiteral("/bericht.pdf"), path, "the folder of the hit contains it");
    send_key(w, Qt::Key_Return);
    M4_CHECK(5721, wait_for_dir(w, folder), "(b) Return opens the folder of the hit");
    finish_incubation(w);
    Browse b = browse_page(w);
    M4_CHECK_EQ(5722, b.page->property("title").toString(), QStringLiteral("2026-11-13 12:00"),
        "(b) the breadcrumb names the snapshot of the hit");
    QQuickItem* list = item_named(w, "browseListView");
    M4_CHECK(5723, list != nullptr, "browseListView missing");
    const int current = list->property("currentIndex").toInt();
    M4_CHECK(5724, current >= 0, "(b) an entry of the folder is selected");
    M4_CHECK_EQ(5725, role_value(b.model, current, "path").toString(), path, "(b) the hit is selected");
    return 0;
}

/// AC-M4-04-2 (smoke part): a search without hits shows (a) «No hits for ‹…›» and (b) no error.
int scenario_search_none(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 5801, 3)) {
        return rc;
    }
    if (int rc = search_with_keyboard(c, QStringLiteral("nirgends"), 5806)) {
        return rc;
    }
    Search s = search_page(w);
    M4_CHECK(5810, s.model->rowCount() == 0, "(a) no hit listed");
    QQuickItem* message = visible_item(w, "searchNoHitsMessage");
    M4_CHECK(5811, message != nullptr, "(a) the no-hits message is shown");
    M4_CHECK_EQ(5812, message->property("text").toString(),
        loc(c.lang, "No hits for ‹nirgends›", "Keine Treffer für ‹nirgends›"), "(a) no-hits text");
    M4_CHECK(5813, visible_item(w, "searchErrorMessage") == nullptr, "(b) no error shown");
    wait_ms(600); // let the page transition end before the screenshot
    M4_CHECK(5814, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// M4-S04: «Cancel» stops a running search (restic find held); the page says the search was
/// cancelled. The Rust test checks that restic got SIGTERM.
int scenario_search_cancel(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 5901, 3)) {
        return rc;
    }
    w->requestActivate();
    QCoreApplication::processEvents();
    send_key(w, Qt::Key_F, Qt::ControlModifier);
    type_text(w, QStringLiteral("bericht"));
    send_key(w, Qt::Key_Return);
    M4_CHECK(5906, wait_until([&] { return visible_item(w, "searchCancelButton") != nullptr; }, 10000),
        "the running search shows «Cancel»");
    finish_incubation(w);
    wait_ms(1500);
    QQuickItem* cancel = visible_item(w, "searchCancelButton");
    M4_CHECK(5907, cancel != nullptr, "«Cancel» still shown while restic find is held");
    M4_CHECK(5908, click_ui_item(w, cancel, "cancel search"), "«Cancel» could not be clicked");
    M4_CHECK(5909, wait_until([&] { return visible_item(w, "searchCancelledMessage") != nullptr; }, 5000),
        "the page says the search was cancelled");
    Search s = search_page(w);
    M4_CHECK_EQ(5910, s.model->property("state").toString(), QStringLiteral("cancelled"), "search cancelled");
    M4_CHECK(5911, visible_item(w, "searchCancelButton") == nullptr, "«Cancel» gone");
    wait_ms(600); // let the page transition end before the screenshot
    M4_CHECK(5912, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
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
        press_and_wait(w, at < target ? Qt::Key_Down : Qt::Key_Up, 80);
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
    QObject* compare = object_named(w, "restoreCompareAction");
    M4_CHECK(6006, compare && compare->property("enabled").toBool(), "«Compare…» enabled");
    trigger(compare);
    M4_CHECK(6007, wait_until([&] { return visible_item(w, "cancelCompareButton") != nullptr; }, 3000),
        "«Compare…» shows the bar with «Cancel»");
    QQuickItem* row = tab_to(w, "restoreSnapshotListItem", 30);
    M4_CHECK(6008, row != nullptr, "Tab reaches the snapshot list");
    M4_CHECK(6009, snapshot_to(w, 1), "arrow keys reach the second snapshot");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    M4_CHECK(6010, snapshot_to(w, 2), "arrow keys reach the oldest snapshot");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    QObject* chosen = object_named(w, "compareChosenLabel");
    M4_CHECK(6011, chosen != nullptr, "compareChosenLabel missing");
    M4_CHECK_EQ(6012, chosen->property("text").toString(),
        loc(c.lang, "2 of 2 chosen", "2 von 2 gewählt"), "two snapshots chosen");
    QQuickItem* show = tab_to(w, "showDifferencesButton", 30);
    M4_CHECK(6013, show != nullptr, "Tab reaches «Show Differences»");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    M4_CHECK(6014, wait_for_compare(w), "«Show Differences» compares the two snapshots");
    finish_incubation(w);

    Compare cmp = compare_page(w);
    M4_CHECK_EQ(6015, cmp.page->property("title").toString(), loc(c.lang, "Compare", "Vergleichen"),
        "the breadcrumb names the comparison");
    QObject* between = object_named(w, "compareSnapshotsLabel");
    M4_CHECK(6016, between != nullptr, "compareSnapshotsLabel missing");
    M4_CHECK_EQ(6017, between->property("text").toString(),
        loc(c.lang, "From 2026-11-13 12:00 to 2026-11-14 12:00", "Von 2026-11-13 12:00 bis 2026-11-14 12:00"),
        "from the older to the newer snapshot");
    M4_CHECK(6018, cmp.model->rowCount() == 9, "nine changes listed");
    const int a = change_row(cmp.model, QStringLiteral("/docs/a.txt"));
    const int b = change_row(cmp.model, QStringLiteral("/docs/b.txt"));
    const int cc = change_row(cmp.model, QStringLiteral("/docs/c.txt"));
    M4_CHECK(6019, a >= 0 && b >= 0 && cc >= 0, "(a) a.txt, b.txt and c.txt listed");
    M4_CHECK_EQ(6020, role_value(cmp.model, a, "changeText").toString(), loc(c.lang, "Changed", "Geändert"),
        "(a) a.txt changed");
    M4_CHECK_EQ(6021, role_value(cmp.model, b, "changeText").toString(), loc(c.lang, "New", "Neu"),
        "(a) b.txt new");
    M4_CHECK_EQ(6022, role_value(cmp.model, cc, "changeText").toString(), loc(c.lang, "Removed", "Entfernt"),
        "(a) c.txt removed");
    QObject* count = object_named(w, "compareCountLabel");
    M4_CHECK(6023, count != nullptr, "compareCountLabel missing");
    M4_CHECK_EQ(6024, count->property("text").toString(), loc(c.lang, "9 changes", "9 Änderungen"),
        "number of changes");
    // The kinds of change and their numbers arrive after the list.
    M4_CHECK(6025, wait_until([&] {
        return object_named(w, "compareKindChip_added") != nullptr && object_named(w, "compareKindChip_removed") != nullptr;
    }, 10000), "the chip of new files is shown");
    QObject* added_chip = object_named(w, "compareKindChip_added");
    M4_CHECK_EQ(6026, added_chip->property("text").toString(), loc(c.lang, "New · 3", "Neu · 3"),
        "the chip names the kind and its number");
    QObject* removed_chip = object_named(w, "compareKindChip_removed");
    M4_CHECK(6027, removed_chip != nullptr, "the chip of removed files is shown");
    M4_CHECK_EQ(6028, removed_chip->property("text").toString(), loc(c.lang, "Removed · 2", "Entfernt · 2"),
        "the chip of removed files names their number");
    wait_ms(600); // let the page transition end before the screenshot
    M4_CHECK(6029, save_screenshot(w, c.out_path), "screenshot failed");

    M4_CHECK(6030, click_ui_item(w, qobject_cast<QQuickItem*>(added_chip), "chip New"), "chip could not be clicked");
    M4_CHECK(6031, wait_until([&] { return cmp.model->rowCount() == 3; }, 5000),
        "AC-M4-02-4 (a) the chip leaves the 3 added files");
    for (int r = 0; r < 3; ++r) {
        M4_CHECK_EQ(6032, role_value(cmp.model, r, "change").toString(), QStringLiteral("added"),
            "AC-M4-02-4 (a) only added files");
    }
    M4_CHECK(6033, cmp.model->property("matched").toInt() == 3, "AC-M4-02-4 (b) matched is 3");
    M4_CHECK_EQ(6034, count->property("text").toString(), loc(c.lang, "3 of 9", "3 von 9"),
        "the count names the filtered changes");
    M4_CHECK_EQ(6035, removed_chip->property("text").toString(), loc(c.lang, "Removed · 2", "Entfernt · 2"),
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
    M4_CHECK(6106, row != nullptr, "Tab reaches the snapshot list");
    M4_CHECK(6107, snapshot_to(w, 0), "the newest snapshot holds the focus");
    QObject* menu = object_named(w, "snapshotContextMenu");
    M4_CHECK(6108, menu != nullptr, "snapshotContextMenu missing");
    send_key(w, Qt::Key_Menu);
    M4_CHECK(6109, wait_until([&] { return is_open(menu); }, 2000), "Menu key opens the context menu");
    M4_CHECK(6110, menu_to(w, menu, 1), "arrow keys reach «Compare with Previous»");
    send_key(w, Qt::Key_Return);
    M4_CHECK(6111, wait_for_compare(w), "the snapshot is compared with the previous one");
    finish_incubation(w);
    Compare cmp = compare_page(w);
    M4_CHECK(6112, cmp.model->property("identical").toBool(), "(a) the snapshots are identical");
    M4_CHECK(6113, cmp.model->rowCount() == 0, "(a) no change listed");
    QQuickItem* message = visible_item(w, "compareNoDifferencesMessage");
    M4_CHECK(6114, message != nullptr, "(a) «No differences» is shown");
    M4_CHECK_EQ(6115, message->property("text").toString(),
        loc(c.lang, "No differences", "Keine Unterschiede"), "(a) text instead of an empty list");
    M4_CHECK(6116, visible_item(w, "compareErrorMessage") == nullptr, "no error shown");
    wait_ms(600); // let the page transition end before the screenshot
    M4_CHECK(6117, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// Repaints the whole window. The offscreen software renderer repaints only changed regions, so
// items of a lower layer that changed under an open dialog are drawn over it in a screenshot;
// the desktop renders whole frames.
void repaint_fully(QQuickWindow* w) {
    const QSize size = w->size();
    w->resize(size.width() + 1, size.height());
    wait_ms(100);
    w->resize(size);
    wait_ms(300);
}

// Scrolls the Flickables around `item` that have more content than they show, so that `item`
// shows near the top of each, for a screenshot. The content may still grow while its parts are
// laid out, so it scrolls again until the content height has settled.
void scroll_into_view(QQuickItem* item) {
    for (QQuickItem* p = item ? item->parentItem() : nullptr; p; p = p->parentItem()) {
        if (!p->inherits("QQuickFlickable")) {
            continue;
        }
        qreal settled = -1.0;
        for (int i = 0; i < 20; ++i) {
            const qreal height = p->property("contentHeight").toReal();
            const auto* content = p->property("contentItem").value<QQuickItem*>();
            const QPointF at = item->mapToItem(content, QPointF(0, 0));
            const qreal end = qMax(0.0, height - p->height());
            p->setProperty("contentY", qMin(end, qMax(0.0, at.y() - 120.0)));
            if (qAbs(height - settled) < 0.5) {
                break;
            }
            settled = height;
            wait_ms(100);
        }
    }
}

// Whether `item` lies wholly inside the visible area of every Flickable around it.
bool shown_in_its_flickables(QQuickItem* item) {
    for (QQuickItem* p = item->parentItem(); p; p = p->parentItem()) {
        if (!p->inherits("QQuickFlickable")) {
            continue;
        }
        const QRectF r = item->mapRectToItem(p, QRectF(0, 0, item->width(), item->height()));
        if (r.top() < -0.5 || r.bottom() > p->height() + 0.5) {
            return false;
        }
    }
    return true;
}

// `out_path` with `suffix` before `.png`, for a second screenshot of one scene.
QString screenshot_variant(const QString& out_path, const char* suffix) {
    QString path = out_path;
    path.insert(path.length() - 4, QLatin1String(suffix));
    return path;
}

// Whether the one-line path row of the browse page starts with a whole folder: scrolled to
// the start of one of its folders (or not scrolled at all), never into a folder name.
bool path_row_starts_with_whole_folder(QQuickWindow* w) {
    QObject* flick = object_named(w, "browsePathFlick");
    QQuickItem* row = visible_item(w, "browsePathRow");
    if (!flick || !row) {
        return false;
    }
    const qreal x = flick->property("contentX").toReal();
    if (x < 0.5) {
        return true;
    }
    for (QQuickItem* crumb : row->childItems()) {
        if (crumb->width() > 0 && qAbs(crumb->x() - x) < 0.5) {
            return true;
        }
    }
    return false;
}

bool wait_for_preview(QObject* model) {
    return wait_until([&] { return model->property("state").toString() == QLatin1String("done"); }, 20000);
}

// Marks `name` in the open folder of the browse page and opens the restore dialog for it.
int open_restore_dialog_for(const Ctx& c, const QString& name, int code) {
    QQuickWindow* w = c.window;
    Browse b = browse_page(w);
    M4_CHECK(code, b.model != nullptr, "browse page shown");
    const int row = browse_names(b.model).indexOf(name);
    M4_CHECK(code + 1, row >= 0, "the file is listed");
    if (!role_value(b.model, row, "selected").toBool()) {
        QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, row));
    }
    QObject* action = object_named(w, "restoreSelectionAction");
    M4_CHECK(code + 2, action && action->property("enabled").toBool(), "«Restore Selection…» enabled");
    trigger(action);
    Restore r = restore_dialog(w);
    M4_CHECK(code + 3, r.dialog && r.model, "restore dialog and model exist");
    M4_CHECK(code + 4, wait_until([&] { return is_open(r.dialog); }, 3000), "the restore dialog opens");
    finish_incubation(w);
    return 0;
}

/// A selected name that is not valid UTF-8 (restic shows U+FFFD): the dialog (a) runs no preview
/// and says why, (b) offers «Restore Folder», which (c) replaces the name by its folder and runs
/// the preview for that folder.
int scenario_restore_unmatchable(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6301, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6306)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(6309, wait_for_dir(w, top), "the snapshot opens in its folder");
    // The name also holds markup: the message must show it literally.
    const QString name = QStringLiteral("<b>bad<b>") + QChar(0xFFFD) + QStringLiteral("name.txt");
    if (int rc = open_restore_dialog_for(c, name, 6310)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* error = object_named(w, "restorePreviewError");
    QObject* instead = object_named(w, "restoreFolderInsteadAction");
    QObject* restore_now = object_named(w, "restoreNowAction");
    M4_CHECK(6315, error && instead && restore_now, "dialog controls exist");
    M4_CHECK(6316, wait_until([&] { return r.model->property("state").toString() == QLatin1String("error"); }, 3000),
        "(a) no preview runs");
    M4_CHECK(6317, error->property("visible").toBool(), "(a) the reason is shown");
    const QString folder = top.section(QLatin1Char('/'), -1);
    M4_CHECK(6318, error->property("text").toString().contains(
                       loc(c.lang, "'%1'", "«%1»").arg(folder)),
        "(a) the message names the folder to restore instead");
    M4_CHECK(6319, !restore_now->property("enabled").toBool(), "«Restore Now» stays disabled");
    QQuickItem* shown = nullptr;
    QList<QQuickItem*> parts;
    collect_visual_descendants(qobject_cast<QQuickItem*>(error), parts);
    for (QQuickItem* it : parts) {
        if (it->inherits("QQuickTextEdit") && it->isVisible()) {
            shown = it;
        }
    }
    QString plain;
    if (shown) {
        QMetaObject::invokeMethod(shown, "getText", Q_RETURN_ARG(QString, plain), Q_ARG(int, 0),
            Q_ARG(int, shown->property("length").toInt()));
    }
    M4_CHECK(6328, plain.contains(QStringLiteral("<b>bad<b>")), qPrintable(QStringLiteral(
        "the name with markup is shown literally: ") + plain));
    M4_CHECK(6327, path_row_starts_with_whole_folder(w), "the path row starts with a whole folder");
    M4_CHECK(6320, instead->property("visible").toBool(), "(b) «Restore Folder» offered");
    repaint_fully(w);
    M4_CHECK(6321, save_screenshot(w, c.out_path), "screenshot failed");

    const qreal width_with_message = r.dialog->property("width").toReal();
    trigger(instead);
    M4_CHECK(6322, wait_for_preview(r.model), "(c) the preview of the folder is done");
    M4_CHECK(6326, qAbs(r.dialog->property("width").toReal() - width_with_message) < 1.0,
        "the dialog keeps its width for the message");
    M4_CHECK(6323, !error->property("visible").toBool(), "(c) the message is gone");
    M4_CHECK(6324, restore_now->property("enabled").toBool(), "(c) «Restore Now» enabled for the folder");
    M4_CHECK_EQ(6325, r.dialog->property("title").toString(),
        loc(c.lang, "Restore 1 Item", "1 Element wiederherstellen"), "(c) one folder selected");
    return 0;
}

/// AC-M4-05-16 (f) and AC-M4-05-25 (c), smoke part: at the original location of `docs` (its
/// subfolder exists there) the dialog shows the note on folder permissions; for `b.bin`, which
/// is a folder with content at the target, it shows with «Always» the translated refusal naming
/// that folder and offers «Retry».
int scenario_restore_target_texts(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6501, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6506)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(6509, wait_for_dir(w, top), "the snapshot opens in its folder");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("docs"), 6510)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* original_radio = object_named(w, "restoreOriginalRadio");
    QObject* notes = object_named(w, "restoreNotesMessage");
    M4_CHECK(6515, original_radio && notes, "dialog controls exist");
    M4_CHECK(6516, wait_for_preview(r.model), "the preview of the new folder is done");
    M4_CHECK(6517, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    M4_CHECK(6518, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    M4_CHECK(6519, wait_for_preview(r.model), "the preview at the original location is done");
    M4_CHECK(6520, notes->property("visible").toBool(), "AC-M4-05-16 (f) the note is shown");
    M4_CHECK(6521, notes->property("text").toString().contains(loc(c.lang,
                       "Existing folders at the target get the permissions and modification time from the snapshot.",
                       "Vorhandene Ordner am Ziel erhalten Zugriffsrechte und Änderungszeit aus dem Snapshot.")),
        qPrintable(QStringLiteral("AC-M4-05-16 (f) the folder note: ") + notes->property("text").toString()));
    repaint_fully(w);
    M4_CHECK(6522, save_screenshot(w, c.out_path), "screenshot failed");

    QMetaObject::invokeMethod(r.dialog, "reject");
    M4_CHECK(6523, wait_until([&] { return is_closed(r.dialog); }, 3000), "the dialog closes");
    QObject* clear = object_named(w, "clearSelectionButton");
    M4_CHECK(6533, clear != nullptr, "clearSelectionButton missing");
    M4_CHECK(6534, click_ui_item(w, qobject_cast<QQuickItem*>(clear), "clear selection"),
        "the selection could not be cleared");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("b.bin"), 6524)) {
        return rc;
    }
    Restore refused = restore_dialog(w);
    QObject* refused_original = object_named(w, "restoreOriginalRadio");
    QObject* refusal = object_named(w, "restorePreviewError");
    M4_CHECK(6532, refused_original && refusal, "dialog controls exist");
    M4_CHECK(6525, wait_for_preview(refused.model), "the preview of the new folder is done");
    M4_CHECK(6526, click_ui_item(w, qobject_cast<QQuickItem*>(refused_original), "original location"),
        "the original location could not be chosen");
    // With «Never» restic leaves the folder alone; with «Always» it would fail there.
    M4_CHECK(6538, wait_for_preview(refused.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(refused.model, "chooseOverwrite", Q_ARG(int, 3));
    M4_CHECK(6527, wait_until([&] { return refused.model->property("state").toString() == QLatin1String("error"); },
                       30000),
        "AC-M4-05-25 the preview refuses the target");
    M4_CHECK(6528, refusal->property("visible").toBool(), "AC-M4-05-25 (c) the refusal is shown");
    const QString text = refusal->property("text").toString();
    // The message wraps the text in a span, so markup in a path shows literally.
    const QString expected =
        loc(c.lang, "Cannot safely restore to %1. Choose 'Into a new folder' instead.",
            "Wiederherstellen nach %1 ist nicht sicher möglich. Stattdessen «In neuen Ordner» wählen.")
            .arg(top + QStringLiteral("/b.bin"));
    M4_CHECK(6529, text.contains(expected),
        qPrintable(QStringLiteral("AC-M4-05-25 (c) translated text naming the folder: ") + text));
    M4_CHECK(6530, !text.contains(QStringLiteral("invalid restore target")), "AC-M4-05-25 (c) no raw text");
    QObject* retry = object_named(w, "restoreRetryAction");
    M4_CHECK(6535, retry && !retry->property("visible").toBool(),
        "a refused target offers no «Retry», which would be refused again");
    repaint_fully(w);
    M4_CHECK(6531, save_screenshot(w, screenshot_variant(c.out_path, "-refused")), "screenshot failed");
    // The refusal offers the new folder, which then previews without an error.
    QObject* new_folder = object_named(w, "restoreNewFolderInsteadAction");
    M4_CHECK(6539, new_folder && new_folder->property("visible").toBool(),
        "the refusal offers «Into a New Folder»");
    trigger(new_folder);
    M4_CHECK(6540, wait_until([&] { return !refused.model->property("target_original").toBool(); }, 3000),
        "«Into a New Folder» chooses the new folder");
    M4_CHECK(6541, wait_for_preview(refused.model), "the preview of the new folder is done");
    M4_CHECK(6542, !refusal->property("visible").toBool(), "the refusal is gone");
    return 0;
}

/// AC-M4-06-1 (smoke part): the snapshot holds `/home/<user>/Dokumente/a.txt` and the home of
/// the test has `Dokumente`. The restore dialog for `a.txt` (a) offers `~/Dokumente` as the
/// original location and (b) shows the mapping «Original: /home/<user>/Dokumente (<user> →
/// <home>)»; the preview at the mapped folder runs.
int scenario_restore_mapping(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6601, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6606)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    // A snapshot of one file opens at that file; its folder holds it.
    const QString file = role_value(snaps, 0, "startDir").toString();
    const QString folder = file.section(QLatin1Char('/'), 0, -2);
    const QString user = folder.section(QLatin1Char('/'), 2, 2);
    M4_CHECK(6609, folder == QStringLiteral("/home/") + user + QStringLiteral("/Dokumente"),
        qPrintable(QStringLiteral("the snapshot holds /home/<user>/Dokumente: ") + file));
    Browse b = browse_page(w);
    M4_CHECK(6610, QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, folder)), "openDir failed");
    M4_CHECK(6611, wait_for_dir(w, folder), "the folder Dokumente opens");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6612)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* original_radio = object_named(w, "restoreOriginalRadio");
    QObject* original_dir = object_named(w, "restoreOriginalDirLabel");
    QObject* mapping = object_named(w, "restoreMappingLabel");
    M4_CHECK(6617, original_radio && original_dir && mapping, "dialog controls exist");
    M4_CHECK(6618, wait_for_preview(r.model), "the preview of the new folder is done");
    M4_CHECK(6619, original_radio->property("enabled").toBool(), "(a) the original location can be chosen");
    M4_CHECK_EQ(6620, original_dir->property("text").toString(), QStringLiteral("~/Dokumente"),
        "(a) the original location is Dokumente in the user's home");
    const QString home = QString::fromLocal8Bit(qgetenv("HOME")).section(QLatin1Char('/'), -1);
    M4_CHECK(6621, mapping->property("visible").toBool(), "(b) the mapping is shown");
    M4_CHECK_EQ(6622, mapping->property("text").toString(),
        loc(c.lang, "Original: %1 (%2 → %3)", "Original: %1 (%2 → %3)").arg(folder, user, home),
        "(b) the mapping names the original folder and old → new user");
    M4_CHECK(6623, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    M4_CHECK(6624, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    M4_CHECK(6625, wait_for_preview(r.model), "the preview at the mapped folder is done");
    M4_CHECK(6626, r.model->property("can_restore").toBool(), "the mapped folder can be restored into");
    repaint_fully(w);
    M4_CHECK(6627, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// AC-M4-05-1, AC-M4-05-4 (smoke part), AC-M4-05-11, AC-M4-05-15 (c, e): the restore dialog
/// (AC-M4-05-1) preselects (a) a new folder and (b) not the original location. At the original
/// location (AC-M4-05-15) (c) «Never» is preselected and (e) «Restore Now» stays disabled until
/// the preview is done; (AC-M4-05-4) the preview names the outcomes and, when files are
/// overwritten, the consequence («1 file will be overwritten. This cannot be undone.») and
/// «Overwrite and Restore». After a restore with «Always» (AC-M4-05-11) (a) a new dialog starts
/// with «Never» again.
int scenario_restore_dialog(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6201, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6206)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(6209, wait_for_dir(w, top), "the snapshot opens in its folder");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6210)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* new_radio = object_named(w, "restoreNewFolderRadio");
    QObject* original_radio = object_named(w, "restoreOriginalRadio");
    QObject* restore_now = object_named(w, "restoreNowAction");
    QObject* overwrite_box = object_named(w, "restoreOverwriteBox");
    M4_CHECK(6215, new_radio && original_radio && restore_now && overwrite_box, "dialog controls exist");
    M4_CHECK(6216, new_radio->property("checked").toBool(), "AC-M4-05-1 (a) a new folder is preselected");
    M4_CHECK(6217, !original_radio->property("checked").toBool(), "AC-M4-05-1 (b) not the original location");
    M4_CHECK(6218, wait_for_preview(r.model), "the preview of the new folder is done");
    M4_CHECK_EQ(6219, r.dialog->property("title").toString(),
        loc(c.lang, "Restore 1 Item", "1 Element wiederherstellen"), "dialog title");
    M4_CHECK(6220, restore_now->property("enabled").toBool(), "«Restore Now» enabled for a new folder");
    repaint_fully(w);
    M4_CHECK(6221, save_screenshot(w, c.out_path), "screenshot failed");

    // The preview at the original location waits for its held dry run.
    M4_CHECK(6222, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    M4_CHECK(6223, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    M4_CHECK(6224, r.model->property("state").toString() == QLatin1String("running"), "the preview runs");
    M4_CHECK(6225, !restore_now->property("enabled").toBool(),
        "AC-M4-05-15 (e) «Restore Now» disabled while the preview runs");
    M4_CHECK(6226, overwrite_box->property("currentIndex").toInt() == 0,
        "AC-M4-05-15 (c) «Never» preselected");
    const QString release = qEnvironmentVariable("KESTIC_SMOKE_RELEASE_FILE");
    M4_CHECK(6227, !release.isEmpty() && QFile::remove(release), "the held dry run could not be released");
    M4_CHECK(6228, wait_for_preview(r.model), "the preview at the original location is done");
    M4_CHECK(6229, restore_now->property("enabled").toBool(), "AC-M4-05-15 (e) enabled after the preview");
    QObject* summary = object_named(w, "restoreSummaryLabel");
    M4_CHECK(6230, summary != nullptr, "restoreSummaryLabel missing");
    M4_CHECK_EQ(6231, summary->property("text").toString(),
        loc(c.lang, "1 existing file will not be overwritten",
            "1 vorhandene Datei wird nicht überschrieben"),
        "AC-M4-05-4 the preview names the kept file");

    // The modes chosen the way a user does, with the keyboard on the list: the focus stays on
    // the list while previews that overwrite files come in.
    auto* box_item = qobject_cast<QQuickItem*>(overwrite_box);
    M4_CHECK(6276, box_item != nullptr, "the list of modes is an item");
    box_item->forceActiveFocus(Qt::TabFocusReason);
    for (int mode = 1; mode <= 3; ++mode) {
        send_key(w, Qt::Key_Down);
        M4_CHECK(6277, wait_until([&] { return r.model->property("overwrite").toInt() == mode; }, 2000),
            "a mode chosen with the keyboard");
        M4_CHECK(6278, wait_for_preview(r.model), "the preview of the mode is done");
        M4_CHECK(6279, w->activeFocusItem() == box_item, "the focus stays on the list of modes");
    }
    M4_CHECK(6232, overwrite_box->property("currentIndex").toInt() == 3, "«Always» chosen");
    // Reached with the focus elsewhere, overwriting puts the focus on «Cancel».
    qobject_cast<QQuickItem*>(original_radio)->forceActiveFocus(Qt::TabFocusReason);
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 0));
    M4_CHECK(6280, wait_for_preview(r.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    M4_CHECK(6233, wait_for_preview(r.model), "the preview with «Always» is done");
    QObject* consequence = object_named(w, "restoreConsequenceMessage");
    M4_CHECK(6234, consequence && consequence->property("visible").toBool(), "AC-M4-05-4 (c) consequence shown");
    M4_CHECK_EQ(6235, consequence->property("text").toString(),
        loc(c.lang, "1 file will be overwritten. This cannot be undone.",
            "1 Datei wird überschrieben. Das lässt sich nicht rückgängig machen."),
        "AC-M4-05-4 (c) the consequence before writing");
    M4_CHECK_EQ(6236, restore_now->property("text").toString(),
        loc(c.lang, "Overwrite and Restore", "Überschreiben und wiederherstellen"), "overwrite button");
    QQuickItem* focused = w->activeFocusItem();
    M4_CHECK(6261, focused != nullptr, "an item holds the focus");
    M4_CHECK_EQ(6262, focused->property("text").toString().remove(QLatin1Char('&')),
        loc(c.lang, "Cancel", "Abbrechen"), "«Cancel» is the default while overwriting");
    QObject* list_button = object_named(w, "restoreListButton");
    M4_CHECK(6237, list_button != nullptr, "restoreListButton missing");
    r.dialog->setProperty("listShown", true);
    finish_incubation(w);
    auto* preview_rows = qobject_cast<QAbstractItemModel*>(r.model);
    M4_CHECK(6238, preview_rows && preview_rows->rowCount() == 1, "AC-M4-05-4 (b) one file listed");
    M4_CHECK_EQ(6239, role_value(preview_rows, 0, "outcomeText").toString(),
        loc(c.lang, "Overwritten", "Überschrieben"), "AC-M4-05-4 (b) a.txt overwritten");
    QMetaObject::invokeMethod(r.model, "setFilter", Q_ARG(QString, QStringLiteral("no-such-file")));
    M4_CHECK(6263, wait_until([&] { return preview_rows->rowCount() == 0; }, 3000), "the filter hides the file");
    M4_CHECK(6264, r.model->property("matched").toInt() == 0, "nothing matches the filter");
    QMetaObject::invokeMethod(r.model, "setFilter", Q_ARG(QString, QString()));
    M4_CHECK(6265, wait_until([&] { return preview_rows->rowCount() == 1; }, 3000), "the file again without filter");
    wait_ms(400);
    repaint_fully(w);
    M4_CHECK(6240, save_screenshot(w, screenshot_variant(c.out_path, "-overwrite")), "screenshot failed");

    trigger(restore_now);
    M4_CHECK(6241, wait_until([&] { return is_closed(r.dialog); }, 3000), "the dialog closes on «Overwrite and Restore»");
    M4_CHECK(6242, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("ok"); }, 30000),
        "the restore ends");
    QObject* banner = object_named(w, "restoreRunBanner");
    M4_CHECK(6243, banner && banner->property("visible").toBool(), "the result of the restore is shown");
    M4_CHECK_EQ(6244, r.model->property("run_text").toString(),
        loc(c.lang, "1 file restored", "1 Datei wiederhergestellt"), "result line");
    // The result message must not squeeze the folder list below one entry, also in the
    // narrow window at its minimum height.
    auto* list = item_named(w, "browseListView");
    auto* row = visible_item(w, "browseEntryRow");
    auto* list_header = list ? list->property("headerItem").value<QQuickItem*>() : nullptr;
    M4_CHECK(6273, list && row && list_header, "folder list, its header and an entry exist");
    M4_CHECK(6275, path_row_starts_with_whole_folder(w), "the path row starts with a whole folder");
    M4_CHECK(6274, list->height() >= list_header->height() + row->height(),
        qPrintable(QStringLiteral("the folder list shows at least one entry (%1 px for %2 + %3)")
                       .arg(list->height()).arg(list_header->height()).arg(row->height())));
    repaint_fully(w);
    M4_CHECK(6245, save_screenshot(w, screenshot_variant(c.out_path, "-done")), "screenshot failed");

    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6246)) {
        return rc;
    }
    M4_CHECK(6251, r.model->property("overwrite").toInt() == 0, "AC-M4-05-11 (a) «Never» again");
    M4_CHECK(6252, overwrite_box->property("currentIndex").toInt() == 0, "AC-M4-05-11 (a) list shows «Never»");
    M4_CHECK(6253, new_radio->property("checked").toBool(), "AC-M4-05-11 a new folder again");

    // AC-M4-05-15 (c): another target starts with «Never» again.
    M4_CHECK(6254, wait_for_preview(r.model), "the preview of the new folder is done");
    M4_CHECK(6255, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    M4_CHECK(6256, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    M4_CHECK(6257, wait_until([&] { return r.model->property("overwrite").toInt() == 3; }, 2000), "«Always» chosen");
    M4_CHECK(6258, click_ui_item(w, qobject_cast<QQuickItem*>(new_radio), "new folder"),
        "the new folder could not be chosen");
    M4_CHECK(6259, wait_until([&] { return !r.model->property("target_original").toBool(); }, 3000),
        "the new folder is chosen");
    M4_CHECK(6260, r.model->property("overwrite").toInt() == 0, "AC-M4-05-15 (c) «Never» for another target");
    // AC-M4-05-15 (c), (e) for an existing folder with content (the backed-up folder itself).
    M4_CHECK(6266, wait_for_preview(r.model), "the preview of the new folder is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    M4_CHECK(6267, wait_until([&] { return r.model->property("overwrite").toInt() == 3; }, 2000), "«Always» chosen");
    QMetaObject::invokeMethod(r.model, "chooseFolder", Q_ARG(QString, top));
    M4_CHECK(6268, r.model->property("overwrite").toInt() == 0, "AC-M4-05-15 (c) «Never» for a folder with content");
    M4_CHECK(6269, r.model->property("state").toString() == QLatin1String("running")
        && !restore_now->property("enabled").toBool(), "AC-M4-05-15 (e) disabled while its preview runs");
    M4_CHECK(6270, wait_for_preview(r.model), "the preview of the folder with content is done");
    M4_CHECK(6271, r.model->property("preview_required").toBool(), "AC-M4-05-15 (d) a folder with content needs the preview");
    M4_CHECK(6272, restore_now->property("enabled").toBool(), "AC-M4-05-15 (e) enabled after its preview");
    // Closed without «Cancel» (Escape, a click outside), the dialog stops its running preview.
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 1));
    M4_CHECK(6281, r.model->property("state").toString() == QLatin1String("running"), "a preview runs");
    QMetaObject::invokeMethod(r.dialog, "close");
    M4_CHECK(6282, wait_until([&] { return is_closed(r.dialog); }, 3000), "the dialog closes");
    M4_CHECK_EQ(6283, r.model->property("state").toString(), QStringLiteral("cancelled"),
        "the running preview stopped");
    return 0;
}

/// The preview at the original location of `a.txt` (changed locally), `b.bin` (unchanged) and
/// `docs` (`docs/x.txt` deleted locally) with «Always»: the list of files shows (a) one filter
/// chip per outcome, new, overwritten and unchanged, and (b) a chip with the keyboard focus
/// draws the focus ring.
int scenario_restore_chips(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6550, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6555)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(6558, wait_for_dir(w, top), "the snapshot opens in its folder");
    Browse b = browse_page(w);
    for (const char* name : { "b.bin", "docs" }) {
        const int row = browse_names(b.model).indexOf(QLatin1String(name));
        M4_CHECK(6559, row >= 0, "the entry is listed");
        QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, row));
    }
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6560)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    M4_CHECK(6565, wait_for_preview(r.model), "the preview of the new folder is done");
    QObject* original_radio = object_named(w, "restoreOriginalRadio");
    M4_CHECK(6566, original_radio != nullptr, "restoreOriginalRadio missing");
    M4_CHECK(6567, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    M4_CHECK(6568, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    M4_CHECK(6569, wait_for_preview(r.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    M4_CHECK(6570, wait_for_preview(r.model), "the preview with «Always» is done");
    r.dialog->setProperty("listShown", true);
    finish_incubation(w);
    QQuickItem* chips = visible_item(w, "restoreKindChips");
    M4_CHECK(6571, chips != nullptr, "(a) the row of chips is shown");
    for (const char* kind : { "new", "overwritten", "identical" }) {
        const QByteArray name = QByteArray("restoreKindChip_") + kind;
        M4_CHECK(6572, visible_item(w, name.constData()) != nullptr,
            qPrintable(QStringLiteral("(a) a chip for the outcome ") + QLatin1String(kind)));
    }
    QQuickItem* chip = visible_item(w, "restoreKindChip_overwritten");
    chip->forceActiveFocus(Qt::TabFocusReason);
    M4_CHECK(6573, wait_until([&] { return chip->hasActiveFocus(); }, 2000), "(b) the chip holds the focus");
    bool ring_shown = false;
    for (QQuickItem* child : chip->childItems()) {
        if (QByteArray(child->metaObject()->className()).startsWith("FocusRing") && child->isVisible()) {
            ring_shown = true;
        }
    }
    M4_CHECK(6574, ring_shown, "(b) the focused chip draws the focus ring");
    scroll_into_view(chips);
    wait_ms(400);
    M4_CHECK(6576, shown_in_its_flickables(chips), "the chips are scrolled into view for the screenshot");
    repaint_fully(w);
    M4_CHECK(6575, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// The content of the file at `path` (empty when it cannot be read).
QString file_text(const QString& path) {
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? QString::fromUtf8(f.readAll()) : QString();
}

/// AC-M4-07-2 (a), GUI part: a snapshot without a Kestic tag («Unassigned») of a small folder
/// whose `a.txt` changed locally afterwards. Browsing it shows (AC-M4-03-1 (a)) the folder list
/// with its folders and files and a path row that draws each folder once. Its restore dialog
/// (AC-M4-05-1) preselects (a) a new folder and (b) not the original location. At the original
/// location with «Always» (AC-M4-05-4 (c)) it names the consequence «1 file will be overwritten.
/// This cannot be undone.» and writes nothing before «Overwrite and Restore».
int scenario_foreign_restore(const Ctx& c) {
    QQuickWindow* w = c.window;
    QObject* tab = object_named(w, "restoreTabAction");
    M4_CHECK(6701, tab != nullptr, "restoreTabAction missing");
    trigger(tab);
    M4_CHECK(6702, wait_until([&] {
        return w->property("activeTab").toInt() == 4
            && !c.backend->property("restore_overview_loading").toBool()
            && model_rows(w, "restoreGroupsModel") == 2;
    }, 20000), "the restore area lists the unassigned and the orphaned snapshots");
    finish_incubation(w);
    QAbstractItemModel* groups = model_named(w, "restoreGroupsModel");
    M4_CHECK_EQ(6703, role_value(groups, 0, "group").toString(), QStringLiteral("unassigned"), "first group");
    M4_CHECK_EQ(6737, role_value(groups, 0, "subtitle").toString(),
        loc(c.lang, "1 snapshot without a unique Kestic tag", "1 Snapshot ohne eindeutigen Kestic-Tag"),
        "the snapshot without a Kestic tag is unassigned");
    QQuickItem* group_row = tab_to(w, "restoreGroupListItem", 40);
    M4_CHECK(6704, group_row != nullptr, "Tab reaches the further groups");
    send_key(w, Qt::Key_Return);
    M4_CHECK(6705, wait_for_group(w, c.backend, loc(c.lang, "Unassigned", "Nicht zugeordnet"), 1),
        "Return opens the unassigned snapshot");
    finish_incubation(w);
    if (int rc = open_browser_at(c, 0, 6706)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(6709, top.endsWith(QLatin1String("/trees/src")), "the snapshot is of the folder src");
    M4_CHECK(6710, wait_for_dir(w, top), "the snapshot opens in its folder");
    finish_incubation(w);
    Browse b = browse_page(w);
    M4_CHECK_EQ(6711, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("a.txt,b.bin,docs,empty"),
        "AC-M4-07-2 (a) / AC-M4-03-1 (a) the folder list shows the folders and files");
    QStringList names;
    QMetaObject::invokeMethod(b.model, "segmentNames", Q_RETURN_ARG(QStringList, names), Q_ARG(QString, top));
    M4_CHECK(6712, names.size() >= 4, "AC-M4-07-2 (a) / AC-M4-03-1 (a) path row from the top down to «src»");
    if (int rc = check_path_row_drawn_once(c, names)) {
        return rc;
    }
    QMetaObject::invokeMethod(object_named(w, "browsePathFlick"), "showEnd");
    M4_CHECK(6713, save_screenshot(w, c.out_path), "screenshot failed");

    const QString changed = top + QStringLiteral("/a.txt");
    M4_CHECK_EQ(6714, file_text(changed), QStringLiteral("changed locally"), "a.txt differs locally");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6715)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* new_radio = object_named(w, "restoreNewFolderRadio");
    QObject* original_radio = object_named(w, "restoreOriginalRadio");
    QObject* restore_now = object_named(w, "restoreNowAction");
    M4_CHECK(6720, new_radio && original_radio && restore_now, "dialog controls exist");
    M4_CHECK(6721, new_radio->property("checked").toBool(),
        "AC-M4-07-2 (a) / AC-M4-05-1 (a) a new folder is preselected");
    M4_CHECK(6722, !original_radio->property("checked").toBool(),
        "AC-M4-07-2 (a) / AC-M4-05-1 (b) not the original location");
    M4_CHECK(6723, wait_for_preview(r.model), "the preview of the new folder is done");
    M4_CHECK(6724, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    M4_CHECK(6725, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    M4_CHECK(6726, wait_for_preview(r.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    M4_CHECK(6727, wait_until([&] { return r.model->property("overwrite").toInt() == 3; }, 2000), "«Always» chosen");
    M4_CHECK(6728, wait_for_preview(r.model), "the preview with «Always» is done");
    QObject* consequence = object_named(w, "restoreConsequenceMessage");
    M4_CHECK(6729, consequence && consequence->property("visible").toBool(),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) the consequence is shown");
    M4_CHECK_EQ(6730, consequence->property("text").toString(),
        loc(c.lang, "1 file will be overwritten. This cannot be undone.",
            "1 Datei wird überschrieben. Das lässt sich nicht rückgängig machen."),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) the consequence before writing");
    M4_CHECK_EQ(6731, restore_now->property("text").toString(),
        loc(c.lang, "Overwrite and Restore", "Überschreiben und wiederherstellen"),
        "AC-M4-05-4 (c) the confirmation names overwriting");
    M4_CHECK_EQ(6738, r.model->property("run_state").toString(), QString(),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) no restore runs before the confirmation");
    M4_CHECK_EQ(6732, file_text(changed), QStringLiteral("changed locally"),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) nothing is written before the confirmation");
    repaint_fully(w);
    M4_CHECK(6733, save_screenshot(w, screenshot_variant(c.out_path, "-overwrite")), "screenshot failed");

    trigger(restore_now);
    M4_CHECK(6734, wait_until([&] { return is_closed(r.dialog); }, 3000),
        "the dialog closes on «Overwrite and Restore»");
    M4_CHECK(6735, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("ok"); }, 30000),
        "the restore ends");
    M4_CHECK_EQ(6736, file_text(changed), QStringLiteral("hello"),
        "AC-M4-05-4 (c) the confirmation writes the file from the snapshot");
    return 0;
}

/// AC-M4-05-24 (c) in the GUI: started with `--show-restore` (the tray's «Restore Files…»
/// without a running GUI), the window opens on the tab «Restore». Running on another tab, the
/// GUI follows a click on the tray's «Restore Files…»: the scene writes the marker file
/// `KESTIC_SMOKE_TRAY_MARKER`; once the GUI is also subscribed to the daemon's signals (its
/// `RegisterGui` call passed the bus), the test clicks the entry in the daemon's tray menu, and
/// the daemon's signal switches the window to the tab «Restore».
int scenario_show_restore(const Ctx& c) {
    QQuickWindow* w = c.window;
    M4_CHECK(6401, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 5000),
        "(c) the tab «Restore» is active");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    M4_CHECK(6403, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "the overview is shown");
    // Nothing of the start on the tab «Restore» takes the window back there by itself.
    wait_ms(400);
    M4_CHECK(6406, w->property("activeTab").toInt() == 0, "the overview stays shown");
    QFile marker(qEnvironmentVariable("KESTIC_SMOKE_TRAY_MARKER"));
    M4_CHECK(6404, marker.open(QIODevice::WriteOnly), "tray marker could not be written");
    marker.close();
    M4_CHECK(6405, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 5000),
        "(c) a running GUI switches to the tab «Restore» on a click on the tray's «Restore Files…»");
    M4_CHECK(6407, wait_until([&] { return visible_item(w, "restorePage") != nullptr; }, 3000),
        "(c) the main window shows the restore area");
    finish_incubation(w);
    wait_ms(400);
    M4_CHECK(6402, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// AC-M4-05-22 (b, c): the setting «restic source» shows «· recommended: <tested version>»
/// after the version of restic 0.18.0 (`KESTIC_SMOKE_EXPECT_RECOMMENDED=1`) and no such note with
/// the newest tested version.
int scenario_restic_recommended(const Ctx& c) {
    QQuickWindow* w = c.window;
    M4_CHECK(6301, wait_until([&] { return c.backend->property("restic_valid").toBool(); }, 10000), "restic usable");
    QMetaObject::invokeMethod(w, "openSettingsWindow");
    auto* settings = qobject_cast<QQuickWindow*>(object_named(w, "settingsWindow"));
    M4_CHECK(6302, settings != nullptr, "settingsWindow missing");
    M4_CHECK(6303, wait_until([&] { return settings->isVisible(); }, 3000), "settings open");
    QObject* label = settings->findChild<QObject*>(QStringLiteral("resticSourceLabel"));
    M4_CHECK(6304, label != nullptr, "resticSourceLabel missing");
    QObject* version_label = settings->findChild<QObject*>(QStringLiteral("resticVersionLabel"));
    M4_CHECK(6310, version_label != nullptr, "resticVersionLabel missing");
    const QString text = label->property("text").toString();
    const QString version = version_label->property("text").toString();
    const QString note = loc(c.lang, "recommended:", "empfohlen:");
    for (const char* code : {"system", "app_managed", "custom"}) {
        M4_CHECK(6308, !text.contains(QLatin1String(code)), "no raw provider code in the restic source");
    }
    M4_CHECK_EQ(6309, text, loc(c.lang, "Custom path", "Eigener Pfad"),
        "the source is named like the choice in the source dialog");
    if (qEnvironmentVariable("KESTIC_SMOKE_EXPECT_RECOMMENDED") == QLatin1String("1")) {
        M4_CHECK_EQ(6305, version, loc(c.lang, "0.18.0 · recommended: 0.20.0", "0.18.0 · empfohlen: 0.20.0"),
            "(b) «recommended» after 0.18.0");
    } else {
        M4_CHECK(6306, !version.isEmpty() && !version.contains(note), "(c) no «recommended» with the newest version");
    }
    // Scroll the setting into view for the screenshot.
    auto* label_item = qobject_cast<QQuickItem*>(label);
    scroll_into_view(label_item);
    wait_ms(400);
    M4_CHECK(6330, label_item && shown_in_its_flickables(label_item),
        "the restic source is scrolled into view for the screenshot");
    M4_CHECK(6307, save_screenshot(settings, c.out_path), "screenshot failed");
    settings->close();
    return 0;
}

// Whether `item` is `ancestor` or lies below it.
bool lies_in(QQuickItem* item, QQuickItem* ancestor) {
    for (QQuickItem* it = item; it; it = it->parentItem()) {
        if (it == ancestor) {
            return true;
        }
    }
    return false;
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

// Presses Tab until the keyboard focus is in `list` (at most `max_steps` times).
bool tab_into(QQuickWindow* w, QQuickItem* list, int max_steps) {
    for (int i = 0; i < max_steps; ++i) {
        if (lies_in(w->activeFocusItem(), list)) {
            return true;
        }
        send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    }
    return lies_in(w->activeFocusItem(), list);
}

QString focus_name(QQuickWindow* w) {
    return describe_focus_item(w->activeFocusItem());
}

// AC-M4-01-7 (d) for `list`: no visible item in the list's content (rows, headings) is a Tab
// stop of its own.
int check_no_row_tab_stop(QQuickItem* list, int code) {
    auto* content = list->property("contentItem").value<QQuickItem*>();
    QList<QQuickItem*> items;
    collect_visual_descendants(content, items);
    for (QQuickItem* it : items) {
        M4_CHECK(code, !(it->isVisible() && it->activeFocusOnTab()),
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
    M4_CHECK(code, list != nullptr, name);
    list->setProperty("currentIndex", row);
    M4_CHECK(code + 1, tab_into(w, list, 80), qPrintable(QStringLiteral("Tab reaches %1").arg(name)));
    M4_CHECK(code + 2, focused_row_of(w, list) == row,
        qPrintable(QStringLiteral("(a) Tab enters %1 on its current row %2: %3").arg(name).arg(row).arg(focus_name(w))));
    send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    QQuickItem* next = w->activeFocusItem();
    M4_CHECK(code + 3, next && !lies_in(next, list),
        qPrintable(QStringLiteral("(c) the next Tab leaves %1: %2").arg(name, focus_name(w))));
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
    M4_CHECK(code + 4, focused_row_of(w, list) == row,
        qPrintable(QStringLiteral("(c) Shift+Tab returns to row %1 of %2: %3").arg(row).arg(name, focus_name(w))));
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
    QQuickItem* before = w->activeFocusItem();
    M4_CHECK(code + 5, before && !lies_in(before, list) && before != next,
        qPrintable(QStringLiteral("(c) the next Shift+Tab leaves %1 to the control before it: %2").arg(name, focus_name(w))));
    send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    M4_CHECK(code + 6, focused_row_of(w, list) == row,
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
    M4_CHECK(7033, wait_for_dir(w, top), "the folder with 1 000 entries is listed");
    finish_incubation(w);
    QQuickItem* list = visible_item(w, "browseListView");
    M4_CHECK(7034, list != nullptr, "browseListView missing");
    M4_CHECK(7035, tab_into(w, list, 80), "Tab reaches the folder list");
    M4_CHECK(7036, wait_until([&] { return focused_row_of(w, list) == 0; }, 2000),
        qPrintable(QStringLiteral("(a) the first time on the first row: %1 current=%2 item=%3")
            .arg(focus_name(w)).arg(list->property("currentIndex").toInt())
            .arg(describe_focus_item(list->property("currentItem").value<QQuickItem*>()))));
    send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    M4_CHECK(7037, !lies_in(w->activeFocusItem(), list),
        qPrintable(QStringLiteral("(c) one Tab leaves a list of 1 000 entries: ") + focus_name(w)));
    if (int rc = check_one_tab_stop(w, "browseListView", 150, 7040)) {
        return rc;
    }

    // (b) Home/End and Page Up/Page Down with the focus in the list.
    M4_CHECK(7050, tab_into(w, list, 80), "Tab reaches the folder list again");
    M4_CHECK(7059, b.model->rowCount() < 1000, "not every entry is loaded yet");
    press_and_wait(w, Qt::Key_End, 120);
    // The list moves to the last row once its pages have arrived.
    wait_until([&] { return list->property("currentIndex").toInt() == 999 && focused_row_of(w, list) == 999; }, 10000);
    M4_CHECK(7051, list->property("currentIndex").toInt() == 999 && focused_row_of(w, list) == 999,
        qPrintable(QStringLiteral("(b) one End press moves to the last of 1 000 entries (%1 loaded): %2")
            .arg(b.model->rowCount()).arg(focus_name(w))));
    // Up from the first visible row keeps the new current row below the column headings.
    press_and_wait(w, Qt::Key_PageUp, 120);
    press_and_wait(w, Qt::Key_Up, 80);
    QQuickItem* heading = list->property("headerItem").value<QQuickItem*>();
    QQuickItem* up_row = list->property("currentItem").value<QQuickItem*>();
    M4_CHECK(7064, heading && up_row && up_row->mapToItem(list, QPointF(0, 0)).y() >= heading->height() - 1,
        qPrintable(QStringLiteral("(b) Up keeps the current row in view below the headings: row y=%1, headings %2")
            .arg(up_row ? up_row->mapToItem(list, QPointF(0, 0)).y() : -1).arg(heading ? heading->height() : -1)));
    press_and_wait(w, Qt::Key_Home, 120);
    M4_CHECK(7052, focused_row_of(w, list) == 0, qPrintable(QStringLiteral("(b) Home moves to the first row: ") + focus_name(w)));
    press_and_wait(w, Qt::Key_PageDown, 120);
    const int page = focused_row_of(w, list);
    M4_CHECK(7053, page >= 2, qPrintable(QStringLiteral("(b) Page Down moves a page down: ") + focus_name(w)));
    press_and_wait(w, Qt::Key_Down, 80);
    M4_CHECK(7054, focused_row_of(w, list) == page + 1, "(b) Down moves one row");
    press_and_wait(w, Qt::Key_PageUp, 120);
    M4_CHECK(7055, focused_row_of(w, list) < page + 1, "(b) Page Up moves a page up");
    const int before_up = focused_row_of(w, list);
    press_and_wait(w, Qt::Key_Up, 80);
    M4_CHECK(7058, focused_row_of(w, list) == before_up - 1, "(b) Up moves one row");
    press_and_wait(w, Qt::Key_Home, 120);
    M4_CHECK(7056, focused_row_of(w, list) == 0, "(b) Home again on the first row");
    M4_CHECK(7057, save_screenshot(w, c.out_path), "screenshot failed");

    // (d) every list of every tab.
    int checked = 0;
    for (int tab = 0; tab <= 4; ++tab) {
        QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, tab));
        M4_CHECK(7060, wait_until([&] { return w->property("activeTab").toInt() == tab; }, 3000), "tab switched");
        wait_ms(600);
        finish_incubation(w);
        if (int rc = check_all_lists(w, 7061, &checked)) {
            return rc;
        }
    }
    M4_CHECK(7062, checked >= 5, qPrintable(QStringLiteral("(d) lists checked on the tabs: %1").arg(checked)));
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
    // The tab switch moves the focus to the selected backup a moment later; let it settle.
    wait_ms(1000);
    QQuickItem* list = visible_item(w, "restoreSnapshotsListView");
    QObject* menu = object_named(w, "snapshotContextMenu");
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    M4_CHECK(7106, list && menu && snaps, "snapshot list and menu exist");
    auto snapshot_of_menu = [&] { return menu->property("snapshotId").toString(); };

    // (a) Held for a second before the release.
    QQuickItem* first = row_at(w, 0);
    M4_CHECK(7107, first != nullptr, "first row created");
    const QPointF at = first->mapToScene(QPointF(first->width() * 0.4, first->height() * 0.5));
    send_mouse(w, at, Qt::NoButton, Qt::NoButton, QEvent::MouseMove, 20);
    send_mouse(w, at, Qt::RightButton, Qt::RightButton, QEvent::MouseButtonPress, 20);
    M4_CHECK(7108, wait_until([&] { return is_open(menu); }, 2000),
        "(a) the menu opens on the press, while the button is still down");
    wait_ms(1000);
    send_mouse(w, at, Qt::NoButton, Qt::RightButton, QEvent::MouseButtonRelease, 1200);
    wait_ms(200);
    M4_CHECK(7109, is_open(menu), "(a) the menu stays open after a long press");
    M4_CHECK_EQ(7110, snapshot_of_menu(), role_value(snaps, 0, "snapshotId").toString(),
        "(a) the menu belongs to the row clicked");
    const QPointF pos = popup_scene_pos(menu);
    M4_CHECK(7111, qAbs(pos.x() - at.x()) <= 2.0 && qAbs(pos.y() - at.y()) <= 2.0,
        qPrintable(QStringLiteral("(b) the menu opens at the pointer (%1,%2), not at (%3,%4)")
            .arg(at.x()).arg(at.y()).arg(pos.x()).arg(pos.y())));
    M4_CHECK(7112, save_screenshot(w, c.out_path), "screenshot failed");
    send_key(w, Qt::Key_Escape);
    M4_CHECK(7113, wait_until([&] { return is_closed(menu); }, 2000),
        qPrintable(QStringLiteral("(d) Esc closes the menu, also with the pointer on its first entry: opened=%1 focus=%2")
            .arg(menu->property("opened").toBool()).arg(focus_name(w))));

    // (a) The pointer moves while the button is down.
    QQuickItem* second = row_at(w, 1);
    M4_CHECK(7114, second != nullptr, "second row created");
    const QPointF from = second->mapToScene(QPointF(second->width() * 0.3, second->height() * 0.3));
    const QPointF to = from + QPointF(14.0, 12.0);
    send_mouse(w, from, Qt::NoButton, Qt::NoButton, QEvent::MouseMove, 20);
    send_mouse(w, from, Qt::RightButton, Qt::RightButton, QEvent::MouseButtonPress, 20);
    send_mouse(w, to, Qt::RightButton, Qt::NoButton, QEvent::MouseMove, 40);
    send_mouse(w, to, Qt::NoButton, Qt::RightButton, QEvent::MouseButtonRelease, 40);
    M4_CHECK(7115, wait_until([&] { return is_open(menu); }, 2000), "(a) the menu opens when the pointer moves");
    M4_CHECK_EQ(7116, snapshot_of_menu(), role_value(snaps, 1, "snapshotId").toString(),
        "(a) the menu belongs to the second row");
    send_key(w, Qt::Key_Escape);
    M4_CHECK(7117, wait_until([&] { return is_closed(menu); }, 2000), "Esc closes the menu");

    // (c) Short clicks on every row, twice each.
    for (int round = 0; round < 2; ++round) {
        for (int r = 0; r < 3; ++r) {
            QQuickItem* row = row_at(w, r);
            M4_CHECK(7118, row != nullptr, "row created");
            const QPointF p = row->mapToScene(QPointF(row->width() * 0.6, row->height() * 0.5));
            send_mouse(w, p, Qt::NoButton, Qt::NoButton, QEvent::MouseMove, 300);
            send_mouse(w, p, Qt::RightButton, Qt::RightButton, QEvent::MouseButtonPress, 20);
            send_mouse(w, p, Qt::NoButton, Qt::RightButton, QEvent::MouseButtonRelease, 60);
            M4_CHECK(7119, wait_until([&] { return is_open(menu); }, 2000),
                qPrintable(QStringLiteral("(c) the menu opens on row %1, round %2").arg(r).arg(round)));
            M4_CHECK_EQ(7120, snapshot_of_menu(), role_value(snaps, r, "snapshotId").toString(),
                "(c) the menu belongs to the row clicked");
            send_key(w, Qt::Key_Escape);
            M4_CHECK(7121, wait_until([&] { return is_closed(menu); }, 2000), "Esc closes the menu");
        }
    }
    return 0;
}

// The selection state of the header check box of the folder list (0, 1 or 2).
int header_state(QObject* check) {
    return check->property("checkState").toInt();
}

/// AC-M4-03-8: «Select All» in the folder list. (a) The header check box selects every entry
/// of the folder shown, also those not loaded yet, and adds none to the view; (b) it shows none,
/// some or all selected; (c) clicked with all selected it clears the folder shown; (d) Ctrl+A
/// selects all, while a text field keeps Ctrl+A for its text; (e) with a filter it selects the
/// matching entries.
int scenario_browse_select_all(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7201)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 1, 7206)) {
        return rc;
    }
    Browse b = browse_page(w);
    const QString top = b.model->property("dir").toString();
    M4_CHECK(7209, wait_for_dir(w, top), "the folder with 1 000 entries is listed");
    finish_incubation(w);
    QQuickItem* check = visible_item(w, "browseSelectAllCheck");
    M4_CHECK(7210, check != nullptr, "the header check box is shown");
    M4_CHECK(7211, b.model->rowCount() == 200, "200 of 1 000 entries loaded");
    M4_CHECK(7212, header_state(check) == Qt::Unchecked, "(b) none selected: unchecked");
    M4_CHECK_EQ(7213, accessible_name(check), loc(c.lang, "Select All", "Alles auswählen"),
        "the header check box is named «Select All»");

    M4_CHECK(7214, click_ui_item(w, check, "select all"), "the header check box could not be clicked");
    M4_CHECK(7215, wait_until([&] { return b.model->property("selection_count").toInt() == 1000; }, 10000),
        qPrintable(QStringLiteral("(a) all 1 000 entries selected, also those not loaded: %1")
            .arg(b.model->property("selection_count").toInt())));
    M4_CHECK(7216, b.model->rowCount() == 200, "(a) selecting adds no rows to the view");
    M4_CHECK(7217, role_value(b.model, 0, "selected").toBool() && role_value(b.model, 199, "selected").toBool(),
        "(a) the loaded rows show their marks");
    M4_CHECK(7218, header_state(check) == Qt::Checked, "(b) all selected: checked");
    QMetaObject::invokeMethod(object_named(w, "browseListView"), "positionViewAtEnd");
    M4_CHECK(7219, wait_until([&] { return b.model->rowCount() > 200; }, 5000), "scrolling loads the next page");
    M4_CHECK(7220, role_value(b.model, 200, "selected").toBool(), "(a) a row loaded later shows its mark");
    // The headings stay above the rows while the list scrolls: at row 180 the check box is at the
    // top of the list and can be clicked.
    QQuickItem* list = visible_item(w, "browseListView");
    QMetaObject::invokeMethod(list, "positionViewAtIndex", Q_ARG(int, 180), Q_ARG(int, 0));
    wait_ms(200);
    const QPointF at = check->mapToItem(list, QPointF(0, 0));
    M4_CHECK(7256, check->isVisible() && at.y() >= 0 && at.y() < check->height(),
        qPrintable(QStringLiteral("the header check box stays at the top of the scrolled list: y=%1").arg(at.y())));
    M4_CHECK(7221, save_screenshot(w, screenshot_variant(c.out_path, "-all")), "screenshot failed");
    // A click on a heading reaches no row scrolled beneath it.
    QQuickItem* name_heading = visible_item(w, "browseNameHeading");
    M4_CHECK(7257, name_heading && click_ui_item(w, name_heading, "Name heading"), "the «Name» heading could not be clicked");
    wait_ms(200);
    M4_CHECK(7258, b.model->property("dir").toString() == top && b.model->property("selection_count").toInt() == 1000,
        "a click on the headings opens and marks nothing");

    M4_CHECK(7222, click_ui_item(w, check, "clear all"), "the header check box could not be clicked while scrolled");
    M4_CHECK(7223, wait_until([&] { return b.model->property("selection_count").toInt() == 0; }, 10000),
        "(c) clicked with all selected, it clears the folder");
    M4_CHECK(7224, header_state(check) == Qt::Unchecked, "(b) unchecked again");
    QMetaObject::invokeMethod(list, "positionViewAtBeginning");
    wait_ms(200);
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, 1));
    M4_CHECK(7225, header_state(check) == Qt::PartiallyChecked, "(b) one selected: partially checked");
    repaint_fully(w);
    M4_CHECK(7226, save_screenshot(w, c.out_path), "screenshot failed");
    M4_CHECK(7227, click_ui_item(w, check, "select all from partial"), "the header check box could not be clicked");
    M4_CHECK(7228, wait_until([&] { return b.model->property("selection_count").toInt() == 1000; }, 10000),
        "(b) clicked when partially checked, it selects all");
    QMetaObject::invokeMethod(b.model, "clearSelection");

    // (d) Ctrl+A in the list, and in the filter field.
    M4_CHECK(7229, tab_into(w, visible_item(w, "browseListView"), 80), "Tab reaches the folder list");
    send_key(w, Qt::Key_A, Qt::ControlModifier);
    M4_CHECK(7230, wait_until([&] { return b.model->property("selection_count").toInt() == 1000; }, 10000),
        "(d) Ctrl+A selects every entry");
    M4_CHECK(7231, header_state(check) == Qt::Checked, "(d) the header follows Ctrl+A");
    QMetaObject::invokeMethod(b.model, "clearSelection");
    QQuickItem* filter = tab_to(w, "browseFilterField", 40);
    M4_CHECK(7232, filter != nullptr, "Tab reaches the filter field");
    for (const QChar ch : QStringLiteral("bericht")) {
        send_key(w, Qt::Key_A + (ch.toUpper().unicode() - QLatin1Char('A').unicode()), Qt::NoModifier, QString(ch));
    }
    M4_CHECK(7233, wait_until([&] {
        return b.model->property("matched").toInt() == 37 && b.model->property("state").toString() == QLatin1String("done");
    }, 10000), "the filter leaves the 37 matching entries");
    send_key(w, Qt::Key_A, Qt::ControlModifier);
    wait_ms(200);
    M4_CHECK(7234, b.model->property("selection_count").toInt() == 0, "(d) Ctrl+A in the filter field selects its text, no entry");
    M4_CHECK(7235, filter->property("selectedText").toString() == QLatin1String("bericht"), "(d) the filter text is selected");

    // (e) Select All with the filter.
    M4_CHECK(7236, click_ui_item(w, check, "select matching"), "the header check box could not be clicked");
    M4_CHECK(7237, wait_until([&] { return b.model->property("selection_count").toInt() == 37; }, 10000),
        qPrintable(QStringLiteral("(e) the 37 matching entries selected: %1").arg(b.model->property("selection_count").toInt())));
    M4_CHECK(7238, header_state(check) == Qt::Checked, "(e) all matching selected: checked");
    QMetaObject::invokeMethod(b.model, "clearSelection");
    // (e) More matches than one page: the names with a «1» (271 of 1 000).
    QMetaObject::invokeMethod(b.model, "setFilter", Q_ARG(QString, QStringLiteral("1")));
    M4_CHECK(7239, wait_until([&] {
        return b.model->property("matched").toInt() == 271 && b.model->property("state").toString() == QLatin1String("done");
    }, 10000), qPrintable(QStringLiteral("the filter «1» leaves 271 entries: %1").arg(b.model->property("matched").toInt())));
    M4_CHECK(7240, b.model->rowCount() == 200, "one page of them is loaded");
    M4_CHECK(7241, click_ui_item(w, check, "select 271"), "the header check box could not be clicked");
    M4_CHECK(7242, wait_until([&] { return b.model->property("selection_count").toInt() == 271; }, 10000),
        "(e) all 271 matching entries selected, also those not loaded");
    M4_CHECK(7243, header_state(check) == Qt::Checked, "(b, e) all matching selected: checked");
    M4_CHECK(7244, click_ui_item(w, check, "clear 271"), "the header check box could not be clicked");
    M4_CHECK(7245, wait_until([&] { return b.model->property("selection_count").toInt() == 0; }, 10000),
        "(c, e) clicked with all matching selected, it clears them");
    return 0;
}

/// AC-M4-05-26: with every entry of the snapshot's top folder selected, (a) the action reads
/// «Restore All…» and restores the whole snapshot (empty selection, the dialog «Restore
/// Snapshot»), also while a subfolder is shown; (b) an entry outside the top folder keeps
/// «Restore Selection…»; (c) so do all entries of a subfolder. The test checks the restic call.
int scenario_restore_all(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7301, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7306)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(7309, wait_for_dir(w, top), "the snapshot opens in its folder");
    finish_incubation(w);
    Browse b = browse_page(w);
    QObject* action = object_named(w, "restoreSelectionAction");
    M4_CHECK(7310, action != nullptr, "restoreSelectionAction missing");
    const QString selection_text = loc(c.lang, "Restore Selection…", "Auswahl wiederherstellen…");
    const QString all_text = loc(c.lang, "Restore All…", "Alles wiederherstellen…");
    M4_CHECK_EQ(7311, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("a.txt,b.bin,docs,empty"),
        "four entries in the top folder");
    for (int r = 0; r < 3; ++r) {
        QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, r));
    }
    M4_CHECK_EQ(7312, action->property("text").toString(), selection_text, "three of four: «Restore Selection…»");
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, 3));
    M4_CHECK_EQ(7313, action->property("text").toString(), all_text, "(a) all four: «Restore All…»");
    M4_CHECK(7314, b.model->property("whole_snapshot").toBool(), "(a) the selection is the whole snapshot");

    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, top + QStringLiteral("/docs")));
    M4_CHECK(7315, wait_for_dir(w, top + QStringLiteral("/docs")), "«docs» opens");
    M4_CHECK_EQ(7316, action->property("text").toString(), all_text, "(a) also while a subfolder is shown");
    QMetaObject::invokeMethod(b.model, "clearSelection");
    QMetaObject::invokeMethod(b.model, "selectAll");
    M4_CHECK(7317, wait_until([&] { return b.model->property("selection_count").toInt() == 2; }, 5000),
        "both entries of «docs» selected");
    M4_CHECK_EQ(7318, action->property("text").toString(), selection_text, "(c) all of a subfolder: «Restore Selection…»");

    const int cut = top.lastIndexOf(QLatin1Char('/'));
    const QString above = top.left(cut);
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, above));
    M4_CHECK(7319, wait_for_dir(w, above), "the folder above the top folder opens");
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, top));
    M4_CHECK(7320, wait_for_dir(w, top), "the top folder opens again");
    QMetaObject::invokeMethod(b.model, "selectAll");
    M4_CHECK(7321, wait_until([&] { return b.model->property("selection_count").toInt() == 6; }, 5000),
        "the top folder and «docs» selected");
    M4_CHECK_EQ(7322, action->property("text").toString(), all_text,
        "(a) entries below a selected one change nothing");
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, above));
    M4_CHECK(7323, wait_for_dir(w, above), "the folder above opens again");
    const int src_row = browse_names(b.model).indexOf(top.mid(cut + 1));
    M4_CHECK(7324, src_row >= 0, "the top folder is listed in the folder above");
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, src_row));
    M4_CHECK_EQ(7325, action->property("text").toString(), selection_text,
        "(b) the top folder itself selected from above: «Restore Selection…»");
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, src_row));
    M4_CHECK_EQ(7326, action->property("text").toString(), all_text, "back to every entry of the top folder");

    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, top));
    M4_CHECK(7327, wait_for_dir(w, top), "the top folder opens for the screenshot");
    finish_incubation(w);
    M4_CHECK(7328, save_screenshot(w, c.out_path), "screenshot failed");
    trigger(action);
    Restore r = restore_dialog(w);
    M4_CHECK(7329, r.dialog && wait_until([&] { return is_open(r.dialog); }, 3000), "«Restore All…» opens the dialog");
    M4_CHECK_EQ(7330, r.dialog->property("title").toString(),
        loc(c.lang, "Restore Snapshot", "Snapshot wiederherstellen"),
        "(a) the dialog restores the whole snapshot, like «Restore…» in the context menu");
    M4_CHECK(7331, wait_for_preview(r.model), "the check of the new folder is done");
    QObject* restore_now = object_named(w, "restoreNowAction");
    M4_CHECK(7332, restore_now && restore_now->property("enabled").toBool(), "«Restore Now» enabled");
    trigger(restore_now);
    M4_CHECK(7333, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("ok"); }, 60000),
        qPrintable(QStringLiteral("the restore ends ok: ") + r.model->property("run_state").toString()));
    return 0;
}

// AC-M4-05-26 (a), browser opened from the search hit for `text` (a folder hit when `is_dir`) in
// a snapshot of one file, whose tree does not hold its common folder as a folder: the whole
// snapshot is restored from `/`; with every entry of `/` selected the action reads «Restore
// All…» and restores the whole snapshot.
int restore_all_from_hit(const Ctx& c, const QString& text, bool is_dir, int code) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, code, 1)) {
        return rc;
    }
    if (int rc = search_with_keyboard(c, text, code + 5)) {
        return rc;
    }
    Search s = search_page(w);
    int hit = -1;
    for (int r = 0; r < s.model->rowCount(); ++r) {
        if (role_value(s.model, r, "path").toString().endsWith(QLatin1Char('/') + text)
            && role_value(s.model, r, "isDir").toBool() == is_dir) {
            hit = r;
        }
    }
    M4_CHECK(code + 10, hit >= 0, is_dir ? "the folder is a hit" : "the file is a hit");
    const QString path = role_value(s.model, hit, "path").toString();
    const QString folder = role_value(s.model, hit, "folder").toString();
    M4_CHECK(code + 11, path.startsWith(QLatin1String("/home/")), qPrintable(QStringLiteral("the hit below /home: ") + path));
    M4_CHECK(code + 12, folder == path.section(QLatin1Char('/'), 0, -2), "the folder of the hit holds it");
    QMetaObject::invokeMethod(s.page, "showHit", Q_ARG(QVariant, hit));
    M4_CHECK(code + 13, wait_for_dir(w, folder), "the folder of the hit opens");
    finish_incubation(w);
    Browse b = browse_page(w);
    M4_CHECK_EQ(code + 14, b.model->property("top_dir").toString(), QStringLiteral("/"),
        "(a) a snapshot of one file is restored from `/`");
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, QStringLiteral("/")));
    M4_CHECK(code + 15, wait_for_dir(w, QStringLiteral("/")), "`/` opens");
    QMetaObject::invokeMethod(b.model, "selectAll");
    M4_CHECK(code + 16, wait_until([&] {
            return b.model->property("selection_count").toInt() == b.model->property("total").toInt()
                && b.model->property("selection_count").toInt() > 0;
        }, 5000),
        "every entry of `/` selected");
    QObject* action = object_named(w, "restoreSelectionAction");
    M4_CHECK(code + 17, action != nullptr, "restoreSelectionAction missing");
    M4_CHECK_EQ(code + 18, action->property("text").toString(), loc(c.lang, "Restore All…", "Alles wiederherstellen…"),
        "(a) every entry of `/`: «Restore All…»");
    M4_CHECK(code + 19, b.model->property("whole_snapshot").toBool(), "(a) the selection is the whole snapshot");
    QStringList selection;
    QMetaObject::invokeMethod(b.model, "restoreSelection", Q_RETURN_ARG(QStringList, selection));
    M4_CHECK(code + 20, selection.isEmpty(), "(a) the restore request has an empty selection");
    finish_incubation(w);
    M4_CHECK(code + 21, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// AC-M4-05-26 (a): «Restore All…» in a snapshot of one file opened from the hit of that file.
int scenario_restore_all_from_search(const Ctx& c) {
    return restore_all_from_hit(c, QStringLiteral("a.txt"), false, 7801);
}

/// AC-M4-05-26 (a): «Restore All…» in a snapshot of one file opened from a folder hit above the
/// file; the browser lists the snapshot's common folder first to learn that it is none.
int scenario_restore_all_from_folder_hit(const Ctx& c) {
    return restore_all_from_hit(c, QStringLiteral("Dokumente"), true, 7841);
}

// The name of the control that holds the keyboard focus in the restore dialog: the object name
// of the nearest named restore control, «close» for the close button of the dialog's header,
// otherwise the text of the button.
QString dialog_focus_id(QQuickWindow* w) {
    QQuickItem* f = w->activeFocusItem();
    for (QQuickItem* it = f; it; it = it->parentItem()) {
        if (it->objectName().startsWith(QLatin1String("restore"))) {
            return it->objectName();
        }
    }
    if (f && QQmlProperty::read(f, QStringLiteral("icon.name")).toString().startsWith(QLatin1String("window-close"))) {
        return QStringLiteral("close");
    }
    return f ? f->property("text").toString().remove(QLatin1Char('&')) : QStringLiteral("-");
}

// The Tab order of the restore dialog from the control in focus, `n` steps forward (or with
// Shift+Tab backward), as names.
QStringList dialog_tab_order(QQuickWindow* w, int n, bool forward) {
    QStringList out;
    for (int i = 0; i < n; ++i) {
        if (forward) {
            send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
        } else {
            send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
        }
        out << dialog_focus_id(w);
    }
    return out;
}

/// AC-M4-05-27: the Tab order of the restore dialog is «Into a new folder», «Choose…»,
/// «To the original location», «Existing files» (when enabled), the preview list with its
/// controls (when shown), «Cancel», «Restore Now», the close button; (a) forward and (b)
/// backward, (c) Tab never leaves the dialog, and (d) Esc closes it and the focus returns to the
/// folder list.
int scenario_restore_dialog_tab(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7401, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7406)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(7409, wait_for_dir(w, top), "the snapshot opens in its folder");
    finish_incubation(w);
    M4_CHECK(7410, tab_into(w, visible_item(w, "browseListView"), 60), "Tab reaches the folder list");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7411)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    M4_CHECK(7420, wait_for_preview(r.model), "the check of the new folder is done");
    wait_ms(300);
    const QString cancel = loc(c.lang, "Cancel", "Abbrechen");
    const QString now = loc(c.lang, "Restore Now", "Jetzt wiederherstellen");
    const QString close = QStringLiteral("close");
    M4_CHECK_EQ(7421, dialog_focus_id(w), QStringLiteral("restoreNewFolderRadio"), "the dialog opens on «Into a new folder»");
    const QStringList new_folder = {QStringLiteral("restoreChooseFolderButton"), QStringLiteral("restoreOriginalRadio"),
        cancel, now, close, QStringLiteral("restoreNewFolderRadio")};
    M4_CHECK_EQ(7422, dialog_tab_order(w, 6, true).join(QLatin1String(" > ")), new_folder.join(QLatin1String(" > ")),
        "(a, c) Tab order into a new folder");
    QStringList back = new_folder;
    std::reverse(back.begin(), back.end());
    back.removeFirst();
    back << QStringLiteral("restoreNewFolderRadio");
    M4_CHECK_EQ(7423, dialog_tab_order(w, 6, false).join(QLatin1String(" > ")), back.join(QLatin1String(" > ")),
        "(b, c) Shift+Tab order into a new folder");

    QObject* original = object_named(w, "restoreOriginalRadio");
    M4_CHECK(7424, click_ui_item(w, qobject_cast<QQuickItem*>(original), "original location"), "original location");
    M4_CHECK(7425, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000), "original chosen");
    M4_CHECK(7426, wait_for_preview(r.model), "the preview at the original location is done");
    QObject* list_button = object_named(w, "restoreListButton");
    M4_CHECK(7427, list_button && click_ui_item(w, qobject_cast<QQuickItem*>(list_button), "list"), "the list opens");
    M4_CHECK(7428, wait_until([&] { return visible_item(w, "restorePreviewList") != nullptr; }, 3000), "the preview list is shown");
    finish_incubation(w);
    qobject_cast<QQuickItem*>(original)->forceActiveFocus(Qt::TabFocusReason);
    QStringList preview;
    preview << QStringLiteral("restoreOverwriteBox") << QStringLiteral("restoreListButton");
    if (visible_item(w, "restorePreviewFilter")) {
        preview << QStringLiteral("restorePreviewFilter");
    }
    preview << QStringLiteral("restorePreviewList") << cancel;
    const QString restore_text = r.model->property("overwrites").toBool()
        ? loc(c.lang, "Overwrite and Restore", "Überschreiben und wiederherstellen") : now;
    preview << restore_text << close << QStringLiteral("restoreNewFolderRadio") << QStringLiteral("restoreChooseFolderButton")
            << QStringLiteral("restoreOriginalRadio");
    M4_CHECK_EQ(7429, dialog_tab_order(w, preview.size(), true).join(QLatin1String(" > ")), preview.join(QLatin1String(" > ")),
        "(a, c) Tab order at the original location with the list shown");
    QStringList preview_back = preview;
    std::reverse(preview_back.begin(), preview_back.end());
    preview_back.removeFirst();
    preview_back << QStringLiteral("restoreOriginalRadio");
    M4_CHECK_EQ(7430, dialog_tab_order(w, preview_back.size(), false).join(QLatin1String(" > ")),
        preview_back.join(QLatin1String(" > ")), "(b, c) Shift+Tab order at the original location");
    repaint_fully(w);
    M4_CHECK(7431, save_screenshot(w, c.out_path), "screenshot failed");

    send_key(w, Qt::Key_Escape);
    M4_CHECK(7432, wait_until([&] { return is_closed(r.dialog); }, 3000), "(d) Esc closes the dialog");
    M4_CHECK(7433, wait_until([&] { return focused_entry_row(w) != nullptr; }, 2000),
        qPrintable(QStringLiteral("(d) the focus returns to the folder list: ") + focus_name(w)));
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
    QObject* bottom = object_named(w, "bottomNavBar");
    M4_CHECK(7509, stack && bottom, "page stack and bottom tab bar exist");
    w->resize(1400, w->height());
    QCoreApplication::processEvents();
    M4_CHECK(7510, !w->property("narrowMode").toBool(), "1400 px is wide");
    const int wide = first_wide_width(w, 300, 1400);
    w->resize(wide - 1, w->height());
    wait_ms(300);
    M4_CHECK(7511, w->property("narrowMode").toBool(), "one pixel less is narrow");
    M4_CHECK(7512, bottom->property("visible").toBool(), "(a) narrow: the tab bar is at the bottom");
    M4_CHECK(7513, !stack->property("wideMode").toBool(),
        qPrintable(QStringLiteral("(a) narrow at %1 px: one column").arg(wide - 1)));
    w->resize(wide, w->height());
    wait_ms(300);
    M4_CHECK(7514, !bottom->property("visible").toBool(), "(a) wide: the tab bar is at the top");
    M4_CHECK(7515, stack->property("wideMode").toBool(),
        qPrintable(QStringLiteral("(a) wide at %1 px: two columns").arg(wide)));

    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(7516, wait_for_dir(w, top), "the folder is listed");
    finish_incubation(w);
    wait_ms(300);
    QQuickItem* page = visible_item(w, "restoreBrowsePage");
    M4_CHECK(7517, page && page->width() < wide * 0.6, "the folder list is one of two columns");
    QList<QQuickItem*> items;
    collect_visual_descendants(page, items);
    int names = 0;
    for (QQuickItem* it : items) {
        if (it->objectName() == QLatin1String("browseEntryName") && it->isVisible()) {
            names += 1;
            M4_CHECK(7518, !it->property("truncated").toBool(),
                qPrintable(QStringLiteral("(b) «%1» is shown in full in a column of %2 px")
                    .arg(it->property("text").toString()).arg(page->width())));
        }
    }
    M4_CHECK(7519, names == 4, "(b) the four names of the folder checked");
    M4_CHECK(7520, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// The icon `message` (an InlineMessage) shows: its visible Kirigami icon, or nullptr.
QQuickItem* message_icon(QObject* message) {
    QList<QQuickItem*> items;
    collect_visual_descendants(qobject_cast<QQuickItem*>(message), items);
    for (QQuickItem* it : items) {
        if (it->isVisible() && it->property("valid").isValid() && it->property("source").isValid()
            && it->width() > 0) {
            return it;
        }
    }
    return nullptr;
}

/// AC-M4-05-29: the restore banner (a) shows a running restore with the icon `document-revert`,
/// which the icon theme has, (b) a cancelled restore as neutral information with the icon
/// `process-stop` and «Restore cancelled», and an interrupted one as neutral information with
/// the icon `state-offline`, and (c) comes back for the next restore after its close button.
int scenario_restore_banner(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7601, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7606)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    M4_CHECK(7609, wait_for_dir(w, role_value(snaps, 0, "startDir").toString()), "the snapshot opens");
    finish_incubation(w);
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7610)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    M4_CHECK(7620, wait_for_preview(r.model), "the check of the new folder is done");
    trigger(object_named(w, "restoreNowAction"));
    M4_CHECK(7621, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("running"); }, 10000),
        "the restore runs (restic held)");
    QObject* banner = object_named(w, "restoreRunBanner");
    M4_CHECK(7622, banner && banner->property("visible").toBool(), "the banner shows the running restore");
    wait_ms(400);
    QQuickItem* running_icon = message_icon(banner);
    M4_CHECK(7623, running_icon != nullptr, "the banner shows an icon");
    M4_CHECK_EQ(7624, running_icon->property("source").toString(), QStringLiteral("document-revert"),
        "(a) the running restore shows the restore icon");
    M4_CHECK(7625, running_icon->property("valid").toBool(), "(a) the icon theme has the icon");
    // AC-M4-05-30 (d): the test advances the daemon's clock and the bytes restored.
    M4_CHECK(7634, wait_until([&] {
        return r.model->property("run_text").toString().contains(QStringLiteral("/s"));
    }, 20000), qPrintable(QStringLiteral("AC-M4-05-30 (d) the message names the speed: ")
        + r.model->property("run_text").toString()));
    repaint_fully(w);
    M4_CHECK(7626, save_screenshot(w, screenshot_variant(c.out_path, "-running")), "screenshot failed");

    trigger(object_named(w, "restoreRunCancelAction"));
    M4_CHECK(7627, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("cancelled"); }, 30000),
        qPrintable(QStringLiteral("(b) the run ends as cancelled: ") + r.model->property("run_state").toString()));
    M4_CHECK(7628, banner->property("type").toInt() == 0, "(b) neutral information, no error");
    M4_CHECK_EQ(7629, r.model->property("run_text").toString(),
        loc(c.lang, "Restore cancelled", "Wiederherstellen abgebrochen"), "(b) the text names the run kind");
    wait_ms(300);
    QQuickItem* cancelled_icon = message_icon(banner);
    M4_CHECK(7630, cancelled_icon != nullptr, "the banner shows an icon");
    M4_CHECK_EQ(7631, cancelled_icon->property("source").toString(), QStringLiteral("process-stop"),
        "(b) the icon of «cancelled»");
    M4_CHECK(7632, cancelled_icon->property("valid").toBool(), "(b) the icon theme has the icon");
    repaint_fully(w);
    M4_CHECK(7633, save_screenshot(w, c.out_path), "screenshot failed");

    // (c) Closed with its close button, the message comes back for the next restore.
    QQuickItem* close = nullptr;
    QList<QQuickItem*> items;
    collect_visual_descendants(qobject_cast<QQuickItem*>(banner), items);
    for (QQuickItem* it : items) {
        // A cancelled restore offers no action: the only button is the close button.
        if (it->isVisible() && it->inherits("QQuickAbstractButton")) {
            M4_CHECK(7647, close == nullptr, "one button in the message of a cancelled restore");
            close = it;
        }
    }
    M4_CHECK(7635, close != nullptr, "the message has a close button");
    M4_CHECK(7636, click_ui_item(w, close, "close message"), "the close button could not be clicked");
    M4_CHECK(7637, wait_until([&] { return !banner->property("visible").toBool(); }, 3000), "the message is closed");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7638)) {
        return rc;
    }
    M4_CHECK(7642, wait_for_preview(r.model), "the check of the new folder is done");
    trigger(object_named(w, "restoreNowAction"));
    M4_CHECK(7643, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("running"); }, 10000),
        "the second restore runs");
    M4_CHECK(7644, wait_until([&] { return banner->property("visible").toBool(); }, 3000),
        "(c) the message shows the second restore");
    QObject* cancel_action = object_named(w, "restoreRunCancelAction");
    M4_CHECK(7645, cancel_action && cancel_action->property("visible").toBool(), "(c) with «Cancel»");
    trigger(cancel_action);
    M4_CHECK(7646, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("cancelled"); }, 30000),
        "the second restore ends");

    // (b) An interrupted restore (sleep, daemon start): the model maps the run status to the state
    // «interrupted» (unit test); here the banner shows that state.
    r.model->setProperty("run_state", QStringLiteral("interrupted"));
    M4_CHECK(7648, banner->property("visible").toBool(), "(b) the banner shows the interrupted restore");
    M4_CHECK(7649, banner->property("type").toInt() == 0, "(b) interrupted: neutral information, no error");
    wait_ms(300);
    QQuickItem* interrupted_icon = message_icon(banner);
    M4_CHECK(7650, interrupted_icon != nullptr, "the banner shows an icon");
    M4_CHECK_EQ(7651, interrupted_icon->property("source").toString(), QStringLiteral("state-offline"),
        "(b) the icon of «interrupted»");
    M4_CHECK(7652, interrupted_icon->property("valid").toBool(), "(b) the icon theme has the icon");
    return 0;
}

/// AC-M4-05-28: a network folder (`smb://`, `sftp://`) from the folder chooser (a) leaves the
/// target unchanged, (b) shows that only local folders can be chosen, and (c) asks the daemon
/// nothing; a local folder after it is taken.
int scenario_restore_network_folder(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7701, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7706)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    M4_CHECK(7709, wait_for_dir(w, top), "the snapshot opens");
    finish_incubation(w);
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7710)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    M4_CHECK(7720, wait_for_preview(r.model), "the check of the new folder is done");
    QObject* message = object_named(w, "restoreNetworkFolderMessage");
    M4_CHECK(7721, message && !message->property("visible").toBool(), "no message before a network folder");
    const QString target = r.model->property("target_dir").toString();
    // A chooser closed without a folder changes nothing and is no network folder.
    QMetaObject::invokeMethod(r.dialog, "chooseFolderUrl", Q_ARG(QVariant, QUrl()));
    M4_CHECK(7731, !message->property("visible").toBool(), "no message for an empty choice");
    M4_CHECK_EQ(7732, r.model->property("target_dir").toString(), target, "an empty choice keeps the target");
    const QString text = loc(c.lang,
        "Network folders such as smb:// cannot be chosen directly. Mount the folder first or choose a local folder.",
        "Netzwerkordner wie smb:// lassen sich nicht direkt wählen. Den Ordner zuerst einbinden oder einen lokalen Ordner wählen.");
    for (const char* url : {"smb://ds1525.local/home/upload/restore", "sftp://nas.local/srv/restore",
             "file://nas.local/srv/restore"}) {
        QMetaObject::invokeMethod(r.dialog, "chooseFolderUrl", Q_ARG(QVariant, QUrl(QString::fromLatin1(url))));
        wait_ms(500);
        M4_CHECK_EQ(7722, r.model->property("target_dir").toString(), target, "(a) the target stays");
        M4_CHECK_EQ(7723, r.model->property("state").toString(), QStringLiteral("done"),
            "(c) no new check: the daemon is not asked");
        M4_CHECK(7724, message->property("visible").toBool(), "(b) the message is shown");
        M4_CHECK(7725, message->property("text").toString().contains(text.toHtmlEscaped()),
            qPrintable(QStringLiteral("(b) the message: ") + message->property("text").toString()));
    }
    repaint_fully(w);
    M4_CHECK(7726, save_screenshot(w, c.out_path), "screenshot failed");
    QObject* new_radio = object_named(w, "restoreNewFolderRadio");
    M4_CHECK(7729, click_ui_item(w, qobject_cast<QQuickItem*>(new_radio), "new folder"), "the new folder is chosen");
    M4_CHECK(7730, wait_until([&] { return !message->property("visible").toBool(); }, 3000),
        "the message goes with another target");
    QMetaObject::invokeMethod(r.dialog, "chooseFolderUrl",
        Q_ARG(QVariant, QUrl(QStringLiteral("file://localhost") + top)));
    M4_CHECK(7727, wait_until([&] { return r.model->property("target_dir").toString() == top; }, 5000),
        "a local folder written with the host «localhost» is taken");
    M4_CHECK(7728, !message->property("visible").toBool(), "no message with a local folder");
    return 0;
}

// Where the Tab stop `it` lies: the header with the navigation, a page's content, or the action
// bar of a page at the bottom of the page row.
QString tab_region(QQuickWindow* w, QQuickItem* it) {
    if (!it) {
        return QStringLiteral("none");
    }
    if (lies_in(it, item_named(w, "mainHeaderArea"))) {
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

void press_backtab(QQuickWindow* w) {
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
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
    M4_CHECK(7810, tab_to(w, "restoreSnapshotListItem", 40) != nullptr, "Tab reaches the snapshot list");
    press_tab(w);
    QObject* compare = object_named(w, "restoreCompareAction");
    M4_CHECK(7811, compare != nullptr, "restoreCompareAction missing");
    M4_CHECK(7812, tab_region(w, w->activeFocusItem()) == QLatin1String("action bar"),
        qPrintable(QStringLiteral("one Tab from the snapshot list reaches the action bar: ") + focus_name(w)));
    M4_CHECK(7813, w->activeFocusItem()->property("text").toString() == compare->property("text").toString(),
        qPrintable(QStringLiteral("one Tab from the snapshot list reaches «Compare…»: ") + focus_name(w)));

    M4_CHECK(7820, tab_to(w, "restoreBackupListItem", 40) != nullptr, "Tab reaches the backup list");
    press_backtab(w);
    M4_CHECK(7821, tab_region(w, w->activeFocusItem()) == QLatin1String("navigation"),
        qPrintable(QStringLiteral("Shift+Tab from the backup list reaches the navigation: ") + focus_name(w)));

    const QStringList round = tab_round_regions(w, 60);
    const QStringList expected { QStringLiteral("navigation"), QStringLiteral("content"), QStringLiteral("action bar") };
    M4_CHECK(7830, round == expected,
        qPrintable(QStringLiteral("a full Tab round on «Restore» runs navigation, content, action bar: ")
            + round.join(QStringLiteral(", "))));
    M4_CHECK(7831, save_screenshot(w, c.out_path), "screenshot failed");

    QObject* backups_tab = object_named(w, "backupsTabAction");
    M4_CHECK(7840, backups_tab != nullptr, "backupsTabAction missing");
    trigger(backups_tab);
    M4_CHECK(7841, wait_until([&] {
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
    M4_CHECK(7842, tab_region(w, w->activeFocusItem()) == QLatin1String("action bar"),
        qPrintable(QStringLiteral("Tab reaches the action bar of «Backups»: ") + focus_name(w)));
    M4_CHECK(7843, before == QLatin1String("content"),
        qPrintable(QStringLiteral("on «Backups» the action bar follows the content, not ") + before));
    for (int i = 0; i < 10 && tab_region(w, w->activeFocusItem()) == QLatin1String("action bar"); ++i) {
        press_tab(w);
    }
    M4_CHECK(7844, tab_region(w, w->activeFocusItem()) == QLatin1String("navigation"),
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
    M4_CHECK(7910, list != nullptr, "restoreSnapshotsListView missing");
    const int count = list->property("count").toInt();
    QQuickItem* flick = outer_flickable(list);
    M4_CHECK(7911, flick != nullptr, "no flickable around the snapshot list");
    // The list grows to the height of its rows once the layout has run.
    M4_CHECK(7912, wait_until([&] { return list->height() > flick->height() * 2; }, 5000),
        qPrintable(QStringLiteral("the snapshot list is taller than two visible pages: %1 rows, %2 of %3")
            .arg(count).arg(list->height()).arg(flick->height())));
    M4_CHECK(7913, tab_into(w, list, 40), "Tab reaches the snapshot list");
    press_and_wait(w, Qt::Key_Home, 80);
    M4_CHECK(7914, list->property("currentIndex").toInt() == 0, "Home selects the first snapshot");

    for (int i = 1; i < count; ++i) {
        press_and_wait(w, Qt::Key_Down, 30);
        QQuickItem* row = focused_snapshot_row(w);
        M4_CHECK(7920, row != nullptr && row_in_view(row, flick),
            qPrintable(QStringLiteral("(b) after Down the current row is in view: ") + row_view_state(row, flick)));
    }
    M4_CHECK(7921, list->property("currentIndex").toInt() == count - 1, "(b) Down reaches the last snapshot");
    M4_CHECK(7922, save_screenshot(w, c.out_path), "screenshot failed");

    press_and_wait(w, Qt::Key_Home, 80);
    QQuickItem* first = focused_snapshot_row(w);
    M4_CHECK(7930, first != nullptr && first->property("index").toInt() == 0 && row_in_view(first, flick),
        qPrintable(QStringLiteral("(b) Home shows the first snapshot: ") + row_view_state(first, flick)));
    const int page = static_cast<int>(flick->height() / first->height());
    press_and_wait(w, Qt::Key_PageDown, 80);
    const int after_page_down = list->property("currentIndex").toInt();
    M4_CHECK(7931, after_page_down < count - 1,
        qPrintable(QStringLiteral("(b) Page Down from the first row does not jump to the last row: %1 of %2")
            .arg(after_page_down).arg(count)));
    M4_CHECK(7932, std::abs(after_page_down - page) <= 1,
        qPrintable(QStringLiteral("(b) Page Down moves by about one visible page (%1 rows): to row %2")
            .arg(page).arg(after_page_down)));
    QQuickItem* paged = focused_snapshot_row(w);
    M4_CHECK(7933, paged != nullptr && row_in_view(paged, flick),
        qPrintable(QStringLiteral("(b) after Page Down the current row is in view: ") + row_view_state(paged, flick)));
    press_and_wait(w, Qt::Key_PageUp, 80);
    QQuickItem* back = focused_snapshot_row(w);
    M4_CHECK(7934, list->property("currentIndex").toInt() == 0 && back != nullptr && row_in_view(back, flick),
        qPrintable(QStringLiteral("(b) Page Up goes back one page to the first row, in view: ") + row_view_state(back, flick)));
    return 0;
}

// Whether `item` lies wholly inside the window.
bool inside_window(QQuickWindow* w, QQuickItem* item) {
    const QRectF r = item->mapRectToScene(QRectF(0, 0, item->width(), item->height()));
    return r.top() >= -0.5 && r.left() >= -0.5 && r.bottom() <= w->height() + 0.5
        && r.right() <= w->width() + 0.5;
}

// Whether `item` derives from the QML type `type` (a QML type's class name starts with it).
bool is_of_qml_type(QQuickItem* item, const char* type) {
    for (const QMetaObject* mo = item->metaObject(); mo; mo = mo->superClass()) {
        if (QByteArray(mo->className()).startsWith(type)) {
            return true;
        }
    }
    return false;
}

// The search fields shown in the window: visible Kirigami.SearchField items inside the window,
// without the «Filter» fields of lists.
QList<QQuickItem*> shown_search_fields(QQuickWindow* w) {
    QList<QQuickItem*> items;
    collect_visual_descendants(w->contentItem(), items);
    QList<QQuickItem*> fields;
    for (QQuickItem* it : items) {
        if (it->isVisible() && it->width() > 0 && inside_window(w, it) && is_of_qml_type(it, "SearchField")
            && !is_of_qml_type(it, "FilterField")) {
            fields << it;
        }
    }
    return fields;
}

/// AC-M4-01-4 (b), search field of the snapshot list: with more snapshots than fit, after the
/// last row (a) Shift+Tab focuses the search field and it is in the visible area of the page,
/// (b) the field is not part of the page's scrolling content.
int scenario_search_field_fixed(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 8001, 30)) {
        return rc;
    }
    QQuickItem* page = visible_item(w, "restoreSnapshotsPage");
    QQuickItem* list = visible_item(w, "restoreSnapshotsListView");
    QQuickItem* field = visible_item(w, "snapshotSearchField");
    M4_CHECK(8010, page && list && field, "snapshot page, list or search field missing");
    auto* flick = page->property("flickable").value<QQuickItem*>();
    M4_CHECK(8011, flick != nullptr, "the snapshot page has no flickable");
    M4_CHECK(8012, wait_until([&] { return list->height() > flick->height() * 2; }, 5000),
        "the snapshots do not fit into the page");
    M4_CHECK(8013, tab_into(w, list, 40), "Tab reaches the snapshot list");
    press_and_wait(w, Qt::Key_End, 120);
    M4_CHECK(8014, list->property("currentIndex").toInt() == list->property("count").toInt() - 1,
        "End selects the last snapshot");
    M4_CHECK(8015, flick->property("contentY").toReal() > flick->height(),
        "the page is scrolled down to the last snapshot");
    press_backtab(w);
    M4_CHECK(8020, wait_until([&] { return field->hasActiveFocus(); }, 2000),
        qPrintable(QStringLiteral("(a) Shift+Tab from the list focuses the search field: ") + focus_name(w)));
    wait_ms(200);
    const QRectF r = field->mapRectToItem(page, QRectF(0, 0, field->width(), field->height()));
    M4_CHECK(8021, r.top() >= -0.5 && r.bottom() <= page->height() + 0.5 && inside_window(w, field)
            && shown_in_its_flickables(field),
        qPrintable(QStringLiteral("(a) the search field is in the visible area of the page: %1..%2 of %3")
            .arg(r.top()).arg(r.bottom()).arg(page->height())));
    M4_CHECK(8022, !lies_in(field, flick), "(b) the search field is not part of the scrolling content");
    M4_CHECK(8023, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

// (e) The header with the search field of the snapshot list and the header of the hits beside
// it are equally high and align.
int check_headers_aligned(QQuickWindow* w, int code) {
    auto* left = qobject_cast<QQuickItem*>(object_named(w, "snapshotSearchHeader"));
    auto* right = qobject_cast<QQuickItem*>(object_named(w, "searchResultsHeader"));
    M4_CHECK(code, left && right && left->isVisible() && right->isVisible(),
        "the headers of the snapshot list and of the hits are shown");
    const QRectF a = left->mapRectToScene(QRectF(0, 0, left->width(), left->height()));
    const QRectF b = right->mapRectToScene(QRectF(0, 0, right->width(), right->height()));
    M4_CHECK(code + 1, std::abs(a.height() - b.height()) < 0.5,
        qPrintable(QStringLiteral("(e) the headers of both columns are equally high: %1 and %2")
            .arg(a.height()).arg(b.height())));
    M4_CHECK(code + 2, std::abs(a.top() - b.top()) < 0.5,
        qPrintable(QStringLiteral("(e) the headers of both columns align: %1 and %2").arg(a.top()).arg(b.top())));
    return 0;
}

/// AC-M4-01-4 (b), results of a search: (c) after a search the window shows exactly one search
/// field, (d) while a new search runs no hit row shows under the loading placeholder, (e) the
/// header with the searched text and the number of hits is one line beside «Filter», as high as
/// the header with the search field beside it and aligned with it.
/// `KESTIC_SMOKE_HOLD_FIND` names the marker file that holds `restic find` while it exists.
int scenario_search_header(const Ctx& c) {
    QQuickWindow* w = c.window;
    const bool narrow = w->property("narrowMode").toBool();
    if (int rc = open_restore_snapshots(c, 8101, 3)) {
        return rc;
    }
    if (int rc = search_with_keyboard(c, QStringLiteral("bericht.pdf"), 8106)) {
        return rc;
    }
    wait_ms(600); // let the page transition end
    Search s = search_page(w);
    M4_CHECK(8110, s.model->rowCount() == 2, "two hits");
    if (!narrow) {
        const QList<QQuickItem*> fields = shown_search_fields(w);
        M4_CHECK(8111, fields.size() == 1,
            qPrintable(QStringLiteral("(c) the window shows exactly one search field, not %1").arg(fields.size())));
        M4_CHECK(8112, fields.first()->objectName() == QLatin1String("snapshotSearchField"),
            "(c) the one search field is that of the snapshot list");
    }
    QList<QQuickItem*> page_items;
    collect_visual_descendants(s.page, page_items);
    M4_CHECK(8113, page_items.contains(qobject_cast<QQuickItem*>(object_named(w, "searchFilterField"))),
        "the header of the search page is part of the page");
    for (QQuickItem* it : page_items) {
        M4_CHECK(8114, !(is_of_qml_type(it, "SearchField") && !is_of_qml_type(it, "FilterField")),
            "(c) the search page has no search field of its own");
    }

    auto* label = qobject_cast<QQuickItem*>(object_named(w, "searchCountLabel"));
    auto* filter = qobject_cast<QQuickItem*>(object_named(w, "searchFilterField"));
    M4_CHECK(8120, label && filter && label->isVisible() && filter->isVisible(),
        "the number of hits and «Filter» are shown");
    M4_CHECK_EQ(8121, label->property("text").toString(),
        loc(c.lang, "2 hits for ‹bericht.pdf› in 2 snapshots", "2 Treffer für ‹bericht.pdf› in 2 Snapshots"),
        "(e) the header names the searched text and the number of hits");
    M4_CHECK(8122, label->property("lineCount").toInt() == 1,
        qPrintable(QStringLiteral("(e) the header is one line, not %1").arg(label->property("lineCount").toInt())));
    const QRectF lr = label->mapRectToScene(QRectF(0, 0, label->width(), label->height()));
    const QRectF fr = filter->mapRectToScene(QRectF(0, 0, filter->width(), filter->height()));
    M4_CHECK(8123, lr.center().y() > fr.top() && lr.center().y() < fr.bottom() && lr.right() <= fr.left() + 0.5,
        "(e) the header line stands beside «Filter»");
    if (!narrow) {
        if (int rc = check_headers_aligned(w, 8125)) {
            return rc;
        }
    }
    M4_CHECK(8124, save_screenshot(w, c.out_path), "screenshot failed");

    if (narrow) {
        return 0;
    }

    // A new search from the search field of the snapshot list beside the hits, held while it
    // runs.
    QFile hold(qEnvironmentVariable("KESTIC_SMOKE_HOLD_FIND"));
    M4_CHECK(8130, hold.open(QIODevice::WriteOnly), "the hold marker could not be written");
    hold.write("1");
    hold.close();
    send_key(w, Qt::Key_F, Qt::ControlModifier);
    M4_CHECK(8131, wait_until([&] {
        QQuickItem* field = item_named(w, "snapshotSearchField");
        return field && field->hasActiveFocus();
    }, 2000), "Ctrl+F moves the focus to the search field of the snapshot list");
    send_key(w, Qt::Key_A, Qt::ControlModifier);
    type_text(w, QStringLiteral("bericht"));
    send_key(w, Qt::Key_Return);
    M4_CHECK(8132, wait_until([&] {
        Search running = search_page(w);
        return running.model && running.model->property("state").toString() == QLatin1String("running");
    }, 10000), "Return starts a new search");
    finish_incubation(w);
    wait_ms(600);
    QList<QQuickItem*> items;
    collect_visual_descendants(w->contentItem(), items);
    for (QQuickItem* it : items) {
        M4_CHECK(8140, !(it->objectName() == QLatin1String("searchHitRow") && it->isVisible()
                && inside_window(w, it)),
            "(d) no hit row shows while the search runs");
    }
    QQuickItem* busy = visible_item(w, "searchBusy");
    M4_CHECK(8141, busy != nullptr && inside_window(w, busy), "(d) the loading placeholder is shown");
    M4_CHECK(8142, visible_item(w, "searchCancelButton") != nullptr, "(d) «Cancel» is shown");
    auto* running_label = qobject_cast<QQuickItem*>(visible_item(w, "searchCountLabel"));
    M4_CHECK(8144, running_label != nullptr, "(e) the header stays while the search runs");
    M4_CHECK_EQ(8145, running_label->property("text").toString(),
        loc(c.lang, "Search for ‹bericht›", "Suche nach ‹bericht›"),
        "(e) while the search runs, the header names the searched text");
    M4_CHECK(8146, running_label->property("lineCount").toInt() == 1,
        "(e) while the search runs, the header is one line");
    if (int rc = check_headers_aligned(w, 8147)) {
        return rc;
    }
    M4_CHECK(8143, save_screenshot(w, screenshot_variant(c.out_path, "-running")), "screenshot failed");
    QFile::remove(hold.fileName());
    return 0;
}

/// AC-M4-01-4 (b), any page: a control scrolled out of view that gets the focus by Tab is
/// scrolled into the visible area. The backup detail page in a short window, scrolled to its
/// top; Tab walks down to its last row.
int scenario_focus_in_view(const Ctx& c) {
    QQuickWindow* w = c.window;
    w->resize(w->width(), 420);
    wait_ms(300);
    QObject* backups_tab = object_named(w, "backupsTabAction");
    M4_CHECK(8201, backups_tab != nullptr, "backupsTabAction missing");
    trigger(backups_tab);
    M4_CHECK(8202, wait_until([&] { return visible_item(w, "backupDetailPage") != nullptr; }, 5000),
        "«Backups» shows the detail of a backup");
    finish_incubation(w);
    wait_ms(600);
    QQuickItem* page = visible_item(w, "backupDetailPage");
    QQuickItem* row = visible_item(w, "viewFullConfigRow");
    M4_CHECK(8203, page && row, "detail page or its last row missing");
    auto* flick = page->property("flickable").value<QQuickItem*>();
    M4_CHECK(8204, flick != nullptr, "the detail page has no flickable");
    flick->setProperty("contentY", flick->property("originY").toReal() - flick->property("topMargin").toReal());
    wait_ms(200);
    M4_CHECK(8205, !shown_in_its_flickables(row), "the last row starts out of view");
    M4_CHECK(8206, tab_to(w, "viewFullConfigRow", 60) != nullptr,
        qPrintable(QStringLiteral("Tab reaches the last row: ") + focus_name(w)));
    wait_ms(200);
    M4_CHECK(8207, shown_in_its_flickables(row) && inside_window(w, row),
        "a control that gets the focus by Tab is scrolled into view");
    M4_CHECK(8208, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

} // namespace

void run_m4_smoke_scenario(
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
    if (scen == QLatin1String("m4-restore-tab")) {
        rc = scenario_restore_tab(ctx);
    } else if (scen == QLatin1String("m4-restore-keyboard")) {
        rc = scenario_restore_keyboard(ctx);
    } else if (scen == QLatin1String("m4-browse-keyboard")) {
        rc = scenario_browse_keyboard(ctx);
    } else if (scen == QLatin1String("m4-browse-path-row")) {
        rc = scenario_browse_path_row(ctx);
    } else if (scen == QLatin1String("m4-browse-paging")) {
        rc = scenario_browse_paging(ctx);
    } else if (scen == QLatin1String("m4-browse-leave")) {
        rc = scenario_browse_leave(ctx);
    } else if (scen == QLatin1String("m4-browse-too-large")) {
        rc = scenario_browse_too_large(ctx);
    } else if (scen == QLatin1String("m4-search")) {
        rc = scenario_search(ctx);
    } else if (scen == QLatin1String("m4-search-none")) {
        rc = scenario_search_none(ctx);
    } else if (scen == QLatin1String("m4-search-cancel")) {
        rc = scenario_search_cancel(ctx);
    } else if (scen == QLatin1String("m4-compare")) {
        rc = scenario_compare(ctx);
    } else if (scen == QLatin1String("m4-compare-identical")) {
        rc = scenario_compare_identical(ctx);
    } else if (scen == QLatin1String("m4-restore-dialog")) {
        rc = scenario_restore_dialog(ctx);
    } else if (scen == QLatin1String("m4-restore-chips")) {
        rc = scenario_restore_chips(ctx);
    } else if (scen == QLatin1String("m4-restore-unmatchable")) {
        rc = scenario_restore_unmatchable(ctx);
    } else if (scen == QLatin1String("m4-restore-target-texts")) {
        rc = scenario_restore_target_texts(ctx);
    } else if (scen == QLatin1String("m4-restore-mapping")) {
        rc = scenario_restore_mapping(ctx);
    } else if (scen == QLatin1String("m4-restic-recommended")) {
        rc = scenario_restic_recommended(ctx);
    } else if (scen == QLatin1String("m4-foreign")) {
        rc = scenario_foreign(ctx);
    } else if (scen == QLatin1String("m4-foreign-restore")) {
        rc = scenario_foreign_restore(ctx);
    } else if (scen == QLatin1String("m4-show-restore")) {
        rc = scenario_show_restore(ctx);
    } else if (scen == QLatin1String("m4-list-tab-stop")) {
        rc = scenario_list_tab_stop(ctx);
    } else if (scen == QLatin1String("m4-snapshot-right-click")) {
        rc = scenario_snapshot_right_click(ctx);
    } else if (scen == QLatin1String("m4-browse-select-all")) {
        rc = scenario_browse_select_all(ctx);
    } else if (scen == QLatin1String("m4-restore-all")) {
        rc = scenario_restore_all(ctx);
    } else if (scen == QLatin1String("m4-restore-all-from-search")) {
        rc = scenario_restore_all_from_search(ctx);
    } else if (scen == QLatin1String("m4-restore-all-from-folder-hit")) {
        rc = scenario_restore_all_from_folder_hit(ctx);
    } else if (scen == QLatin1String("m4-restore-dialog-tab")) {
        rc = scenario_restore_dialog_tab(ctx);
    } else if (scen == QLatin1String("m4-tab-order")) {
        rc = scenario_tab_order(ctx);
    } else if (scen == QLatin1String("m4-list-page-keys")) {
        rc = scenario_list_page_keys(ctx);
    } else if (scen == QLatin1String("m4-search-field-fixed")) {
        rc = scenario_search_field_fixed(ctx);
    } else if (scen == QLatin1String("m4-search-header")) {
        rc = scenario_search_header(ctx);
    } else if (scen == QLatin1String("m4-focus-in-view")) {
        rc = scenario_focus_in_view(ctx);
    } else if (scen == QLatin1String("m4-breakpoint")) {
        rc = scenario_breakpoint(ctx);
    } else if (scen == QLatin1String("m4-restore-banner")) {
        rc = scenario_restore_banner(ctx);
    } else if (scen == QLatin1String("m4-restore-network-folder")) {
        rc = scenario_restore_network_folder(ctx);
    } else {
        std::fprintf(stderr, "M4 smoke failed (599): unknown m4 smoke scenario %s\n", qPrintable(scen));
        QCoreApplication::exit(599);
        return;
    }
    if (rc != 0) {
        std::fprintf(stderr, "M4 smoke `%s` failed with code %d\n", qPrintable(scen), rc);
        QCoreApplication::exit(rc);
        return;
    }
    if (!warnings->isEmpty()) {
        for (const auto& w : *warnings) {
            std::fprintf(stderr, "M4 QML warning: %s\n", qPrintable(w));
        }
        QCoreApplication::exit(3);
        return;
    }
    QCoreApplication::exit(0);
}

} // namespace kestic::smoke
