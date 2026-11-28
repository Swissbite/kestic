// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Helpers of the M6 smoke scenes that measure whether the window waits for the daemon.

#pragma once

#include "src/smoke_common.h"
#include <QElapsedTimer>
#include <QQuickWindow>
#include <QTimer>
#include <functional>
#include <memory>

namespace kestic::smoke {

// Largest pause of the main thread (ms) between ticks of a 20 ms timer, until `stop` is set.
// While `skip` is set, the next pause is not counted and `skip` is cleared.
inline std::shared_ptr<qint64> watch_main_thread_gaps(QQuickWindow* window, const std::shared_ptr<bool>& stop,
    const std::shared_ptr<bool>& skip = std::make_shared<bool>(false)) {
    auto largest = std::make_shared<qint64>(0);
    auto since_tick = std::make_shared<QElapsedTimer>();
    since_tick->start();
    auto* ticker = new QTimer(window);
    QObject::connect(ticker, &QTimer::timeout, window, [=]() {
        const qint64 gap = since_tick->restart();
        if (*skip) {
            *skip = false;
        } else {
            *largest = std::max(*largest, gap);
        }
        if (*stop) {
            ticker->stop();
        }
    });
    ticker->start(20);
    return largest;
}

// Polls `done` every 50 ms for at most `limit_ms`; then `finish` runs, or the scene fails with
// `code` and `what`.
inline void wait_then(QQuickWindow* window, int limit_ms, int code, const QString& what,
    const std::function<bool()>& done, const std::function<void()>& finish) {
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (done()) {
            poll->stop();
            finish();
        } else if (waited->elapsed() > limit_ms) {
            poll->stop();
            smoke_fail("M6", code, what);
        }
    });
    poll->start(50);
}

// Calls `method` on the backend and fails the scene unless it returns a request id (> 0) within
// 100 ms (AC-M6-33-1 (a)).
inline bool requests_at_once(int code, const char* method, const std::function<int()>& call) {
    QElapsedTimer took;
    took.start();
    const int request = call();
    const qint64 ms = took.elapsed();
    if (request <= 0) {
        smoke_fail("M6", code, QStringLiteral("%1 returned no request").arg(QLatin1String(method)));
        return false;
    }
    if (ms >= 100) {
        smoke_fail("M6", code, QStringLiteral("(a) %1 kept the main thread for %2 ms").arg(QLatin1String(method)).arg(ms));
        return false;
    }
    return true;
}

// Waits until the connection test of the form `prefix` (its «Testing the connection…» line) has
// answered, at most `limit_ms`.
inline bool wait_for_connection_test(QQuickWindow* window, const QString& prefix, int limit_ms = 30000) {
    const QByteArray name = (prefix + QStringLiteral("TestingMessage")).toUtf8();
    return wait_until([&] {
        QQuickItem* testing = child_item(window, name.constData());
        return !testing || !testing->isVisible();
    }, limit_ms);
}

// AC-M6-33-1 with every daemon reply late: the previews, exclude helpers, source checks,
// connection test, restic source and snapshot queries.
void run_slow_queries_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path);

// AC-M6-33-1 with every daemon reply late: answers that arrive after the window moved on.
void run_slow_stale_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path);

} // namespace kestic::smoke
