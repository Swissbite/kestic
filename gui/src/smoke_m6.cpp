// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_m6.h"
#include "src/smoke_helper.h"
#include <QAbstractItemModel>
#include <QJSValue>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQmlEngine>

namespace kestic::smoke {

namespace {

// Counts the answers of the daemon calls the window made (`daemonCallDone`), by request.
const char* const ANSWERS_QML = R"(
import QtQuick
QtObject {
    property var answers: ({})
    property int count: 0
    property Connections watch: Connections {
        target: appBackend
        function onDaemonCallDone(request, ok, result) {
            answers[request] = result;
            count += 1;
        }
    }
}
)";

const char* const BROWSE_QML = R"(
import net.daester.kestic 1.0
SnapshotBrowseModel {}
)";

const char* const DRAFT_QML = R"(
import net.daester.kestic 1.0
DestinationDraft { appBackend: root.backendRef }
)";

// Creates the QML object `source` in the context of the main window.
QObject* create_in_window(QQuickWindow* window, const char* source) {
    QQmlContext* context = qmlContext(window);
    QQmlComponent component(context->engine());
    component.setData(QByteArray(source), QUrl());
    QObject* object = component.create(context);
    if (object) {
        object->setParent(window);
    }
    return object;
}

// The answer to `request`, empty while none arrived.
QString answer_of(QObject* answers, int request) {
    const QVariantMap map = answers->property("answers").value<QJSValue>().toVariant().toMap();
    return map.value(QString::number(request)).toString();
}

bool answered(QObject* answers, int request) {
    const QVariantMap map = answers->property("answers").value<QJSValue>().toVariant().toMap();
    return map.contains(QString::number(request));
}

bool can_fetch_more(QObject* model) {
    auto* listed = qobject_cast<QAbstractItemModel*>(model);
    return listed && listed->canFetchMore(QModelIndex());
}

int rows_of(QObject* model) {
    auto* listed = qobject_cast<QAbstractItemModel*>(model);
    return listed ? listed->rowCount() : -1;
}

} // namespace

