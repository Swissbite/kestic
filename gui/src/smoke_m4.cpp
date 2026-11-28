// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// M4 GUI smoke dispatcher: the scenarios of the «Restore» area live in `smoke_m4_lists.cpp`,
// `smoke_m4_browse.cpp` and `smoke_m4_restore.cpp`.

#include "src/smoke_m4_util.h"

namespace kestic::smoke {

void run_m4_smoke_scenario(
    QQmlApplicationEngine& engine,
    QQuickWindow* window,
    QObject* backend,
    const std::shared_ptr<QStringList>& warnings,
    const QString& scen,
    const QString& out_path,
    const QString& lang
) {
    (void)engine;
    const m3::Ctx ctx { window, backend, out_path, lang };
    m3::ScenarioFn run = nullptr;
    for (auto find : {find_m4_lists_scenario, find_m4_browse_scenario, find_m4_restore_scenario}) {
        run = run ? run : find(scen);
    }
    exit_scenario("M4", scen, run ? run(ctx) : -1, 499, *warnings);
}

} // namespace kestic::smoke
