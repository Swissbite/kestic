// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Opens every page, editor, wizard and the settings window once, saves a screenshot of each
// and writes all visible texts to the file named by `KESTIC_SMOKE_TEXT_DUMP` (one line per
// text: `page<TAB>text`, line breaks as `\n`). The Rust test checks these texts in both
// languages (`daemon/tests/m3_gui_smoke.rs`).

#include "src/smoke_m3_util.h"

#include <QElapsedTimer>
#include <QFileInfo>
#include <QKeySequence>
#include <QQmlEngine>
#include <QQmlIncubationController>
#include <QTextStream>

namespace kestic::smoke {

namespace {

using namespace m3;

void collect_texts(QQuickItem* root, QStringList& out) {
    QList<QQuickItem*> items;
    collect_visual_descendants(root, items);
    for (QQuickItem* it : items) {
        if (!it->isVisible()) {
            continue;
        }
        for (const char* prop : { "text", "title", "placeholderText" }) {
            const QVariant value = it->property(prop);
            if (value.typeId() == QMetaType::QString) {
                const QString s = value.toString().trimmed();
                if (!s.isEmpty()) {
                    out << s;
                }
            }
        }
    }
}

struct PageRecord {
    bool has_texts;
    bool saved;
};

struct PageDump {
    QFile* file;
    QString screenshot_dir;
    QString lang;
    int pages = 0;

    /// Records the texts of `window` under `name` and saves a screenshot of it.
    PageRecord record(const QString& name, QQuickWindow* window) {
        wait_ms(300);
        finish_incubation(window);
        QStringList texts;
        collect_texts(window->contentItem(), texts);
        texts.removeDuplicates();
        QTextStream out(file);
        for (QString t : texts) {
            t.replace(QLatin1Char('\n'), QLatin1String("\\n"));
            out << name << '\t' << t << '\n';
        }
        out.flush();
        ++pages;
        const QString shot = screenshot_dir + QStringLiteral("/m3-page-") + name + QLatin1Char('-') + lang + QStringLiteral(".png");
        const bool saved = save_screenshot(window, shot);
        finish_incubation(window);
        return { !texts.isEmpty(), saved };
    }
};

/// Whether the page item called `name` exists in `window` (Kirigami does not keep
/// `visible` set on pages inside its page row, so the visible texts are checked instead).
bool page_visible(QQuickWindow* window, const char* name) {
    return wait_until([&] { return item_named(window, name) != nullptr; }, 3000);
}

/// Opens the backup detail page from the backups tab.
bool open_detail(QQuickWindow* w, const QString& bid) {
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 1));
    wait_ms(150);
    const bool opened = open_backup_detail(w, bid);
    finish_incubation(w);
    return opened;
}

/// Records the texts and a screenshot of `window` as page `name`: both must be there.
#define M3_RECORD(code, dump, name, window, what)                                      \
    do {                                                                               \
        const int record_code_ = (code);                                               \
        const PageRecord recorded_ = (dump).record((name), (window));                  \
        M3_CHECK(record_code_, recorded_.has_texts, what " page without texts");     \
        M3_CHECK(record_code_, recorded_.saved, what " page screenshot not saved");  \
    } while (0)

