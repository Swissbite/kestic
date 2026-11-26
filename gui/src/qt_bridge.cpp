// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Qt helpers the product needs from Rust: clipboard, KI18n substitution, local UTC offset and
// the global keyboard handling of the main and settings windows.

#include "src/qt_bridge.h"

#include <KLocalizedString>
#include <QClipboard>
#include <QCoreApplication>
#include <QDateTime>
#include <QGuiApplication>
#include <QKeyEvent>
#include <QLocale>
#include <QMetaObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QVariant>

namespace kestic::bridge {

namespace {

// The action bar of a page in a Kirigami column view: the `globalFooter` of the page's
// `ColumnView` attached object.
QQuickItem* column_footer(QQuickItem* column) {
    for (QObject* child : column->children()) {
        const QVariant footer = child->property("globalFooter");
        if (footer.isValid()) {
            return footer.value<QQuickItem*>();
        }
    }
    return nullptr;
}

// A column view keeps the action bars of its pages in an item it creates before the pages, which
// puts them before the pages in the Tab chain although they are shown below them. Stacking that
// item after the last page makes the chain follow the visible order: header, pages, action bars.
void stack_footers_after_columns(QQuickItem* view) {
    auto* content = view->property("contentItem").value<QQuickItem*>();
    if (!content) {
        return;
    }
    const QList<QQuickItem*> children = content->childItems();
    QQuickItem* last_column = nullptr;
    for (QQuickItem* child : children) {
        bool contains = false;
        QMetaObject::invokeMethod(view, "containsItem", Qt::DirectConnection,
            Q_RETURN_ARG(bool, contains), Q_ARG(QQuickItem*, child));
        if (contains) {
            last_column = child;
        }
    }
    if (!last_column) {
        return;
    }
    for (QQuickItem* child : children) {
        QQuickItem* footer = column_footer(child);
        QQuickItem* holder = footer ? footer->parentItem() : nullptr;
        if (holder && holder != child && holder->parentItem() == content
            && children.indexOf(holder) < children.indexOf(last_column)) {
            holder->stackAfter(last_column);
            return;
        }
    }
}

// A page adds its header after its content, so the Tab chain ran through the content before
// the header above it. Stacking the header before the content makes it follow the visible order.
void stack_header_before_content(QQuickItem* page) {
    auto* header = page->property("header").value<QQuickItem*>();
    auto* content = page->property("contentItem").value<QQuickItem*>();
    if (!header || !content || header->parentItem() != page || content->parentItem() != page) {
        return;
    }
    const QList<QQuickItem*> children = page->childItems();
    if (children.indexOf(header) > children.indexOf(content)) {
        header->stackBefore(content);
    }
}

// Stacks the parts of the pages below `item` so that the Tab chain follows the visible order.
void stack_pages_in_visible_order(QQuickItem* item) {
    if (item->inherits("ColumnView")) {
        stack_footers_after_columns(item);
    }
    if (item->inherits("QQuickPage")) {
        stack_header_before_content(item);
        return;
    }
    const QList<QQuickItem*> children = item->childItems();
    for (QQuickItem* child : children) {
        stack_pages_in_visible_order(child);
    }
}

} // namespace

class KesticKeyNavFilter : public QObject {
public:
    explicit KesticKeyNavFilter(QObject* parent = nullptr) : QObject(parent) {}

