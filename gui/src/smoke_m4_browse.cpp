// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// M4 GUI smoke scenarios of the «Restore» area: browsing a snapshot and searching it.

#include "src/smoke_m4_util.h"

namespace kestic::smoke {

namespace {

using namespace m4;

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
        press_until(w, at < target ? Qt::Key_Down : Qt::Key_Up, [&] {
            QQuickItem* now = focused_entry_row(w);
            return now && now->property("index").toInt() != at;
        }, 60);
    }
    return false;
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
    CHECK(5334, tree == nullptr,
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
        CHECK(5335, prop.isEmpty(),
            qPrintable(QStringLiteral("AC-M4-03-3 (b) no folder list entry can be expanded: ") + prop));
        const QString view = nested_view(it);
        CHECK(5338, view.isEmpty(),
            qPrintable(QStringLiteral("AC-M4-03-3 (b) no folder list entry holds a list of its own: ") + view));
    }
    CHECK(5336, rows > 0, "AC-M4-03-3 (b) folder list entries checked");
    for (int r = 0; r < b.model->rowCount(); ++r) {
        CHECK(5337, !b.model->hasChildren(b.model->index(r, 0)),
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
    CHECK(5306, top.endsWith(QLatin1String("/trees/src")), "newest snapshot opens in its folder");
    QQuickItem* first = tab_to(w, "restoreSnapshotListItem", 40);
    CHECK(5307, first != nullptr, "Tab did not reach the snapshot list");
    CHECK(5308, first->property("index").toInt() == 0, "newest snapshot first");
    send_key(w, Qt::Key_Return);
    CHECK(5309, wait_for_dir(w, top), "Return opens the snapshot in its folder");
    finish_incubation(w);
    Browse b = browse_page(w);
    CHECK_EQ(5310, b.page->property("title").toString(), QStringLiteral("2026-11-15 12:00"),
        "breadcrumb names the snapshot by date and time");
    CHECK_EQ(5311, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("a.txt,b.bin,docs,empty"),
        "AC-M4-03-1 (a) folders and files of the top folder");
    CHECK_EQ(5312, role_value(b.model, 0, "sizeText").toString(), QStringLiteral("5 B"),
        "AC-M4-03-1 (a) size of a file");
    CHECK_EQ(5313, role_value(b.model, 2, "sizeText").toString(), QStringLiteral("–"),
        "AC-M4-03-1 (a) folders have no size");
    CHECK(5314, role_value(b.model, 0, "modifiedText").toString().length() == 10,
        "AC-M4-03-1 (a) modification date shown");

    QQuickItem* row = tab_to(w, "browseEntryRow", 40);
    CHECK(5315, row != nullptr, "Tab did not reach the folder list");
    CHECK(5316, entry_to(w, b.model, QStringLiteral("docs")), "arrow keys reach «docs»");
    send_key(w, Qt::Key_Return);
    CHECK(5317, wait_for_dir(w, top + QStringLiteral("/docs")), "(a) Return opens «docs»");
    finish_incubation(w);
    CHECK_EQ(5318, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("deep,x.txt"),
        "(a) entries of «docs»");
    CHECK(5319, tab_to(w, "browseEntryRow", 40) != nullptr, "Tab reaches the list of «docs»");
    CHECK(5320, entry_to(w, b.model, QStringLiteral("deep")), "arrow keys reach «deep»");
    send_key(w, Qt::Key_Return);
    CHECK(5321, wait_for_dir(w, top + QStringLiteral("/docs/deep")), "(a) Return opens «deep»");
    finish_incubation(w);

    CHECK(5322, tab_to(w, "browseEntryRow", 40) != nullptr, "Tab reaches the list of «deep»");
    CHECK(5323, entry_to(w, b.model, QStringLiteral("y.txt")), "arrow keys reach «y.txt»");
    send_key(w, Qt::Key_Space, Qt::NoModifier, QStringLiteral(" "));
    CHECK(5324, wait_until([&] { return role_value(b.model, 0, "selected").toBool(); }, 2000),
        "(c) Space marks the entry");
    CHECK(5325, b.model->property("selection_count").toInt() == 1, "(c) one entry marked");
    QObject* restore_action = lookup<QObject>(w, "restoreSelectionAction");
    CHECK(5326, restore_action && restore_action->property("enabled").toBool(),
        "(c) «Restore Selection…» enabled with a selection");

    // (b) Back up through every folder above with the path row, down to the top of the snapshot.
    QStringList paths;
    QMetaObject::invokeMethod(b.model, "segmentPaths", Q_RETURN_ARG(QStringList, paths),
        Q_ARG(QString, top + QStringLiteral("/docs/deep")));
    CHECK(5327, paths.size() >= 4, "path row from the top down to «deep»");
    for (int i = paths.size() - 2; i >= 0; --i) {
        const QByteArray name = QStringLiteral("browsePathButton_%1").arg(i).toUtf8();
        QQuickItem* button = tab_to(w, name.constData(), 60);
        CHECK(5328, button != nullptr, "(b) Tab reaches the path row button of a folder above");
        send_key(w, Qt::Key_Return);
        CHECK(5329, wait_for_dir(w, paths.at(i)), "(b) the path row opens the folder above");
        finish_incubation(w);
        CHECK(5330, b.model->property("selection_count").toInt() == 1,
            "(c) the selection stays when the folder changes");
        if (paths.at(i) == top) {
            CHECK_EQ(5331, b.model->property("selection_text").toString(),
                loc(c.lang, "1 item selected (0 here, 1 in ‹deep›)", "1 Element ausgewählt (0 hier, 1 in ‹deep›)"),
                "(c) the selection line names where the marked entry is");
            CHECK(5332, save_screenshot(w, c.out_path), "screenshot failed");
        }
    }
    CHECK(5333, lookup(w, "browseListView") != nullptr, "folder list, no tree");
    return check_no_tree(b);
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
    CHECK(6209, wait_until([&] { return b.model->property("state").toString() == QLatin1String("done"); }, 30000),
        "the folder is listed");
    finish_incubation(w);
    const QString dir = b.model->property("dir").toString();
    const QString odd_name = QStringLiteral("R&D <b>x<b>");
    CHECK(6210, dir.endsWith(QLatin1Char('/') + odd_name + QStringLiteral("/src")),
        "the snapshot opens in its folder");
    QStringList names;
    QMetaObject::invokeMethod(b.model, "segmentNames", Q_RETURN_ARG(QStringList, names), Q_ARG(QString, dir));
    CHECK(6211, names.size() >= 4, "path row from the top down to «src»");
    CHECK(6226, names.contains(odd_name), "the path holds «R&D <b>x<b>»");
    if (int rc = check_path_row_drawn_once(c, names)) {
        return rc;
    }
    QMetaObject::invokeMethod(lookup<QObject>(w, "browsePathFlick"), "showEnd");
    return shot(c, 6225);
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
    CHECK(5409, b.model->property("state").toString() == QLatin1String("running"),
        "the folder is still loading (restic ls held)");
    QQuickItem* filter = tab_to(w, "browseFilterField", 40);
    CHECK(5410, filter != nullptr, "AC-M4-03-2 (c) Tab moves the focus while the folder loads");
    send_key(w, Qt::Key_X, Qt::NoModifier, QStringLiteral("x"));
    CHECK_EQ(5411, filter->property("text").toString(), QStringLiteral("x"),
        "AC-M4-03-2 (c) typing reaches the filter field while the folder loads");
    send_key(w, Qt::Key_Backspace);
    CHECK_EQ(5412, filter->property("text").toString(), QString(), "Backspace empties the filter field");
    CHECK(5413, b.model->property("state").toString() == QLatin1String("running"),
        "the input happened while the folder loaded");
    QFile marker(qEnvironmentVariable("KESTIC_SMOKE_TYPED_MARKER"));
    CHECK(5414, marker.open(QIODevice::WriteOnly), "typed marker could not be written");
    marker.close();

    const QString top = b.model->property("dir").toString();
    CHECK(5415, wait_for_dir(w, top), "the folder loads after restic ls is released");
    finish_incubation(w);
    CHECK(5416, b.model->property("total").toInt() == 1000, "(c) total of 1 000 known");
    CHECK(5417, b.model->rowCount() == 200, "(a) the first page holds 200 entries");
    QObject* count = lookup<QObject>(w, "browseCountLabel");
    CHECK(5418, count != nullptr, "browseCountLabel missing");
    CHECK_EQ(5419, count->property("text").toString(),
        QLocale().toString(1000) + loc(c.lang, " entries", " Einträge"),
        "(c) the view names 1 000 before scrolling");
    CHECK(5420, save_screenshot(w, c.out_path), "screenshot failed");

    QQuickItem* list = lookup(w, "browseListView");
    CHECK(5421, list != nullptr, "browseListView missing");
    int rows = b.model->rowCount();
    for (int step = 0; step < 20 && rows < 1000; ++step) {
        QMetaObject::invokeMethod(list, "positionViewAtEnd");
        wait_until([&] { return b.model->rowCount() > rows; }, 3000);
        const int now = b.model->rowCount();
        CHECK(5422, now - rows <= 200, "(a) at most 200 entries per page");
        CHECK(5423, now > rows, "(a) scrolling to the end fetches the next page");
        rows = now;
    }
    CHECK(5424, rows == 1000, "(b) every entry fetched");
    QSet<QString> paths;
    for (int r = 0; r < rows; ++r) {
        paths.insert(role_value(b.model, r, "path").toString());
    }
    CHECK(5425, paths.size() == 1000, "(b) each entry exactly once");
    CHECK(5426, !b.model->canFetchMore(QModelIndex()), "nothing beyond the last entry");
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
    CHECK(5509, wait_for_dir(w, top), "the folder query finishes");
    finish_incubation(w);
    CHECK(5510, b.model->property("too_large").toBool(), "the list is too large");
    CHECK(5511, b.model->rowCount() == 0, "(c) no partial list");
    QQuickItem* message = visible_item(w, "browseTooLargeMessage");
    CHECK(5512, message != nullptr, "(c) the too-large message is shown");
    CHECK_EQ(5513, message->property("text").toString(),
        loc(c.lang, "Too many entries to show (%1). Choose a subfolder or narrow the search.",
            "Zu viele Einträge für die Anzeige (%1). Einen Unterordner wählen oder enger suchen.")
            .arg(QLocale().toString(1000)),
        "(c) text names the number of entries");
    CHECK(5514, visible_item(w, "browseEmptyMessage") == nullptr, "not shown as an empty folder");
    return shot(c, 5515);
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
    CHECK(5609, b.model->property("state").toString() == QLatin1String("running"),
        "the folder is still loading (restic ls held)");
    wait_ms(1500); // quiet window: the held folder stays loading
    QQuickItem* back = visible_item(w, "breadcrumbBackButton");
    CHECK(5610, back != nullptr, "breadcrumbBackButton missing");
    CHECK(5611, click_ui_item(w, back, "breadcrumb back"), "back button could not be clicked");
    CHECK(5612, wait_until([&] { return visible_item(w, "restoreBrowsePage") == nullptr; }, 3000),
        "the browse page closes");
    return shot(c, 5614);
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
    CHECK(5710, s.model->rowCount() == 2, "(a) two hits");
    CHECK(5711, role_value(s.model, 0, "path").toString().endsWith(QLatin1String("/t2/src/Archiv/2026/bericht.pdf")),
        "(a) path of the hit in the newest snapshot first");
    CHECK(5712, role_value(s.model, 1, "path").toString().endsWith(QLatin1String("/t1/src/Dokumente/bericht.pdf")),
        "(a) path of the hit in the older snapshot");
    CHECK(5713, role_value(s.model, 0, "subtitle").toString().startsWith(QLatin1String("2026-11-14 12:00")),
        "(a) the hit names the date of its snapshot");
    CHECK(5714, role_value(s.model, 1, "subtitle").toString().startsWith(QLatin1String("2026-11-13 12:00")),
        "(a) the other hit names the date of its snapshot");
    QObject* count = lookup<QObject>(w, "searchCountLabel");
    CHECK(5715, count != nullptr, "searchCountLabel missing");
    CHECK_EQ(5716, count->property("text").toString(),
        loc(c.lang, "2 hits for ‹bericht.pdf› in 2 snapshots", "2 Treffer für ‹bericht.pdf› in 2 Snapshots"),
        "(a) number of hits and snapshots");
    wait_ms(600); // quiet window: the page transition ends before the screenshot
    CHECK(5717, save_screenshot(w, c.out_path), "screenshot failed");

    QQuickItem* row = tab_to(w, "searchHitRow", 40);
    CHECK(5718, row != nullptr, "Tab reaches the hits");
    CHECK(5719, row->property("index").toInt() == 0, "the first hit holds the focus");
    for (int i = 0; i < 4 && focused_hit_index(w) != 1; ++i) {
        const int at = focused_hit_index(w);
        press_until(w, Qt::Key_Down, [&] { return focused_hit_index(w) != at; }, 80);
    }
    CHECK(5727, focused_hit_index(w) == 1, "the arrow keys reach the second hit");
    const QString path = role_value(s.model, 1, "path").toString();
    const QString folder = role_value(s.model, 1, "folder").toString();
    CHECK_EQ(5720, folder + QStringLiteral("/bericht.pdf"), path, "the folder of the hit contains it");
    send_key(w, Qt::Key_Return);
    CHECK(5721, wait_for_dir(w, folder), "(b) Return opens the folder of the hit");
    finish_incubation(w);
    Browse b = browse_page(w);
    CHECK_EQ(5722, b.page->property("title").toString(), QStringLiteral("2026-11-13 12:00"),
        "(b) the breadcrumb names the snapshot of the hit");
    QQuickItem* list = lookup(w, "browseListView");
    CHECK(5723, list != nullptr, "browseListView missing");
    const int current = list->property("currentIndex").toInt();
    CHECK(5724, current >= 0, "(b) an entry of the folder is selected");
    CHECK_EQ(5725, role_value(b.model, current, "path").toString(), path, "(b) the hit is selected");
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
    CHECK(5810, s.model->rowCount() == 0, "(a) no hit listed");
    QQuickItem* message = visible_item(w, "searchNoHitsMessage");
    CHECK(5811, message != nullptr, "(a) the no-hits message is shown");
    CHECK_EQ(5812, message->property("text").toString(),
        loc(c.lang, "No hits for ‹nirgends›", "Keine Treffer für ‹nirgends›"), "(a) no-hits text");
    CHECK(5813, visible_item(w, "searchErrorMessage") == nullptr, "(b) no error shown");
    wait_ms(600); // quiet window: the page transition ends before the screenshot
    return shot(c, 5814);
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
    CHECK(5906, wait_until([&] { return visible_item(w, "searchCancelButton") != nullptr; }, 10000),
        "the running search shows «Cancel»");
    finish_incubation(w);
    wait_ms(1500); // quiet window: the held search keeps running
    QQuickItem* cancel = visible_item(w, "searchCancelButton");
    CHECK(5907, cancel != nullptr, "«Cancel» still shown while restic find is held");
    CHECK(5908, click_ui_item(w, cancel, "cancel search"), "«Cancel» could not be clicked");
    CHECK(5909, wait_until([&] { return visible_item(w, "searchCancelledMessage") != nullptr; }, 5000),
        "the page says the search was cancelled");
    Search s = search_page(w);
    CHECK_EQ(5910, s.model->property("state").toString(), QStringLiteral("cancelled"), "search cancelled");
    CHECK(5911, visible_item(w, "searchCancelButton") == nullptr, "«Cancel» gone");
    wait_ms(600); // quiet window: the page transition ends before the screenshot
    return shot(c, 5912);
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
    CHECK(7209, wait_for_dir(w, top), "the folder with 1 000 entries is listed");
    finish_incubation(w);
    QQuickItem* check = visible_item(w, "browseSelectAllCheck");
    CHECK(7210, check != nullptr, "the header check box is shown");
    CHECK(7211, b.model->rowCount() == 200, "200 of 1 000 entries loaded");
    CHECK(7212, header_state(check) == Qt::Unchecked, "(b) none selected: unchecked");
    CHECK_EQ(7213, accessible_name(check), loc(c.lang, "Select All", "Alles auswählen"),
        "the header check box is named «Select All»");

    CHECK(7214, click_ui_item(w, check, "select all"), "the header check box could not be clicked");
    CHECK(7215, wait_until([&] { return b.model->property("selection_count").toInt() == 1000; }, 10000),
        qPrintable(QStringLiteral("(a) all 1 000 entries selected, also those not loaded: %1")
            .arg(b.model->property("selection_count").toInt())));
    CHECK(7216, b.model->rowCount() == 200, "(a) selecting adds no rows to the view");
    CHECK(7217, role_value(b.model, 0, "selected").toBool() && role_value(b.model, 199, "selected").toBool(),
        "(a) the loaded rows show their marks");
    CHECK(7218, header_state(check) == Qt::Checked, "(b) all selected: checked");
    QMetaObject::invokeMethod(lookup<QObject>(w, "browseListView"), "positionViewAtEnd");
    CHECK(7219, wait_until([&] { return b.model->rowCount() > 200; }, 5000), "scrolling loads the next page");
    CHECK(7220, role_value(b.model, 200, "selected").toBool(), "(a) a row loaded later shows its mark");
    // The headings stay above the rows while the list scrolls: at row 180 the check box is at the
    // top of the list and can be clicked.
    QQuickItem* list = visible_item(w, "browseListView");
    QMetaObject::invokeMethod(list, "positionViewAtIndex", Q_ARG(int, 180), Q_ARG(int, 0));
    QPointF at;
    CHECK(7256, wait_until([&] {
        at = check->mapToItem(list, QPointF(0, 0));
        return check->isVisible() && at.y() >= 0 && at.y() < check->height();
    }, 200),
        qPrintable(QStringLiteral("the header check box stays at the top of the scrolled list: y=%1").arg(at.y())));
    CHECK(7221, save_screenshot(w, screenshot_variant(c.out_path, "-all")), "screenshot failed");
    // A click on a heading reaches no row scrolled beneath it.
    QQuickItem* name_heading = visible_item(w, "browseNameHeading");
    CHECK(7257, name_heading && click_ui_item(w, name_heading, "Name heading"), "the «Name» heading could not be clicked");
    wait_ms(200); // quiet window: a click that reached a row would open or mark it by now
    CHECK(7258, b.model->property("dir").toString() == top && b.model->property("selection_count").toInt() == 1000,
        "a click on the headings opens and marks nothing");

    CHECK(7222, click_ui_item(w, check, "clear all"), "the header check box could not be clicked while scrolled");
    CHECK(7223, wait_until([&] { return b.model->property("selection_count").toInt() == 0; }, 10000),
        "(c) clicked with all selected, it clears the folder");
    CHECK(7224, header_state(check) == Qt::Unchecked, "(b) unchecked again");
    QMetaObject::invokeMethod(list, "positionViewAtBeginning");
    wait_ms(200); // quiet window: the list settles at its beginning
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, 1));
    CHECK(7225, header_state(check) == Qt::PartiallyChecked, "(b) one selected: partially checked");
    repaint_fully(w);
    CHECK(7226, save_screenshot(w, c.out_path), "screenshot failed");
    CHECK(7227, click_ui_item(w, check, "select all from partial"), "the header check box could not be clicked");
    CHECK(7228, wait_until([&] { return b.model->property("selection_count").toInt() == 1000; }, 10000),
        "(b) clicked when partially checked, it selects all");
    QMetaObject::invokeMethod(b.model, "clearSelection");