int scenario_all_pages(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString path = qEnvironmentVariable("KESTIC_SMOKE_TEXT_DUMP");
    M3_CHECK(4901, !path.isEmpty(), "KESTIC_SMOKE_TEXT_DUMP must name the text dump file");
    QFile file(path);
    M3_CHECK(4902, file.open(QIODevice::WriteOnly | QIODevice::Truncate | QIODevice::Text), "cannot write text dump");
    PageDump dump { &file, QFileInfo(c.out_path).absolutePath(), c.lang };

    const QString bid = backup_id_at(w, 0);
    const QString did = destination_id_at(w, 0);
    M3_CHECK2(4903, !bid.isEmpty(), !did.isEmpty(), "needs one backup and one destination");
    const QString bname = role_value(model_named(w, "backupsModel"), 0, "name").toString();

    struct Tab {
        const char* name;
        const char* page;
    };
    const Tab tabs[] = {
        { "overview", "overviewPage" },
        { "backups", "backupsListPage" },
        { "destinations", "destinationsListPage" },
        { "history", "historyPage" },
    };
    for (int i = 0; i < 4; ++i) {
        QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, i));
        M3_CHECK(4910 + i, page_visible(w, tabs[i].page), tabs[i].page);
        M3_RECORD(4915 + i, dump, QString::fromLatin1(tabs[i].name), w, "tab");
        if (i == 1) {
            // AC-M0-04-1: the QML list shows exactly the rows of the Rust list model.
            QObject* list = object_named(w, "backupsListView");
            QAbstractItemModel* backups = model_named(w, "backupsModel");
            M3_CHECK3(4904, list, backups, backups->rowCount() > 0, "backups list or model missing");
            M3_CHECK(4905, list->property("count").toInt() == backups->rowCount(), "list shows other rows than the model");
        }
    }

    M3_CHECK(4919, open_detail(w, bid), "backupDetailPage did not open");
    M3_RECORD(4920, dump, QStringLiteral("backup-detail"), w, "backup detail");

    // Editors open the way a user opens them: a click on their row in the detail page.
    struct Editor {
        const char* name;
        const char* row;
        const char* page;
    };
    const Editor editors[] = {
        { "backup-source", "editSourceRow", "backupSourceEditPage" },
        { "backup-excludes", "editExcludesRow", "backupExcludesEditPage" },
        { "backup-retention", "editRetentionRow", "backupRetentionPage" },
        { "backup-schedule", "editScheduleRow", "backupScheduleEditPage" },
        { "backup-conditions", "editConditionsRow", "backupConditionsEditPage" },
        { "backup-reassign", "editDestinationRow", "backupReassignPage" },
        { "backup-full-config", "viewFullConfigRow", "backupFullConfigPage" },
    };
    int code = 4921;
    for (const Editor& e : editors) {
        M3_CHECK(code++, open_detail(w, bid), "backupDetailPage did not open");
        QQuickItem* row = find_visual_item(w, QString::fromLatin1(e.row));
        M3_CHECK(code, row, e.row);
        M3_CHECK(code++, click_ui_item(w, row, e.row), e.row);
        M3_CHECK(code++, page_visible(w, e.page), e.page);
        M3_RECORD(code++, dump, QString::fromLatin1(e.name), w, "editor");
    }

    M3_CHECK(4960, open_detail(w, bid), "backupDetailPage did not open");
    QMetaObject::invokeMethod(w, "openBackupSnapshotsPage", Q_ARG(QVariant, bid), Q_ARG(QVariant, bname));
    M3_CHECK(4961, page_visible(w, "backupSnapshotsPage"), "backupSnapshotsPage");
    M3_RECORD(4962, dump, QStringLiteral("backup-snapshots"), w, "snapshots");

    struct DestPage {
        const char* name;
        const char* method;
        const char* page;
    };
    const DestPage dest_pages[] = {
        { "destination-detail", nullptr, "destinationDetailPage" },
        { "destination-edit", "openDestinationEditPage", "destinationEditPage" },
        { "destination-maintenance", "openDestinationMaintenancePage", "destinationMaintenancePage" },
    };
    code = 4965;
    for (const DestPage& p : dest_pages) {
        QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 2));
        wait_ms(150);
        QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
        M3_CHECK(code++, page_visible(w, "destinationDetailPage"), "destinationDetailPage");
        wait_ms(200);
        finish_incubation(w);
        if (p.method) {
            QMetaObject::invokeMethod(w, p.method, Q_ARG(QVariant, did));
        }
        M3_CHECK(code++, page_visible(w, p.page), p.page);
        M3_RECORD(code++, dump, QString::fromLatin1(p.name), w, "destination");
    }

    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    wait_ms(150);
    QMetaObject::invokeMethod(w, "openNewBackupWizard", Q_ARG(QVariant, false));
    M3_CHECK(4980, page_visible(w, "newBackupWizard"), "newBackupWizard");
    M3_RECORD(4981, dump, QStringLiteral("new-backup-wizard"), w, "backup wizard");
    QMetaObject::invokeMethod(w, "closeWizardLayer");
    wait_ms(150);
    QMetaObject::invokeMethod(w, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QStringLiteral("init")));
    M3_CHECK(4982, page_visible(w, "addDestinationWizard"), "addDestinationWizard");
    M3_RECORD(4983, dump, QStringLiteral("add-destination-wizard"), w, "destination wizard");
    QMetaObject::invokeMethod(w, "closeWizardLayer");
    wait_ms(150);

    QMetaObject::invokeMethod(w, "openSettingsWindow");
    wait_ms(250);
    auto* settings = qobject_cast<QQuickWindow*>(object_named(w, "settingsWindow"));
    M3_CHECK2(4990, settings != nullptr, settings->isVisible(), "settingsWindow not open");
    M3_RECORD(4991, dump, QStringLiteral("settings"), settings, "settings");
    settings->close();

    file.close();
    M3_CHECK(4995, dump.pages == 19, "expected 19 recorded pages");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    wait_ms(150);
    M3_CHECK(4996, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// AC-M0-11-8: in the backup detail page, Ctrl+Shift+B starts "Back Up Now" for the shown
/// backup, and the action shows the shortcut. Without an open backup (overview) it starts nothing.
int scenario_backup_now_shortcut(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    M3_CHECK(4801, !bid.isEmpty(), "needs one backup");
    QAbstractItemModel* runs = model_named(w, "runsModel");
    M3_CHECK(4806, runs != nullptr, "runsModel missing");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    w->requestActivate();
    wait_ms(150);
    const int on_overview = runs->rowCount();
    send_key(w, Qt::Key_B, Qt::ControlModifier | Qt::ShiftModifier);
    wait_ms(1500);
    M3_CHECK(4809, runs->rowCount() == on_overview, "Ctrl+Shift+B on the overview must not start a run");
    M3_CHECK(4802, open_detail(w, bid), "backupDetailPage did not open");
    QObject* action = object_named(w, "detailBackupNowAction");
    M3_CHECK(4803, action != nullptr, "detailBackupNowAction missing");
    M3_CHECK(4804, action->property("enabled").toBool(), "Back Up Now must be enabled");
    const QKeySequence shown = action->property("shortcut").value<QKeySequence>();
    M3_CHECK(4805, shown == QKeySequence(QStringLiteral("Ctrl+Shift+B")), "action must show Ctrl+Shift+B");
    const int before = runs->rowCount();
    w->requestActivate();
    wait_ms(100);
    send_key(w, Qt::Key_B, Qt::ControlModifier | Qt::ShiftModifier);
    M3_CHECK(4807, wait_until([&] { return runs->rowCount() > before; }, 15000), "Ctrl+Shift+B did not start a run");
    M3_CHECK(4808, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

} // namespace

int run_m3_pages_scenario(const m3::Ctx& ctx, const QString& scen, bool* handled) {
    if (scen == QLatin1String("m3-backup-now-shortcut")) {
        *handled = true;
        return scenario_backup_now_shortcut(ctx);
    }
    *handled = scen == QLatin1String("m3-all-pages-texts");
    return *handled ? scenario_all_pages(ctx) : 0;
}

} // namespace kestic::smoke
