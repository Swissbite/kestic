// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#pragma once

#include <QQmlApplicationEngine>
#include <cstdint>
#include "rust/cxx.h"

namespace kestic::bridge {

void copy_to_clipboard(rust::Str text);

rust::String i18n_subs(rust::Str msgid, rust::Slice<const rust::Str> args);

rust::String i18np_subs(rust::Str singular, rust::Str plural, std::int64_t n, rust::Slice<const rust::Str> args);
rust::String format_count(std::int64_t n);

std::int32_t local_offset_seconds(std::int64_t unix_secs);

void install_keyboard_handlers(QQmlApplicationEngine& engine);

} // namespace kestic::bridge
