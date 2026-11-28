// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#pragma once

// Shared helpers of the M4 GUI smoke scenarios (the «Restore» area): `smoke_m4.cpp` and the
// `smoke_m4_<area>.cpp` files. Every scenario returns 0 on success and a unique non-zero code on
// the first failed check; one precise condition per check (no `||` fallbacks).

#include "src/smoke_m3_util.h"

#include <algorithm>
#include <cstdlib>

#include <QFile>
#include <QImage>
#include <QLocale>
#include <QMetaMethod>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQmlProperty>
#include <QSet>
#include <QUrl>

namespace kestic::smoke::m4 {

using m3::accessible_name;
using m3::Ctx;
using m3::loc;
using m3::lookup;
using m3::Scenario;
using m3::ScenarioFn;

// The visible item called `name` (nullptr when there is none or it is hidden).
inline QQuickItem* visible_item(QQuickWindow* w, const char* name) {
    for (QObject* o : m3::objects_named(w, name)) {
        auto* it = qobject_cast<QQuickItem*>(o);
        if (it && it->isVisible()) {
            return it;
        }
    }
    return nullptr;
}

inline int model_rows(QQuickWindow* w, const char* model) {
    QAbstractItemModel* m = model_named(w, model);
    return m ? m->rowCount() : -1;
}

// Waits until the restore area shows the snapshots of its first backup (`expected` rows).
inline bool wait_for_snapshots(QQuickWindow* w, QObject* backend, int expected) {
    return wait_until([&] {
        return !backend->property("restore_snapshots_loading").toBool()
            && model_rows(w, "restoreSnapshotsModel") == expected;
    }, 20000);
}

// The snapshot list shows the further group `title` with `rows` snapshots.
inline bool wait_for_group(QQuickWindow* w, QObject* backend, const QString& title, int rows) {
    return wait_until([&] {
        QQuickItem* p = visible_item(w, "restoreSnapshotsPage");
        return p && p->property("title").toString() == title
            && !backend->property("restore_overview_loading").toBool()
            && model_rows(w, "restoreSnapshotsModel") == rows;
    }, 20000);
}

// The restore dialog's model and the dialog itself.
struct Restore {
    QObject* dialog = nullptr;
    QObject* model = nullptr;
};

inline Restore restore_dialog(QQuickWindow* w) {
    return Restore { lookup<QObject>(w, "restoreDialog"), lookup<QObject>(w, "restorePreviewModel") };
}

// The browse page and its folder model.
struct Browse {
    QQuickItem* page = nullptr;
    QAbstractItemModel* model = nullptr;
};

inline Browse browse_page(QQuickWindow* w) {
    Browse b;
    b.page = visible_item(w, "restoreBrowsePage");
    b.model = b.page ? qobject_cast<QAbstractItemModel*>(
                           b.page->findChild<QObject*>(QStringLiteral("snapshotBrowseModel")))
                     : nullptr;
    return b;
}

// Waits until the browse page lists `dir` (state «done»).
inline bool wait_for_dir(QQuickWindow* w, const QString& dir) {
    return wait_until([&] {
        Browse b = browse_page(w);
        return b.model && b.model->property("state").toString() == QLatin1String("done")
            && b.model->property("dir").toString() == dir;
    }, 30000);
}

inline QStringList browse_names(QAbstractItemModel* m) {
    QStringList out;
    for (int r = 0; r < m->rowCount(); ++r) {
        out << role_value(m, r, "name").toString();
    }
    return out;
}

// The delegate of the folder list that holds the keyboard focus, or nullptr.
inline QQuickItem* focused_entry_row(QQuickWindow* w) {
    for (QQuickItem* it = w->activeFocusItem(); it; it = it->parentItem()) {
        if (it->objectName() == QLatin1String("browseEntryRow")) {
            return it;
        }
    }
    return nullptr;
}

// Opens the restore area and, in the narrow layout, the snapshot list of the first backup,
// which holds `snapshots` snapshots.
inline int open_restore_snapshots(const Ctx& c, int code, int snapshots = 2) {
    QQuickWindow* w = c.window;
    QObject* tab = lookup<QObject>(w, "restoreTabAction");
    CHECK(code, tab != nullptr, "restoreTabAction missing");
    trigger(tab);
    CHECK(code + 1, wait_until([&] { return w->property("activeTab").toInt() == 4; }, 3000),
        "Restore tab active");
    CHECK(code + 2, wait_until([&] { return !c.backend->property("restore_overview_loading").toBool(); }, 20000),
        "snapshot counts loaded");
    finish_incubation(w);
    if (w->property("narrowMode").toBool()) {
        QQuickItem* backup_row = tab_to(w, "restoreBackupListItem", 30);
        CHECK(code + 3, backup_row != nullptr, "Tab did not reach the backup row");
        send_key(w, Qt::Key_Return);
    }
    CHECK(code + 4, wait_for_snapshots(w, c.backend, snapshots), "the snapshots of «Home» listed");
    finish_incubation(w);
    return 0;
}

// Opens snapshot `row` of the snapshot list for browsing (not by keyboard).
inline int open_browser_at(const Ctx& c, int row, int code) {
    QQuickWindow* w = c.window;
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    CHECK(code, snaps != nullptr, "restoreSnapshotsModel missing");
    const QVariant dest = role_value(snaps, row, "destinationId");
    const QVariant snap = role_value(snaps, row, "snapshotId");
    CHECK(code + 1, QMetaObject::invokeMethod(w, "openSnapshotBrowser", Q_ARG(QVariant, dest), Q_ARG(QVariant, snap)),
        "openSnapshotBrowser failed");
    CHECK(code + 2, wait_until([&] { return browse_page(w).model != nullptr; }, 5000),
        "browse page shown");
    finish_incubation(w);
    return 0;
}

// The part of the window that `item` covers, as drawn right now.
inline QImage drawn_under(QQuickWindow* w, QQuickItem* item) {
    const QImage all = w->grabWindow().convertToFormat(QImage::Format_ARGB32);
    const qreal dpr = w->effectiveDevicePixelRatio();
    const QRectF r = item->mapRectToScene(QRectF(0.0, 0.0, item->width(), item->height()));
    return all.copy(QRectF(r.topLeft() * dpr, r.size() * dpr).toAlignedRect());
}

// The visible Text item below `root` that shows `text` (nullptr when there is none).
inline QQuickItem* text_item_showing(QQuickItem* root, const QString& text) {
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
inline int check_path_row_drawn_once(const Ctx& c, const QStringList& names) {
    QQuickWindow* w = c.window;
    QQuickItem* row = visible_item(w, "browsePathRow");
    CHECK(10700, row != nullptr, "browsePathRow missing");
    // The page transition has ended once two grabs in a row are the same.
    QImage last;
    CHECK(10701, wait_until([&] {
        const QImage now = drawn_under(w, row);
        const bool still = !last.isNull() && now == last;
        last = now;
        return still;
    }, 5000), "the path row stops moving");
    QObject* flick = lookup<QObject>(w, "browsePathFlick");
    CHECK(8323, flick != nullptr, "browsePathFlick missing");
    for (int i = 0; i < names.size(); ++i) {
        const QByteArray name = QStringLiteral("browsePathButton_%1").arg(i).toUtf8();
        QQuickItem* button = visible_item(w, name.constData());
        CHECK(10702, button != nullptr, "path row button missing");
        // A path longer than the row scrolls sideways: bring the folder into view first.
        QMetaObject::invokeMethod(flick, "ensureVisible",
            Q_ARG(QVariant, QVariant::fromValue(button->parentItem())));
        CHECK(8324, !button->property("hovered").toBool(), "button not hovered");
        CHECK(8325, !button->hasActiveFocus(), "button without focus");
        auto* content = button->property("contentItem").value<QQuickItem*>();
        CHECK(8326, content != nullptr, "button without content");

        const QImage drawn = drawn_under(w, button);
        button->setOpacity(0.0);
        const QImage without_button = drawn_under(w, button);
        button->setOpacity(1.0);
        content->setOpacity(0.0);
        const QImage without_content = drawn_under(w, button);
        content->setOpacity(1.0);
        CHECK(8327, drawn != without_button, "the button shows its folder");
        CHECK(8328, drawn != without_content, "hiding the content changes the picture");
        CHECK(8329, without_content == without_button,
            qPrintable(QStringLiteral("path row button %1 is drawn a second time beneath its content").arg(i)));

        if (i == 0) {
            CHECK_EQ(8330, accessible_name(button), loc(c.lang, "Top folder", "Oberster Ordner"),
                "accessible name of the top folder");
        } else {
            CHECK_EQ(8331, accessible_name(button), names.at(i), "accessible name of a folder");
            QQuickItem* label = text_item_showing(button, names.at(i));
            CHECK(8332, label != nullptr, "the folder name is shown");
            CHECK(8333, label->property("textFormat").toInt() == 0, "the folder name is plain text");
        }
    }
    return 0;
}

// Types `text` into the item with the keyboard focus, one key per character.
inline void type_text(QQuickWindow* w, const QString& text) {
    for (const QChar ch : text) {
        send_key(w, ch.toUpper().unicode(), Qt::NoModifier, QString(ch));
    }
}

// The search page and its model.
struct Search {
    QQuickItem* page = nullptr;
    QAbstractItemModel* model = nullptr;
};

inline Search search_page(QQuickWindow* w) {
    Search s;
    s.page = visible_item(w, "restoreSearchPage");
    s.model = s.page ? qobject_cast<QAbstractItemModel*>(
                           s.page->findChild<QObject*>(QStringLiteral("snapshotSearchModel")))
                     : nullptr;
    return s;
}

// Ctrl+F in the snapshot list, `text` and Return: waits until the search page lists its hits.
inline int search_with_keyboard(const Ctx& c, const QString& text, int code) {
    QQuickWindow* w = c.window;
    w->requestActivate();
    QCoreApplication::processEvents();
    send_key(w, Qt::Key_F, Qt::ControlModifier);
    CHECK(code, wait_until([&] {
        QQuickItem* field = lookup(w, "snapshotSearchField");
        return field && field->hasActiveFocus();
    }, 2000), "Ctrl+F moves the focus to the search field of the snapshot list");
    type_text(w, text);
    send_key(w, Qt::Key_Return);
    CHECK(code + 1, wait_until([&] {
        Search s = search_page(w);
        return s.model && s.model->property("state").toString() == QLatin1String("done");
    }, 30000), "Return searches the snapshots and shows the hits next to the list");
    finish_incubation(w);
    Search s = search_page(w);
    CHECK_EQ(code + 2, s.page->property("title").toString(), loc(c.lang, "Search", "Suche"),
        "the breadcrumb names the search");
    CHECK_EQ(code + 3, s.model->property("search_text").toString(), text, "searched text");
    return 0;
}

// Repaints the whole window. The offscreen software renderer repaints only changed regions, so
// items of a lower layer that changed under an open dialog are drawn over it in a screenshot;
// the desktop renders whole frames.
inline void repaint_fully(QQuickWindow* w) {
    const QSize size = w->size();
    w->resize(size.width() + 1, size.height());
    wait_ms(100); // quiet window: one frame at the other size
    w->resize(size);
    wait_ms(300); // quiet window: the frames at the old size repaint everything
}

// Scrolls the Flickables around `item` that have more content than they show, so that `item`
// shows near the top of each, for a screenshot. The content may still grow while its parts are
// laid out, so it scrolls again until the content height has settled.
inline void scroll_into_view(QQuickItem* item) {
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
            wait_ms(100); // quiet window: the content may grow while its parts are laid out
        }
    }
}