    // (d) Ctrl+A in the list, and in the filter field.
    CHECK(7229, tab_into(w, visible_item(w, "browseListView"), 80), "Tab reaches the folder list");
    send_key(w, Qt::Key_A, Qt::ControlModifier);
    CHECK(7230, wait_until([&] { return b.model->property("selection_count").toInt() == 1000; }, 10000),
        "(d) Ctrl+A selects every entry");
    CHECK(7231, header_state(check) == Qt::Checked, "(d) the header follows Ctrl+A");
    QMetaObject::invokeMethod(b.model, "clearSelection");
    QQuickItem* filter = tab_to(w, "browseFilterField", 40);
    CHECK(7232, filter != nullptr, "Tab reaches the filter field");
    for (const QChar ch : QStringLiteral("bericht")) {
        send_key(w, Qt::Key_A + (ch.toUpper().unicode() - QLatin1Char('A').unicode()), Qt::NoModifier, QString(ch));
    }
    CHECK(7233, wait_until([&] {
        return b.model->property("matched").toInt() == 37 && b.model->property("state").toString() == QLatin1String("done");
    }, 10000), "the filter leaves the 37 matching entries");
    send_key(w, Qt::Key_A, Qt::ControlModifier);
    CHECK(7235, wait_until([&] { return filter->property("selectedText").toString() == QLatin1String("bericht"); }, 200),
        "(d) the filter text is selected");
    CHECK(7234, b.model->property("selection_count").toInt() == 0, "(d) Ctrl+A in the filter field selects its text, no entry");

