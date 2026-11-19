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
#include <memory>

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

void send_key(
    QWindow* window,
    int key,
    Qt::KeyboardModifiers mods = Qt::NoModifier,
    const QString& text = QString()
);

QQuickItem* tab_until_object(QQuickWindow* window, const QString& obj_name, int max_steps);

QQuickItem* tab_until_text(QQuickWindow* window, const QString& text_needle, int max_steps);

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
