// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#pragma once

#include <QAccessible>

#include "src/smoke_common.h"

#include <QByteArray>
#include <QDir>
#include <QFile>
#include <QMetaMethod>
#include <QSet>
#include <functional>
#include <memory>

// Shared helpers for the M3 GUI smoke scenarios (`smoke_m3.cpp`, `smoke_m3_editors.cpp`).
// Every scenario returns 0 on success and a unique non-zero code on the first failed
// assertion; one precise condition per assertion (no `||` fallbacks).

namespace kestic::smoke::m3 {

struct Ctx {
    QQuickWindow* window;
    QObject* backend;
    QString out_path;
    QString lang;
};

#define M3_CHECK(code, cond, msg)                                                      \
    do {                                                                               \
        if (!(cond)) {                                                                 \
            std::fprintf(stderr, "M3 check failed (%d): %s [%s]\n", (code), (msg), #cond); \
            return (code);                                                             \
        }                                                                              \
    } while (0)

// N independent checks that share one message; each condition is checked (and named) on its own.
#define M3_CHECK2(code, c1, c2, msg) \
    M3_CHECK(code, c1, msg);         \
    M3_CHECK(code, c2, msg)
#define M3_CHECK3(code, c1, c2, c3, msg) \
    M3_CHECK2(code, c1, c2, msg);        \
    M3_CHECK(code, c3, msg)
#define M3_CHECK4(code, c1, c2, c3, c4, msg) \
    M3_CHECK3(code, c1, c2, c3, msg);        \
    M3_CHECK(code, c4, msg)

#define M3_CHECK_EQ(code, actual, expected, what)                                      \
    do {                                                                               \
        const QString a_ = (actual);                                                   \
        const QString e_ = (expected);                                                 \
        if (a_ != e_) {                                                                \
            std::fprintf(                                                              \
                stderr,                                                                \
                "M3 check failed (%d): %s: expected '%s' got '%s'\n",                  \
                (code),                                                                \
                (what),                                                                \
                qPrintable(e_),                                                        \
                qPrintable(a_)                                                         \
            );                                                                         \
            return (code);                                                             \
        }                                                                              \
    } while (0)

#define M3_CHECK_STARTS(code, actual, prefix, what)                                    \
    do {                                                                               \
        const QString a_ = (actual);                                                   \
        const QString e_ = (prefix);                                                   \
        if (!a_.startsWith(e_)) {                                                      \
            std::fprintf(                                                              \
                stderr,                                                                \
                "M3 check failed (%d): %s: expected prefix '%s' got '%s'\n",           \
                (code),                                                                \
                (what),                                                                \
                qPrintable(e_),                                                        \
                qPrintable(a_)                                                         \
            );                                                                         \
            return (code);                                                             \
        }                                                                              \
    } while (0)

/// English or German text depending on the smoke language.
inline QString loc(const QString& lang, const char* en, const char* de) {
    return QString::fromUtf8(lang == QLatin1String("de") ? de : en);
}

inline int row_by_name(QAbstractItemModel* model, const QString& name) {
    for (int row = 0; row < model->rowCount(); ++row) {
        if (role_value(model, row, "name").toString() == name) {
            return row;
        }
    }
    return -1;
}

/// The accessible name a screen reader gets for `obj` (Qt's accessibility interface, as exported
/// over AT-SPI), falling back to an explicitly attached `Accessible.name`.
inline QString accessible_name(QObject* obj) {
    if (!obj) {
        return QString();
    }
    // Offscreen there is no assistive technology connected: enable Qt's accessibility layer so
    // the interface reports the name AT-SPI would export.
    if (!QAccessible::isActive()) {
        QAccessible::setActive(true);
    }
    if (QAccessibleInterface* iface = QAccessible::queryAccessibleInterface(obj)) {
        const QString name = iface->text(QAccessible::Name);
        if (!name.trimmed().isEmpty()) {
            return name;
        }
    }
    for (QObject* ch : obj->children()) {
        if (ch && QString::fromLatin1(ch->metaObject()->className()).contains(QLatin1String("Accessible"))) {
            const QString candidate = ch->property("name").toString();
            if (!candidate.trimmed().isEmpty()) {
                return candidate;
            }
        }
    }
    return QString();
}

/// Every object called `name` below `window`: the QObject tree plus the visual item tree
/// (Repeater delegates and menus are only reachable through one of them).
inline QList<QObject*> objects_named(QQuickWindow* window, const char* name) {
    const QString n = QString::fromLatin1(name);
    QSet<QObject*> seen;
    QList<QObject*> out;
    auto add = [&](QObject* o) {
        if (!seen.contains(o)) {
            seen.insert(o);
            out.append(o);
        }
    };
    for (QObject* o : window->findChildren<QObject*>(n)) {
        add(o);
    }
    QList<QQuickItem*> visual;
    collect_visual_descendants(window->contentItem(), visual);
    for (QQuickItem* it : visual) {
        if (it->objectName() == n) {
            add(it);
        }
        for (QObject* o : it->findChildren<QObject*>(n)) {
            add(o);
        }
    }
    return out;
}

inline QObject* object_named(QQuickWindow* window, const char* name) {
    const QList<QObject*> all = objects_named(window, name);
    return all.isEmpty() ? nullptr : all.first();
}

inline QQuickItem* item_named(QQuickWindow* window, const char* name) {
    for (QObject* o : objects_named(window, name)) {
        if (auto* it = qobject_cast<QQuickItem*>(o)) {
            return it;
        }
    }
    return nullptr;
}

inline int count_visible(QQuickWindow* window, const char* name) {
    int n = 0;
    for (QObject* o : objects_named(window, name)) {
        auto* it = qobject_cast<QQuickItem*>(o);
        if (it && it->isVisible()) {
            ++n;
        }
    }
    return n;
}

/// Text shown by a (Qt Quick Controls) SpinBox, e.g. «00».
inline QString spin_text(QObject* spin) {
    QObject* content = qvariant_cast<QObject*>(spin->property("contentItem"));
    return content ? content->property("text").toString() : QString();
}

inline int border_width(QObject* rect) {
    QObject* border = qvariant_cast<QObject*>(rect->property("border"));
    return border ? border->property("width").toInt() : -1;
}

inline QString combo_text(QObject* combo, int index) {
    QString text;
    QMetaObject::invokeMethod(
        combo,
        "textAt",
        Qt::DirectConnection,
        Q_RETURN_ARG(QString, text),
        Q_ARG(int, index)
    );
    return text;
}

/// Opens the detail page of backup `bid` and gives it time to settle.
[[nodiscard]] inline bool open_backup_detail(QQuickWindow* window, const QString& bid) {
    QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
    const bool opened = wait_until([&] { return item_named(window, "backupDetailPage") != nullptr; }, 3000);
    wait_ms(250);
    return opened;
}

inline QString config_path() {
    return qEnvironmentVariable("XDG_CONFIG_HOME") + QStringLiteral("/kestic/config.toml");
}

inline QByteArray read_config_bytes() {
    QFile f(config_path());
    return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
}

/// Presses Tab until every object name in `wanted` held keyboard focus (focus item or one
/// of its ancestors). Returns the names that were never focused.
inline QStringList tab_missing(QQuickWindow* window, const QStringList& wanted, int max_steps) {
    QStringList found;
    auto note = [&]() {
        for (QQuickItem* it = window->activeFocusItem(); it; it = it->parentItem()) {
            if (wanted.contains(it->objectName())) {
                if (!found.contains(it->objectName())) {
                    found << it->objectName();
                }
                return;
            }
        }
    };
    note();
    for (int i = 0; i < max_steps && found.size() < wanted.size(); ++i) {
        send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
        note();
    }
    QStringList missing;
    for (const QString& w : wanted) {
        if (!found.contains(w)) {
            missing << w;
        }
    }
    return missing;
}

/// First control whose accessible name is empty (empty list when all are named).
inline QString first_unnamed(QQuickWindow* window, const QStringList& names) {
    for (const QString& n : names) {
        QQuickItem* it = item_named(window, qPrintable(n));
        if (!it) {
            return n + QStringLiteral(" (missing)");
        }
        if (accessible_name(it).trimmed().isEmpty()) {
            return n;
        }
    }
    return QString();
}

} // namespace kestic::smoke::m3
