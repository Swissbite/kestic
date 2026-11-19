// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "src/smoke_helper.h"
#include "src/smoke_common.h"
#include <QColor>

namespace kestic::smoke {

void run_m2_smoke_scenario(
    QQmlApplicationEngine& engine,
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& scen,
    const QString& out_path,
    const QString& lang
);

void install_m1_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str scenario,
    rust::Str screenshot_path,
    rust::Str layout_mode,
    rust::Str expected_lang
) {
    const QString scen = QString::fromUtf8(scenario.data(), static_cast<qsizetype>(scenario.size()));
    const QString out_path = QString::fromUtf8(screenshot_path.data(), static_cast<qsizetype>(screenshot_path.size()));
    const QString mode = QString::fromUtf8(layout_mode.data(), static_cast<qsizetype>(layout_mode.size()));
    const QString lang = QString::fromUtf8(expected_lang.data(), static_cast<qsizetype>(expected_lang.size()));
    auto warnings = std::make_shared<QStringList>();
    finish_incubation_on_quit(engine);

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

    QTimer::singleShot(250, &engine, [&engine, warnings, scen, out_path, mode, lang]() {
        const auto roots = engine.rootObjects();
        if (roots.isEmpty()) {
            std::fprintf(stderr, "M1 smoke failed: no root objects loaded\n");
            QCoreApplication::exit(2);
            return;
        }
        auto* window = qobject_cast<QQuickWindow*>(roots.first());
        if (!window) {
            std::fprintf(stderr, "M1 smoke failed: root object is not a QQuickWindow\n");
            QCoreApplication::exit(4);
            return;
        }
        QObject* backend = window->findChild<QObject*>(QStringLiteral("appBackend"));
        if (!backend) {
            std::fprintf(stderr, "M1 smoke failed: appBackend not found\n");
            QCoreApplication::exit(5);
            return;
        }

        if (mode == QLatin1String("narrow")) {
            QMetaObject::invokeMethod(window, "resizeToMinWidthForSmoke");
        }

        // Perform scenario navigation setup
        if (scen == QLatin1String("backups-detail-show-cmd")) {
            QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 1));
        } else if (scen == QLatin1String("destinations-reassign")) {
            QObject* backups_model = window->findChild<QObject*>(QStringLiteral("backupsModel"));
            auto* bm = qobject_cast<QAbstractItemModel*>(backups_model);
            if (bm && bm->rowCount() > 0) {
                const QString bid = bm->data(bm->index(0, 0), 0).toString();
                QMetaObject::invokeMethod(window, "openBackupDetailById", Q_ARG(QVariant, bid));
                QMetaObject::invokeMethod(window, "openBackupReassignPage", Q_ARG(QVariant, bid));
            }
        } else if (scen == QLatin1String("snapshots-previous")) {
            QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 1));
            QObject* backups_model = window->findChild<QObject*>(QStringLiteral("backupsModel"));
            auto* bm = qobject_cast<QAbstractItemModel*>(backups_model);
            if (bm && bm->rowCount() > 0) {
                const QString bid = bm->data(bm->index(0, 0), 0).toString();
                const QString bname = bm->data(bm->index(0, 0), 1).toString();
                QMetaObject::invokeMethod(window, "openBackupSnapshotsPage", Q_ARG(QVariant, bid), Q_ARG(QVariant, bname));
            }
        } else if (scen == QLatin1String("history-run-detail")) {
            QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 3));
            QObject* runs_model = window->findChild<QObject*>(QStringLiteral("runsModel"));
            auto* rm = qobject_cast<QAbstractItemModel*>(runs_model);
            if (rm && rm->rowCount() > 0) {
                const QString rid = rm->data(rm->index(0, 0), 0).toString();
                QMetaObject::invokeMethod(window, "openRunDetailById", Q_ARG(QVariant, rid));
            }
        } else if (scen == QLatin1String("manual-run-blocked")) {
            QObject* backups_model = window->findChild<QObject*>(QStringLiteral("backupsModel"));
            auto* bm = qobject_cast<QAbstractItemModel*>(backups_model);
            if (bm && bm->rowCount() > 0) {
                const QString bid = bm->data(bm->index(0, 0), 0).toString();
                QMetaObject::invokeMethod(backend, "startBackup", Q_ARG(QString, bid));
            }
        } else if (scen == QLatin1String("wizard-validation")) {
            QMetaObject::invokeMethod(window, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QVariant()));
        } else if (scen == QLatin1String("e2e-edit-reinit-and-nested-wizard")) {
            QObject* dests_model = window->findChild<QObject*>(QStringLiteral("destinationsModel"));
            auto* dm = qobject_cast<QAbstractItemModel*>(dests_model);
            if (dm && dm->rowCount() > 0) {
                const QString did = dm->data(dm->index(0, 0), 0).toString();
                QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 2));
                QMetaObject::invokeMethod(window, "openDestinationEditPage", Q_ARG(QVariant, did));
            }
        } else if (scen == QLatin1String("settings")) {
            QMetaObject::invokeMethod(window, "openSettingsWindow");
        }

        // Allow async QML creations / wizard advance / snapshots load
        QTimer::singleShot(400, &engine, [&engine, window, backend, warnings, scen, out_path, mode, lang]() {
            if (!warnings->isEmpty()) {
                for (const auto& w : *warnings) {
                    std::fprintf(stderr, "M1 QML warning: %s\n", qPrintable(w));
                }
                QCoreApplication::exit(3);
                return;
            }

            if (window->title() != QLatin1String("Kestic")) {
                std::fprintf(stderr, "M1 smoke failed: expected title 'Kestic', got '%s'\n", qPrintable(window->title()));
                QCoreApplication::exit(11);
                return;
            }

            // Common M1 invariants across main window scenarios
            auto* top_nav = window->findChild<QQuickItem*>(QStringLiteral("topNavBar"));
            auto* bottom_nav = window->findChild<QQuickItem*>(QStringLiteral("bottomNavBar"));
            if (!top_nav || !bottom_nav) {
                std::fprintf(stderr, "M1 smoke failed: NavigationTabBars missing\n");
                QCoreApplication::exit(12);
                return;
            }

            const QStringList required_tabs = (lang == QLatin1String("de"))
                ? QStringList {
                    QStringLiteral("Übersicht"),
                    QStringLiteral("Backups"),
                    QStringLiteral("Ziele"),
                    QStringLiteral("Verlauf"),
                }
                : QStringList {
                    QStringLiteral("Overview"),
                    QStringLiteral("Backups"),
                    QStringLiteral("Destinations"),
                    QStringLiteral("History"),
                };
            const QString forbidden_restore = (lang == QLatin1String("de"))
                ? QStringLiteral("Wiederherstellen")
                : QStringLiteral("Restore");

            const bool in_wizard = window->property("inWizardLayer").toBool();
            if (in_wizard) {
                if (top_nav->isVisible() || bottom_nav->isVisible()) {
                    std::fprintf(stderr, "M1 smoke failed: NavigationTabBar must be hidden inside wizard layer\n");
                    QCoreApplication::exit(13);
                    return;
                }
            } else if (mode == QLatin1String("narrow")) {
                if (top_nav->isVisible() || !bottom_nav->isVisible()) {
                    std::fprintf(stderr, "M1 smoke failed: expected bottomNavBar visible in narrow mode\n");
                    QCoreApplication::exit(13);
                    return;
                }
                if (!verify_tab_bar_labels(bottom_nav, 4, required_tabs)) {
                    QCoreApplication::exit(14);
                    return;
                }
            } else {
                if (!verify_tab_bar_labels(top_nav, 4, required_tabs)) {
                    QCoreApplication::exit(15);
                    return;
                }
            }

            // Verify NO Restore tab in M1 and NO New Backup / Add Destination / Quit Kestic in hamburger menu
            QList<QQuickItem*> nav_items;
            collect_visual_descendants(mode == QLatin1String("narrow") ? bottom_nav : top_nav, nav_items);
            for (QQuickItem* it : nav_items) {
                if (it->property("text").toString() == forbidden_restore && it->isVisible()) {
                    std::fprintf(stderr, "M1 smoke failed: Restore tab must not exist in M1\n");
                    QCoreApplication::exit(16);
                    return;
                }
            }

            if (QObject* hamburger = window->findChild<QObject*>(QStringLiteral("hamburgerMenu"))) {
                QList<QObject*> objs;
                collect_all_qobjects(hamburger, objs);
                for (QObject* o : objs) {
                    const QString txt = o->property("text").toString();
                    if (txt.contains(QStringLiteral("New Backup"))
                        || txt.contains(QStringLiteral("Neues Backup"))
                        || txt.contains(QStringLiteral("Add Destination"))
                        || txt.contains(QStringLiteral("Ziel hinzufügen"))
                        || txt.contains(QStringLiteral("Quit Kestic"))
                        || txt.contains(QStringLiteral("Kestic beenden"))) {
                        std::fprintf(stderr, "M1 smoke failed: forbidden hamburger item '%s'\n", qPrintable(txt));
                        QCoreApplication::exit(17);
                        return;
                    }
                }
            }

            QQuickWindow* screenshot_target = window;

            if (scen.startsWith(QLatin1String("m2-"))) {
                run_m2_smoke_scenario(engine, window, backend, warnings, scen, out_path, lang);
                return;
            } else if (scen.startsWith(QLatin1String("m3-"))) {
                run_m3_smoke_scenario(engine, window, backend, warnings, scen, out_path, lang);
                return;
            } else if (scen == QLatin1String("welcome")) {
                QQuickItem* wv = window->findChild<QQuickItem*>(QStringLiteral("welcomeView"));
                if (!wv || !wv->isVisible()) {
                    std::fprintf(stderr, "M1 smoke failed: welcomeView should be visible when unconfigured\n");
                    QCoreApplication::exit(20);
                    return;
                }
            } else if (scen == QLatin1String("overview")) {
                QQuickItem* ov = window->findChild<QQuickItem*>(QStringLiteral("overviewContent"));
                if (!ov || !ov->isVisible()) {
                    std::fprintf(stderr, "M1 smoke failed: overviewContent should be visible\n");
                    QCoreApplication::exit(21);
                    return;
                }
            } else if (scen.startsWith(QLatin1String("blocking-"))) {
                const QString expected_kind = scen.mid(QStringLiteral("blocking-").size());
                const QString actual_kind = backend->property("blocking_kind").toString();
                if (actual_kind != expected_kind) {
                    std::fprintf(
                        stderr,
                        "M1 smoke failed: expected blocking_kind='%s', got '%s'\n",
                        qPrintable(expected_kind),
                        qPrintable(actual_kind)
                    );
                    QCoreApplication::exit(22);
                    return;
                }
                if (expected_kind == QLatin1String("restic_unavailable")) {
                    QObject* dl_action = window->findChild<QObject*>(QStringLiteral("downloadResticDisabledAction"));
                    if (!dl_action || dl_action->property("enabled").toBool()) {
                        std::fprintf(stderr, "M1 smoke failed: Download restic action must be present and disabled in M1\n");
                        QCoreApplication::exit(23);
                        return;
                    }
                }
            } else if (scen == QLatin1String("backups-detail-show-cmd")) {
                QObject* show_cmd = window->findChild<QObject*>(QStringLiteral("backupDetailShowCommandAction"));
                if (!show_cmd || !show_cmd->property("enabled").toBool()) {
                    std::fprintf(stderr, "M1 smoke failed: Show Command action on BackupDetailPage must be enabled even when expert_mode=false\n");
                    QCoreApplication::exit(24);
                    return;
                }
                QMetaObject::invokeMethod(show_cmd, "trigger");
                QTimer::singleShot(250, &engine, [window, screenshot_target, out_path]() {
                    QObject* cmd_area = window->findChild<QObject*>(QStringLiteral("showCommandTextArea"));
                    if (!cmd_area || !cmd_area->property("text").toString().contains(QStringLiteral("RESTIC_PASSWORD_COMMAND"))) {
                        std::fprintf(stderr, "M1 smoke failed: Show Command dialog missing transparent command\n");
                        QCoreApplication::exit(25);
                        return;
                    }
                    if (!save_screenshot(screenshot_target, out_path)) {
                        QCoreApplication::exit(10);
                        return;
                    }
                    QCoreApplication::exit(0);
                });
                return;
            } else if (scen == QLatin1String("destinations-reassign")) {
                QObject* switch_act = window->findChild<QObject*>(QStringLiteral("switchDestinationAction"));
                if (!switch_act || switch_act->property("enabled").toBool()) {
                    std::fprintf(stderr, "M1 smoke failed: Switch Destination must be disabled before selecting a destination\n");
                    QCoreApplication::exit(26);
                    return;
                }
            } else if (scen == QLatin1String("snapshots-previous")) {
                QQuickItem* snap_list = window->findChild<QQuickItem*>(QStringLiteral("snapshotsListView"));
                if (!snap_list) {
                    std::fprintf(stderr, "M1 smoke failed: snapshotsListView not found\n");
                    QCoreApplication::exit(27);
                    return;
                }
                QList<QQuickItem*> snap_items;
                collect_visual_descendants(snap_list, snap_items);
                bool found_prev_header = false;
                for (QQuickItem* it : snap_items) {
                    if (it->objectName() == QLatin1String("snapshotSectionHeader")) {
                        const QString hdr = it->property("text").toString();
                        if (hdr.contains(QStringLiteral("Earlier snapshots on '"))
                            || hdr.contains(QStringLiteral("Frühere Snapshots auf '"))) {
                            found_prev_header = true;
                            break;
                        }
                    }
                }
                if (!found_prev_header) {
                    std::fprintf(stderr, "M1 smoke failed: previous destination snapshots section header not found\n");
                    QCoreApplication::exit(28);
                    return;
                }
            } else if (scen == QLatin1String("history-run-detail")) {
                QObject* tab_bar = window->findChild<QObject*>(QStringLiteral("runDetailTabBar"));
                QObject* show_cmd = window->findChild<QObject*>(QStringLiteral("runDetailShowCommandAction"));
                if (!tab_bar || tab_bar->property("count").toInt() != 3 || !show_cmd || !show_cmd->property("enabled").toBool()) {
                    std::fprintf(stderr, "M1 smoke failed: RunDetailPage 3 tabs or Show Command missing\n");
                    QCoreApplication::exit(29);
                    return;
                }
            } else if (scen == QLatin1String("manual-run-blocked")) {
                QQuickItem* banner = window->findChild<QQuickItem*>(QStringLiteral("manualRunBlockedBanner"));
                if (!banner || !banner->isVisible() || backend->property("manual_run_blocked_message").toString().isEmpty()) {
                    std::fprintf(stderr, "M1 smoke failed: ManualRunBlocked inline banner not shown\n");
                    QCoreApplication::exit(30);
                    return;
                }
            } else if (scen == QLatin1String("wizard-validation")) {
                QObject* wiz = window->findChild<QObject*>(QStringLiteral("addDestinationWizard"));
                QObject* next_btn = window->findChild<QObject*>(QStringLiteral("wizDestNextButton"));
                if (!wiz || !next_btn) {
                    std::fprintf(stderr, "M1 smoke failed: AddDestinationWizard not opened\n");
                    QCoreApplication::exit(31);
                    return;
                }
                wiz->setProperty("step", 2);
                QCoreApplication::processEvents();
                QQuickItem* name_err = window->findChild<QQuickItem*>(QStringLiteral("wizDestNameError"));
                QQuickItem* loc_err = window->findChild<QQuickItem*>(QStringLiteral("wizDestLocationError"));
                if (!name_err || !name_err->isVisible() || !loc_err || !loc_err->isVisible() || next_btn->property("enabled").toBool()) {
                    std::fprintf(stderr, "M1 smoke failed: wizard validation errors / disabled Next not enforced\n");
                    QCoreApplication::exit(32);
                    return;
                }
            } else if (scen == QLatin1String("quit-dialog")) {
                QObject* dlg = window->findChild<QObject*>(QStringLiteral("quitConfirmDialog"));
                if (!dlg || !dlg->property("visible").toBool()) {
                    std::fprintf(stderr, "M1 smoke failed: quitConfirmDialog must open when --confirm-quit is passed\n");
                    QCoreApplication::exit(33);
                    return;
                }
            } else if (scen == QLatin1String("settings")) {
                auto* sw = window->findChild<QQuickWindow*>(QStringLiteral("settingsWindow"));
                if (!sw || !sw->isVisible()) {
                    std::fprintf(stderr, "M1 smoke failed: settingsWindow not visible\n");
                    QCoreApplication::exit(34);
                    return;
                }
                QObject* upd_btn = sw->findChild<QObject*>(QStringLiteral("updateResticButton"));
                if (!upd_btn || upd_btn->property("enabled").toBool()) {
                    std::fprintf(stderr, "M1 smoke failed: Update Now button must be disabled in M1\n");
                    QCoreApplication::exit(35);
                    return;
                }
                screenshot_target = sw;
            } else if (scen == QLatin1String("destinations-empty")) {
                QQuickItem* dest_tab = find_visible_item_by_text(
                    top_nav,
                    QStringList { QStringLiteral("Ziele"), QStringLiteral("Destinations") }
                );
                if (!click_ui_item(window, dest_tab, "Ziele tab")) {
                    QCoreApplication::exit(36);
                    return;
                }
                QTimer::singleShot(250, &engine, [&engine, window, top_nav, warnings, out_path]() {
                    if (!has_visible_text(
                            window->contentItem(),
                            QString::fromUtf8("Noch keine Ziele eingerichtet")
                        )) {
                        std::fprintf(stderr, "M1 smoke failed: DestinationsPage empty PlaceholderMessage not visible\n");
                        QCoreApplication::exit(36);
                        return;
                    }
                    QQuickItem* add_btn = find_visible_item_by_text(
                        window->contentItem(),
                        QStringList {
                            QString::fromUtf8("Ziel hinzufügen"),
                            QStringLiteral("Add Destination")
                        }
                    );
                    if (!add_btn) {
                        std::fprintf(stderr, "M1 smoke failed: visible Add Destination button not found\n");
                        QCoreApplication::exit(36);
                        return;
                    }
                    if (!save_screenshot(window, out_path)) {
                        QCoreApplication::exit(10);
                        return;
                    }
                    if (!click_ui_item(window, add_btn, "Placeholder Add Destination")) {
                        QCoreApplication::exit(37);
                        return;
                    }
                    QTimer::singleShot(350, &engine, [&engine, window, top_nav, warnings]() {
                        QQuickItem* wiz = window->findChild<QQuickItem*>(QStringLiteral("addDestinationWizard"));
                        if (!wiz || !wiz->isVisible() || !window->property("inWizardLayer").toBool()
                            || top_nav->isVisible() || wiz->width() < window->width() - 20) {
                            std::fprintf(stderr, "M1 smoke failed: AddDestinationWizard must open full-width without tab bar\n");
                            QCoreApplication::exit(37);
                            return;
                        }
                        QQuickItem* cancel_btn = find_visible_item_by_text(
                            wiz,
                            QStringList { QStringLiteral("Abbrechen"), QStringLiteral("Cancel") }
                        );
                        if (!click_ui_item(window, cancel_btn, "Cancel AddDestinationWizard")) {
                            QCoreApplication::exit(37);
                            return;
                        }
                        QTimer::singleShot(350, &engine, [&engine, window, top_nav, warnings]() {
                            QQuickItem* ov_tab = find_visible_item_by_text(
                                top_nav,
                                QStringList { QString::fromUtf8("Übersicht"), QStringLiteral("Overview") }
                            );
                            if (!click_ui_item(window, ov_tab, "Overview tab")) {
                                QCoreApplication::exit(38);
                                return;
                            }
                            QTimer::singleShot(250, &engine, [&engine, window, top_nav, warnings]() {
                                QQuickItem* setup_btn = window->findChild<QQuickItem*>(QStringLiteral("setupFirstBackupButton"));
                                if (!click_ui_item(window, setup_btn, "Set Up First Backup")) {
                                    QCoreApplication::exit(38);
                                    return;
                                }
                                QTimer::singleShot(350, &engine, [&engine, window, top_nav, warnings]() {
                                    QQuickItem* nbw = window->findChild<QQuickItem*>(QStringLiteral("newBackupWizard"));
                                    QQuickItem* next_btn = window->findChild<QQuickItem*>(QStringLiteral("wizBackupNextButton"));
                                    if (!nbw || !window->property("inWizardLayer").toBool() || top_nav->isVisible()
                                        || !nbw->property("integratedDestFlow").toBool()
                                        || nbw->width() < window->width() - 20) {
                                        std::fprintf(stderr, "M1 smoke failed: Setup First Backup must open full-width with integrated destination flow\n");
                                        QCoreApplication::exit(39);
                                        return;
                                    }
                                    if (!click_ui_item(window, next_btn, "First Backup Step 1 Next")) {
                                        QCoreApplication::exit(39);
                                        return;
                                    }
                                    QTimer::singleShot(80, &engine, [&engine, window, top_nav, warnings, nbw, next_btn]() {
                                        if (window->findChild<QQuickItem*>(QStringLiteral("addDestinationWizard"))
                                            || nbw->property("currentStage").toString() != QLatin1String("dest_type")) {
                                            std::fprintf(stderr, "M1 smoke failed: expected inline dest_type stage without nested AddDestinationWizard\n");
                                            QCoreApplication::exit(40);
                                            return;
                                        }
                                        if (!click_ui_item(window, next_btn, "First Backup Step 2 Next")) {
                                            QCoreApplication::exit(40);
                                            return;
                                        }
                                        QTimer::singleShot(80, &engine, [&engine, window, top_nav, warnings, nbw]() {
                                            auto* dest_name_field = window->findChild<QQuickItem*>(QStringLiteral("wizBackupDestNameField"));
                                            if (nbw->property("currentStage").toString() != QLatin1String("dest_details")
                                                || !dest_name_field || !dest_name_field->isVisible()) {
                                                std::fprintf(stderr, "M1 smoke failed: expected inline dest_details stage in First Backup wizard\n");
                                                QCoreApplication::exit(40);
                                                return;
                                            }
                                            QQuickItem* nbw_cancel = find_visible_item_by_text(
                                                nbw,
                                                QStringList { QStringLiteral("Abbrechen"), QStringLiteral("Cancel") }
                                            );
                                            if (!click_ui_item(window, nbw_cancel, "Cancel First Backup Wizard")) {
                                                QCoreApplication::exit(40);
                                                return;
                                            }
                                            QTimer::singleShot(350, &engine, [&engine, window, top_nav, warnings]() {
                                                QQuickItem* backups_tab = find_visible_item_by_text(
                                                    top_nav,
                                                    QStringList { QStringLiteral("Backups") }
                                                );
                                                if (!click_ui_item(window, backups_tab, "Backups tab")) {
                                                    QCoreApplication::exit(41);
                                                    return;
                                                }
                                                QTimer::singleShot(250, &engine, [&engine, window, top_nav, warnings]() {
                                                    if (!has_visible_text(
                                                            window->contentItem(),
                                                            QString::fromUtf8("Noch keine Backups eingerichtet")
                                                        )) {
                                                        std::fprintf(stderr, "M1 smoke failed: BackupsPage empty PlaceholderMessage not visible\n");
                                                        QCoreApplication::exit(41);
                                                        return;
                                                    }
                                                    QQuickItem* history_tab = find_visible_item_by_text(
                                                        top_nav,
                                                        QStringList { QStringLiteral("Verlauf"), QStringLiteral("History") }
                                                    );
                                                    if (!click_ui_item(window, history_tab, "History tab")) {
                                                        QCoreApplication::exit(42);
                                                        return;
                                                    }
                                                    QTimer::singleShot(250, &engine, [window, warnings]() {
                                                        if (!has_visible_text(
                                                                window->contentItem(),
                                                                QString::fromUtf8("Keine passenden Läufe gefunden")
                                                            )) {
                                                            std::fprintf(stderr, "M1 smoke failed: HistoryPage empty PlaceholderMessage not visible\n");
                                                            QCoreApplication::exit(42);
                                                            return;
                                                        }
                                                        if (!warnings->isEmpty()) {
                                                            for (const auto& w : *warnings) {
                                                                std::fprintf(stderr, "M1 QML warning: %s\n", qPrintable(w));
                                                            }
                                                            QCoreApplication::exit(3);
                                                            return;
                                                        }
                                                        QCoreApplication::exit(0);
                                                    });
                                                });
                                            });
                                        });
                                    });
                                });
                            });
                        });
                    });
                });
                return;
            } else if (scen == QLatin1String("e2e-wizard-backup")) {
                const QString repo_dir = qEnvironmentVariable("KESTIC_SMOKE_REPO_DIR");
                const QString src_dir = qEnvironmentVariable("KESTIC_SMOKE_SRC_DIR");
                if (repo_dir.isEmpty() || src_dir.isEmpty()) {
                    std::fprintf(stderr, "E2E smoke failed: KESTIC_SMOKE_REPO_DIR / KESTIC_SMOKE_SRC_DIR not set\n");
                    QCoreApplication::exit(50);
                    return;
                }
                QQuickItem* dest_tab = find_visible_item_by_text(
                    top_nav,
                    QStringList { QStringLiteral("Destinations") }
                );
                if (!click_ui_item(window, dest_tab, "E2E Destinations tab")) {
                    QCoreApplication::exit(51);
                    return;
                }
                QTimer::singleShot(250, &engine, [&engine, window, top_nav, warnings, out_path, repo_dir, src_dir]() {
                    QQuickItem* add_btn = find_visible_item_by_text(
                        window->contentItem(),
                        QStringList { QStringLiteral("Add Destination") }
                    );
                    if (!click_ui_item(window, add_btn, "E2E Add Destination")) {
                        QCoreApplication::exit(52);
                        return;
                    }
                    QTimer::singleShot(350, &engine, [&engine, window, top_nav, warnings, out_path, repo_dir, src_dir]() {
                        QQuickItem* next_btn = window->findChild<QQuickItem*>(QStringLiteral("wizDestNextButton"));
                        if (!click_ui_item(window, next_btn, "E2E Dest Step 1 Next")) {
                            QCoreApplication::exit(53);
                            return;
                        }
                        QTimer::singleShot(80, &engine, [&engine, window, top_nav, warnings, out_path, repo_dir, src_dir, next_btn]() {
                            auto* name_field = window->findChild<QQuickItem*>(QStringLiteral("wizDestNameField"));
                            auto* loc_field = window->findChild<QQuickItem*>(QStringLiteral("wizDestLocationField"));
                            if (!enter_text_field(window, name_field, QStringLiteral("E2E Local Repo"), "Dest Name")
                                || !enter_text_field(window, loc_field, repo_dir, "Dest Location")) {
                                QCoreApplication::exit(54);
                                return;
                            }
                            if (next_btn->isEnabled()) {
                                std::fprintf(stderr, "E2E failed: Next must be disabled before connection test\n");
                                QCoreApplication::exit(55);
                                return;
                            }
                            auto* test_btn = window->findChild<QQuickItem*>(QStringLiteral("wizDestTestButton"));
                            if (!click_ui_item(window, test_btn, "E2E Test Connection")) {
                                QCoreApplication::exit(56);
                                return;
                            }
                            if (!next_btn->isEnabled() || !click_ui_item(window, next_btn, "E2E Dest Step 2 Next")) {
                                std::fprintf(stderr, "E2E failed: Test Connection did not enable Next\n");
                                QCoreApplication::exit(57);
                                return;
                            }
                            QTimer::singleShot(80, &engine, [&engine, window, top_nav, warnings, out_path, src_dir, next_btn]() {
                                auto* pw_field = window->findChild<QQuickItem*>(QStringLiteral("wizDestPasswordField"));
                                auto* pw2_field = window->findChild<QQuickItem*>(QStringLiteral("wizDestPasswordConfirmField"));
                                if (!enter_text_field(window, pw_field, QStringLiteral("e2e-test-pw"), "Repo Password")
                                    || !enter_text_field(window, pw2_field, QStringLiteral("e2e-test-pw"), "Repeat Password")) {
                                    QCoreApplication::exit(58);
                                    return;
                                }
                                if (!click_ui_item(window, next_btn, "E2E Save Destination")) {
                                    QCoreApplication::exit(59);
                                    return;
                                }
                                QTimer::singleShot(400, &engine, [&engine, window, top_nav, warnings, out_path, src_dir]() {
                                    if (window->findChild<QObject*>(QStringLiteral("addDestinationWizard"))) {
                                        std::fprintf(stderr, "E2E failed: AddDestinationWizard did not close after Save Destination\n");
                                        QCoreApplication::exit(60);
                                        return;
                                    }
                                    auto* dm = qobject_cast<QAbstractItemModel*>(
                                        window->findChild<QObject*>(QStringLiteral("destinationsModel"))
                                    );
                                    if (!dm || dm->rowCount() != 1
                                        || dm->data(dm->index(0, 0), 1).toString() != QLatin1String("E2E Local Repo")) {
                                        std::fprintf(stderr, "E2E failed: destinationsModel does not contain E2E Local Repo\n");
                                        QCoreApplication::exit(61);
                                        return;
                                    }
                                    QQuickItem* backups_tab = find_visible_item_by_text(
                                        top_nav,
                                        QStringList { QStringLiteral("Backups") }
                                    );
                                    if (!click_ui_item(window, backups_tab, "E2E Backups tab")) {
                                        QCoreApplication::exit(62);
                                        return;
                                    }
                                    QTimer::singleShot(250, &engine, [&engine, window, top_nav, warnings, out_path, src_dir]() {
                                        QQuickItem* new_bk_btn = find_visible_item_by_text(
                                            window->contentItem(),
                                            QStringList { QStringLiteral("New Backup") }
                                        );
                                        if (!click_ui_item(window, new_bk_btn, "E2E New Backup")) {
                                            QCoreApplication::exit(63);
                                            return;
                                        }
                                        QTimer::singleShot(350, &engine, [&engine, window, top_nav, warnings, out_path, src_dir]() {
                                            auto* src_field = window->findChild<QQuickItem*>(QStringLiteral("wizBackupSourceField"));
                                            auto* nbw_next = window->findChild<QQuickItem*>(QStringLiteral("wizBackupNextButton"));
                                            if (!enter_text_field(window, src_field, src_dir, "Backup Source")
                                                || !click_ui_item(window, nbw_next, "E2E Backup Step 1 Next")) {
                                                QCoreApplication::exit(64);
                                                return;
                                            }
                                            QTimer::singleShot(80, &engine, [&engine, window, top_nav, warnings, out_path, nbw_next]() {
                                                QQuickItem* nbw = window->findChild<QQuickItem*>(QStringLiteral("newBackupWizard"));
                                                if (!nbw || !has_visible_text(nbw, QStringLiteral("E2E Local Repo"))) {
                                                    std::fprintf(stderr, "E2E failed: E2E Local Repo not listed in NewBackupWizard\n");
                                                    QCoreApplication::exit(65);
                                                    return;
                                                }
                                                if (!click_ui_item(window, nbw_next, "E2E Backup Step 2 Next")) {
                                                    QCoreApplication::exit(66);
                                                    return;
                                                }
                                                QTimer::singleShot(80, &engine, [&engine, window, top_nav, warnings, out_path]() {
                                                    auto* start_btn = window->findChild<QQuickItem*>(QStringLiteral("wizBackupStartNowButton"));
                                                    if (!click_ui_item(window, start_btn, "E2E Back Up Now")) {
                                                        QCoreApplication::exit(67);
                                                        return;
                                                    }
                                                    auto* poll_timer = new QTimer(window);
                                                    auto ticks = std::make_shared<int>(0);
                                                    QObject::connect(
                                                        poll_timer,
                                                        &QTimer::timeout,
                                                        window,
                                                        [&engine, poll_timer, ticks, window, top_nav, warnings, out_path]() {
                                                            *ticks += 1;
                                                            auto* bm = qobject_cast<QAbstractItemModel*>(
                                                                window->findChild<QObject*>(QStringLiteral("backupsModel"))
                                                            );
                                                            auto* rm = qobject_cast<QAbstractItemModel*>(
                                                                window->findChild<QObject*>(QStringLiteral("runsModel"))
                                                            );
                                                            const bool has_backup = bm && bm->rowCount() == 1;
                                                            const bool is_running = has_backup && bm->data(bm->index(0, 0), 16).toBool();
                                                            const bool has_error = has_backup && bm->data(bm->index(0, 0), 14).toBool();
                                                            const bool has_run = rm && rm->rowCount() >= 1;
                                                            if (has_backup && !is_running && has_run) {
                                                                poll_timer->stop();
                                                                if (has_error) {
                                                                    std::fprintf(stderr, "E2E failed: backup run finished with error\n");
                                                                    QCoreApplication::exit(68);
                                                                    return;
                                                                }
                                                                QQuickItem* ov_tab = find_visible_item_by_text(
                                                                    top_nav,
                                                                    QStringList { QStringLiteral("Overview") }
                                                                );
                                                                if (!click_ui_item(window, ov_tab, "E2E Overview tab")) {
                                                                    QCoreApplication::exit(69);
                                                                    return;
                                                                }
                                                                QTimer::singleShot(250, &engine, [window, warnings, out_path]() {
                                                                    QQuickItem* ov = window->findChild<QQuickItem*>(QStringLiteral("overviewContent"));
                                                                    if (!ov || !ov->isVisible()) {
                                                                        std::fprintf(stderr, "E2E failed: overviewContent not visible after backup\n");
                                                                        QCoreApplication::exit(70);
                                                                        return;
                                                                    }
                                                                    if (!warnings->isEmpty()) {
                                                                        for (const auto& w : *warnings) {
                                                                            std::fprintf(stderr, "E2E QML warning: %s\n", qPrintable(w));
                                                                        }
                                                                        QCoreApplication::exit(3);
                                                                        return;
                                                                    }
                                                                    if (!save_screenshot(window, out_path)) {
                                                                        QCoreApplication::exit(10);
                                                                        return;
                                                                    }
                                                                    QCoreApplication::exit(0);
                                                                });
                                                                return;
                                                            }
                                                            if (*ticks >= 75) {
                                                                poll_timer->stop();
                                                                std::fprintf(stderr, "E2E failed: timed out waiting for real restic backup run\n");
                                                                QCoreApplication::exit(71);
                                                            }
                                                        }
                                                    );
                                                    poll_timer->start(200);
                                                });
                                            });
                                        });
                                    });
                                });
                            });
                        });
                    });
                });
                return;
            } else if (scen == QLatin1String("e2e-first-backup")) {
                const QString repo_dir = qEnvironmentVariable("KESTIC_SMOKE_REPO_DIR");
                const QString src_dir = qEnvironmentVariable("KESTIC_SMOKE_SRC_DIR");
                if (repo_dir.isEmpty() || src_dir.isEmpty()) {
                    std::fprintf(stderr, "E2E first-backup failed: KESTIC_SMOKE_REPO_DIR / KESTIC_SMOKE_SRC_DIR not set\n");
                    QCoreApplication::exit(80);
                    return;
                }
                QQuickItem* setup_btn = window->findChild<QQuickItem*>(QStringLiteral("setupFirstBackupButton"));
                if (!click_ui_item(window, setup_btn, "E2E First Backup Welcome Button")) {
                    QCoreApplication::exit(81);
                    return;
                }
                QTimer::singleShot(350, &engine, [&engine, window, warnings, out_path, repo_dir, src_dir]() {
                    QQuickItem* nbw = window->findChild<QQuickItem*>(QStringLiteral("newBackupWizard"));
                    auto* src_field = window->findChild<QQuickItem*>(QStringLiteral("wizBackupSourceField"));
                    auto* next_btn = window->findChild<QQuickItem*>(QStringLiteral("wizBackupNextButton"));
                    if (!nbw || !nbw->property("integratedDestFlow").toBool()
                        || !enter_text_field(window, src_field, src_dir, "First Backup Source")
                        || !click_ui_item(window, next_btn, "First Backup Step 1 Next")) {
                        QCoreApplication::exit(82);
                        return;
                    }
                    QTimer::singleShot(80, &engine, [&engine, window, warnings, out_path, repo_dir, next_btn]() {
                        if (!click_ui_item(window, next_btn, "First Backup Step 2 Next")) {
                            QCoreApplication::exit(83);
                            return;
                        }
                        QTimer::singleShot(80, &engine, [&engine, window, warnings, out_path, repo_dir, next_btn]() {
                            auto* name_field = window->findChild<QQuickItem*>(QStringLiteral("wizBackupDestNameField"));
                            auto* loc_field = window->findChild<QQuickItem*>(QStringLiteral("wizBackupDestLocationField"));
                            auto* test_btn = window->findChild<QQuickItem*>(QStringLiteral("wizBackupDestTestButton"));
                            if (!enter_text_field(window, name_field, QStringLiteral("First Local Repo"), "First Dest Name")
                                || !enter_text_field(window, loc_field, repo_dir, "First Dest Location")
                                || !click_ui_item(window, test_btn, "First Dest Test Connection")
                                || !next_btn->isEnabled()
                                || !click_ui_item(window, next_btn, "First Backup Step 3 Next")) {
                                QCoreApplication::exit(84);
                                return;
                            }
                            QTimer::singleShot(80, &engine, [&engine, window, warnings, out_path, next_btn]() {
                                auto* pw_field = window->findChild<QQuickItem*>(QStringLiteral("wizBackupDestPasswordField"));
                                auto* pw2_field = window->findChild<QQuickItem*>(QStringLiteral("wizBackupDestPasswordConfirmField"));
                                if (!enter_text_field(window, pw_field, QStringLiteral("e2e-first-pw"), "First Repo Password")
                                    || !enter_text_field(window, pw2_field, QStringLiteral("e2e-first-pw"), "First Repeat Password")
                                    || !click_ui_item(window, next_btn, "First Backup Step 4 Next")) {
                                    QCoreApplication::exit(85);
                                    return;
                                }
                                QTimer::singleShot(80, &engine, [window, warnings, out_path]() {
                                    auto* start_btn = window->findChild<QQuickItem*>(QStringLiteral("wizBackupStartNowButton"));
                                    if (!click_ui_item(window, start_btn, "First Backup Back Up Now")) {
                                        QCoreApplication::exit(86);
                                        return;
                                    }
                                    auto* poll_timer = new QTimer(window);
                                    auto ticks = std::make_shared<int>(0);
                                    QObject::connect(
                                        poll_timer,
                                        &QTimer::timeout,
                                        window,
                                        [poll_timer, ticks, window, warnings, out_path]() {
                                            *ticks += 1;
                                            auto* bm = qobject_cast<QAbstractItemModel*>(
                                                window->findChild<QObject*>(QStringLiteral("backupsModel"))
                                            );
                                            auto* rm = qobject_cast<QAbstractItemModel*>(
                                                window->findChild<QObject*>(QStringLiteral("runsModel"))
                                            );
                                            const bool has_backup = bm && bm->rowCount() == 1;
                                            const bool is_running = has_backup && bm->data(bm->index(0, 0), 16).toBool();
                                            const bool has_error = has_backup && bm->data(bm->index(0, 0), 14).toBool();
                                            const bool has_run = rm && rm->rowCount() >= 1;
                                            if (has_backup && !is_running && has_run) {
                                                poll_timer->stop();
                                                if (has_error) {
                                                    std::fprintf(stderr, "E2E first-backup failed: run ended with error\n");
                                                    QCoreApplication::exit(87);
                                                    return;
                                                }
                                                QQuickItem* ov = window->findChild<QQuickItem*>(QStringLiteral("overviewContent"));
                                                if (!ov || !ov->isVisible()) {
                                                    std::fprintf(stderr, "E2E first-backup failed: overviewContent not visible\n");
                                                    QCoreApplication::exit(88);
                                                    return;
                                                }
                                                if (!warnings->isEmpty()) {
                                                    for (const auto& w : *warnings) {
                                                        std::fprintf(stderr, "E2E QML warning: %s\n", qPrintable(w));
                                                    }
                                                    QCoreApplication::exit(3);
                                                    return;
                                                }
                                                if (!save_screenshot(window, out_path)) {
                                                    QCoreApplication::exit(10);
                                                    return;
                                                }
                                                QCoreApplication::exit(0);
                                                return;
                                            }
                                            if (*ticks >= 75) {
                                                poll_timer->stop();
                                                std::fprintf(stderr, "E2E first-backup failed: timed out waiting for real restic run\n");
                                                QCoreApplication::exit(89);
                                            }
                                        }
                                    );
                                    poll_timer->start(200);
                                });
                            });
                        });
                    });
                });
                return;
            } else if (scen == QLatin1String("e2e-edit-reinit-and-nested-wizard")) {
                QQuickItem* edit_page = window->findChild<QQuickItem*>(QStringLiteral("destinationEditPage"));
                auto* test_btn = window->findChild<QQuickItem*>(QStringLiteral("destEditTestButton"));
                if (!edit_page || !click_ui_item(window, test_btn, "Edit Dest Test Connection")) {
                    std::fprintf(stderr, "E2E edit-reinit failed: DestinationEditPage or Test button missing\n");
                    QCoreApplication::exit(90);
                    return;
                }
                if (backend->property("connection_test_ok").toBool()
                    || backend->property("connection_test_error_kind").toString() != QLatin1String("repository_missing")) {
                    std::fprintf(
                        stderr,
                        "E2E edit-reinit failed: expected repository_missing, got ok=%d kind='%s'\n",
                        backend->property("connection_test_ok").toBool() ? 1 : 0,
                        qPrintable(backend->property("connection_test_error_kind").toString())
                    );
                    QCoreApplication::exit(91);
                    return;
                }
                QObject* switch_init_act = window->findChild<QObject*>(QStringLiteral("destEditSwitchToInitAction"));
                if (!switch_init_act || !switch_init_act->property("visible").toBool()) {
                    std::fprintf(stderr, "E2E edit-reinit failed: destEditSwitchToInitAction not visible\n");
                    QCoreApplication::exit(92);
                    return;
                }
                QMetaObject::invokeMethod(switch_init_act, "trigger");
                QCoreApplication::processEvents();
                if (!backend->property("connection_test_ok").toBool()
                    || edit_page->property("setupMode").toString() != QLatin1String("init")) {
                    std::fprintf(stderr, "E2E edit-reinit failed: switching to init did not pass connection test\n");
                    QCoreApplication::exit(93);
                    return;
                }
                QObject* save_act = window->findChild<QObject*>(QStringLiteral("saveDestinationAction"));
                if (!save_act || !save_act->property("enabled").toBool()) {
                    std::fprintf(stderr, "E2E edit-reinit failed: Save action not enabled after init test\n");
                    QCoreApplication::exit(94);
                    return;
                }
                QMetaObject::invokeMethod(save_act, "trigger");
                QTimer::singleShot(350, &engine, [&engine, window, warnings, out_path]() {
                    QMetaObject::invokeMethod(window, "openNewBackupWizard", Q_ARG(QVariant, false));
                    QTimer::singleShot(350, &engine, [&engine, window, warnings, out_path]() {
                        QQuickItem* nbw = window->findChild<QQuickItem*>(QStringLiteral("newBackupWizard"));
                        auto* nbw_next = window->findChild<QQuickItem*>(QStringLiteral("wizBackupNextButton"));
                        if (!nbw || nbw->property("integratedDestFlow").toBool()
                            || !click_ui_item(window, nbw_next, "Nested Wizard Step 1 Next")) {
                            std::fprintf(stderr, "E2E nested-wizard failed: NewBackupWizard step 1\n");
                            QCoreApplication::exit(95);
                            return;
                        }
                        QTimer::singleShot(120, &engine, [&engine, window, warnings, out_path, nbw]() {
                            QQuickItem* add_dest_btn = find_visible_item_by_text(
                                nbw,
                                QStringList {
                                    QString::fromUtf8("Add New Destination"),
                                    QString::fromUtf8("Neues Ziel hinzufügen")
                                }
                            );
                            if (!click_ui_item(window, add_dest_btn, "Nested Add New Destination")) {
                                QCoreApplication::exit(96);
                                return;
                            }
                            QTimer::singleShot(350, &engine, [window, warnings, out_path]() {
                                QQuickItem* add_wiz = window->findChild<QQuickItem*>(QStringLiteral("addDestinationWizard"));
                                auto* dest_next = window->findChild<QQuickItem*>(QStringLiteral("wizDestNextButton"));
                                if (!add_wiz || !add_wiz->isVisible() || !dest_next
                                    || add_wiz->width() < window->width() - 20) {
                                    std::fprintf(
                                        stderr,
                                        "E2E nested-wizard failed: nested AddDestinationWizard not full-width (w=%.1f win=%d)\n",
                                        add_wiz ? add_wiz->width() : -1.0,
                                        window->width()
                                    );
                                    QCoreApplication::exit(97);
                                    return;
                                }
                                const QPointF next_center = dest_next->mapToScene(
                                    QPointF(dest_next->width() / 2.0, dest_next->height() / 2.0)
                                );
                                if (next_center.x() < 0.0 || next_center.x() > window->width()
                                    || next_center.y() < 0.0 || next_center.y() > window->height()) {
                                    std::fprintf(
                                        stderr,
                                        "E2E nested-wizard failed: wizDestNextButton clipped at (%.1f,%.1f) in (%dx%d)\n",
                                        next_center.x(),
                                        next_center.y(),
                                        window->width(),
                                        window->height()
                                    );
                                    QCoreApplication::exit(98);
                                    return;
                                }
                                if (!warnings->isEmpty()) {
                                    for (const auto& w : *warnings) {
                                        std::fprintf(stderr, "E2E QML warning: %s\n", qPrintable(w));
                                    }
                                    QCoreApplication::exit(3);
                                    return;
                                }
                                if (!save_screenshot(window, out_path)) {
                                    QCoreApplication::exit(10);
                                    return;
                                }
                                QCoreApplication::exit(0);
                            });
                        });
                    });
                });
                return;
            } else if (scen == QLatin1String("keyboard-navigation")) {
                auto fail_kbd = [](int code, const char* msg) {
                    std::fprintf(stderr, "Keyboard navigation test failed: %s\n", msg);
                    QCoreApplication::exit(code);
                };

                // 1. OverviewPage -> Tab reaches overviewBackupCard -> Return opens BackupDetailPage
                QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 0));
                QCoreApplication::processEvents();
                QQuickItem* card = tab_until_object(window, QStringLiteral("overviewBackupCard"), 20);
                if (!card) {
                    fail_kbd(101, "Tab did not reach overviewBackupCard on OverviewPage");
                    return;
                }
                send_key(window, Qt::Key_Return);
                if (!window->findChild<QQuickItem*>(QStringLiteral("backupDetailPage"))) {
                    fail_kbd(102, "Return on overviewBackupCard did not open BackupDetailPage");
                    return;
                }

                // 2. BackupDetailPage -> Tab reaches all 5 rows -> Return on showSnapshotsButton opens BackupSnapshotsPage -> Esc closes
                const QStringList detail_rows = {
                    QStringLiteral("showSnapshotsButton"),
                    QStringLiteral("editSourceRow"),
                    QStringLiteral("editExcludesRow"),
                    QStringLiteral("editDestinationRow"),
                    QStringLiteral("viewFullConfigRow"),
                };
                for (const QString& row_name : detail_rows) {
                    if (!tab_until_object(window, row_name, 25)) {
                        std::fprintf(stderr, "Keyboard navigation failed: BackupDetailPage row missing from Tab chain: %s\n", qPrintable(row_name));
                        QCoreApplication::exit(103);
                        return;
                    }
                }
                QQuickItem* snap_btn = tab_until_object(window, QStringLiteral("showSnapshotsButton"), 25);
                if (!snap_btn) {
                    fail_kbd(104, "Could not Tab back to showSnapshotsButton");
                    return;
                }
                send_key(window, Qt::Key_Return);
                if (!window->findChild<QQuickItem*>(QStringLiteral("backupSnapshotsPage"))) {
                    fail_kbd(105, "Return on showSnapshotsButton did not open BackupSnapshotsPage");
                    return;
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(100);
                if (window->findChild<QQuickItem*>(QStringLiteral("backupSnapshotsPage"))) {
                    fail_kbd(106, "Esc did not close BackupSnapshotsPage");
                    return;
                }

                // 3. BackupExcludesEditPage -> Simple Mode CheckBox toggles once on Return, dirty Esc -> unsaved dialog -> Esc, then Expert Mode TextArea Tab escape
                QQuickItem* excl_row = tab_until_object(window, QStringLiteral("editExcludesRow"), 25);
                if (!excl_row) {
                    fail_kbd(107, "Could not Tab to editExcludesRow");
                    return;
                }
                send_key(window, Qt::Key_Return);
                QQuickItem* excl_page = window->findChild<QQuickItem*>(QStringLiteral("backupExcludesEditPage"));
                if (!excl_page) {
                    fail_kbd(108, "Return on editExcludesRow did not open BackupExcludesEditPage");
                    return;
                }
                QQuickItem* cache_cb = tab_until_object(window, QStringLiteral("excludeCachesCheckbox"), 20);
                if (!cache_cb) {
                    fail_kbd(112, "Tab did not reach excludeCachesCheckbox");
                    return;
                }
                const bool cb0 = cache_cb->property("checked").toBool();
                send_key(window, Qt::Key_Return);
                const bool cb1 = cache_cb->property("checked").toBool();
                if (cb1 == cb0) {
                    fail_kbd(113, "Return did not toggle excludeCachesCheckbox (or double-toggled)");
                    return;
                }
                if (!excl_page->property("dirty").toBool()) {
                    fail_kbd(114, "BackupExcludesEditPage did not mark dirty after checkbox toggle");
                    return;
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(120);
                QObject* unsaved_dlg = window->findChild<QObject*>(QStringLiteral("unsavedExcludesDialog"));
                if (!unsaved_dlg || (!unsaved_dlg->property("visible").toBool() && !unsaved_dlg->property("opened").toBool())) {
                    fail_kbd(115, "Esc on dirty BackupExcludesEditPage did not open unsavedExcludesDialog");
                    return;
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(300);
                if (unsaved_dlg->property("visible").toBool() || unsaved_dlg->property("opened").toBool()) {
                    fail_kbd(116, "Esc did not close unsavedExcludesDialog");
                    return;
                }
                // Restore checkbox to clean state
                cache_cb->forceActiveFocus(Qt::TabFocusReason);
                QCoreApplication::processEvents();
                send_key(window, Qt::Key_Return);
                if (cache_cb->property("checked").toBool() != cb0 || excl_page->property("dirty").toBool()) {
                    fail_kbd(117, "Second Return did not restore excludeCachesCheckbox to clean state");
                    return;
                }
                // Enable Expert Mode temporarily to verify excludesTextArea Tab-escape
                const bool prev_expert_mode = backend->property("expert_mode").toBool();
                if (!prev_expert_mode) {
                    QMetaObject::invokeMethod(backend, "updateExpertMode", Q_ARG(bool, true));
                    wait_ms(80);
                }
                QQuickItem* excl_area = tab_until_object(window, QStringLiteral("excludesTextArea"), 20);
                if (!excl_area) {
                    fail_kbd(109, "Tab did not reach excludesTextArea");
                    return;
                }
                const QString excl_before = excl_area->property("text").toString();
                send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
                if (window->activeFocusItem() == excl_area) {
                    fail_kbd(110, "Tab remained trapped inside excludesTextArea");
                    return;
                }
                if (excl_area->property("text").toString() != excl_before) {
                    fail_kbd(111, "Tab inserted a character into excludesTextArea");
                    return;
                }
                if (!prev_expert_mode) {
                    QMetaObject::invokeMethod(backend, "updateExpertMode", Q_ARG(bool, false));
                    wait_ms(80);
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(120);
                if (window->findChild<QQuickItem*>(QStringLiteral("backupExcludesEditPage"))) {
                    fail_kbd(118, "Clean Esc did not close BackupExcludesEditPage");
                    return;
                }

                // 4. NavigationTabButton Return + HistoryPage Issues Chip single toggle + RunDetailPage tabs & TextArea & Show Command Esc
                QQuickItem* hist_tab = tab_until_text(window, QStringLiteral("History"), 25);
                if (!hist_tab) {
                    fail_kbd(119, "Tab did not reach History NavigationTabButton");
                    return;
                }
                send_key(window, Qt::Key_Return);
                if (window->property("activeTab").toInt() != 3) {
                    fail_kbd(120, "Return on History NavigationTabButton did not switch activeTab to 3");
                    return;
                }
                QQuickItem* issues_chip = tab_until_object(window, QStringLiteral("historyIssuesChip"), 25);
                if (!issues_chip) {
                    fail_kbd(121, "Tab did not reach historyIssuesChip");
                    return;
                }
                const bool chip0 = issues_chip->property("checked").toBool();
                send_key(window, Qt::Key_Return);
                if (issues_chip->property("checked").toBool() == chip0) {
                    fail_kbd(122, "Return did not toggle historyIssuesChip (or double-toggled)");
                    return;
                }
                send_key(window, Qt::Key_Return);
                if (issues_chip->property("checked").toBool() != chip0) {
                    fail_kbd(123, "Second Return did not restore historyIssuesChip");
                    return;
                }
                QQuickItem* run_row = tab_until_object(window, QStringLiteral("runListItem"), 25);
                if (!run_row) {
                    fail_kbd(124, "Tab did not reach runListItem on HistoryPage");
                    return;
                }
                send_key(window, Qt::Key_Return);
                if (!window->findChild<QQuickItem*>(QStringLiteral("runDetailPage"))) {
                    fail_kbd(125, "Return on runListItem did not open RunDetailPage");
                    return;
                }
                // M3 manual acceptance (2026-11-19): the selected row must use the selection
                // text colour, or it is unreadable on a light selection in the dark theme.
                {
                    QQuickItem* title = run_row->findChild<QQuickItem*>(QStringLiteral("runRowTitle"));
                    if (!title || !run_row->property("highlighted").toBool()
                        || title->property("color").value<QColor>()
                            != run_row->property("selectedTextColor").value<QColor>()) {
                        fail_kbd(155, "selected run row title does not use the selection text colour");
                        return;
                    }
                }

                // RunDetailPage TabBar arrow navigation + Tab 1 Log controls + Tab 2 Commands TextArea escape + Show Command Esc
                QQuickItem* first_subtab = tab_until_text(window, QStringLiteral("Result"), 25);
                QObject* tab_bar = window->findChild<QObject*>(QStringLiteral("runDetailTabBar"));
                if (!first_subtab || !tab_bar) {
                    fail_kbd(126, "Tab did not reach Result TabButton on RunDetailPage");
                    return;
                }
                send_key(window, Qt::Key_Right);
                if (tab_bar->property("currentIndex").toInt() != 1) {
                    fail_kbd(127, "Right arrow on RunDetail TabBar did not switch to Log tab (1)");
                    return;
                }
                if (!tab_until_object(window, QStringLiteral("runLogSearchField"), 10)
                    || !tab_until_object(window, QStringLiteral("runLogFilterCombo"), 10)
                    || !tab_until_text(window, QStringLiteral("Copy Log"), 10)
                    || !tab_until_text(window, QStringLiteral("Open Log File"), 10)) {
                    fail_kbd(128, "Tab chain on RunDetail Log tab did not reach all toolbar controls");
                    return;
                }
                // Next Tab enters runLogsListView item
                QQuickItem* logs_view = window->findChild<QQuickItem*>(QStringLiteral("runLogsListView"));
                send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
                if (logs_view && logs_view->isVisible()) {
                    QQuickItem* f = window->activeFocusItem();
                    bool inside_logs = false;
                    for (QQuickItem* p = f; p; p = p->parentItem()) {
                        if (p == logs_view) {
                            inside_logs = true;
                            break;
                        }
                    }
                    if (!inside_logs) {
                        fail_kbd(129, "Tab from Open Log File did not enter runLogsListView");
                        return;
                    }
                }
                if (!tab_until_text(window, QStringLiteral("Log"), 25)) {
                    fail_kbd(130, "Could not Tab back to Log TabButton");
                    return;
                }
                send_key(window, Qt::Key_Right);
                if (tab_bar->property("currentIndex").toInt() != 2) {
                    fail_kbd(131, "Right arrow on RunDetail TabBar did not switch to Commands tab (2)");
                    return;
                }
                QQuickItem* run_cmd_area = tab_until_object(window, QStringLiteral("runCommandTextArea"), 10);
                if (!run_cmd_area) {
                    fail_kbd(132, "Tab did not reach runCommandTextArea on Commands tab");
                    return;
                }
                const QString cmd_before = run_cmd_area->property("text").toString();
                send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
                if (window->activeFocusItem() == run_cmd_area || run_cmd_area->property("text").toString() != cmd_before) {
                    fail_kbd(133, "Tab was trapped or inserted \\t in runCommandTextArea");
                    return;
                }

                if (QObject* show_cmd = window->findChild<QObject*>(QStringLiteral("runDetailShowCommandAction"))) {
                    QMetaObject::invokeMethod(show_cmd, "trigger");
                    wait_ms(120);
                    QObject* cmd_dlg = window->findChild<QObject*>(QStringLiteral("showCommandDialog"));
                    if (!cmd_dlg || !cmd_dlg->property("visible").toBool()) {
                        fail_kbd(134, "showCommandDialog failed to open");
                        return;
                    }
                    send_key(window, Qt::Key_Escape);
                    wait_ms(300);
                    if (cmd_dlg->property("visible").toBool() || cmd_dlg->property("opened").toBool()) {
                        fail_kbd(135, "Esc did not close showCommandDialog");
                        return;
                    }
                } else {
                    fail_kbd(136, "runDetailShowCommandAction missing");
                    return;
                }

                // 5. Wizards: NewBackupWizard Step 3 TextArea Tab escape + Esc, AddDestinationWizard clean & dirty Esc
                QMetaObject::invokeMethod(window, "openNewBackupWizard", Q_ARG(QVariant, false));
                wait_ms(120);
                if (QObject* nbw = window->findChild<QObject*>(QStringLiteral("newBackupWizard"))) {
                    nbw->setProperty("stepIndex", 2);
                    QCoreApplication::processEvents();
                } else {
                    fail_kbd(137, "NewBackupWizard failed to open");
                    return;
                }
                QQuickItem* wiz_excl = tab_until_object(window, QStringLiteral("wizBackupExcludesArea"), 15);
                if (!wiz_excl) {
                    fail_kbd(138, "Tab did not reach wizBackupExcludesArea in NewBackupWizard Step 3");
                    return;
                }
                send_key(window, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
                if (window->activeFocusItem() == wiz_excl || wiz_excl->property("text").toString().contains(QLatin1Char('\t'))) {
                    fail_kbd(139, "Tab trapped or inserted \\t in wizBackupExcludesArea");
                    return;
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(150);
                if (window->property("inWizardLayer").toBool()) {
                    fail_kbd(140, "Esc did not close clean NewBackupWizard");
                    return;
                }

                QMetaObject::invokeMethod(window, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QVariant()));
                wait_ms(120);
                if (!window->property("inWizardLayer").toBool()) {
                    fail_kbd(141, "AddDestinationWizard did not open");
                    return;
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(150);
                if (window->property("inWizardLayer").toBool()) {
                    fail_kbd(142, "Esc did not close clean AddDestinationWizard");
                    return;
                }
                QMetaObject::invokeMethod(window, "openAddDestinationWizard", Q_ARG(QVariant, QVariant()), Q_ARG(QVariant, QVariant()));
                wait_ms(120);
                if (QObject* wiz = window->findChild<QObject*>(QStringLiteral("addDestinationWizard"))) {
                    wiz->setProperty("destName", QStringLiteral("Unsaved Name"));
                    QCoreApplication::processEvents();
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(120);
                QObject* discard_dlg = window->findChild<QObject*>(QStringLiteral("discardDestDialog"));
                if (!discard_dlg || (!discard_dlg->property("visible").toBool() && !discard_dlg->property("opened").toBool())) {
                    fail_kbd(143, "Esc on dirty AddDestinationWizard did not open discardDestDialog");
                    return;
                }
                send_key(window, Qt::Key_Escape);
                wait_ms(300);
                if (discard_dlg->property("visible").toBool() || discard_dlg->property("opened").toBool()) {
                    fail_kbd(144, "Esc did not close discardDestDialog");
                    return;
                }
                QMetaObject::invokeMethod(window, "closeWizardLayer");
                wait_ms(100);

                // 6. SettingsDialog: Tab -> expertModeSwitch single toggle on Return -> resticSourceDialog -> Esc closes dialog, second Esc closes SettingsWindow
                QMetaObject::invokeMethod(window, "openSettingsWindow");
                wait_ms(120);
                auto* sw = window->findChild<QQuickWindow*>(QStringLiteral("settingsWindow"));
                if (!sw || !sw->isVisible()) {
                    fail_kbd(145, "SettingsDialog failed to open");
                    return;
                }
                const bool exp0 = backend->property("expert_mode").toBool();
                QQuickItem* exp_sw = tab_until_object(sw, QStringLiteral("expertModeSwitch"), 10);
                if (!exp_sw) {
                    fail_kbd(146, "Tab did not reach expertModeSwitch in SettingsDialog");
                    return;
                }
                send_key(sw, Qt::Key_Return);
                if (backend->property("expert_mode").toBool() == exp0) {
                    fail_kbd(147, "Return did not toggle expertModeSwitch in SettingsDialog (or double-toggled)");
                    return;
                }
                send_key(sw, Qt::Key_Return);
                if (backend->property("expert_mode").toBool() != exp0) {
                    fail_kbd(148, "Second Return did not restore expertModeSwitch in SettingsDialog");
                    return;
                }
                if (!tab_until_object(sw, QStringLiteral("changeResticSourceButton"), 10)) {
                    fail_kbd(149, "Tab did not reach changeResticSourceButton in SettingsDialog");
                    return;
                }
                send_key(sw, Qt::Key_Return);
                wait_ms(120);
                QObject* restic_dlg = sw->findChild<QObject*>(QStringLiteral("resticSourceDialog"));
                if (!restic_dlg || (!restic_dlg->property("visible").toBool() && !restic_dlg->property("opened").toBool())) {
                    fail_kbd(150, "Return on changeResticSourceButton did not open resticSourceDialog");
                    return;
                }
                send_key(sw, Qt::Key_Escape);
                wait_ms(300);
                if (restic_dlg->property("visible").toBool() || restic_dlg->property("opened").toBool() || !sw->isVisible()) {
                    fail_kbd(151, "First Esc in SettingsDialog did not close resticSourceDialog while keeping SettingsDialog open");
                    return;
                }
                send_key(sw, Qt::Key_Escape);
                wait_ms(150);
                if (sw->isVisible()) {
                    fail_kbd(152, "Second Esc in SettingsDialog did not close SettingsDialog window");
                    return;
                }

                // Return to Overview and focus overviewBackupCard so the artifact screenshot also captures the visible 2px focus ring
                window->requestActivate();
                QMetaObject::invokeMethod(window, "switchTab", Q_ARG(QVariant, 0));
                QCoreApplication::processEvents();
                QPointer<QQuickItem> final_card(tab_until_object(window, QStringLiteral("overviewBackupCard"), 20));
                if (final_card.isNull()) {
                    fail_kbd(153, "Could not re-focus overviewBackupCard on OverviewPage");
                    return;
                }
                wait_ms(120);
                if (final_card.isNull() || !final_card->property("activeFocus").toBool()) {
                    fail_kbd(154, "overviewBackupCard lost activeFocus across idle state sync");
                    return;
                }

                if (!warnings->isEmpty()) {
                    for (const auto& w : *warnings) {
                        std::fprintf(stderr, "M1 keyboard smoke QML warning: %s\n", qPrintable(w));
                    }
                    QCoreApplication::exit(3);
                    return;
                }
                if (!save_screenshot(window, out_path)) {
                    QCoreApplication::exit(10);
                    return;
                }
                QCoreApplication::exit(0);
                return;
            }

            if (!save_screenshot(screenshot_target, out_path)) {
                QCoreApplication::exit(10);
                return;
            }

            QCoreApplication::exit(0);
        });
    });
}

} // namespace kestic::smoke