// Whether `item` lies wholly inside the visible area of every Flickable around it.
inline bool shown_in_its_flickables(QQuickItem* item) {
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
inline QString screenshot_variant(const QString& out_path, const char* suffix) {
    QString path = out_path;
    path.insert(path.length() - 4, QLatin1String(suffix));
    return path;
}

// Whether `item` is `ancestor` or lies below it.
inline bool lies_in(QQuickItem* item, QQuickItem* ancestor) {
    for (QQuickItem* it = item; it; it = it->parentItem()) {
        if (it == ancestor) {
            return true;
        }
    }
    return false;
}

// Presses Tab until the keyboard focus is in `list` (at most `max_steps` times).
inline bool tab_into(QQuickWindow* w, QQuickItem* list, int max_steps) {
    for (int i = 0; i < max_steps; ++i) {
        if (lies_in(w->activeFocusItem(), list)) {
            return true;
        }
        send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
    }
    return lies_in(w->activeFocusItem(), list);
}

inline QString focus_name(QQuickWindow* w) {
    return describe_focus_item(w->activeFocusItem());
}

inline void press_backtab(QQuickWindow* w) {
    send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
}

// Whether `item` lies wholly inside the window.
inline bool inside_window(QQuickWindow* w, QQuickItem* item) {
    const QRectF r = item->mapRectToScene(QRectF(0, 0, item->width(), item->height()));
    return r.top() >= -0.5 && r.left() >= -0.5 && r.bottom() <= w->height() + 0.5
        && r.right() <= w->width() + 0.5;
}

} // namespace kestic::smoke::m4

namespace kestic::smoke {

// The scenario tables of the M4 smoke files (nullptr for an unknown name).
m3::ScenarioFn find_m4_lists_scenario(const QString& scen);
m3::ScenarioFn find_m4_browse_scenario(const QString& scen);
m3::ScenarioFn find_m4_restore_scenario(const QString& scen);

} // namespace kestic::smoke
