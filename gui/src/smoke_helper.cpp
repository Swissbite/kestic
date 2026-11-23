// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_helper.h"
#include "src/smoke_common.h"

#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImage>
#include <QKeyEvent>
#include <QMetaObject>
#include <QMouseEvent>
#include <QPointer>
#include <QQmlEngine>
#include <QQmlError>
#include <QQmlIncubationController>
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
        if (item->property("lineCount").toInt() > 1) {
            std::fprintf(
                stderr,
                "QML smoke failed: tab label %s breaks onto %d lines at width=%f\n",
                qPrintable(text),
                item->property("lineCount").toInt(),
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

namespace {

// Kirigami's page row moves and crossfades pages in the narrow layout; a screenshot taken
// meanwhile shows the page below shining through the new one.
bool page_rows_moving(QQuickWindow* window) {
    QList<QQuickItem*> items;
    collect_visual_descendants(window->contentItem(), items);
    for (QQuickItem* it : items) {
        const QVariant moving = it->property("moving");
        if (moving.isValid() && moving.toBool()
            && QString::fromLatin1(it->metaObject()->className()).contains(QLatin1String("ColumnView"))) {
            return true;
        }
    }
    return false;
}

// The offscreen platform renders with the software scene graph, which repaints only the
// regions that changed. Pixels an earlier frame left outside a clip stay (the background of a
// Kirigami card below the visible part of a page, over the tab bar). A frame at another window
// size makes the next frame repaint the whole window, as the desktop shows it.
void repaint_whole_window(QQuickWindow* window) {
    const QSize size = window->size();
    window->resize(size.width(), size.height() + 1);
    (void)window->grabWindow();
    window->resize(size);
    wait_ms(40);
}

} // namespace

bool save_screenshot(QQuickWindow* window, const QString& out_path) {
    if (out_path.isEmpty()) {
        return true;
    }
    const QFileInfo fi(out_path);
    QDir().mkpath(fi.absolutePath());
    if (window) {
        if (!wait_until([&] { return !page_rows_moving(window); }, 3000)) {
            std::fprintf(
                stderr,
                "QML smoke failed: pages still moving after 3 s, no screenshot %s\n",
                qPrintable(out_path)
            );
            return false;
        }
        repaint_whole_window(window);
    }
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

    for (const char32_t code_point : text.toUcs4()) {
        const QString s = QString::fromUcs4(&code_point, 1);
        QKeyEvent kp(QEvent::KeyPress, 0, Qt::NoModifier, s);
        QKeyEvent kr(QEvent::KeyRelease, 0, Qt::NoModifier, s);
        QCoreApplication::sendEvent(window, &kp);
        QCoreApplication::sendEvent(window, &kr);
    }
    QCoreApplication::processEvents();
    const QString actual = field->property("text").toString();
    if (actual != text) {
        const QVariant echo_mode = field->property("echoMode");
        const bool is_password = echo_mode.isValid() && echo_mode.toInt() != 0;
        if (is_password) {
            // Never print the text of a password field, only its length.
            std::fprintf(
                stderr,
                "UI text entry failed (%s): password field holds %lld characters, expected %lld\n",
                label,
                static_cast<long long>(actual.size()),
                static_cast<long long>(text.size())
            );
        } else {
            std::fprintf(
                stderr,
                "UI text entry failed (%s): expected '%s', got '%s'\n",
                label,
                qPrintable(text),
                qPrintable(actual)
            );
        }
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

void finish_incubation(QQmlEngine* engine) {
    QQmlIncubationController* controller = engine ? engine->incubationController() : nullptr;
    if (!controller) {
        return;
    }
    QElapsedTimer elapsed;
    elapsed.start();
    while (controller->incubatingObjectCount() > 0 && elapsed.elapsed() < 2000) {
        controller->incubateFor(10);
    }
}

void finish_incubation(QQuickWindow* window) {
    finish_incubation(qmlEngine(window));
}

void finish_incubation_on_quit(QQmlApplicationEngine& engine) {
    QObject::connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, &engine, [&engine]() {
        finish_incubation(&engine);
    });
}

bool wait_until(const std::function<bool()>& pred, int timeout_ms) {
    for (int waited = 0; waited <= timeout_ms; waited += 40) {
        if (pred()) {
            return true;
        }
        wait_ms(40);
    }
    return pred();
}

namespace {
bool g_smoke_failed = false;

void run_steps_from(QObject* context, const std::shared_ptr<std::vector<SmokeStep>>& steps, size_t index) {
    if (index >= steps->size() || g_smoke_failed) {
        return;
    }
    auto run = [context, steps, index]() {
        if (g_smoke_failed) {
            return;
        }
        (*steps)[index].run();
        run_steps_from(context, steps, index + 1);
    };
    if ((*steps)[index].delay_ms <= 0) {
        run();
    } else {
        QTimer::singleShot((*steps)[index].delay_ms, context, run);
    }
}
} // namespace

void run_steps(QObject* context, std::vector<SmokeStep> steps) {
    run_steps_from(context, std::make_shared<std::vector<SmokeStep>>(std::move(steps)), 0);
}

bool backup_run_finished(QQuickWindow* window, bool* has_error) {
    QAbstractItemModel* backups = model_named(window, "backupsModel");
    QAbstractItemModel* runs = model_named(window, "runsModel");
    const bool has_backup = backups && backups->rowCount() == 1;
    const bool running = has_backup && role_value(backups, 0, "isRunning").toBool();
    *has_error = has_backup && role_value(backups, 0, "hasError").toBool();
    return has_backup && !running && runs && runs->rowCount() >= 1;
}

void poll_backup_run(
    QQuickWindow* window,
    const char* tag,
    int error_code,
    int timeout_code,
    const std::function<void()>& on_finished
) {
    auto* timer = new QTimer(window);
    auto ticks = std::make_shared<int>(0);
    QObject::connect(timer, &QTimer::timeout, window, [=]() {
        bool has_error = false;
        if (backup_run_finished(window, &has_error)) {
            timer->stop();
            if (has_error) {
                smoke_fail(tag, error_code, QStringLiteral("backup run finished with error"));
                return;
            }
            on_finished();
        } else if (++*ticks >= 75) {
            timer->stop();
            smoke_fail(tag, timeout_code, QStringLiteral("timed out waiting for the real restic backup run"));
        }
    });
    timer->start(200);
}

void smoke_fail(const char* tag, int code, const QString& msg) {
    g_smoke_failed = true;
    std::fprintf(stderr, "%s smoke failed (%d): %s\n", tag, code, qPrintable(msg));
    QCoreApplication::exit(code);
}

void smoke_finish(
    const char* tag,
    const QStringList* warnings,
    QQuickWindow* shot,
    const QString& out_path
) {
    if (warnings && !warnings->isEmpty()) {
        for (const auto& w : *warnings) {
            std::fprintf(stderr, "%s QML warning: %s\n", tag, qPrintable(w));
        }
        QCoreApplication::exit(3);
        return;
    }
    if (shot && !save_screenshot(shot, out_path)) {
        QCoreApplication::exit(10);
        return;
    }
    QCoreApplication::exit(0);
}

} // namespace kestic::smoke
