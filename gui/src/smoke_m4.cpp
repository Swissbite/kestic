// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// M4 GUI smoke scenarios: the «Restore» area (backup list and snapshot list).
// Every scenario returns 0 on success and a unique non-zero code on the first failed
// assertion; one precise condition per assertion (no `||` fallbacks).

#include "src/smoke_m3_util.h"

#include <QMetaMethod>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QUrl>

namespace kestic::smoke {

namespace {

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
    M4_CHECK(5106, model_rows(w, "restoreBackupsModel") == 1, "(b) one backup listed");
    QAbstractItemModel* backups = model_named(w, "restoreBackupsModel");
    M4_CHECK_EQ(5107, role_value(backups, 0, "subtitle").toString(),
        loc(c.lang, "3 snapshots", "3 Snapshots"), "(b) number of snapshots of the backup");
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
    send_key(w, Qt::Key_Return);
    M4_CHECK(5205, wait_for_snapshots(w, c.backend, 3), "Return on the backup row lists its snapshots");
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

    press_and_wait(w, Qt::Key_Up, 80);
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
    M4_CHECK(5223, wait_until([&] { return focused_snapshot_row(w) != nullptr; }, 2000),
        "focus returns to the snapshot list");
    M4_CHECK(5224, save_screenshot(w, c.out_path), "screenshot failed");
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
