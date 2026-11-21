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
#include <QMetaObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QVariant>

namespace kestic::bridge {

class KesticKeyNavFilter : public QObject {
public:
    explicit KesticKeyNavFilter(QObject* parent = nullptr) : QObject(parent) {}

    bool eventFilter(QObject* watched, QEvent* event) override {
        const QEvent::Type et = event->type();
        if (et != QEvent::ShortcutOverride && et != QEvent::KeyPress) {
            return QObject::eventFilter(watched, event);
        }

        auto* ke = static_cast<QKeyEvent*>(event);
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