    // (e) Select All with the filter.
    CHECK(7236, click_ui_item(w, check, "select matching"), "the header check box could not be clicked");
    CHECK(7237, wait_until([&] { return b.model->property("selection_count").toInt() == 37; }, 10000),
        qPrintable(QStringLiteral("(e) the 37 matching entries selected: %1").arg(b.model->property("selection_count").toInt())));
    CHECK(7238, header_state(check) == Qt::Checked, "(e) all matching selected: checked");
    QMetaObject::invokeMethod(b.model, "clearSelection");
    // (e) More matches than one page: the names with a «1» (271 of 1 000).
    QMetaObject::invokeMethod(b.model, "setFilter", Q_ARG(QString, QStringLiteral("1")));
    CHECK(7239, wait_until([&] {
        return b.model->property("matched").toInt() == 271 && b.model->property("state").toString() == QLatin1String("done");
    }, 10000), qPrintable(QStringLiteral("the filter «1» leaves 271 entries: %1").arg(b.model->property("matched").toInt())));
    CHECK(7240, b.model->rowCount() == 200, "one page of them is loaded");
    CHECK(7241, click_ui_item(w, check, "select 271"), "the header check box could not be clicked");
    CHECK(7242, wait_until([&] { return b.model->property("selection_count").toInt() == 271; }, 10000),
        "(e) all 271 matching entries selected, also those not loaded");
    CHECK(7243, header_state(check) == Qt::Checked, "(b, e) all matching selected: checked");
    CHECK(7244, click_ui_item(w, check, "clear 271"), "the header check box could not be clicked");
    CHECK(7245, wait_until([&] { return b.model->property("selection_count").toInt() == 0; }, 10000),
        "(c, e) clicked with all matching selected, it clears them");
    return 0;
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
    CHECK(8010, page && list && field, "snapshot page, list or search field missing");
    auto* flick = page->property("flickable").value<QQuickItem*>();
    CHECK(8011, flick != nullptr, "the snapshot page has no flickable");
    CHECK(8012, wait_until([&] { return list->height() > flick->height() * 2; }, 5000),
        "the snapshots do not fit into the page");
    CHECK(8013, tab_into(w, list, 40), "Tab reaches the snapshot list");
    CHECK(8014, press_until(w, Qt::Key_End, [&] {
        return list->property("currentIndex").toInt() == list->property("count").toInt() - 1;
    }, 120), "End selects the last snapshot");
    CHECK(8015, wait_until([&] { return flick->property("contentY").toReal() > flick->height(); }, 120),
        "the page is scrolled down to the last snapshot");
    press_backtab(w);
    CHECK(8020, wait_until([&] { return field->hasActiveFocus(); }, 2000),
        qPrintable(QStringLiteral("(a) Shift+Tab from the list focuses the search field: ") + focus_name(w)));
    QRectF r;
    CHECK(8021, wait_until([&] {
        r = field->mapRectToItem(page, QRectF(0, 0, field->width(), field->height()));
        return r.top() >= -0.5 && r.bottom() <= page->height() + 0.5 && inside_window(w, field)
            && shown_in_its_flickables(field);
    }, 200),
        qPrintable(QStringLiteral("(a) the search field is in the visible area of the page: %1..%2 of %3")
            .arg(r.top()).arg(r.bottom()).arg(page->height())));
    CHECK(8022, !lies_in(field, flick), "(b) the search field is not part of the scrolling content");
    return shot(c, 8023);
}

