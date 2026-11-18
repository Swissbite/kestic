// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_helper.h"
#include "src/smoke_common.h"

#include <KLocalizedString>
#include <QAbstractItemModel>
#include <QClipboard>
#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QEventLoop>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImage>
#include <QKeyEvent>
#include <QMetaObject>
#include <QMouseEvent>
#include <QPointer>
#include <QQmlError>
#include <QQuickItem>
#include <QQuickWindow>
#include <QStringList>
#include <QTimer>
#include <QVariant>
#include <cstdio>
#include <memory>

QT_BEGIN_NAMESPACE
Q_GUI_EXPORT void qt_handleMouseEvent(
    QWindow* window,
    const QPointF& local,
    const QPointF& global,
    Qt::MouseButtons state,
    Qt::MouseButton button,
    QEvent::Type type,
    Qt::KeyboardModifiers mods,
    int timestamp
);
Q_GUI_EXPORT void qt_handleKeyEvent(
    QWindow* window,
    QEvent::Type type,
    int key,
    Qt::KeyboardModifiers mods,
    const QString& text = QString(),
    bool autorep = false,
    ushort count = 1
);
QT_END_NAMESPACE

namespace kestic::smoke {

int next_mouse_timestamp(int delta_ms) {
    static int ts = 100;
    ts += delta_ms;
    return ts;
}

void collect_visual_descendants(QQuickItem* root, QList<QQuickItem*>& out) {
    if (!root) {
        return;
    }
    const QList<QQuickItem*> children = root->childItems();
    for (QQuickItem* child : children) {
        out.append(child);
        collect_visual_descendants(child, out);
    }
}

QQuickItem* find_visual_item(QQuickWindow* window, const QString& object_name) {
    if (!window) {
        return nullptr;
    }
    QList<QQuickItem*> items;
    collect_visual_descendants(window->contentItem(), items);
    for (QQuickItem* it : items) {
        if (it->objectName() == object_name && it->isVisible()) {
            return it;
        }
    }
    for (QQuickItem* it : items) {
        if (it->objectName() == object_name) {
            return it;
        }
    }
    return nullptr;
}

void collect_all_qobjects(QObject* root, QList<QObject*>& out) {
    if (!root) {
        return;
    }
    out.append(root);
    for (QObject* child : root->children()) {
        collect_all_qobjects(child, out);
    }
}

bool verify_tab_bar_labels(
    QQuickItem* nav_bar,
    int expected_tabs,
    const QStringList& required_labels
) {
    QList<QQuickItem*> items;
    collect_visual_descendants(nav_bar, items);
    int checked_labels = 0;
    QStringList seen_labels;
    for (QQuickItem* item : items) {
        const QVariant truncated_var = item->property("truncated");
        const QVariant text_var = item->property("text");
        if (!truncated_var.isValid() || !text_var.isValid()) {
            continue;
        }
        const QString text = text_var.toString();
        if (text.isEmpty() || !item->isVisible()) {
            continue;
        }
        checked_labels += 1;
        seen_labels << text;
        if (truncated_var.toBool()) {
            std::fprintf(
                stderr,
                "QML smoke failed: tab label %s is truncated at width=%f\n",
                qPrintable(text),
                item->width()
            );
            return false;
        }
    }
    if (checked_labels < expected_tabs) {
        std::fprintf(
            stderr,
            "QML smoke failed: expected at least %d visible tab labels, checked %d\n",
            expected_tabs,
            checked_labels
        );
        return false;
    }
    for (const QString& req : required_labels) {
        if (!seen_labels.contains(req)) {
            std::fprintf(
                stderr,
                "QML smoke failed: expected localized tab label '%s', saw [%s]\n",
                qPrintable(req),
                qPrintable(seen_labels.join(QStringLiteral(", ")))
            );
            return false;
        }
    }
    return true;
}

bool save_screenshot(QQuickWindow* window, const QString& out_path) {
    if (out_path.isEmpty()) {
        return true;
    }
    const QFileInfo fi(out_path);
    QDir().mkpath(fi.absolutePath());
    QImage img;
    for (int attempt = 0; attempt < 5; ++attempt) {
        if (window) {
            window->requestUpdate();
        }
        wait_ms(40);
        img = window ? window->grabWindow() : QImage();
        if (!img.isNull() && img.width() > 0 && img.height() > 0) {
            break;
        }
    }
    const QString tmp_path = out_path + QStringLiteral(".tmp.%1").arg(QCoreApplication::applicationPid());
    QFile::remove(tmp_path);
    if (img.isNull() || !img.save(tmp_path, "PNG")) {
        QFile::remove(tmp_path);
        std::fprintf(
            stderr,
            "QML smoke failed: could not save screenshot to %s\n",
            qPrintable(out_path)
        );
        return false;
    }
    const QFileInfo tmp_fi(tmp_path);
    if (!tmp_fi.exists() || tmp_fi.size() <= 2048) {
        QFile::remove(tmp_path);
        std::fprintf(
            stderr,
            "QML smoke failed: saved screenshot %s was empty or too small (%lld bytes)\n",
            qPrintable(out_path),
            static_cast<long long>(tmp_fi.size())
        );
        return false;
    }
    QFile::remove(out_path);
    if (!QFile::rename(tmp_path, out_path)) {
        QFile::remove(tmp_path);
        std::fprintf(
            stderr,
            "QML smoke failed: could not rename screenshot %s\n",
            qPrintable(out_path)
        );
        return false;
    }
    wait_ms(60);
    return true;
}

QQuickItem* find_visible_item_by_text(
    QQuickItem* root,
    const QStringList& candidate_texts,
    bool button_only
) {
    if (!root) {
        return nullptr;
    }
    QList<QQuickItem*> items;
    collect_visual_descendants(root, items);
    QQuickItem* preferred = nullptr;
    QQuickItem* fallback = nullptr;
    for (QQuickItem* it : items) {
        if (!it->isVisible() || it->width() <= 0.0 || it->height() <= 0.0) {
            continue;
        }
        const QString cls = QString::fromLatin1(it->metaObject()->className());
        if (cls.contains(QLatin1String("KQuickStyleItem"))) {
            continue;
        }
        if (button_only && !cls.contains(QLatin1String("Button"))) {
            continue;
        }
        QString txt = it->property("text").toString();
        txt.remove(QLatin1Char('&'));
        if (txt.isEmpty()) {
            continue;
        }
        for (const QString& target : candidate_texts) {
            if (txt.contains(target)) {
                if (cls.contains(QLatin1String("Button"))
                    && !cls.contains(QLatin1String("PrivateActionToolButton"))) {
                    preferred = it;
                } else if (!fallback) {
                    fallback = it;
                }
                break;
            }
        }
    }
    return preferred ? preferred : fallback;
}

bool has_visible_text(QQuickItem* root, const QString& needle) {
    return find_visible_item_by_text(root, QStringList { needle }, false) != nullptr;
}

bool click_ui_item(QQuickWindow* window, QQuickItem* item, const char* label) {
    if (!window || !item) {
        std::fprintf(stderr, "UI click failed (%s): null item or window\n", label);
        return false;
    }
    if (!item->isVisible() || !item->isEnabled() || item->width() <= 0.0 || item->height() <= 0.0) {
        std::fprintf(
            stderr,
            "UI click failed (%s): item not clickable (vis=%d en=%d w=%.1f h=%.1f)\n",
            label,
            item->isVisible() ? 1 : 0,
            item->isEnabled() ? 1 : 0,
            item->width(),
            item->height()
        );
        return false;
    }
    const QPointF center(item->width() / 2.0, item->height() / 2.0);
    const QPointF scene = item->mapToScene(center);
    if (scene.x() < 0.0 || scene.y() < 0.0 || scene.x() > window->width() || scene.y() > window->height()) {
        std::fprintf(
            stderr,
            "UI click failed (%s): scene center (%.1f,%.1f) outside window (%dx%d)\n",
            label,
            scene.x(),
            scene.y(),
            window->width(),
            window->height()
        );
        return false;
    }
    const QPointF global = window->mapToGlobal(scene);
    qt_handleMouseEvent(
        window,
        scene,
        global,
        Qt::NoButton,
        Qt::NoButton,
        QEvent::MouseMove,
        Qt::NoModifier,
        next_mouse_timestamp(20)
    );
    qt_handleMouseEvent(
        window,
        scene,
        global,
        Qt::LeftButton,
        Qt::LeftButton,
        QEvent::MouseButtonPress,
        Qt::NoModifier,
        next_mouse_timestamp(20)
    );
    QCoreApplication::processEvents();
    const QVariant pressed_prop = item->property("pressed");
    const bool hit_pressed = pressed_prop.isValid() && pressed_prop.toBool();
    qt_handleMouseEvent(
        window,
        scene,
        global,
        Qt::NoButton,
        Qt::LeftButton,
        QEvent::MouseButtonRelease,
        Qt::NoModifier,
        next_mouse_timestamp(20)
    );
    QCoreApplication::processEvents();
    next_mouse_timestamp(500);
    if (pressed_prop.isValid() && !hit_pressed) {
        std::fprintf(
            stderr,
            "UI click failed (%s): window press at (%.1f,%.1f) did not hit target item\n",
            label,
            scene.x(),
            scene.y()
        );
        return false;
    }
    return true;
}

bool enter_text_field(QQuickWindow* window, QQuickItem* field, const QString& text, const char* label) {
    if (!window || !field || !field->isVisible() || !field->isEnabled()
        || field->width() <= 0.0 || field->height() <= 0.0) {
        std::fprintf(stderr, "UI text entry failed (%s): field unavailable\n", label);
        return false;
    }
    const QPointF center(field->width() / 2.0, field->height() / 2.0);
    const QPointF scene = field->mapToScene(center);
    const QPointF global = window->mapToGlobal(scene);
    qt_handleMouseEvent(
        window,
        scene,
        global,
        Qt::LeftButton,
        Qt::LeftButton,
        QEvent::MouseButtonPress,
        Qt::NoModifier,
        next_mouse_timestamp(20)
    );
    qt_handleMouseEvent(
        window,
        scene,
        global,
        Qt::NoButton,
        Qt::LeftButton,
        QEvent::MouseButtonRelease,
        Qt::NoModifier,
        next_mouse_timestamp(20)
    );
    next_mouse_timestamp(500);
    field->forceActiveFocus(Qt::MouseFocusReason);
    QCoreApplication::processEvents();

    QKeyEvent sel_all_press(QEvent::KeyPress, Qt::Key_A, Qt::ControlModifier);
    QKeyEvent sel_all_rel(QEvent::KeyRelease, Qt::Key_A, Qt::ControlModifier);
    QKeyEvent del_press(QEvent::KeyPress, Qt::Key_Backspace, Qt::NoModifier);
    QKeyEvent del_rel(QEvent::KeyRelease, Qt::Key_Backspace, Qt::NoModifier);
    QCoreApplication::sendEvent(window, &sel_all_press);
    QCoreApplication::sendEvent(window, &sel_all_rel);
    QCoreApplication::sendEvent(window, &del_press);
    QCoreApplication::sendEvent(window, &del_rel);

    for (const QChar ch : text) {
        const QString s(ch);
        QKeyEvent kp(QEvent::KeyPress, 0, Qt::NoModifier, s);
        QKeyEvent kr(QEvent::KeyRelease, 0, Qt::NoModifier, s);
        QCoreApplication::sendEvent(window, &kp);
        QCoreApplication::sendEvent(window, &kr);
    }
    QCoreApplication::processEvents();
    const QString actual = field->property("text").toString();
    if (actual != text) {
        std::fprintf(
            stderr,
            "UI text entry failed (%s): expected '%s', got '%s'\n",
            label,
            qPrintable(text),
            qPrintable(actual)
        );
        return false;
    }
    return true;
}

QString describe_focus_item(QQuickItem* it) {
    if (!it) {
        return QStringLiteral("<null>");
    }
    const QString cls = QString::fromLatin1(it->metaObject()->className());
    const QString obj = it->objectName();
    QString txt = it->property("text").toString();
    txt.remove(QLatin1Char('&'));
    txt.replace(QLatin1Char('\n'), QLatin1Char(' '));
    if (txt.size() > 40) {
        txt = txt.left(40) + QStringLiteral("...");
    }
    const QPointF sc = it->mapToScene(QPointF(0, 0));
    const bool vf = it->property("visualFocus").toBool();
    const bool af = it->hasActiveFocus();
    return QStringLiteral("%1[obj='%2' text='%3' vis=%4 en=%5 vf=%6 af=%7 rect=(%8,%9 %10x%11)]")
        .arg(cls, obj, txt)
        .arg(it->isVisible() ? 1 : 0)
        .arg(it->isEnabled() ? 1 : 0)
        .arg(vf ? 1 : 0)
        .arg(af ? 1 : 0)
        .arg(sc.x(), 0, 'f', 0)
        .arg(sc.y(), 0, 'f', 0)
        .arg(it->width(), 0, 'f', 0)
        .arg(it->height(), 0, 'f', 0);
}

void wait_ms(int ms) {
    QEventLoop loop;
    QTimer::singleShot(ms, &loop, &QEventLoop::quit);
    loop.exec();
}

void send_key(QWindow* window, int key, Qt::KeyboardModifiers mods, const QString& text) {
    qt_handleKeyEvent(window, QEvent::KeyPress, key, mods, text);
    qt_handleKeyEvent(window, QEvent::KeyRelease, key, mods, text);
    QCoreApplication::processEvents();
    if (key == Qt::Key_Return || key == Qt::Key_Enter) {
        wait_ms(160);
    }
}

QQuickItem* tab_until_object(QQuickWindow* window, const QString& obj_name, int max_steps) {
    window->requestActivate();
    QCoreApplication::processEvents();
    auto matches_focus = [&obj_name](QQuickItem* cur) -> QQuickItem* {
        QQuickItem* it = cur;
        while (it) {
            if (it->objectName() == obj_name) {
                return it;
            }
            if (!it->hasActiveFocus()) {
                break;
            }
            it = it->parentItem();
        }
        return nullptr;
    };
    if (QQuickItem* matched = matches_focus(window->activeFocusItem())) {
        return matched;
    }
    for (int i = 0; i < max_steps; ++i) {
        send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
        if (QQuickItem* matched = matches_focus(window->activeFocusItem())) {
            return matched;
        }
    }
    std::fprintf(
        stderr,
        "Keyboard navigation failed: Tab did not reach objectName='%s' within %d steps (last focus=%s)\n",
        qPrintable(obj_name),
        max_steps,
        qPrintable(describe_focus_item(window->activeFocusItem()))
    );
    return nullptr;
}

QQuickItem* tab_until_text(QQuickWindow* window, const QString& text_needle, int max_steps) {
    window->requestActivate();
    QCoreApplication::processEvents();
    for (int i = 0; i < max_steps; ++i) {
        send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
        QQuickItem* cur = window->activeFocusItem();
        if (cur) {
            QString txt = cur->property("text").toString();
            txt.remove(QLatin1Char('&'));
            if (txt.contains(text_needle)) {
                return cur;
            }
        }
    }
    std::fprintf(
        stderr,
        "Keyboard navigation failed: Tab did not reach text='%s' within %d steps (last focus=%s)\n",
        qPrintable(text_needle),
        max_steps,
        qPrintable(describe_focus_item(window->activeFocusItem()))
    );
    return nullptr;
}

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

std::int32_t local_offset_seconds(std::int64_t unix_secs) {
    const QDateTime dt = QDateTime::fromSecsSinceEpoch(static_cast<qint64>(unix_secs)).toLocalTime();
    return static_cast<std::int32_t>(dt.offsetFromUtc());
}

void install_keyboard_handlers(QQmlApplicationEngine& engine) {
    if (QCoreApplication* app = QCoreApplication::instance()) {
        app->installEventFilter(new KesticKeyNavFilter(&engine));
    }
}

void install_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str screenshot_path,
    int32_t expected_list_count,
    rust::Str layout_mode,
    rust::Str expected_lang,
    bool trigger_shortcut
) {
    const QString out_path = QString::fromUtf8(screenshot_path.data(), static_cast<qsizetype>(screenshot_path.size()));
    const QString mode = QString::fromUtf8(layout_mode.data(), static_cast<qsizetype>(layout_mode.size()));
    const QString lang = QString::fromUtf8(expected_lang.data(), static_cast<qsizetype>(expected_lang.size()));
    auto warnings = std::make_shared<QStringList>();

    QObject::connect(
        &engine,
        &QQmlEngine::warnings,
        &engine,
        [warnings](const QList<QQmlError>& qml_warnings) {
            for (const auto& err : qml_warnings) {
                warnings->append(err.toString());
            }
        }
    );

    QTimer::singleShot(150, &engine, [&engine, warnings, out_path, expected_list_count, mode, lang, trigger_shortcut]() {
        const auto roots = engine.rootObjects();
        if (roots.isEmpty()) {
            std::fprintf(stderr, "QML smoke failed: no root objects loaded\n");
            QCoreApplication::exit(2);
            return;
        }

        auto* window = qobject_cast<QQuickWindow*>(roots.first());
        if (!window) {
            std::fprintf(stderr, "QML smoke failed: root object is not a QQuickWindow\n");
            QCoreApplication::exit(4);
            return;
        }

        if (mode == QLatin1String("narrow")) {
            QMetaObject::invokeMethod(window, "resizeToMinWidthForSmoke");
            QMetaObject::invokeMethod(window, "openDeepSubpageForSmoke");
        } else if (mode == QLatin1String("wide")) {
            QMetaObject::invokeMethod(window, "openDeepSubpageForSmoke");
        }

        if (trigger_shortcut) {
            window->requestActivate();
            QCoreApplication::processEvents();
            QKeyEvent press(QEvent::KeyPress, Qt::Key_B, Qt::ControlModifier | Qt::ShiftModifier);
            QKeyEvent release(QEvent::KeyRelease, Qt::Key_B, Qt::ControlModifier | Qt::ShiftModifier);
            QCoreApplication::sendEvent(window, &press);
            QCoreApplication::sendEvent(window, &release);
            QCoreApplication::processEvents();
        }

        QTimer::singleShot(250, &engine, [window, warnings, out_path, expected_list_count, mode, lang, trigger_shortcut]() {
            if (!warnings->isEmpty()) {
                for (const auto& w : *warnings) {
                    std::fprintf(stderr, "QML warning: %s\n", qPrintable(w));
                }
                QCoreApplication::exit(3);
                return;
            }

            if (expected_list_count >= 0) {
                QObject* list_obj = window->findChild<QObject*>(QStringLiteral("spikeBackupList"));
                if (!list_obj) {
                    std::fprintf(stderr, "QML smoke failed: spikeBackupList not found\n");
                    QCoreApplication::exit(5);
                    return;
                }
                const int count = list_obj->property("count").toInt();
                if (count != expected_list_count) {
                    std::fprintf(
                        stderr,
                        "QML smoke failed: expected spikeBackupList count=%d, got %d\n",
                        expected_list_count,
                        count
                    );
                    QCoreApplication::exit(6);
                    return;
                }

                QObject* model_obj = window->findChild<QObject*>(QStringLiteral("spikeModel"));
                auto* item_model = qobject_cast<QAbstractItemModel*>(model_obj);
                if (!item_model || item_model->rowCount() != expected_list_count) {
                    std::fprintf(stderr, "QML smoke failed: spikeModel rowCount mismatch\n");
                    QCoreApplication::exit(7);
                    return;
                }

                const QHash<int, QByteArray> roles = item_model->roleNames();
                int title_role = -1;
                for (auto it = roles.constBegin(); it != roles.constEnd(); ++it) {
                    if (it.value() == "title") {
                        title_role = it.key();
                        break;
                    }
                }
                if (title_role < 0) {
                    std::fprintf(stderr, "QML smoke failed: title role missing\n");
                    QCoreApplication::exit(8);
                    return;
                }

                QStringList titles;
                for (int r = 0; r < item_model->rowCount(); ++r) {
                    titles << item_model->data(item_model->index(r, 0), title_role).toString();
                }
                const QStringList expected_titles = (lang == QLatin1String("de"))
                    ? QStringList {
                        QStringLiteral("Persönlicher Ordner"),
                        QStringLiteral("Fotos"),
                        QStringLiteral("Projekte"),
                    }
                    : QStringList {
                        QStringLiteral("Personal folder"),
                        QStringLiteral("Photos"),
                        QStringLiteral("Projects"),
                    };
                if (titles != expected_titles) {
                    std::fprintf(
                        stderr,
                        "QML smoke failed (lang=%s): unexpected titles: %s\n",
                        qPrintable(lang),
                        qPrintable(titles.join(QStringLiteral(", ")))
                    );
                    QCoreApplication::exit(9);
                    return;
                }
            }

            if (mode == QLatin1String("wide") || mode == QLatin1String("narrow")) {
                auto* top_nav = window->findChild<QQuickItem*>(QStringLiteral("topNavBar"));
                auto* bottom_nav = window->findChild<QQuickItem*>(QStringLiteral("bottomNavBar"));
                if (!top_nav || !bottom_nav) {
                    std::fprintf(stderr, "QML smoke failed: NavigationTabBar instances not found\n");
                    QCoreApplication::exit(11);
                    return;
                }
                const QStringList required_tabs = (lang == QLatin1String("de"))
                    ? QStringList {
                        QStringLiteral("Übersicht"),
                        QStringLiteral("Wiederherstellen"),
                        QStringLiteral("Backups"),
                        QStringLiteral("Ziele"),
                        QStringLiteral("Verlauf"),
                    }
                    : QStringList {
                        QStringLiteral("Overview"),
                        QStringLiteral("Restore"),
                        QStringLiteral("Backups"),
                        QStringLiteral("Destinations"),
                        QStringLiteral("History"),
                    };
                if (mode == QLatin1String("wide")) {
                    if (!top_nav->isVisible() || bottom_nav->isVisible()) {
                        std::fprintf(stderr, "QML smoke failed: expected topNavBar visible in wide mode\n");
                        QCoreApplication::exit(12);
                        return;
                    }
                    if (!verify_tab_bar_labels(top_nav, 5, required_tabs)) {
                        QCoreApplication::exit(13);
                        return;
                    }
                } else {
                    if (top_nav->isVisible() || !bottom_nav->isVisible()) {
                        std::fprintf(stderr, "QML smoke failed: expected bottomNavBar visible in narrow mode\n");
                        QCoreApplication::exit(14);
                        return;
                    }
                    if (!verify_tab_bar_labels(bottom_nav, 5, required_tabs)) {
                        QCoreApplication::exit(15);
                        return;
                    }
                }
            }

            if (trigger_shortcut) {
                const int triggered = window->property("backupNowTriggerCount").toInt();
                if (triggered < 1) {
                    std::fprintf(
                        stderr,
                        "QML smoke failed: Ctrl+Shift+B did not trigger Back Up Now (count=%d)\n",
                        triggered
                    );
                    QCoreApplication::exit(16);
                    return;
                }
            }

            if (!save_screenshot(window, out_path)) {
                QCoreApplication::exit(10);
                return;
            }

            wait_ms(60);
            QCoreApplication::exit(0);
        });
    });
}


} // namespace kestic::smoke
