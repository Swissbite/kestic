// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("net.daester.kestic").qml_file("src/qml/Main.qml"))
        .file("src/list_model.rs")
        .file("src/smoke.rs")
        .cpp_file("src/smoke_helper.cpp")
        .qt_module("Widgets")
        .qt_module("Quick")
        .build();
}
