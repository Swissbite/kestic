// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#pragma once

#include <QAbstractItemModel>
#include <QCoreApplication>
#include <QEvent>
#include <QKeyEvent>
#include <QList>
#include <QMetaObject>
#include <QObject>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlError>
#include <QQuickItem>
#include <QQuickWindow>
#include <QString>
#include <QStringList>
#include <QTimer>
#include <QVariant>
#include <QWindow>
#include <cstdio>
#include <functional>
#include <memory>
#include <vector>

namespace kestic::smoke {

void collect_visual_descendants(QQuickItem* root, QList<QQuickItem*>& out);

QQuickItem* find_visual_item(QQuickWindow* window, const QString& object_name);

void collect_all_qobjects(QObject* root, QList<QObject*>& out);

bool verify_tab_bar_labels(
    QQuickItem* nav_bar,
    int expected_tabs,
    const QStringList& required_labels
);

bool save_screenshot(QQuickWindow* window, const QString& out_path);

QQuickItem* find_visible_item_by_text(
    QQuickItem* root,
    const QStringList& candidate_texts,
    bool button_only = true
);

bool has_visible_text(QQuickItem* root, const QString& needle);

bool click_ui_item(QQuickWindow* window, QQuickItem* item, const char* label);

bool enter_text_field(
    QQuickWindow* window,
    QQuickItem* field,
    const QString& text,
    const char* label
);

QString describe_focus_item(QQuickItem* it);

void wait_ms(int ms);

// The model called `name` below `window` (nullptr when there is none).
inline QAbstractItemModel* model_named(QQuickWindow* window, const char* name) {
    return qobject_cast<QAbstractItemModel*>(window->findChild<QObject*>(QString::fromLatin1(name)));
}

// The value of the role called `role` in `row` of `model` (invalid when the role is unknown).
inline QVariant role_value(QAbstractItemModel* model, int row, const char* role) {
    const auto names = model->roleNames();
    for (auto it = names.constBegin(); it != names.constEnd(); ++it) {
        if (it.value() == role) {
            return model->data(model->index(row, 0), it.key());
        }
    }
    return QVariant();
}

// The id in `row` of the model called `model` (empty when the model or the row is missing).
inline QString id_at(QQuickWindow* window, const char* model, const char* id_role, int row) {
    QAbstractItemModel* m = model_named(window, model);
    return (m && row < m->rowCount()) ? role_value(m, row, id_role).toString() : QString();
}

inline QString backup_id_at(QQuickWindow* window, int row) {
    return id_at(window, "backupsModel", "backupId", row);
}

inline QString destination_id_at(QQuickWindow* window, int row) {
    return id_at(window, "destinationsModel", "destinationId", row);
}

inline QString run_id_at(QQuickWindow* window, int row) {
    return id_at(window, "runsModel", "runId", row);
}

// Polls `pred` every 40 ms until it holds or `timeout_ms` has passed; returns its last value.
bool wait_until(const std::function<bool()>& pred, int timeout_ms);

// A Popup/Dialog that finished opening (`opened`); false for a null popup.
inline bool is_open(QObject* popup) {
    return popup && popup->property("opened").toBool();
}

// A Popup/Dialog that is gone: not `visible` any more (`opened` implies `visible`, so no popup
// is still on its way out); false for a null popup.
inline bool is_closed(QObject* popup) {
    return popup && !popup->property("visible").toBool();
}

// Finishes pending asynchronous QML creations (e.g. ListView cache delegates). Nothing drives
// them between events in an offscreen run.
void finish_incubation(QQmlEngine* engine);

// Same for the engine that created `window`.
void finish_incubation(QQuickWindow* window);

// Finishes pending asynchronous QML creations once the event loop has quit, before the engine
// is destroyed. Otherwise a smoke run that exits mid-incubation makes ~QQmlEngine warn on stderr.
void finish_incubation_on_quit(QQmlApplicationEngine& engine);

void send_key(
    QWindow* window,
    int key,
    Qt::KeyboardModifiers mods = Qt::NoModifier,
    const QString& text = QString()
);

QQuickItem* tab_until_object(QQuickWindow* window, const QString& obj_name, int max_steps);

QQuickItem* tab_until_text(QQuickWindow* window, const QString& text_needle, int max_steps);

// Ends the smoke run with a failure: prints `<tag> smoke failed (<code>): <msg>` and quits the event
// loop with `code`.
void smoke_fail(const char* tag, int code, const QString& msg);

// Ends the smoke run successfully. Prints and fails with exit code 3 when `warnings` (optional)
// holds QML warnings, then saves the screenshot of `shot` to `out_path` (optional, exit code 10 on
// failure) and quits with 0.
void smoke_finish(
    const char* tag,
    const QStringList* warnings,
    QQuickWindow* shot = nullptr,
    const QString& out_path = QString()
);

// The child item / object called `name` below `root` (nullptr when there is none).
inline QQuickItem* child_item(QObject* root, const char* name) {
    return root->findChild<QQuickItem*>(QString::fromLatin1(name));
}

inline QObject* child_object(QObject* root, const char* name) {
    return root->findChild<QObject*>(QString::fromLatin1(name));
}

// The visible button below `root` whose text contains one of `texts` ("A|B": UTF-8 alternatives,
// e.g. English and German).
inline QQuickItem* visible_button(QQuickItem* root, const char* texts) {
    return find_visible_item_by_text(root, QString::fromUtf8(texts).split(QLatin1Char('|')));
}

// True once the only backup in `backupsModel` is no longer running and `runsModel` holds a run;
// `*has_error` then tells whether that backup ended with an error.
bool backup_run_finished(QQuickWindow* window, bool* has_error);

// Polls every 200 ms (up to 15 s) until `backup_run_finished`, then calls `on_finished`. Fails the
// smoke with `error_code` when the run ended with an error and with `timeout_code` on timeout.
void poll_backup_run(
    QQuickWindow* window,
    const char* tag,
    int error_code,
    int timeout_code,
    const std::function<void()>& on_finished
);

// One step of a timed smoke scenario: `run` fires `delay_ms` after the previous step.
struct SmokeStep {
    int delay_ms;
    std::function<void()> run;
};

// Runs `steps` in order on `context`'s event loop (a step with delay 0 runs immediately) and
// stops at the first failed step.
void run_steps(QObject* context, std::vector<SmokeStep> steps);

inline QQuickItem* tab_to(QQuickWindow* window, const char* object_name, int max_steps) {
    return tab_until_object(window, QString::fromLatin1(object_name), max_steps);
}

inline QQuickItem* tab_to_text(QQuickWindow* window, const char* text, int max_steps) {
    return tab_until_text(window, QString::fromUtf8(text), max_steps);
}

// Presses Tab the way a user does (`\t` is the text a text area would insert).
inline void press_tab(QWindow* window) {
    send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
}

// Presses `key`, then waits `ms` for the UI to react.
inline void press_and_wait(QWindow* window, int key, int ms) {
    send_key(window, key);
    wait_ms(ms);
}

inline void trigger(QObject* action) {
    QMetaObject::invokeMethod(action, "trigger");
}

// Reports a failed check and leaves the calling `void` smoke step.
#define SMOKE_FAIL(tag, code, msg)                                                     \
    do {                                                                               \
        ::kestic::smoke::smoke_fail((tag), (code), QString(msg));                      \
        return;                                                                        \
    } while (0)

// Reports a failed check unless `cond` holds (the message names the failed condition) and leaves
// the calling `void` smoke step.
#define SMOKE_CHECK(tag, code, cond, msg)                                                \
    do {                                                                                 \
        if (!(cond)) {                                                                   \
            SMOKE_FAIL((tag), (code), QString(msg) + QString::fromUtf8(" [" #cond "]")); \
        }                                                                                \
    } while (0)

// N independent checks that share one message; each condition is checked (and named) on its own.
#define SMOKE_CHECK2(tag, code, c1, c2, msg) \
    SMOKE_CHECK(tag, code, c1, msg);         \
    SMOKE_CHECK(tag, code, c2, msg)
#define SMOKE_CHECK3(tag, code, c1, c2, c3, msg) \
    SMOKE_CHECK2(tag, code, c1, c2, msg);        \
    SMOKE_CHECK(tag, code, c3, msg)
#define SMOKE_CHECK4(tag, code, c1, c2, c3, c4, msg) \
    SMOKE_CHECK3(tag, code, c1, c2, c3, msg);        \
    SMOKE_CHECK(tag, code, c4, msg)
#define SMOKE_CHECK5(tag, code, c1, c2, c3, c4, c5, msg) \
    SMOKE_CHECK4(tag, code, c1, c2, c3, c4, msg);        \
    SMOKE_CHECK(tag, code, c5, msg)

// `ptr` exists, then its `visible` / `enabled` property is true (or false for DISABLED): two
// separate checks with one message.
#define SMOKE_CHECK_VISIBLE(tag, code, ptr, msg) SMOKE_CHECK2(tag, code, ptr, (ptr)->property("visible").toBool(), msg)
#define SMOKE_CHECK_ENABLED(tag, code, ptr, msg) SMOKE_CHECK2(tag, code, ptr, (ptr)->property("enabled").toBool(), msg)
#define SMOKE_CHECK_DISABLED(tag, code, ptr, msg) SMOKE_CHECK2(tag, code, ptr, !(ptr)->property("enabled").toBool(), msg)

// `dlg` exists and finished opening / exists and is gone (`is_closed`).
#define SMOKE_CHECK_OPENED(tag, code, dlg, msg) SMOKE_CHECK2(tag, code, dlg, wait_until([&] { return is_open(dlg); }, 1000), msg)
#define SMOKE_CHECK_CLOSED(tag, code, dlg, msg) SMOKE_CHECK(tag, code, is_closed(dlg), msg)

// Clicks `item` (`label` names it in diagnostics) or fails the smoke step.
#define SMOKE_CLICK(tag, code, window, item, label) \
    SMOKE_CHECK(tag, code, click_ui_item((window), (item), label), QStringLiteral("click on '" label "' failed"))

// Types `text` into `field` or fails the smoke step.
#define SMOKE_ENTER_TEXT(tag, code, window, field, text, label) \
    SMOKE_CHECK(tag, code, enter_text_field((window), (field), (text), label), QStringLiteral("entering text into '" label "' failed"))

void run_m2_smoke_scenario(
    QQmlApplicationEngine& engine,
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& scen,
    const QString& out_path,
    const QString& lang
);

void run_m3_smoke_scenario(
    QQmlApplicationEngine& engine,
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& scen,
    const QString& out_path,
    const QString& lang
);

} // namespace kestic::smoke