// (e) The header with the search field of the snapshot list and the header of the hits beside
// it are equally high and align.
int check_headers_aligned(QQuickWindow* w, int code) {
    auto* left = qobject_cast<QQuickItem*>(lookup<QObject>(w, "snapshotSearchHeader"));
    auto* right = qobject_cast<QQuickItem*>(lookup<QObject>(w, "searchResultsHeader"));
    CHECK(code, left && right && left->isVisible() && right->isVisible(),
        "the headers of the snapshot list and of the hits are shown");
    const QRectF a = left->mapRectToScene(QRectF(0, 0, left->width(), left->height()));
    const QRectF b = right->mapRectToScene(QRectF(0, 0, right->width(), right->height()));
    CHECK(code + 1, std::abs(a.height() - b.height()) < 0.5,
        qPrintable(QStringLiteral("(e) the headers of both columns are equally high: %1 and %2")
            .arg(a.height()).arg(b.height())));
    CHECK(code + 2, std::abs(a.top() - b.top()) < 0.5,
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
    wait_ms(600); // quiet window: the page transition ends before the layout is measured
    Search s = search_page(w);
    CHECK(8110, s.model->rowCount() == 2, "two hits");
    if (!narrow) {
        const QList<QQuickItem*> fields = shown_search_fields(w);
        CHECK(8111, fields.size() == 1,
            qPrintable(QStringLiteral("(c) the window shows exactly one search field, not %1").arg(fields.size())));
        CHECK(8112, fields.first()->objectName() == QLatin1String("snapshotSearchField"),
            "(c) the one search field is that of the snapshot list");
    }
    QList<QQuickItem*> page_items;
    collect_visual_descendants(s.page, page_items);
    CHECK(8113, page_items.contains(qobject_cast<QQuickItem*>(lookup<QObject>(w, "searchFilterField"))),
        "the header of the search page is part of the page");
    for (QQuickItem* it : page_items) {
        CHECK(8114, !(is_of_qml_type(it, "SearchField") && !is_of_qml_type(it, "FilterField")),
            "(c) the search page has no search field of its own");
    }

    auto* label = qobject_cast<QQuickItem*>(lookup<QObject>(w, "searchCountLabel"));
    auto* filter = qobject_cast<QQuickItem*>(lookup<QObject>(w, "searchFilterField"));
    CHECK(8120, label && filter && label->isVisible() && filter->isVisible(),
        "the number of hits and «Filter» are shown");
    CHECK_EQ(8121, label->property("text").toString(),
        loc(c.lang, "2 hits for ‹bericht.pdf› in 2 snapshots", "2 Treffer für ‹bericht.pdf› in 2 Snapshots"),
        "(e) the header names the searched text and the number of hits");
    CHECK(8122, label->property("lineCount").toInt() == 1,
        qPrintable(QStringLiteral("(e) the header is one line, not %1").arg(label->property("lineCount").toInt())));
    const QRectF lr = label->mapRectToScene(QRectF(0, 0, label->width(), label->height()));
    const QRectF fr = filter->mapRectToScene(QRectF(0, 0, filter->width(), filter->height()));
    CHECK(8123, lr.center().y() > fr.top() && lr.center().y() < fr.bottom() && lr.right() <= fr.left() + 0.5,
        "(e) the header line stands beside «Filter»");
    if (!narrow) {
        if (int rc = check_headers_aligned(w, 8125)) {
            return rc;
        }
    }
    CHECK(8124, save_screenshot(w, c.out_path), "screenshot failed");

    if (narrow) {
        return 0;
    }

    // A new search from the search field of the snapshot list beside the hits, held while it
    // runs.
    QFile hold(qEnvironmentVariable("KESTIC_SMOKE_HOLD_FIND"));
    CHECK(8130, hold.open(QIODevice::WriteOnly), "the hold marker could not be written");
    hold.write("1");
    hold.close();
    send_key(w, Qt::Key_F, Qt::ControlModifier);
    CHECK(8131, wait_until([&] {
        QQuickItem* field = lookup(w, "snapshotSearchField");
        return field && field->hasActiveFocus();
    }, 2000), "Ctrl+F moves the focus to the search field of the snapshot list");
    send_key(w, Qt::Key_A, Qt::ControlModifier);
    type_text(w, QStringLiteral("bericht"));
    send_key(w, Qt::Key_Return);
    CHECK(8132, wait_until([&] {
        Search running = search_page(w);
        return running.model && running.model->property("state").toString() == QLatin1String("running");
    }, 10000), "Return starts a new search");
    finish_incubation(w);
    wait_ms(600); // quiet window: hits of the old search would still show by now
    QList<QQuickItem*> items;
    collect_visual_descendants(w->contentItem(), items);
    for (QQuickItem* it : items) {
        CHECK(8140, !(it->objectName() == QLatin1String("searchHitRow") && it->isVisible()
                && inside_window(w, it)),
            "(d) no hit row shows while the search runs");
    }
    QQuickItem* busy = visible_item(w, "searchBusy");
    CHECK(8141, busy != nullptr && inside_window(w, busy), "(d) the loading placeholder is shown");
    CHECK(8142, visible_item(w, "searchCancelButton") != nullptr, "(d) «Cancel» is shown");
    auto* running_label = qobject_cast<QQuickItem*>(visible_item(w, "searchCountLabel"));
    CHECK(8144, running_label != nullptr, "(e) the header stays while the search runs");
    CHECK_EQ(8145, running_label->property("text").toString(),
        loc(c.lang, "Search for ‹bericht›", "Suche nach ‹bericht›"),
        "(e) while the search runs, the header names the searched text");
    CHECK(8146, running_label->property("lineCount").toInt() == 1,
        "(e) while the search runs, the header is one line");
    if (int rc = check_headers_aligned(w, 8147)) {
        return rc;
    }
    CHECK(8143, save_screenshot(w, screenshot_variant(c.out_path, "-running")), "screenshot failed");
    QFile::remove(hold.fileName());
    return 0;
}

const Scenario kScenarios[] = {
    {"m4-browse-keyboard", scenario_browse_keyboard},
    {"m4-browse-path-row", scenario_browse_path_row},
    {"m4-browse-paging", scenario_browse_paging},
    {"m4-browse-leave", scenario_browse_leave},
    {"m4-browse-too-large", scenario_browse_too_large},
    {"m4-search", scenario_search},
    {"m4-search-none", scenario_search_none},
    {"m4-search-cancel", scenario_search_cancel},
    {"m4-browse-select-all", scenario_browse_select_all},
    {"m4-search-field-fixed", scenario_search_field_fixed},
    {"m4-search-header", scenario_search_header},
};

} // namespace

m3::ScenarioFn find_m4_browse_scenario(const QString& scen) {
    return find_scenario(kScenarios, scen);
}

} // namespace kestic::smoke
