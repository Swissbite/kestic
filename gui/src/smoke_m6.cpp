// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_m6.h"
#include "src/smoke_helper.h"
#include "src/smoke_m3_util.h"
#include <QAbstractItemModel>
#include <QFile>
#include <QJSValue>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQmlExpression>

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

const char* const DIFF_QML = R"(
import net.daester.kestic 1.0
SnapshotDiffModel {}
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

// `text` as a JavaScript string literal.
QString js_text(const QString& text) {
    return QString::fromUtf8(QJsonDocument(QJsonArray{text}).toJson(QJsonDocument::Compact)).mid(1).chopped(1);
}

// The row of the search hit at `path` on the search page, -1 while it is not listed.
int search_hit_row(QQuickWindow* window, const QString& path) {
    auto* hits = qobject_cast<QAbstractItemModel*>(child_object(window, "snapshotSearchModel"));
    for (int row = 0; hits && row < hits->rowCount(); ++row) {
        if (hits->data(hits->index(row, 0), 0).toString() == path) return row;
    }
    return -1;
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
// arrived; (e) a hit the user left for a filter before its page arrived is no longer looked for;
// (f) a hit above the snapshot's top folder is still found after the page checked that folder.
void run_slow_stale_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    const QString bid = qEnvironmentVariable("KESTIC_TEST_BACKUP");
    const QString dest = qEnvironmentVariable("KESTIC_TEST_DEST");
    const QString snapshot = qEnvironmentVariable("KESTIC_TEST_SNAPSHOT");
    const QString source = qEnvironmentVariable("KESTIC_TEST_SOURCE");
    const QString probe = qEnvironmentVariable("KESTIC_TEST_PROBE");
    SMOKE_CHECK("M6", 1900, !bid.isEmpty() && !dest.isEmpty() && !snapshot.isEmpty() && !source.isEmpty() && !probe.isEmpty(),
        "precondition: backup, destination, snapshot and paths");
    QObject* answers = create_in_window(window, ANSWERS_QML);
    QObject* draft = create_in_window(window, DRAFT_QML);
    QObject* browse = create_in_window(window, BROWSE_QML);
    SMOKE_CHECK3("M6", 1901, answers, draft, browse, "precondition: the QML helpers");
    auto step = std::make_shared<int>(-1);
    auto dry_run = std::make_shared<int>(0);
    auto settle = std::make_shared<QElapsedTimer>();
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (waited->elapsed() > 240000) {
            poll->stop();
            smoke_fail("M6", 1902, *step == 4 ? QStringLiteral("(c) the list loads no further pages after a further page arrived for an old filter")
                : *step == 6 ? QStringLiteral("(d) a search hit beyond the first page of its folder never became the current row")
                : *step == 11 ? QStringLiteral("(f) a hit above the snapshot's top folder never became the current row")
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
            SMOKE_CHECK("M6", 1903, draft->property("testing").toBool(), "precondition: the connection test runs");
            draft->setProperty("location", probe + QStringLiteral("/b"));
            SMOKE_CHECK("M6", 1904, !draft->property("testing").toBool(),
                "(a) after the location changed the form still shows the test of the old location");
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
            SMOKE_CHECK2("M6", 1905, rows_of(browse) > 0, can_fetch_more(browse),
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
            // The old connection test has answered (its request is no longer waited for).
            if (rows_of(browse) < 200 || draft->property("testAnswer").value<QObject*>()->property("waiting").toBool()
                || !answered(answers, *dry_run)) {
                return;
            }
            SMOKE_CHECK("M6", 1906, !draft->property("connectionVerified").toBool(),
                "(a) a connection test answered for the old location marks the changed form as tested");
            SMOKE_CHECK("M6", 1907, answer_of(answers, *dry_run).isEmpty(),
                "(b) the dry run nobody follows was handed to the window");
            qobject_cast<QAbstractItemModel*>(browse)->fetchMore(QModelIndex());
            break;
        case 4:
            // The timeout fails the scene when no further page arrives.
            if (rows_of(browse) <= 200) return;
            // (d) The way a user gets to a hit: the restore area, the backup's snapshots, a
            // search, then [Show] on the hit beyond the first page of its folder.
            QMetaObject::invokeMethod(window, "openRestoreSnapshots", Q_ARG(QVariant, bid));
            QMetaObject::invokeMethod(window, "openSnapshotSearch", Q_ARG(QVariant, bid),
                Q_ARG(QVariant, QStringLiteral("file-9")));
            break;
        case 5: {
            const int hit = search_hit_row(window, source + QStringLiteral("/file-99"));
            if (hit < 0) return;
            QMetaObject::invokeMethod(child_object(window, "restoreSearchPage"), "showHit", Q_ARG(QVariant, hit));
            break;
        }
        case 6: {
            QQuickItem* list = child_item(window, "browseListView");
            if (!list || list->property("currentIndex").toInt() < 200) return;
            auto* rows = qobject_cast<QAbstractItemModel*>(list->property("model").value<QObject*>());
            SMOKE_CHECK2("M6", 1908, rows, rows->data(rows->index(list->property("currentIndex").toInt(), 0), 0).toString()
                    == source + QStringLiteral("/file-99"),
                QStringLiteral("(d) the current row %1 is '%2', not the search hit")
                    .arg(list->property("currentIndex").toInt())
                    .arg(rows ? rows->data(rows->index(list->property("currentIndex").toInt(), 0), 0).toString() : QString()));
            SMOKE_CHECK("M6", 1909, !child_item(window, "restoreBrowsePage")->property("title").toString().isEmpty(),
                "the snapshot browser has no title for its breadcrumb");
            // (e) The next hit, left for a filter before its page arrived.
            QMetaObject::invokeMethod(window, "popSubPage");
            break;
        }
        case 7: {
            if (child_item(window, "restoreBrowsePage")) return;
            const int hit = search_hit_row(window, source + QStringLiteral("/file-98"));
            SMOKE_CHECK("M6", 1910, hit >= 0, "precondition: the hit file-98");
            QMetaObject::invokeMethod(child_object(window, "restoreSearchPage"), "showHit", Q_ARG(QVariant, hit));
            break;
        }
        case 8: {
            QQuickItem* page = child_item(window, "restoreBrowsePage");
            if (!page || page->property("focusPages").toInt() == 0) return;
            QMetaObject::invokeMethod(child_object(window, "browseFilterField"), "filterRequested",
                Q_ARG(QString, QStringLiteral("file-1")));
            settle->start();
            break;
        }
        case 9: {
            QQuickItem* page = child_item(window, "restoreBrowsePage");
            QQuickItem* list = child_item(window, "browseListView");
            // The filtered list and the page the hit asked for are in.
            if (!list || list->property("count").toInt() >= 200 || settle->elapsed() < 12000) return;
            SMOKE_CHECK("M6", 1911, page->property("focusPath").toString().isEmpty(),
                "(e) a hit the user left for a filter is still looked for");
            QMetaObject::invokeMethod(window, "popSubPage");
            break;
        }
        case 10: {
            if (child_item(window, "restoreBrowsePage")) return;
            // (f) A hit that is a folder above the top folder. In the app the top folder is the
            // snapshot's start folder, and the folders above it hold one entry each, so the scene
            // opens the browser as [Show] does but names a deeper top folder (`sub/deeper`): the
            // hit `sub` is then beyond the first page of the start folder.
            auto* hits = qobject_cast<QAbstractItemModel*>(child_object(window, "snapshotSearchModel"));
            const int any = search_hit_row(window, source + QStringLiteral("/file-99"));
            SMOKE_CHECK2("M6", 1912, hits, any >= 0, "precondition: a hit with its snapshot text");
            const QString snapshot_text = hits->data(hits->index(any, 0), 2).toString();
            QQmlExpression open(qmlContext(window), window, QStringLiteral(
                "pushSubPageAtDepth(restoreBrowseComponent.createObject(pageStack, {destinationId: %1, snapshotId: %2,"
                " startDir: %3, topDir: %4, focusPath: %5, focusIsDir: true, snapshotTitle: %6}), 3)")
                .arg(js_text(dest), js_text(snapshot), js_text(source), js_text(source + QStringLiteral("/sub/deeper")),
                    js_text(source + QStringLiteral("/sub")), js_text(snapshot_text)));
            open.evaluate();
            SMOKE_CHECK("M6", 1913, !open.hasError(), open.error().toString());
            break;
        }
        case 11: {
            QQuickItem* list = child_item(window, "browseListView");
            const int row = list ? list->property("currentIndex").toInt() : -1;
            if (row < 200) return;
            poll->stop();
            auto* rows = qobject_cast<QAbstractItemModel*>(list->property("model").value<QObject*>());
            SMOKE_CHECK2("M6", 1914, rows, rows->data(rows->index(row, 0), 0).toString() == source + QStringLiteral("/sub"),
                "(f) the current row is not the hit above the snapshot's top folder");
            SMOKE_CHECK("M6", 1915, !child_item(window, "restoreBrowsePage")->property("title").toString().isEmpty(),
                "the snapshot browser has no title for its breadcrumb");
            smoke_finish("M6", warnings.get(), window, out_path);
            return;
        }
        }
        ++*step;
    });
    poll->start(250);
}

