// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#pragma once

#include <QString>
#include <QQmlApplicationEngine>
#include <cstdint>
#include "rust/cxx.h"

namespace kestic::smoke {

void copy_to_clipboard(rust::Str text);

rust::String i18n_subs(rust::Str msgid, rust::Slice<const rust::Str> args);

std::int32_t local_offset_seconds(std::int64_t unix_secs);

void install_keyboard_handlers(QQmlApplicationEngine& engine);

void install_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str screenshot_path,
    int32_t expected_list_count,
    rust::Str layout_mode,
    rust::Str expected_lang,
    bool trigger_shortcut
);

void install_m1_smoke_check(
    QQmlApplicationEngine& engine,
    rust::Str scenario,
    rust::Str screenshot_path,
    rust::Str layout_mode,
    rust::Str expected_lang
);

} // namespace kestic::smoke
