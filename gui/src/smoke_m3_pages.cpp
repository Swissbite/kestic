// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Opens every page, editor, wizard, the settings window and «About Kestic» once, saves a
// screenshot of each and writes all visible texts to the file named by `KESTIC_SMOKE_TEXT_DUMP`
// (one line per text: `page<TAB>text`, line breaks as `\n`). The Rust test checks these texts in both
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

/// The first visible control the style draws cut off: a spin box whose text field is lower than
/// its digits, or a progress bar without height. Empty when there is none.
QString cut_control(QQuickItem* root) {
    QList<QQuickItem*> items;
    collect_visual_descendants(root, items);
    for (QQuickItem* it : items) {
        if (!it->isVisible()) {
            continue;
        }
        const QString cls = QString::fromLatin1(it->metaObject()->className());
        if (cls.contains(QLatin1String("SpinBox"))) {
            auto* field = it->property("contentItem").value<QQuickItem*>();
            if (field && field->height() + 0.5 < field->implicitHeight()) {
                return QStringLiteral("spin box %1").arg(it->objectName());
            }
        } else if (cls.contains(QLatin1String("ProgressBar"))) {
            auto* bar = it->property("background").value<QQuickItem*>();
            if (bar && bar->height() <= 0.0) {
                return QStringLiteral("progress bar %1").arg(it->objectName());
            }
        }
    }
    return {};
}

struct PageRecord {
    bool has_texts;
    bool saved;
    QString cut_control;
};

struct PageDump {
    QFile* file;
    QString screenshot_dir;
    QString lang;
    int pages = 0;

    /// Records the texts of `window` under `name` and saves a screenshot of it.
    PageRecord record(const QString& name, QQuickWindow* window) {
        wait_ms(300); // quiet window: the page ends its transition before its texts are read
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
        const QString cut = cut_control(window->contentItem());
        const bool saved = save_screenshot(window, shot);
        finish_incubation(window);
        return { !texts.isEmpty(), saved, cut };
    }
};

/// Whether the page item called `name` exists in `window` (Kirigami does not keep
/// `visible` set on pages inside its page row, so the visible texts are checked instead).
bool page_visible(QQuickWindow* window, const char* name) {
    return wait_until([&] { return lookup(window, name) != nullptr; }, 3000);
}

/// Opens the backup detail page from the backups tab.
bool open_detail(QQuickWindow* w, const QString& bid) {
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 1));
    wait_until([&] { return w->property("activeTab").toInt() == 1; }, 150);
    const bool opened = open_backup_detail(w, bid);
    finish_incubation(w);
    return opened;
}

/// Records the texts and a screenshot of `window` as page `name`: both must be there.
#define RECORD_PAGE(code, dump, name, window, what)                                      \
    do {                                                                               \
        const int record_code_ = (code);                                               \
        const PageRecord recorded_ = (dump).record((name), (window));                  \
        CHECK(record_code_, recorded_.has_texts, what " page without texts");     \
        CHECK(record_code_, recorded_.saved, what " page screenshot not saved");  \
        CHECK(record_code_, recorded_.cut_control.isEmpty(),                       \
                 qPrintable(QStringLiteral(what " page draws a control cut off: ")      \
                                + recorded_.cut_control));                             \
    } while (0)