// AC-M6-33-1: every reply comes 5 s late. Answers that arrive after the window moved on do not
// count: (a) a connection test answered after the form changed leaves the form untested; (b) a
// dry run whose page closed before it started is cancelled (the test watches for the cancel);
// (c) a further page answered after the filter changed does not stop the list from loading more;
// (d) a search hit beyond the first page of its folder becomes the current row once its page
// arrived; (e) a hit the user left for a filter before its page arrived is no longer looked for.
void run_slow_stale_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    const QString bid = qEnvironmentVariable("KESTIC_TEST_BACKUP");
    const QString dest = qEnvironmentVariable("KESTIC_TEST_DEST");
    const QString snapshot = qEnvironmentVariable("KESTIC_TEST_SNAPSHOT");
    const QString source = qEnvironmentVariable("KESTIC_TEST_SOURCE");
    const QString probe = qEnvironmentVariable("KESTIC_TEST_PROBE");
    SMOKE_CHECK("M6", 120, !bid.isEmpty() && !dest.isEmpty() && !snapshot.isEmpty() && !source.isEmpty() && !probe.isEmpty(),
        "precondition: backup, destination, snapshot and paths");
    QObject* answers = create_in_window(window, ANSWERS_QML);
    QObject* draft = create_in_window(window, DRAFT_QML);
    QObject* browse = create_in_window(window, BROWSE_QML);
    SMOKE_CHECK3("M6", 120, answers, draft, browse, "precondition: the QML helpers");
    auto step = std::make_shared<int>(-1);
    auto dry_run = std::make_shared<int>(0);
    auto settle = std::make_shared<QElapsedTimer>();
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (waited->elapsed() > 240000) {
            poll->stop();
            smoke_fail("M6", 121, *step == 4 ? QStringLiteral("(c) the list loads no further pages after a further page arrived for an old filter")
                : *step == 5 ? QStringLiteral("(d) a search hit beyond the first page of its folder never became the current row")
                : QStringLiteral("timed out at step %1").arg(*step));
            return;
        }
        switch (*step) {
        case -1:
            if (!backend->property("daemon_reachable").toBool() || backup_id_at(window, 0).isEmpty()) return;
            break;
        case 0:
            draft->setProperty("name", QStringLiteral("Probe"));
            draft->setProperty("location", probe + QStringLiteral("/a"));
            draft->setProperty("repoPassword", QStringLiteral("probe-password"));
            QMetaObject::invokeMethod(draft, "runConnectionTest");
            draft->setProperty("location", probe + QStringLiteral("/b"));
            *dry_run = 0;
            QMetaObject::invokeMethod(backend, "startBackupDryRun", Q_RETURN_ARG(int, *dry_run), Q_ARG(QString, bid),
                Q_ARG(QString, QString()));
            QMetaObject::invokeMethod(backend, "forgetDaemonCall", Q_ARG(int, *dry_run));
            QMetaObject::invokeMethod(browse, "open", Q_ARG(QString, dest), Q_ARG(QString, snapshot), Q_ARG(QString, source));
            break;
        case 1:
            if (browse->property("state").toString() == QLatin1String("running")) {
                QMetaObject::invokeMethod(browse, "poll");
                return;
            }
            SMOKE_CHECK2("M6", 122, rows_of(browse) > 0, can_fetch_more(browse),
                "precondition: the folder list holds more than one page");
            qobject_cast<QAbstractItemModel*>(browse)->fetchMore(QModelIndex());
            QMetaObject::invokeMethod(browse, "setFilter", Q_ARG(QString, QStringLiteral("file-1")));
            break;
        case 2: {
            // Both answers (the further page, then the filtered list) are in.
            const int filtered = rows_of(browse);
            if (filtered <= 0 || can_fetch_more(browse) || filtered >= 200) return;
            QMetaObject::invokeMethod(browse, "setFilter", Q_ARG(QString, QString()));
            break;
        }
        case 3:
            if (rows_of(browse) < 200 || draft->property("testing").toBool() || !answered(answers, *dry_run)) return;
            SMOKE_CHECK("M6", 123, !draft->property("connectionVerified").toBool(),
                "(a) a connection test answered for the old location marks the changed form as tested");
            SMOKE_CHECK("M6", 124, answer_of(answers, *dry_run).isEmpty(),
                "(b) the dry run nobody follows was handed to the window");
            qobject_cast<QAbstractItemModel*>(browse)->fetchMore(QModelIndex());
            break;
        case 4:
            // The timeout fails the scene when no further page arrives.
            if (rows_of(browse) <= 200) return;
            // (d) A search hit beyond the first page of its folder.
            QMetaObject::invokeMethod(window, "openSearchHit", Q_ARG(QVariant, dest), Q_ARG(QVariant, snapshot),
                Q_ARG(QVariant, source), Q_ARG(QVariant, source + QStringLiteral("/file-99")), Q_ARG(QVariant, false),
                Q_ARG(QVariant, QString()));
            break;
        case 5: {
            QQuickItem* list = child_item(window, "browseListView");
            if (!list || list->property("currentIndex").toInt() < 200) return;
            auto* rows = qobject_cast<QAbstractItemModel*>(list->property("model").value<QObject*>());
            SMOKE_CHECK2("M6", 125, rows, rows->data(rows->index(list->property("currentIndex").toInt(), 0), 0).toString()
                    == source + QStringLiteral("/file-99"),
                QStringLiteral("(d) the current row %1 of %2 (model %3) is '%4', not the search hit")
                    .arg(list->property("currentIndex").toInt())
                    .arg(list->property("count").toInt())
                    .arg(rows ? rows->rowCount() : -1)
                    .arg(rows ? rows->data(rows->index(list->property("currentIndex").toInt(), 0), 0).toString() : QString()));
            // (e) The next hit, left for a filter before its page arrived.
            QMetaObject::invokeMethod(window, "popSubPage");
            break;
        }
        case 6:
            if (child_item(window, "restoreBrowsePage")) return;
            QMetaObject::invokeMethod(window, "openSearchHit", Q_ARG(QVariant, dest), Q_ARG(QVariant, snapshot),
                Q_ARG(QVariant, source), Q_ARG(QVariant, source + QStringLiteral("/file-98")), Q_ARG(QVariant, false),
                Q_ARG(QVariant, QString()));
            break;
        case 7: {
            QQuickItem* page = child_item(window, "restoreBrowsePage");
            if (!page || page->property("focusPages").toInt() == 0) return;
            QMetaObject::invokeMethod(child_object(window, "browseFilterField"), "filterRequested",
                Q_ARG(QString, QStringLiteral("file-1")));
            settle->start();
            break;
        }
        case 8: {
            QQuickItem* page = child_item(window, "restoreBrowsePage");
            QQuickItem* list = child_item(window, "browseListView");
            // The filtered list and the page the hit asked for are in.
            if (!list || list->property("count").toInt() >= 200 || settle->elapsed() < 12000) return;
            poll->stop();
            SMOKE_CHECK("M6", 126, page->property("focusPath").toString().isEmpty(),
                "(e) a hit the user left for a filter is still looked for");
            smoke_finish("M6", warnings.get(), window, out_path);
            return;
        }
        }
        ++*step;
    });
    poll->start(250);
}