    bool eventFilter(QObject* watched, QEvent* event) override {
        const QEvent::Type et = event->type();
        if (et != QEvent::ShortcutOverride && et != QEvent::KeyPress) {
            return QObject::eventFilter(watched, event);
        }

        auto* ke = static_cast<QKeyEvent*>(event);
        if (et == QEvent::KeyPress && (ke->key() == Qt::Key_Tab || ke->key() == Qt::Key_Backtab)) {
            if (auto* win = qobject_cast<QQuickWindow*>(watched)) {
                stack_pages_in_visible_order(win->contentItem());
            }
        }
        const Qt::KeyboardModifiers mods = ke->modifiers() & ~Qt::KeypadModifier;
        if (mods != Qt::NoModifier) {
            return QObject::eventFilter(watched, event);
        }

        const int key = ke->key();
        if (key == Qt::Key_Tab || key == Qt::Key_Backtab) {
            if (et == QEvent::KeyPress) {
                if (auto* win = qobject_cast<QQuickWindow*>(watched)) {
                    if (win->objectName() == QLatin1String("kesticMainWindow")) {
                        QMetaObject::invokeMethod(win, "cancelFocusRestoreTimer", Qt::DirectConnection);
                    }
                }
            }
            return QObject::eventFilter(watched, event);
        }
        if (key != Qt::Key_Escape && key != Qt::Key_Return && key != Qt::Key_Enter) {
            return QObject::eventFilter(watched, event);
        }

        QQuickWindow* win = qobject_cast<QQuickWindow*>(watched);
        QQuickWindow* target_win = win;
        if (!target_win) {
            if (auto* item = qobject_cast<QQuickItem*>(watched)) {
                target_win = item->window();
            }
        }
        if (!target_win) {
            return QObject::eventFilter(watched, event);
        }
        const QString obj_name = target_win->objectName();
        if (obj_name != QLatin1String("kesticMainWindow")
            && obj_name != QLatin1String("settingsWindow")) {
            return QObject::eventFilter(watched, event);
        }

        // Prevent Qt/Kirigami internal Escape shortcuts from hijacking or cycling Escape
        // before KeyPress reaches our handler.
        if (et == QEvent::ShortcutOverride) {
            if (key == Qt::Key_Escape) {
                ke->accept();
                return true;
            }
            return QObject::eventFilter(watched, event);
        }

        // Only handle top-level QQuickWindow delivery on KeyPress so each keypress fires once.
        if (!win || ke->isAutoRepeat()) {
            return QObject::eventFilter(watched, event);
        }

        if (key == Qt::Key_Escape) {
            QVariant handled = false;
            if (obj_name == QLatin1String("settingsWindow")) {
                QMetaObject::invokeMethod(
                    win,
                    "handleEscape",
                    Qt::DirectConnection,
                    Q_RETURN_ARG(QVariant, handled)
                );
            } else {
                QVariant menu_focused = false;
                QMetaObject::invokeMethod(
                    win,
                    "isMenuPopupFocused",
                    Qt::DirectConnection,
                    Q_RETURN_ARG(QVariant, menu_focused)
                );
                if (menu_focused.toBool()) {
                    return false;
                }
                QMetaObject::invokeMethod(
                    win,
                    "handleGlobalEscape",
                    Qt::DirectConnection,
                    Q_RETURN_ARG(QVariant, handled)
                );
            }
            if (handled.toBool()) {
                ke->accept();
                return true;
            }
            return false;
        }

        // Key_Return / Key_Enter: activate focused control if appropriate.
        QVariant can_activate = false;
        QMetaObject::invokeMethod(
            win,
            "canActivateFocusedControlWithReturn",
            Qt::DirectConnection,
            Q_RETURN_ARG(QVariant, can_activate)
        );
        if (can_activate.toBool()) {
            QMetaObject::invokeMethod(
                win,
                "activateFocusedControlWithReturn",
                Qt::DirectConnection
            );
            ke->accept();
            return true;
        }
        return false;
    }
};

void copy_to_clipboard(rust::Str text) {
    const QString qtext = QString::fromUtf8(text.data(), static_cast<qsizetype>(text.size()));
    if (QClipboard* clipboard = QGuiApplication::clipboard()) {
        clipboard->setText(qtext);
    }
}

rust::String i18n_subs(rust::Str msgid, rust::Slice<const rust::Str> args) {
    const QByteArray raw(msgid.data(), static_cast<qsizetype>(msgid.size()));
    KLocalizedString localized = ki18n(raw.constData());
    for (const rust::Str& arg : args) {
        localized = localized.subs(QString::fromUtf8(arg.data(), static_cast<qsizetype>(arg.size())));
    }
    const QByteArray utf8 = localized.toString().toUtf8();
    return rust::String(utf8.constData(), static_cast<size_t>(utf8.size()));
}

rust::String i18np_subs(rust::Str singular, rust::Str plural, std::int64_t n, rust::Slice<const rust::Str> args) {
    const QByteArray raw_singular(singular.data(), static_cast<qsizetype>(singular.size()));
    const QByteArray raw_plural(plural.data(), static_cast<qsizetype>(plural.size()));
    KLocalizedString localized = ki18np(raw_singular.constData(), raw_plural.constData());
    localized = localized.subs(static_cast<qlonglong>(n));
    for (const rust::Str& arg : args) {
        localized = localized.subs(QString::fromUtf8(arg.data(), static_cast<qsizetype>(arg.size())));
    }
    const QByteArray utf8 = localized.toString().toUtf8();
    return rust::String(utf8.constData(), static_cast<size_t>(utf8.size()));
}

rust::String format_count(std::int64_t n) {
    const QByteArray utf8 = QLocale().toString(static_cast<qlonglong>(n)).toUtf8();
    return rust::String(utf8.constData(), static_cast<size_t>(utf8.size()));
}

rust::String format_decimal(double value, std::int32_t decimals) {
    const QByteArray utf8 = QLocale().toString(value, 'f', decimals).toUtf8();
    return rust::String(utf8.constData(), static_cast<size_t>(utf8.size()));
}

std::int32_t local_offset_seconds(std::int64_t unix_secs) {
    const QDateTime dt = QDateTime::fromSecsSinceEpoch(static_cast<qint64>(unix_secs)).toLocalTime();
    return static_cast<std::int32_t>(dt.offsetFromUtc());
}

void install_keyboard_handlers(QQmlApplicationEngine& engine) {
    if (QCoreApplication* app = QCoreApplication::instance()) {
        app->installEventFilter(new KesticKeyNavFilter(&engine));
    }
}

} // namespace kestic::bridge