int scenario_all_pages(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString path = qEnvironmentVariable("KESTIC_SMOKE_TEXT_DUMP");
    CHECK(4901, !path.isEmpty(), "KESTIC_SMOKE_TEXT_DUMP must name the text dump file");
    QFile file(path);
    CHECK(4902, file.open(QIODevice::WriteOnly | QIODevice::Truncate | QIODevice::Text), "cannot write text dump");
    PageDump dump { &file, QFileInfo(c.out_path).absolutePath(), c.lang };

    const QString bid = backup_id_at(w, 0);
    const QString did = destination_id_at(w, 0);
    CHECK2(4903, !bid.isEmpty(), !did.isEmpty(), "needs one backup and one destination");
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
        CHECK(4910 + i, page_visible(w, tabs[i].page), tabs[i].page);
        RECORD_PAGE(4915 + i, dump, QString::fromLatin1(tabs[i].name), w, "tab");
        if (i == 1) {
            // AC-M0-04-1: the QML list shows exactly the rows of the Rust list model.
            QObject* list = lookup<QObject>(w, "backupsListView");
            QAbstractItemModel* backups = model_named(w, "backupsModel");
            CHECK3(4904, list, backups, backups->rowCount() > 0, "backups list or model missing");
            CHECK(4905, list->property("count").toInt() == backups->rowCount(), "list shows other rows than the model");
        }
    }

    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 4));
    CHECK(4906, page_visible(w, "restorePage"), "restorePage");
    CHECK(4907, wait_until([&] { return !c.backend->property("restore_overview_loading").toBool(); }, 20000),
             "restore area still loading");
    // The selected backup row holds the focus, not the first row of «Other snapshots».
    CHECK(4909, wait_until([&] {
        QQuickItem* focused = w->activeFocusItem();
        return focused && focused->objectName() == QLatin1String("restoreBackupListItem")
            && focused->property("highlighted").toBool();
    }, 3000), "the selected backup row of the restore area holds the focus");
    RECORD_PAGE(4908, dump, QStringLiteral("restore"), w, "restore");

    CHECK(4919, open_detail(w, bid), "backupDetailPage did not open");
    RECORD_PAGE(4920, dump, QStringLiteral("backup-detail"), w, "backup detail");
    // The selected row of the backup list puts its source icon on the background colour: the
    // folder icon has the colour of the selection and would lose its body.
    QQuickItem* icon_backdrop = find_visual_item(w, QStringLiteral("backupRowSourceIconBackdrop"));
    CHECK2(4999, icon_backdrop != nullptr, icon_backdrop->isVisible(),
              "source icon of the selected backup row must sit on the background colour");

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
        CHECK(code++, open_detail(w, bid), "backupDetailPage did not open");
        QQuickItem* row = find_visual_item(w, QString::fromLatin1(e.row));
        CHECK(code, row, e.row);
        CHECK(code++, click_ui_item(w, row, e.row), e.row);
        CHECK(code++, page_visible(w, e.page), e.page);
        RECORD_PAGE(code++, dump, QString::fromLatin1(e.name), w, "editor");
    }

    CHECK(4960, open_detail(w, bid), "backupDetailPage did not open");
    QMetaObject::invokeMethod(w, "openBackupSnapshotsPage", Q_ARG(QVariant, bid), Q_ARG(QVariant, bname));
    CHECK(4961, page_visible(w, "backupSnapshotsPage"), "backupSnapshotsPage");
    RECORD_PAGE(4962, dump, QStringLiteral("backup-snapshots"), w, "snapshots");

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
        wait_until([&] { return w->property("activeTab").toInt() == 2; }, 150);
        QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
        CHECK(code++, page_visible(w, "destinationDetailPage"), "destinationDetailPage");
        wait_ms(200); // quiet window: the page ends its transition
        finish_incubation(w);
        if (p.method) {
            QMetaObject::invokeMethod(w, p.method, Q_ARG(QVariant, did));
        }
        CHECK(code++, page_visible(w, p.page), p.page);
        RECORD_PAGE(code++, dump, QString::fromLatin1(p.name), w, "destination");
    }
    // Like the backup row: the selected destination row puts its icon on the background colour.
    QQuickItem* dest_backdrop = find_visual_item(w, QStringLiteral("destinationRowIconBackdrop"));
    CHECK2(4964, dest_backdrop != nullptr, dest_backdrop->isVisible(),
              "icon of the selected destination row must sit on the background colour");

    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    wait_until([&] { return w->property("activeTab").toInt() == 0; }, 150);
    QMetaObject::invokeMethod(w, "openNewBackupWizard", Q_ARG(QVariant, false));
    CHECK(4980, page_visible(w, "newBackupWizard"), "newBackupWizard");
    RECORD_PAGE(4981, dump, QStringLiteral("new-backup-wizard"), w, "backup wizard");
    QMetaObject::invokeMethod(w, "closeWizardLayer");
    wait_until([&] { return !w->property("inWizardLayer").toBool(); }, 150);
    QMetaObject::invokeMethod(w, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QStringLiteral("init")));
    CHECK(4982, page_visible(w, "addDestinationWizard"), "addDestinationWizard");
    RECORD_PAGE(4983, dump, QStringLiteral("add-destination-wizard"), w, "destination wizard");
    QMetaObject::invokeMethod(w, "closeWizardLayer");
    wait_until([&] { return !w->property("inWizardLayer").toBool(); }, 150);

    QMetaObject::invokeMethod(w, "openSettingsWindow");
    QQuickWindow* settings = nullptr;
    wait_until([&] {
        settings = qobject_cast<QQuickWindow*>(lookup<QObject>(w, "settingsWindow"));
        return settings && settings->isVisible();
    }, 250);
    CHECK2(4990, settings != nullptr, settings->isVisible(), "settingsWindow not open");
    RECORD_PAGE(4991, dump, QStringLiteral("settings"), settings, "settings");
    settings->close();

    QMetaObject::invokeMethod(w, "openAboutPage");
    auto* about = qobject_cast<QQuickWindow*>(lookup<QObject>(w, "aboutWindow"));
    CHECK2(4994, about != nullptr, wait_until([&] { return about->isVisible(); }, 3000), "aboutWindow not open");
    CHECK(4992, find_visual_item(about, QStringLiteral("mainAboutPage")) != nullptr, "mainAboutPage not in the About window");
    RECORD_PAGE(4993, dump, QStringLiteral("about"), about, "about");
    // The homepage and licence links start where the text around them starts (Kirigami
    // centres the text of a link button that fills its row).
    QList<QQuickItem*> about_items;
    collect_visual_descendants(about->contentItem(), about_items);
    int about_links = 0;
    for (QQuickItem* it : about_items) {
        if (!it->isVisible() || !it->property("normalColor").isValid() || it->property("text").toString().isEmpty()) {
            continue;
        }
        ++about_links;
        CHECK(4997, it->property("horizontalAlignment").toInt() == Qt::AlignLeft,
                 qPrintable(QStringLiteral("about link not aligned left: ") + it->property("text").toString()));
    }
    CHECK(4998, about_links >= 3, "about page must show the homepage, licence and issue links");
    about->close();

    file.close();
    CHECK(4995, dump.pages == 21, "expected 21 recorded pages");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    wait_until([&] { return w->property("activeTab").toInt() == 0; }, 150);
    return shot(c, 4996);
}

