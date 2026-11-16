// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#include "src/smoke_helper.h"

#include <QAbstractItemModel>
#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImage>
#include <QKeyEvent>
#include <QMetaObject>
#include <QQmlError>
#include <QQuickItem>
#include <QQuickWindow>
#include <QStringList>
#include <QTimer>
#include <QVariant>
#include <cstdio>
#include <memory>

namespace kestic::smoke {

namespace {

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

} // namespace

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

            if (!out_path.isEmpty()) {
                const QFileInfo fi(out_path);
                QDir().mkpath(fi.absolutePath());
                const QImage img = window->grabWindow();
                if (img.isNull() || !img.save(out_path)) {
                    std::fprintf(
                        stderr,
                        "QML smoke failed: could not save screenshot to %s\n",
                        qPrintable(out_path)
                    );
                    QCoreApplication::exit(10);
                    return;
                }
            }

            QCoreApplication::exit(0);
        });
    });
}

} // namespace kestic::smoke
