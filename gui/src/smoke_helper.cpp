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
#include <QQmlComponent>
#include <QQmlEngine>
#include <QQmlError>
#include <QQmlIncubationController>
#include <QQuickItem>
#include <QQuickWindow>
#include <QStringList>
#include <QTimer>
#include <QUrl>
#include <QVariant>
#include <algorithm>
#include <cstdio>
#include <memory>
#include <qtestkeyboard.h>
#include <qtestmouse.h>


namespace kestic::smoke {

// The window point of `scene` for QTest, inside `window`: rounding may reach its width or height,
// where QTest warns. QTest sends an event at (0,0) to the middle of the window instead, so that
// point moves by one pixel.
QPoint window_point(const QWindow* window, const QPointF& scene) {
    const QPoint point(
        qBound(0, qRound(scene.x()), qMax(0, window->width() - 1)),
        qBound(0, qRound(scene.y()), qMax(0, window->height() - 1)));
    return point.isNull() ? QPoint(1, 1) : point;
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
    wait_ms(40); // quiet window: the next frame repaints the whole window
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
        wait_ms(40); // quiet window: the requested frame is rendered
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
    wait_ms(60); // quiet window: the window draws its next frame after the grab
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

// The topmost visible item at window point `at` that takes mouse buttons, for failure messages.
QQuickItem* mouse_item_at(QQuickItem* root, const QPointF& at) {
    if (!root || !root->isVisible()) {
        return nullptr;
    }
    QList<QQuickItem*> children = root->childItems();
    std::stable_sort(children.begin(), children.end(), [](QQuickItem* a, QQuickItem* b) { return a->z() < b->z(); });
    for (auto it = children.crbegin(); it != children.crend(); ++it) {
        if (QQuickItem* hit = mouse_item_at(*it, at)) {
            return hit;
        }
    }
    const QPointF local = root->mapFromScene(at);
    if (root->acceptedMouseButtons() != Qt::NoButton && root->contains(local)) {
        return root;
    }
    return nullptr;
}

QString click_failure(QQuickWindow* window, QQuickItem* item) {
    if (!window || !item) {
        return QStringLiteral("null item or window");
    }
    if (!item->isVisible() || !item->isEnabled() || item->width() <= 0.0 || item->height() <= 0.0) {
        return QStringLiteral("item not clickable (vis=%1 en=%2 w=%3 h=%4)")
            .arg(item->isVisible() ? 1 : 0)
            .arg(item->isEnabled() ? 1 : 0)
            .arg(item->width(), 0, 'f', 1)
            .arg(item->height(), 0, 'f', 1);
    }
    const QPointF center(item->width() / 2.0, item->height() / 2.0);
    const QPointF scene = item->mapToScene(center);
    if (scene.x() < 0.0 || scene.y() < 0.0 || scene.x() >= window->width() || scene.y() >= window->height()) {
        return QStringLiteral("scene center (%1,%2) outside window (%3x%4)")
            .arg(scene.x(), 0, 'f', 1)
            .arg(scene.y(), 0, 'f', 1)
            .arg(window->width())
            .arg(window->height());
    }
    // The pointer moves over the item first, as a real one does; then the press must reach it.
    // QTest handles pending events after each step, which may move the item (a message above it
    // appears), so the press goes to where the item is then.
    QTest::mouseMove(window, window_point(window, scene), 20);
    const QPoint at = window_point(window, item->mapToScene(center));
    if (at != window_point(window, scene)) {
        QTest::mouseMove(window, at, 20);
    }
    QTest::mousePress(window, Qt::LeftButton, Qt::NoModifier, at, 20);
    QCoreApplication::processEvents();
    const QVariant pressed_prop = item->property("pressed");
    const bool hit_pressed = pressed_prop.isValid() && pressed_prop.toBool();
    // Without a delay QTest waits a double-click interval after the release, so the next click
    // counts as a click of its own.
    QTest::mouseRelease(window, Qt::LeftButton, Qt::NoModifier, at);
    QCoreApplication::processEvents();
    if (pressed_prop.isValid() && !hit_pressed) {
        QStringList open_popups;
        for (QObject* obj : window->findChildren<QObject*>()) {
            if (obj->inherits("QQuickPopup") && obj->property("opened").toBool()) {
                open_popups << QStringLiteral("%1[%2]").arg(
                    QString::fromLatin1(obj->metaObject()->className()), obj->objectName());
            }
        }
        return QStringLiteral("window press at (%1,%2) did not hit target item; topmost there: %3; open popups: %4")
            .arg(at.x())
            .arg(at.y())
            .arg(describe_focus_item(mouse_item_at(window->contentItem(), at)), open_popups.join(QStringLiteral(", ")));
    }
    return QString();
}

namespace {

// Records when the first mouse move and the first press reach a window. Qt sends a move of its
// own right before a press at a new position, with the press's timestamp, so only a move with
// an earlier timestamp is one the helper sent.
class MouseEventLog : public QObject {
public:
    qint64 first_move = -1;
    qint64 first_press = -1;

    bool eventFilter(QObject* watched, QEvent* event) override {
        const auto* input = dynamic_cast<QInputEvent*>(event);
        if (input && event->type() == QEvent::MouseMove && first_move < 0) {
            first_move = static_cast<qint64>(input->timestamp());
        }
        if (input && event->type() == QEvent::MouseButtonPress && first_press < 0) {
            first_press = static_cast<qint64>(input->timestamp());
        }
        return QObject::eventFilter(watched, event);
    }
};

}  // namespace

QString input_self_check(QQuickWindow* window) {
    QQmlEngine* engine = qmlEngine(window);
    if (!engine) {
        engine = qmlEngine(window->contentItem());
    }
    if (!engine) {
        return QStringLiteral("no QML engine");
    }
    // A target centred on (0,0) that counts clicks, double clicks and hovers, and a target under
    // another mouse area.
    QQmlComponent component(engine);
    component.setData(
        QByteArrayLiteral(
            "import QtQuick\n"
            "Item {\n"
            "  anchors.fill: parent; z: 100000\n"
            "  property int clicks: 0; property int doubleClicks: 0; property int entered: 0\n"
            "  MouseArea { objectName: 'selfCheckCorner'; x: -10; y: -10; width: 20; height: 20;"
            "    hoverEnabled: true; onClicked: parent.clicks += 1;"
            "    onDoubleClicked: parent.doubleClicks += 1; onEntered: parent.entered += 1 }\n"
            "  MouseArea { objectName: 'selfCheckCovered'; x: 100; y: 100; width: 40; height: 40 }\n"
            "  MouseArea { x: 90; y: 90; width: 60; height: 60 }\n"
            "}\n"
        ),
        QUrl()
    );
    std::unique_ptr<QObject> root(component.create());
    auto* overlay = qobject_cast<QQuickItem*>(root.get());
    if (!overlay) {
        return QStringLiteral("self-check overlay not created: %1").arg(component.errorString());
    }
    overlay->setParentItem(window->contentItem());
    QCoreApplication::processEvents();
    auto* corner = overlay->findChild<QQuickItem*>(QStringLiteral("selfCheckCorner"));
    auto* covered = overlay->findChild<QQuickItem*>(QStringLiteral("selfCheckCovered"));
    QString result;
    MouseEventLog log;
    window->installEventFilter(&log);
    const QString first = click_failure(window, corner);
    window->removeEventFilter(&log);
    const QString second = click_failure(window, corner);
    if (!first.isEmpty() || !second.isEmpty()) {
        result = QStringLiteral("a click on the item centred on (0,0) failed: %1 %2").arg(first, second);
    } else if (log.first_move < 0 || log.first_press < 0 || log.first_move >= log.first_press
               || overlay->property("entered").toInt() < 1) {
        result = QStringLiteral("the pointer did not move over the item before the press");
    } else if (overlay->property("clicks").toInt() != 2 || overlay->property("doubleClicks").toInt() != 0) {
        result = QStringLiteral("two clicks gave %1 clicks and %2 double clicks")
                     .arg(overlay->property("clicks").toInt())
                     .arg(overlay->property("doubleClicks").toInt());
    } else if (!covered
               || !click_failure(window, covered).contains(QStringLiteral("did not hit target item"))) {
        result = QStringLiteral("a click on a covered item did not fail for missing it");
    }
    overlay->setParentItem(nullptr);
    return result;
}

bool click_ui_item(QQuickWindow* window, QQuickItem* item, const char* label) {
    const QString failure = click_failure(window, item);
    if (!failure.isEmpty()) {
        std::fprintf(stderr, "UI click failed (%s): %s\n", label, qPrintable(failure));
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
    // A field scrolled out of the window gets the focus without the click.
    if (scene.x() >= 0.0 && scene.y() >= 0.0 && scene.x() < window->width() && scene.y() < window->height()) {
        QTest::mouseClick(window, Qt::LeftButton, Qt::NoModifier, window_point(window, scene));
    }
    field->forceActiveFocus(Qt::MouseFocusReason);
    QCoreApplication::processEvents();

    QTest::keyClick(window, Qt::Key_A, Qt::ControlModifier);
    QTest::keyClick(window, Qt::Key_Backspace);
    // Key code 0 with the text: any character arrives, also a tab or one outside ASCII.
    for (const char32_t code_point : text.toUcs4()) {
        const QString s = QString::fromUcs4(&code_point, 1);
        QTest::sendKeyEvent(QTest::Press, window, Qt::Key(0), s, Qt::NoModifier);
        QTest::sendKeyEvent(QTest::Release, window, Qt::Key(0), s, Qt::NoModifier);
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

void send_mouse(QWindow* window, const QPointF& scene, Qt::MouseButton button, QEvent::Type type, int delta_ms) {
    const QPoint at = window_point(window, scene);
    switch (type) {
    case QEvent::MouseButtonPress:
        QTest::mousePress(window, button, Qt::NoModifier, at, delta_ms);
        break;
    case QEvent::MouseButtonRelease:
        QTest::mouseRelease(window, button, Qt::NoModifier, at, delta_ms);
        break;
    default:
        QTest::mouseMove(window, at, delta_ms);
        break;
    }
    QCoreApplication::processEvents();
}

void send_key(QWindow* window, int key, Qt::KeyboardModifiers mods, const QString& text) {
    QTest::sendKeyEvent(QTest::Press, window, Qt::Key(key), text, mods);
    QTest::sendKeyEvent(QTest::Release, window, Qt::Key(key), text, mods);
    QCoreApplication::processEvents();
    if (key == Qt::Key_Return || key == Qt::Key_Enter) {
        wait_ms(160); // quiet window: Return starts actions whose effects the callers check
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

void exit_scenario(const char* tag, const QString& scen, int rc, int unknown_code, const QStringList& warnings) {
    if (rc < 0) {
        std::fprintf(stderr, "%s smoke failed (%d): unknown smoke scenario %s\n", tag, unknown_code, qPrintable(scen));
        QCoreApplication::exit(unknown_code);
        return;
    }
    if (rc != 0) {
        std::fprintf(stderr, "%s smoke `%s` failed with code %d\n", tag, qPrintable(scen), rc);
        QCoreApplication::exit(rc);
        return;
    }
    if (!warnings.isEmpty()) {
        for (const auto& w : warnings) {
            std::fprintf(stderr, "%s QML warning: %s\n", tag, qPrintable(w));
        }
        QCoreApplication::exit(3);
        return;
    }
    QCoreApplication::exit(0);
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