/// AC-M0-11-8: in the backup detail page, Ctrl+Shift+B starts "Back Up Now" for the shown
/// backup, and the action shows the shortcut. Without an open backup (overview) it starts nothing.
int scenario_backup_now_shortcut(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString bid = backup_id_at(w, 0);
    CHECK(4801, !bid.isEmpty(), "needs one backup");
    QAbstractItemModel* runs = model_named(w, "runsModel");
    CHECK(4806, runs != nullptr, "runsModel missing");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    w->requestActivate();
    wait_until([&] { return w->property("activeTab").toInt() == 0 && w->isActive(); }, 150);
    const int on_overview = runs->rowCount();
    send_key(w, Qt::Key_B, Qt::ControlModifier | Qt::ShiftModifier);
    wait_ms(1500); // quiet window: a run the shortcut must not start would show by now
    CHECK(4809, runs->rowCount() == on_overview, "Ctrl+Shift+B on the overview must not start a run");
    CHECK(4802, open_detail(w, bid), "backupDetailPage did not open");
    QObject* action = lookup<QObject>(w, "detailBackupNowAction");
    CHECK(4803, action != nullptr, "detailBackupNowAction missing");
    CHECK(4804, action->property("enabled").toBool(), "Back Up Now must be enabled");
    const QKeySequence shown = action->property("shortcut").value<QKeySequence>();
    CHECK(4805, shown == QKeySequence(QStringLiteral("Ctrl+Shift+B")), "action must show Ctrl+Shift+B");
    const int before = runs->rowCount();
    w->requestActivate();
    wait_until([&] { return w->isActive(); }, 100);
    send_key(w, Qt::Key_B, Qt::ControlModifier | Qt::ShiftModifier);
    CHECK(4807, wait_until([&] { return runs->rowCount() > before; }, 15000), "Ctrl+Shift+B did not start a run");
    return shot(c, 4808);
}