// AC-M6-33-1: every reply comes 5 s late. (a) The previews, exclude helpers, source checks,
// connection test, restic source and the snapshot query of a folder list return at once and the
// main thread never pauses longer than 100 ms; (b) their answers arrive.
void run_slow_queries_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    const QString bid = qEnvironmentVariable("KESTIC_TEST_BACKUP");
    const QString dest = qEnvironmentVariable("KESTIC_TEST_DEST");
    const QString snapshot = qEnvironmentVariable("KESTIC_TEST_SNAPSHOT");
    const QString source = qEnvironmentVariable("KESTIC_TEST_SOURCE");
    const QString probe = qEnvironmentVariable("KESTIC_TEST_PROBE");
    SMOKE_CHECK("M6", 100, !bid.isEmpty() && !dest.isEmpty() && !snapshot.isEmpty() && !source.isEmpty() && !probe.isEmpty(),
        "precondition: backup, destination, snapshot and paths");
    QObject* answers = create_in_window(window, ANSWERS_QML);
    SMOKE_CHECK("M6", 100, answers, "precondition: the answer counter");
    auto stop = std::make_shared<bool>(false);
    auto building = std::make_shared<bool>(false);
    auto largest_gap = watch_main_thread_gaps(window, stop, building);
    auto requests = std::make_shared<QMap<QString, int>>();
    auto browse = std::make_shared<QObject*>(nullptr);
    auto step = std::make_shared<int>(-1);
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    const QString excludes = QStringLiteral(R"({"folders":["/tmp"],"patterns":"*.tmp\n","exclude_caches":true})");
    const QString retention = QStringLiteral(R"({"keep_last":3})");
    const QString fields = QString::fromUtf8(QJsonDocument(QJsonObject{{"kind", "local"}, {"name", "Probe"},
        {"location", probe}, {"username", ""}, {"endpoint", ""}, {"bucket", ""}, {"path", ""}, {"region", ""},
        {"host", ""}, {"port", 22}, {"env", QJsonArray()}}).toJson(QJsonDocument::Compact));
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (waited->elapsed() > 120000) {
            poll->stop();
            smoke_fail("M6", 101, QStringLiteral("timed out at step %1").arg(*step));
            return;
        }
        if (*step < 0) {
            if (backend->property("daemon_reachable").toBool() && !backup_id_at(window, 0).isEmpty()) {
                *step = 0;
            }
            return;
        }
        // One call per tick, so each pause of the main thread belongs to one call.
        bool ok = true;
        auto ask = [&](int code, const char* method, const std::function<int()>& call) {
            ok = requests_at_once(code, method, [&]() {
                const int request = call();
                requests->insert(QLatin1String(method), request);
                return request;
            });
        };
        switch ((*step)++) {
        case 0:
            ask(102, "lintExcludes", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "lintExcludes", Q_RETURN_ARG(int, r), Q_ARG(QString, excludes));
                return r;
            });
            break;
        case 1:
            ask(103, "testExcludePath", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "testExcludePath", Q_RETURN_ARG(int, r), Q_ARG(QString, bid),
                    Q_ARG(QString, excludes), Q_ARG(QString, QStringLiteral("/tmp/a.tmp")));
                return r;
            });
            break;
        case 2:
            ask(104, "formatExcludeRuleForItem", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "formatExcludeRuleForItem", Q_RETURN_ARG(int, r),
                    Q_ARG(QString, source + QStringLiteral("/file.txt")));
                return r;
            });
            break;
        case 3:
            ask(105, "detectSource", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "detectSource", Q_RETURN_ARG(int, r), Q_ARG(QString, source));
                return r;
            });
            break;
        case 4:
            ask(106, "checkNestedSources", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "checkNestedSources", Q_RETURN_ARG(int, r), Q_ARG(QString, QString()),
                    Q_ARG(QString, source), Q_ARG(QString, dest));
                return r;
            });
            break;
        case 5:
            ask(107, "startBackupDryRun", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "startBackupDryRun", Q_RETURN_ARG(int, r), Q_ARG(QString, bid),
                    Q_ARG(QString, QString()));
                return r;
            });
            break;
        case 6:
            ask(108, "startRetentionPreview", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "startRetentionPreview", Q_RETURN_ARG(int, r), Q_ARG(QString, bid),
                    Q_ARG(QString, retention));
                return r;
            });
            break;
        case 7:
            ask(109, "testDestination", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "testDestination", Q_RETURN_ARG(int, r), Q_ARG(QString, fields),
                    Q_ARG(QString, QStringLiteral("init")), Q_ARG(QString, QStringLiteral("probe-password")),
                    Q_ARG(QString, QStringLiteral("{}")));
                return r;
            });
            break;
        case 8:
            ask(110, "applyResticSource", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "applyResticSource", Q_RETURN_ARG(int, r),
                    Q_ARG(QString, QStringLiteral("automatic")), Q_ARG(QString, QString()));
                return r;
            });
            break;
        case 9:
            // Making the model is QML work, not a wait for the daemon.
            *building = true;
            *browse = create_in_window(window, BROWSE_QML);
            SMOKE_CHECK("M6", 111, *browse, "the folder list model could not be made");
            break;
        case 10: {
            QElapsedTimer took;
            took.start();
            QMetaObject::invokeMethod(*browse, "open", Q_ARG(QString, dest), Q_ARG(QString, snapshot),
                Q_ARG(QString, QStringLiteral("/")));
            SMOKE_CHECK("M6", 111, took.elapsed() < 100,
                QStringLiteral("(a) opening a folder of a snapshot kept the main thread for %1 ms").arg(took.elapsed()));
            break;
        }
        default: {
            // The folder list polls its query like the restore dialog does.
            if ((*browse)->property("state").toString() == QLatin1String("running")) {
                QElapsedTimer took;
                took.start();
                QMetaObject::invokeMethod(*browse, "poll");
                SMOKE_CHECK("M6", 112, took.elapsed() < 100,
                    QStringLiteral("(a) polling the folder list kept the main thread for %1 ms").arg(took.elapsed()));
                return;
            }
            bool all = true;
            for (auto it = requests->cbegin(); it != requests->cend(); ++it) {
                all = all && answered(answers, it.value());
            }
            if (!all) return;
            poll->stop();
            *stop = true;
            SMOKE_CHECK("M6", 113, *largest_gap <= 100,
                QStringLiteral("(a) the main thread paused %1 ms while the daemon was slow").arg(*largest_gap));
            SMOKE_CHECK("M6", 114, !answer_of(answers, requests->value(QStringLiteral("startBackupDryRun"))).isEmpty(),
                "(b) the dry run never answered with its id");
            SMOKE_CHECK("M6", 114, !answer_of(answers, requests->value(QStringLiteral("detectSource"))).isEmpty(),
                "(b) the source check never answered");
            auto* listed = qobject_cast<QAbstractItemModel*>(*browse);
            SMOKE_CHECK3("M6", 115, (*browse)->property("state").toString() == QLatin1String("done"), listed,
                listed->rowCount() > 0, QStringLiteral("(b) the folder list of the snapshot never arrived: state '%1', error '%2'")
                    .arg((*browse)->property("state").toString(), (*browse)->property("error_text").toString()));
            smoke_finish("M6", warnings.get(), window, out_path);
            return;
        }
        }
        if (!ok) {
            poll->stop();
        }
    });
    poll->start(50);
}

} // namespace kestic::smoke
