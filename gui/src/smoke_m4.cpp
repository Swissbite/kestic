// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// M4 GUI smoke scenarios: the «Restore» area (backup list and snapshot list).
// Every scenario returns 0 on success and a unique non-zero code on the first failed
// assertion; one precise condition per assertion (no `||` fallbacks).

#include "src/smoke_m3_util.h"

#include <QFile>
#include <QImage>
#include <QLocale>
#include <QMetaMethod>
#include <QQmlComponent>
#include <QQmlEngine>
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
    if (w->property("narrowMode").toBool()) {
        // One column: the snapshot list opens from the backup row.
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
    M4_CHECK(5113, save_screenshot(w, c.out_path), "screenshot failed");
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

/// AC-M4-01-4: with the keyboard alone (a) every snapshot can be selected and (b) browsing,
/// comparing and restoring can be started.
int scenario_restore_keyboard(const Ctx& c) {
    QQuickWindow* w = c.window;
    QQuickItem* tab_button = tab_to_text(w, loc(c.lang, "Restore", "Wiederherstellen").toUtf8().constData(), 30);
    M4_CHECK(5201, tab_button != nullptr, "Tab did not reach the Restore tab");
    send_key(w, Qt::Key_Return);
    M4_CHECK(5202, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "Return on the Restore tab did not open the area");
    M4_CHECK(5203, wait_for_snapshots(w, c.backend, 3), "snapshots loaded");
    finish_incubation(w);

    QQuickItem* backup_row = tab_to(w, "restoreBackupListItem", 30);
    M4_CHECK(5204, backup_row != nullptr, "Tab did not reach the backup row");
    press_and_wait(w, Qt::Key_Down, 80);
    send_key(w, Qt::Key_Return);
    M4_CHECK(5205, wait_until([&] {
        QQuickItem* p = visible_item(w, "restoreSnapshotsPage");
        return p && p->property("title").toString() == QLatin1String("Photos")
            && !c.backend->property("restore_snapshots_loading").toBool();
    }, 20000), "Down and Return on the second backup list its snapshots");
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
    return 0;
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
    QMetaObject::invokeMethod(flick, "showEnd");
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
    QObject* added_chip = object_named(w, "compareKindChip_added");
    M4_CHECK(6025, added_chip != nullptr, "the chip of new files is shown");
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
    const QString name = QStringLiteral("bad") + QChar(0xFFFD) + QStringLiteral("name.txt");
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
    M4_CHECK(6318, error->property("text").toString().contains(QStringLiteral("«%1»").arg(folder)),
        "(a) the message names the folder to restore instead");
    M4_CHECK(6319, !restore_now->property("enabled").toBool(), "«Restore Now» stays disabled");
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

    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    M4_CHECK(6232, wait_until([&] { return r.model->property("overwrite").toInt() == 3; }, 2000), "«Always» chosen");
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
    M4_CHECK_EQ(6244, banner->property("text").toString(),
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
    QMetaObject::invokeMethod(r.dialog, "reject");
    return 0;
}

/// AC-M4-05-24 (c) in the GUI: started with `--show-restore` (the tray's «Restore Files…»
/// without a running GUI), the window opens on the tab «Restore».
int scenario_show_restore(const Ctx& c) {
    QQuickWindow* w = c.window;
    M4_CHECK(6401, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 5000),
        "(c) the tab «Restore» is active");
    // A running GUI on another tab follows the tray's request (signal ShowRestoreRequested).
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    M4_CHECK(6403, wait_until([&] { return w->property("activeTab").toInt() == 0; }, 3000), "the overview is shown");
    M4_CHECK(6404, QMetaObject::invokeMethod(c.backend, "restoreTabRequested"), "restoreTabRequested could not be emitted");
    M4_CHECK(6405, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "(c) a running GUI switches to the tab «Restore»");
    finish_incubation(w);
    wait_ms(400);
    M4_CHECK(6402, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// AC-M4-05-22 (b, c): the setting «restic source» shows «(outdated)» after the version of
/// restic 0.18.0 (`KESTIC_SMOKE_EXPECT_OUTDATED=1`) and no such note with the newest tested
/// version.
int scenario_restic_outdated(const Ctx& c) {
    QQuickWindow* w = c.window;
    M4_CHECK(6301, wait_until([&] { return c.backend->property("restic_valid").toBool(); }, 10000), "restic usable");
    QMetaObject::invokeMethod(w, "openSettingsWindow");
    auto* settings = qobject_cast<QQuickWindow*>(object_named(w, "settingsWindow"));
    M4_CHECK(6302, settings != nullptr, "settingsWindow missing");
    M4_CHECK(6303, wait_until([&] { return settings->isVisible(); }, 3000), "settings open");
    QObject* label = settings->findChild<QObject*>(QStringLiteral("resticSourceLabel"));
    M4_CHECK(6304, label != nullptr, "resticSourceLabel missing");
    const QString text = label->property("text").toString();
    const QString note = loc(c.lang, "(outdated)", "(veraltet)");
    for (const char* code : {"system", "app_managed", "custom"}) {
        M4_CHECK(6308, !text.contains(QLatin1String(code)), "no raw provider code in the restic source");
    }
    M4_CHECK(6309, text.startsWith(loc(c.lang, "Custom program path", "Eigener Programmpfad")),
        "the source is named like the choice in the source dialog");
    if (qEnvironmentVariable("KESTIC_SMOKE_EXPECT_OUTDATED") == QLatin1String("1")) {
        M4_CHECK(6305, text.contains(QStringLiteral("0.18.0 ") + note), "(b) «(outdated)» after 0.18.0");
    } else {
        M4_CHECK(6306, !text.contains(note), "(c) no «(outdated)» with the newest version");
    }
    // Scroll the setting into view for the screenshot.
    if (auto* item = qobject_cast<QQuickItem*>(label)) {
        for (QQuickItem* p = item->parentItem(); p; p = p->parentItem()) {
            if (p->inherits("QQuickFlickable")) {
                const QPointF at = item->mapToItem(p->property("contentItem").value<QQuickItem*>(), QPointF(0, 0));
                p->setProperty("contentY", qMax(0.0, at.y() - 120.0));
                break;
            }
        }
    }
    wait_ms(400);
    M4_CHECK(6307, save_screenshot(settings, c.out_path), "screenshot failed");
    settings->close();
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
    } else if (scen == QLatin1String("m4-restore-unmatchable")) {
        rc = scenario_restore_unmatchable(ctx);
    } else if (scen == QLatin1String("m4-restic-outdated")) {
        rc = scenario_restic_outdated(ctx);
    } else if (scen == QLatin1String("m4-show-restore")) {
        rc = scenario_show_restore(ctx);
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