/// AC-M3-05-9: a seeded maintenance run of «Restic Synology» (pending forget removed two
/// snapshots of «Documents», prune freed 1.3 MB, check ok). The detail page shows (a) the title
/// «Maintenance on …», (b) the status line «Cleaned up and verified · …», (c) the destination
/// below it, and in the «Result» tab (d) the removed snapshots per backup, (e) the freed space
/// and (f) the check result. The history row names the completed steps as well.
int scenario_maintenance_run_detail(const Ctx& c) {
    QQuickWindow* w = c.window;
    QAbstractItemModel* runs = model_named(w, "runsModel");
    CHECK(4701, runs != nullptr, "runsModel missing");
    const QString title = loc(c.lang, "Maintenance Restic Synology", "Wartung Restic Synology");
    int row = -1;
    const bool listed = wait_until([&] {
        for (int r = 0; r < runs->rowCount(); ++r) {
            if (role_value(runs, r, "title").toString() == title) {
                row = r;
                return true;
            }
        }
        return false;
    }, 10000);
    CHECK(4702, listed, "maintenance run not in the history");
    CHECK_EQ(4703, role_value(runs, row, "statusText").toString(),
                loc(c.lang, "Cleaned up and verified", "Aufgeräumt und geprüft"), "history row status");
    const QString run_id = role_value(runs, row, "runId").toString();
    QMetaObject::invokeMethod(w, "openRunDetailById", Q_ARG(QVariant, run_id));
    CHECK(4704, page_visible(w, "runDetailPage"), "runDetailPage did not open");
    // The run's detail arrives from the daemon after the page opened.
    wait_until([&] {
        QObject* shown = lookup<QObject>(w, "runDetailPage");
        return shown && shown->property("title").toString().startsWith(loc(c.lang, "Maintenance on ", "Wartung vom "));
    }, 10000);
    finish_incubation(w);
    QObject* page = lookup<QObject>(w, "runDetailPage");
    CHECK_STARTS(4705, page->property("title").toString(),
                    loc(c.lang, "Maintenance on ", "Wartung vom "), "(a) page title");
    QObject* status = lookup<QObject>(w, "runStatusLineLabel");
    QObject* subtitle = lookup<QObject>(w, "runSubtitleLabel");
    QObject* result = lookup<QObject>(w, "runResultSummaryLabel");
    CHECK3(4706, status, subtitle, result, "run detail labels missing");
    CHECK_STARTS(4707, status->property("text").toString(),
                    loc(c.lang, "Cleaned up and verified · ", "Aufgeräumt und geprüft · "), "(b) status line");
    CHECK_EQ(4708, subtitle->property("text").toString(), QStringLiteral("Restic Synology"),
                "(c) destination below the status");
    const QStringList lines = result->property("text").toString().split(QLatin1Char('\n'));
    CHECK(4709, lines.size() == 3, "result tab has three lines");
    CHECK_EQ(4710, lines.at(0), loc(c.lang, "Documents: 2 snapshots removed", "Documents: 2 Snapshots entfernt"),
                "(d) removed snapshots");
    CHECK_EQ(4711, lines.at(1), loc(c.lang, "Clean up: 1.3 MB freed", "Aufräumen: 1,3 MB freigegeben"),
                "(e) freed space");
    CHECK_EQ(4712, lines.at(2), loc(c.lang, "Verify: no errors found", "Prüfen: keine Fehler gefunden"),
                "(f) check result");
    return shot(c, 4713);
}