// AC-M6-33-1: every reply comes 5 s late. (a) The previews, exclude helpers, source checks,
// connection test, restic source and the snapshot queries of a folder list and a comparison
// return at once and the main thread never pauses longer than 100 ms; (b) their answers arrive,
// the comparison with its kinds of change and without a later reset of its list.
void run_slow_queries_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    const QString bid = qEnvironmentVariable("KESTIC_TEST_BACKUP");
    const QString dest = qEnvironmentVariable("KESTIC_TEST_DEST");
    const QString snapshot = qEnvironmentVariable("KESTIC_TEST_SNAPSHOT");
    const QString later = qEnvironmentVariable("KESTIC_TEST_SNAPSHOT_B");
    const QString source = qEnvironmentVariable("KESTIC_TEST_SOURCE");
    const QString probe = qEnvironmentVariable("KESTIC_TEST_PROBE");
    SMOKE_CHECK("M6", 100, !bid.isEmpty() && !dest.isEmpty() && !snapshot.isEmpty() && !later.isEmpty()
            && !source.isEmpty() && !probe.isEmpty(),
        "precondition: backup, destination, snapshots and paths");
    QObject* answers = create_in_window(window, ANSWERS_QML);
    SMOKE_CHECK("M6", 1916, answers, "precondition: the answer counter");
    auto stop = std::make_shared<bool>(false);
    auto building = std::make_shared<bool>(false);
    auto largest_gap = watch_main_thread_gaps(window, stop, building);
    auto requests = std::make_shared<QMap<QString, int>>();
    auto browse = std::make_shared<QObject*>(nullptr);
    auto diff = std::make_shared<QObject*>(nullptr);
    // Resets of the comparison's list, and their number once it was done.
    auto diff_resets = std::make_shared<int>(0);
    auto resets_when_done = std::make_shared<int>(-1);
    auto since_done = std::make_shared<QElapsedTimer>();
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
            smoke_fail("M6", 1917, QStringLiteral("timed out at step %1").arg(*step));
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
            ask(10903, "lintExcludes", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "lintExcludes", Q_RETURN_ARG(int, r), Q_ARG(QString, excludes));
                return r;
            });
            break;
        case 1:
            ask(10904, "testExcludePath", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "testExcludePath", Q_RETURN_ARG(int, r), Q_ARG(QString, bid),
                    Q_ARG(QString, excludes), Q_ARG(QString, QStringLiteral("/tmp/a.tmp")));
                return r;
            });
            break;
        case 2:
            ask(10905, "formatExcludeRuleForItem", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "formatExcludeRuleForItem", Q_RETURN_ARG(int, r),
                    Q_ARG(QString, source + QStringLiteral("/file.txt")));
                return r;
            });
            break;
        case 3:
            ask(10906, "detectSource", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "detectSource", Q_RETURN_ARG(int, r), Q_ARG(QString, source));
                return r;
            });
            break;
        case 4:
            ask(10907, "checkNestedSources", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "checkNestedSources", Q_RETURN_ARG(int, r), Q_ARG(QString, QString()),
                    Q_ARG(QString, source), Q_ARG(QString, dest));
                return r;
            });
            break;
        case 5:
            ask(10908, "startBackupDryRun", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "startBackupDryRun", Q_RETURN_ARG(int, r), Q_ARG(QString, bid),
                    Q_ARG(QString, QString()));
                return r;
            });
            break;
        case 6:
            ask(10909, "startRetentionPreview", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "startRetentionPreview", Q_RETURN_ARG(int, r), Q_ARG(QString, bid),
                    Q_ARG(QString, retention));
                return r;
            });
            break;
        case 7:
            ask(10910, "testDestination", [&]() {
                int r = 0;
                QMetaObject::invokeMethod(backend, "testDestination", Q_RETURN_ARG(int, r), Q_ARG(QString, fields),
                    Q_ARG(QString, QStringLiteral("init")), Q_ARG(QString, QStringLiteral("probe-password")),
                    Q_ARG(QString, QStringLiteral("{}")));
                return r;
            });
            break;
        case 8:
            ask(10911, "applyResticSource", [&]() {
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
            *diff = create_in_window(window, DIFF_QML);
            SMOKE_CHECK2("M6", 1918, *browse, *diff, "the list models could not be made");
            QObject::connect(qobject_cast<QAbstractItemModel*>(*diff), &QAbstractItemModel::modelReset, window,
                [diff_resets]() { ++*diff_resets; });
            break;
        case 10: {
            QElapsedTimer took;
            took.start();
            QMetaObject::invokeMethod(*browse, "open", Q_ARG(QString, dest), Q_ARG(QString, snapshot),
                Q_ARG(QString, QStringLiteral("/")));
            SMOKE_CHECK("M6", 1919, took.elapsed() < 100,
                QStringLiteral("(a) opening a folder of a snapshot kept the main thread for %1 ms").arg(took.elapsed()));
            took.restart();
            QMetaObject::invokeMethod(*diff, "compare", Q_ARG(QString, dest), Q_ARG(QString, snapshot), Q_ARG(QString, later));
            SMOKE_CHECK("M6", 1920, took.elapsed() < 100,
                QStringLiteral("(a) starting a comparison kept the main thread for %1 ms").arg(took.elapsed()));
            break;
        }
        default: {
            // The lists poll their queries like their pages do.
            bool running = false;
            for (QObject* model : {*browse, *diff}) {
                if (model->property("state").toString() != QLatin1String("running")) continue;
                running = true;
                QElapsedTimer took;
                took.start();
                QMetaObject::invokeMethod(model, "poll");
                SMOKE_CHECK("M6", 1921, took.elapsed() < 100,
                    QStringLiteral("(a) polling a list kept the main thread for %1 ms").arg(took.elapsed()));
            }
            if (running) return;
            bool all = true;
            for (auto it = requests->cbegin(); it != requests->cend(); ++it) {
                all = all && answered(answers, it.value());
            }
            if (!all) return;
            // The comparison's list, once shown, is not reset again by a late first page (one
            // reply delay later).
            if (*resets_when_done < 0) {
                *resets_when_done = *diff_resets;
                since_done->start();
                return;
            }
            if (since_done->elapsed() < 6000) return;
            poll->stop();
            SMOKE_CHECK("M6", 1922, *diff_resets == *resets_when_done,
                QStringLiteral("(b) the comparison's list was reset %1 more times after it was shown")
                    .arg(*diff_resets - *resets_when_done));
            *stop = true;
            SMOKE_CHECK("M6", 1923, *largest_gap <= 100,
                QStringLiteral("(a) the main thread paused %1 ms while the daemon was slow").arg(*largest_gap));
            SMOKE_CHECK("M6", 1924, !answer_of(answers, requests->value(QStringLiteral("startBackupDryRun"))).isEmpty(),
                "(b) the dry run never answered with its id");
            SMOKE_CHECK("M6", 1925, !answer_of(answers, requests->value(QStringLiteral("detectSource"))).isEmpty(),
                "(b) the source check never answered");
            auto* listed = qobject_cast<QAbstractItemModel*>(*browse);
            SMOKE_CHECK3("M6", 1926, (*browse)->property("state").toString() == QLatin1String("done"), listed,
                listed->rowCount() > 0, QStringLiteral("(b) the folder list of the snapshot never arrived: state '%1', error '%2'")
                    .arg((*browse)->property("state").toString(), (*browse)->property("error_text").toString()));
            // The comparison shows its list and the chips of its kinds of change at once.
            SMOKE_CHECK2("M6", 1927, (*diff)->property("state").toString() == QLatin1String("done"),
                !(*diff)->property("kind_codes").toStringList().isEmpty(),
                QStringLiteral("(b) the comparison is done without its kinds of change (state '%1', kinds '%2')")
                    .arg((*diff)->property("state").toString(), (*diff)->property("kind_codes").toStringList().join(QLatin1Char(','))));
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

namespace {

// Scrolls the page `page_name` so that `item` is in view (the narrow window does not show all of
// it).
void show_on_page(QQuickWindow* w, const char* page_name, QQuickItem* item) {
    QQuickItem* page = m3::lookup(w, page_name);
    auto* flick = page ? qvariant_cast<QQuickItem*>(page->property("flickable")) : nullptr;
    auto* content = flick ? qvariant_cast<QQuickItem*>(flick->property("contentItem")) : nullptr;
    if (!content) {
        return;
    }
    const double top = item->mapToItem(content, QPointF(0, 0)).y();
    const double max_y = std::max(0.0, flick->property("contentHeight").toDouble() - flick->height());
    flick->setProperty("contentY", std::clamp(top - 20.0, 0.0, max_y));
    wait_ms(300); // quiet window: the page repaints at its new position
}

// The other screenshot of a scene: `out_path` with `from` replaced by `to`.
QString other_shot(const QString& out_path, const char* from, const char* to) {
    QString shot = out_path;
    shot.replace(QLatin1String(from), QLatin1String(to));
    return shot;
}

// AC-M6-18-2 in the form `prefix` (`wizDest` or `destEdit`): the address `login_url` shows
// `message` below the field, and «Test Connection» and `locked` (Next or Save) are disabled; the
// same address without the login (`plain_url`) enables «Test Connection» first.
int check_rest_login_form(QQuickWindow* w, const char* prefix, const char* page, QObject* locked,
    const QString& plain_url, const QString& login_url, const QString& message, int code) {
    using namespace m3;
    const QByteArray field_name = QByteArray(prefix) + "LocationField";
    const QByteArray error_name = QByteArray(prefix) + "LocationError";
    const QByteArray test_name = QByteArray(prefix) + "TestButton";
    QQuickItem* field = lookup(w, field_name.constData());
    QQuickItem* test = lookup(w, test_name.constData());
    CHECK2(code, field != nullptr, test != nullptr, "address field or «Test Connection» missing");
    field->setProperty("text", plain_url);
    CHECK(code + 1, wait_until([&] { return test->isEnabled(); }, 2000), "«Test Connection» not enabled without a login");
    CHECK(code + 2, count_visible(w, error_name.constData()) == 0, "address error shown without a login");
    field->setProperty("text", login_url);
    QQuickItem* error = lookup(w, error_name.constData());
    CHECK(code + 3, wait_until([&] { return error && error->isVisible(); }, 2000), "(a) no message below the address");
    CHECK_EQ(code + 4, error->property("text").toString(), message, "(a) the message of the daemon");
    CHECK(code + 5, !test->isEnabled(), "(b) «Test Connection» enabled");
    CHECK(code + 6, locked != nullptr && !locked->property("enabled").toBool(), "(b) «Next» or «Save» enabled");
    show_on_page(w, page, error);
    return 0;
}

int rest_login_checks(QQuickWindow* w, const QString& lang, const QString& out_path) {
    using namespace m3;
    const QString login_url = qEnvironmentVariable("KESTIC_SMOKE_REST_LOGIN_URL");
    const QString plain_url = qEnvironmentVariable("KESTIC_SMOKE_REST_URL");
    const QString dest_id = qEnvironmentVariable("KESTIC_SMOKE_DEST_ID");
    CHECK(1800, !login_url.isEmpty() && !plain_url.isEmpty() && !dest_id.isEmpty(), "KESTIC_SMOKE_* not set");
    const QString message = loc(lang,
        "The address must not contain a user name or password. Enter them in the ‘Username’ and ‘Server password’ fields.",
        "Benutzername und Passwort gehören nicht in die Adresse, sondern in die Felder «Benutzername» und «Server-Passwort».");

    QMetaObject::invokeMethod(w, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QStringLiteral("init")));
    QQuickItem* card = nullptr;
    CHECK(1801, wait_until([&] { return (card = lookup(w, "wizDestTypeCard_rest")) != nullptr; }, 3000), "REST card missing");
    wait_ms(250); // quiet window: the wizard ends its transition
    finish_incubation(w);
    show_on_page(w, "addDestinationWizard", card);
    CHECK(1802, click_ui_item(w, card, "REST card"), "click on the REST card failed");
    QQuickItem* next = lookup(w, "wizDestNextButton");
    CHECK(1803, wait_until([&] { return next && next->isEnabled(); }, 2000), "«Next» not enabled after choosing REST");
    CHECK(1804, click_ui_item(w, next, "Next"), "click on «Next» failed");
    CHECK(1805, wait_until([&] { return count_visible(w, "wizDestLocationField") == 1; }, 2000), "REST form did not open");
    m3::lookup(w, "wizDestNameField")->setProperty("text", QStringLiteral("NAS"));
    if (const int rc = check_rest_login_form(w, "wizDest", "addDestinationWizard", next, plain_url, login_url, message, 1810)) {
        return rc;
    }
    CHECK(1820, save_screenshot(w, out_path), "screenshot failed");
    QMetaObject::invokeMethod(w, "closeWizardLayer");
    wait_until([&] { return !w->property("inWizardLayer").toBool(); }, 250);

    QMetaObject::invokeMethod(w, "openDestinationEditPage", Q_ARG(QVariant, dest_id));
    CHECK(1821, wait_until([&] { return count_visible(w, "destEditLocationField") == 1; }, 3000), "edit page did not open");
    wait_ms(150); // quiet window: the page ends its transition
    QObject* save = lookup<QObject>(w, "saveDestinationAction");
    if (const int rc = check_rest_login_form(w, "destEdit", "destinationEditPage", save, plain_url, login_url, message, 1830)) {
        return rc;
    }
    CHECK(1840, save_screenshot(w, other_shot(out_path, "-wizard-", "-edit-")), "screenshot failed");
    return 0;
}

} // namespace

void run_rest_login_scene(QQuickWindow* window, const std::shared_ptr<QStringList>& warnings,
    const QString& out_path, const QString& lang) {
    if (const int rc = rest_login_checks(window, lang, out_path)) {
        smoke_fail("M6", rc, QStringLiteral("AC-M6-18-2 failed with code %1, see stderr").arg(rc));
        return;
    }
    smoke_finish("M6", warnings.get());
}

// AC-M6-33-1: «Back Up Now» and «Back Up All Now» answered late (the test holds StartBackup and
// StartAllBackups back 30 s) do not hold back pausing that backup or all backups.
void run_slow_start_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    wait_then(window, 30000, 10901, QStringLiteral("the state never arrived"),
        [=]() { return backend->property("daemon_reachable").toBool(); },
        [=]() {
            const QString bid = backup_id_at(window, 0);
            QMetaObject::invokeMethod(backend, "startBackup", Q_ARG(QString, bid));
            QMetaObject::invokeMethod(backend, "startAllBackups");
            QMetaObject::invokeMethod(backend, "pauseBackup", Q_ARG(QString, bid), Q_ARG(QString, QStringLiteral("1h")));
            QMetaObject::invokeMethod(backend, "pauseGlobal", Q_ARG(QString, QStringLiteral("1h")));
            wait_then(window, 5000, 10902, QStringLiteral("pausing waited for the slow starts"),
                [=]() {
                    return backend->property("global_paused").toBool()
                        && role_value(model_named(window, "backupsModel"), 0, "isPaused").toBool();
                },
                [=]() { smoke_finish("M6", warnings.get(), window, out_path); });
        });
}

// AC-M6-33-1: «Back Up Now» before the first state arrived, for a backup whose source is gone:
// the refusal names the backup and the reason the daemon gave.
void run_refused_start_scene(QQuickWindow* window, QObject* backend,
    const std::shared_ptr<QStringList>& warnings, const QString& out_path) {
    const QString bid = qEnvironmentVariable("KESTIC_TEST_BACKUP_ID");
    SMOKE_CHECK("M6", 662, !bid.isEmpty(), "precondition: backup id");
    QMetaObject::invokeMethod(backend, "startBackup", Q_ARG(QString, bid));
    wait_then(window, 30000, 10900, QStringLiteral("the refusal never arrived"),
        [=]() { return !backend->property("manual_run_blocked_message").toString().isEmpty(); },
        [=]() {
            const QString shown = backend->property("manual_run_blocked_message").toString();
            if (shown != QStringLiteral("Cannot back up 'Documents': source is unavailable.")) {
                smoke_fail("M6", 663, QStringLiteral("the refusal reads '%1'").arg(shown));
                return;
            }
            smoke_finish("M6", warnings.get(), window, out_path);
        });
}

// AC-M6-33-4 (a): the test changes the daemon state; the window shows it within 1 s, with no
// timer of its own. The scene writes `<marker>.ready` when it is up; the test writes
// `<marker>.go` and then changes expert mode over D-Bus.
void run_state_change_scene(
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& out_path
) {
    const QString marker = qEnvironmentVariable("KESTIC_TEST_STATE_CHANGE_MARKER");
    SMOKE_CHECK("M6", 30, !marker.isEmpty(), "precondition: marker path");
    const bool before = backend->property("expert_mode").toBool();
    QFile ready(marker + QStringLiteral(".ready"));
    SMOKE_CHECK("M6", 31, ready.open(QIODevice::WriteOnly), "write the ready marker");
    ready.close();
    auto since_go = std::make_shared<QElapsedTimer>();
    auto waited = std::make_shared<QElapsedTimer>();
    waited->start();
    auto* poll = new QTimer(window);
    QObject::connect(poll, &QTimer::timeout, window, [=]() {
        if (!since_go->isValid()) {
            if (QFile::exists(marker + QStringLiteral(".go"))) {
                since_go->start();
            } else if (waited->elapsed() > 60000) {
                poll->stop();
                smoke_fail("M6", 32, QStringLiteral("the test never changed the daemon state"));
            }
            return;
        }
        if (backend->property("expert_mode").toBool() != before) {
            poll->stop();
            const qint64 ms = since_go->elapsed();
            SMOKE_CHECK("M6", 33, ms <= 1000,
                QStringLiteral("(a) the window showed the change after %1 ms").arg(ms));
            smoke_finish("M6", warnings.get(), window, out_path);
        } else if (since_go->elapsed() > 10000) {
            poll->stop();
            smoke_fail("M6", 34, QStringLiteral("(a) the change never reached the window"));
        }
    });
    poll->start(20);
}

} // namespace kestic::smoke
