// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#pragma once

#include <QString>
#include <QQmlApplicationEngine>
#include <cstdint>
#include "rust/cxx.h"

namespace kestic::smoke {

void install_m1_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str scenario,
    rust::Str screenshot_path,
    rust::Str layout_mode,
    rust::Str expected_lang
);

} // namespace kestic::smoke
