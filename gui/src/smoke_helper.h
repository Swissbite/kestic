// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

#pragma once

#include <QString>
#include <QQmlApplicationEngine>
#include "rust/cxx.h"

namespace kestic::smoke {

void install_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str screenshot_path,
    int32_t expected_list_count,
    rust::Str layout_mode,
    rust::Str expected_lang,
    bool trigger_shortcut
);

} // namespace kestic::smoke
