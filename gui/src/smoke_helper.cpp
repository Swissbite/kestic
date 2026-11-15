// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#include "src/smoke_helper.h"

#include <QAbstractItemModel>
#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QImage>
#include <QQmlError>
#include <QQuickItem>
#include <QQuickWindow>
#include <QStringList>
#include <QTimer>
#include <QVariant>
#include <cstdio>
#include <memory>

namespace kestic::smoke {

void install_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str screenshot_path,
    int32_t expected_list_count
) {
    const QString out_path = QString::fromUtf8(screenshot_path.data(), static_cast<qsizetype>(screenshot_path.size()));
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

    QTimer::singleShot(250, &engine, [&engine, warnings, out_path, expected_list_count]() {
        if (!warnings->isEmpty()) {
            for (const auto& w : *warnings) {
                std::fprintf(stderr, "QML warning: %s\n", qPrintable(w));
            }
            QCoreApplication::exit(3);
            return;
        }

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
            const QStringList expected_titles = {
                QStringLiteral("Personal folder"),
                QStringLiteral("Photos"),
                QStringLiteral("Projects"),
            };
            if (titles != expected_titles) {
                std::fprintf(
                    stderr,
                    "QML smoke failed: unexpected titles: %s\n",
                    qPrintable(titles.join(QStringLiteral(", ")))
                );
                QCoreApplication::exit(9);
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
}

} // namespace kestic::smoke