/// With one backup on one destination: (a) the backups list header
/// says «1 Backup» and (b) the destinations list header «1 Destination» («1 Ziel»); on the
/// destination detail page (c) «Remove Destination…» is disabled with the hint as tooltip and
/// (d) the hint is not shown in the page body.
int scenario_singular_counts_and_remove_hint(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString did = destination_id_at(w, 0);
    CHECK(4721, !did.isEmpty(), "needs one destination");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 1));
    CHECK(4722, page_visible(w, "backupsListPage"), "backupsListPage");
    QObject* backups_header = lookup<QObject>(w, "backupsListHeader");
    CHECK(4723, backups_header != nullptr, "backupsListHeader missing");
    CHECK_EQ(4724, backups_header->property("text").toString(), QStringLiteral("1 Backup"),
                "(a) backups header");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 2));
    CHECK(4725, page_visible(w, "destinationsListPage"), "destinationsListPage");
    QObject* dest_header = lookup<QObject>(w, "destinationsListHeader");
    CHECK(4726, dest_header != nullptr, "destinationsListHeader missing");
    CHECK_EQ(4727, dest_header->property("text").toString(), loc(c.lang, "1 Destination", "1 Ziel"),
                "(b) destinations header");
    QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
    CHECK(4728, page_visible(w, "destinationDetailPage"), "destinationDetailPage");
    wait_ms(300); // quiet window: the page ends its transition
    finish_incubation(w);
    const QString hint = loc(c.lang, "Remove or reassign all backups on this destination first.",
                             "Zuerst alle Backups auf diesem Ziel entfernen oder einem anderen Ziel zuweisen.");
    QObject* remove = lookup<QObject>(w, "destRemoveAction");
    CHECK(4729, remove != nullptr, "destRemoveAction missing");
    CHECK(4730, !remove->property("enabled").toBool(), "(c) remove action disabled");
    CHECK_EQ(4731, remove->property("tooltip").toString(), hint, "(c) remove action tooltip");
    auto* page = qobject_cast<QQuickItem*>(lookup<QObject>(w, "destinationDetailPage"));
    CHECK(4732, page != nullptr, "destinationDetailPage item missing");
    CHECK(4733, !has_visible_text(page, hint), "(d) hint shown in the page body");
    return shot(c, 4734);
}

/// The destination detail page (a) shows «Maintenance running» while a
/// maintenance run is active and (b) no longer shows it once the run has ended. The scene
/// writes the file `KESTIC_SMOKE_MARKER` after (a); the test then lets the run finish.
int scenario_maintenance_status_reset(const Ctx& c) {
    QQuickWindow* w = c.window;
    const QString marker = qEnvironmentVariable("KESTIC_SMOKE_MARKER");
    CHECK(4741, !marker.isEmpty(), "KESTIC_SMOKE_MARKER must name the marker file");
    const QString did = destination_id_at(w, 0);
    CHECK(4742, !did.isEmpty(), "needs one destination");
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 2));
    wait_until([&] { return w->property("activeTab").toInt() == 2; }, 150);
    QMetaObject::invokeMethod(w, "openDestinationDetailById", Q_ARG(QVariant, did));
    CHECK(4743, page_visible(w, "destinationDetailPage"), "destinationDetailPage");
    const QString running = loc(c.lang, "Maintenance running", "Wartung läuft");
    auto status_text = [&]() -> QString {
        QObject* label = lookup<QObject>(w, "maintenanceStatusLabel");
        if (!label || !label->property("visible").toBool()) {
            return QString();
        }
        return label->property("text").toString();
    };
    CHECK(4744, wait_until([&] { return status_text() == running; }, 15000), "(a) running status shown");
    QFile flag(marker);
    CHECK(4745, flag.open(QIODevice::WriteOnly), "cannot write marker file");
    flag.close();
    CHECK(4746, wait_until([&] { return status_text() != running; }, 60000),
             "(b) running status still shown after the run ended");
    return shot(c, 4747);
}

const Scenario kScenarios[] = {
    {"m3-backup-now-shortcut", scenario_backup_now_shortcut},
    {"m3-maintenance-run-detail", scenario_maintenance_run_detail},
    {"m3-singular-counts-and-remove-hint", scenario_singular_counts_and_remove_hint},
    {"m3-maintenance-status-reset", scenario_maintenance_status_reset},
    {"m3-all-pages-texts", scenario_all_pages},
};

} // namespace

m3::ScenarioFn find_m3_pages_scenario(const QString& scen) {
    return find_scenario(kScenarios, scen);
}

} // namespace kestic::smoke
