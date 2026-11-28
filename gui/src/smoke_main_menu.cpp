// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// Main menu (☰ in the header) and «About Kestic»: the button is visible and works with the
// mouse and with the keyboard alone; the About page names version and build commit of the
// GUI and of the background service. The Rust tests are in `daemon/tests/m3_gui_smoke.rs`.

#include "src/smoke_m3_util.h"

#include <QFileInfo>

namespace kestic::smoke {

namespace {

using namespace m3;

/// The screenshot next to `c.out_path` called `m3-<name>-<layout>-<lang>.png`.
QString sibling_shot(const Ctx& c, const char* name) {
    const QString layout = c.window->property("narrowMode").toBool() ? QStringLiteral("narrow") : QStringLiteral("wide");
    return QFileInfo(c.out_path).absolutePath() + QStringLiteral("/m3-") + QLatin1String(name) + QLatin1Char('-')
        + layout + QLatin1Char('-') + c.lang + QStringLiteral(".png");
}

/// The window that shows «About Kestic», or nullptr while it is not open.
QQuickWindow* about_window(QQuickWindow* w) {
    auto* about = qobject_cast<QQuickWindow*>(lookup<QObject>(w, "aboutWindow"));
    const bool open = about && wait_until([&] { return about->isVisible(); }, 3000);
    return open ? about : nullptr;
}

/// Presses Escape in the About window; true once it is closed again.
bool escape_about(QQuickWindow* w) {
    QQuickWindow* about = about_window(w);
    if (!about) {
        return false;
    }
    about->requestActivate();
    wait_until([&] { return about->isActive(); }, 100);
    return press_until(about, Qt::Key_Escape, [&] { return !about->isVisible(); }, 2000);
}

/// In German, the texts Kirigami adds to the About page (list of libraries) are German too.
int check_kirigami_texts_translated(const Ctx& c, QQuickWindow* about, int code) {
    if (c.lang != QLatin1String("de")) {
        return 0;
    }
    CHECK(code, !has_visible_text(about->contentItem(), QStringLiteral("built against")), "Kirigami text «built against» in English");
    CHECK(code + 1, has_visible_text(about->contentItem(), QStringLiteral("kompiliert gegen")), "Kirigami text «kompiliert gegen» missing");
    return 0;
}

/// The two entries of the open menu: text and accessible name.
int check_menu_entries(const Ctx& c) {
    struct Entry {
        const char* name;
        const char* en;
        const char* de;
    };
    const Entry entries[] = {
        { "configureKesticMenuItem", "Configure Kestic…", "Kestic einrichten…" },
        { "aboutKesticMenuItem", "About Kestic", "Über Kestic" },
    };
    int code = 5110;
    for (const Entry& e : entries) {
        QQuickItem* item = lookup(c.window, e.name);
        CHECK(code, item != nullptr, e.name);
        CHECK(code + 1, item->isVisible(), e.name);
        CHECK_EQ(code + 2, item->property("text").toString(), loc(c.lang, e.en, e.de), e.name);
        CHECK(code + 3, item->isEnabled(), e.name);
        CHECK(code + 4, !accessible_name(item).isEmpty(), e.name);
        code += 5;
    }
    auto* menu = qobject_cast<QQuickItem*>(lookup<QObject>(c.window, "hamburgerMenu")->property("contentItem").value<QObject*>());
    CHECK(5120, menu != nullptr, "menu content missing");
    CHECK(5121, menu->property("count").toInt() == 2, "the menu has other entries than the two");
    return 0;
}

/// «About Kestic» opens in a window of its own titled «About Kestic» (not as a column of the
/// main window) and shows the running build for the GUI and the background service (the same
/// binary here, so the same build) without a hint about different builds.
int check_about_page(const Ctx& c, int depth_before, int code) {
    QQuickWindow* about = about_window(c.window);
    CHECK(code, about != nullptr, "About Kestic did not open");
    CHECK(code + 1, about != c.window, "About Kestic opened inside the main window");
    CHECK(code + 2, about->isVisible(), "About window not visible");
    CHECK_EQ(code + 3, about->title(), loc(c.lang, "About Kestic", "Über Kestic"), "About window title");
    auto* stack = c.window->property("pageStack").value<QObject*>();
    CHECK(code + 4, stack->property("depth").toInt() == depth_before, "About Kestic added a column");
    wait_ms(300); // quiet window: the About window lays out its pages
    finish_incubation(about);
    const QString version = qEnvironmentVariable("KESTIC_SMOKE_VERSION");
    const QString commit = qEnvironmentVariable("KESTIC_SMOKE_COMMIT");
    CHECK2(code + 5, !version.isEmpty(), !commit.isEmpty(), "KESTIC_SMOKE_VERSION and KESTIC_SMOKE_COMMIT must be set");
    QQuickItem* app_row = lookup(c.window, "appBuildRow");
    QQuickItem* daemon_row = lookup(c.window, "daemonBuildRow");
    CHECK2(code + 6, app_row, daemon_row, "build rows missing");
    CHECK(code + 7, has_visible_text(app_row, version), "GUI version not shown");
    CHECK(code + 8, has_visible_text(app_row, commit), "GUI commit not shown");
    CHECK(code + 9, has_visible_text(daemon_row, version), "background service version not shown");
    CHECK(code + 10, has_visible_text(daemon_row, commit), "background service commit not shown");
    QObject* hint = lookup<QObject>(c.window, "buildsDifferMessage");
    CHECK2(code + 11, hint, !hint->property("visible").toBool(), "hint about different builds shown for the same build");
    CHECK(code + 12, !has_visible_text(about->contentItem(), QStringLiteral("david@")), "author e-mail shown");
    return check_kirigami_texts_translated(c, about, 5170);
}

/// (a) The ☰ button is visible at the right end of the header and has an accessible name;
/// (b) a mouse click opens the menu with exactly «Configure Kestic…» and «About Kestic»; (c) a
/// click on «About Kestic» opens it in a window of its own; (d) Tab reaches the button, Space
/// opens the menu, the arrow keys reach «About Kestic» and Return opens it; (e) Escape closes
/// the open menu and the focus stays on the button; (f) Ctrl+Shift+, opens the settings while
/// the menu is closed; (g) Escape closes the About window and the focus returns to the button.
int scenario_main_menu(const Ctx& c) {
    QQuickWindow* w = c.window;
    QQuickItem* button = lookup(w, "mainMenuButton");
    QObject* menu = lookup<QObject>(w, "hamburgerMenu");
    CHECK2(9500, button, menu, "main menu button or menu missing");
    CHECK(9501, button->isVisible(), "(a) button not visible");
    CHECK2(9502, button->width() > 0.0, button->height() > 0.0, "(a) button has no size");
    const QPointF top_left = button->mapToScene(QPointF(0.0, 0.0));
    const qreal right_gap = w->width() - (top_left.x() + button->width());
    CHECK2(9503, right_gap >= 0.0, right_gap <= 24.0, "(a) button not at the right end of the window");
    auto* header = w->property("header").value<QQuickItem*>();
    CHECK(9504, header != nullptr, "window header missing");
    CHECK(9505, top_left.y() + button->height() <= header->mapToScene(QPointF(0.0, 0.0)).y() + header->height(),
             "(a) button not in the window header");
    CHECK_EQ(9506, accessible_name(button), loc(c.lang, "Main Menu", "Hauptmenü"), "(a) accessible name");

    // (f) before the menu was ever opened: its action carries the shortcut from the start.
    auto* settings = qobject_cast<QQuickWindow*>(lookup<QObject>(w, "settingsWindow"));
    CHECK2(5151, settings, !settings->isVisible(), "settings window missing or already open");
    w->requestActivate();
    wait_until([&] { return w->isActive(); }, 100);
    send_key(w, Qt::Key_Comma, Qt::ControlModifier | Qt::ShiftModifier);
    CHECK(5152, wait_until([&] { return settings->isVisible(); }, 2000), "(f) Ctrl+Shift+, did not open the settings");
    settings->close();
    wait_until([&] { return w->isActive(); }, 150);

    const int depth = w->property("pageStack").value<QObject*>()->property("depth").toInt();
    CHECK(9507, click_ui_item(w, button, "main menu button"), "(b) click on the button failed");
    CHECK(9508, wait_until([&] { return is_open(menu); }, 1000), "(b) menu did not open on click");
    if (const int rc = check_menu_entries(c); rc != 0) {
        return rc;
    }
    CHECK(5130, save_screenshot(w, c.out_path), "screenshot of the open menu failed");

    QQuickItem* about_item = lookup(w, "aboutKesticMenuItem");
    CHECK(5131, click_ui_item(w, about_item, "About Kestic"), "(c) click on About Kestic failed");
    CHECK(5132, wait_until([&] { return is_closed(menu); }, 1000), "(c) menu stayed open");
    if (const int rc = check_about_page(c, depth, 5133); rc != 0) {
        return rc;
    }
    CHECK(5160, save_screenshot(about_window(w), sibling_shot(c, "about")), "screenshot of the About window failed");
    CHECK(5161, escape_about(w), "Escape did not close About Kestic");

    QQuickItem* focused = tab_to(w, "mainMenuButton", 60);
    CHECK(5143, focused == button, "(d) Tab does not reach the main menu button");
    CHECK(5144, press_until(w, Qt::Key_Space, [&] { return is_open(menu); }, 1000), "(d) Space did not open the menu");
    for (int i = 0; i < 4 && !about_item->hasActiveFocus(); ++i) {
        QQuickItem* before = w->activeFocusItem();
        press_until(w, Qt::Key_Down, [&] { return w->activeFocusItem() != before; }, 80);
    }
    CHECK(5145, about_item->hasActiveFocus(), "(d) arrow keys do not reach About Kestic");
    CHECK(5146, press_until(w, Qt::Key_Return, [&] { return is_closed(menu); }, 1000), "(d) menu stayed open after Return");
    CHECK(5147, about_window(w) != nullptr, "(d) Return did not open About Kestic");
    CHECK(5162, escape_about(w), "(g) Escape did not close About Kestic");
    CHECK(5163, wait_until([&] { return w->isActive(); }, 1000), "(g) main window not active again");
    CHECK(5164, button->hasActiveFocus(), "(g) focus did not return to the main menu button");

    CHECK(5149, press_until(w, Qt::Key_Space, [&] { return is_open(menu); }, 1000), "(e) Space did not open the menu");
    CHECK(5150, press_until(w, Qt::Key_Escape, [&] { return is_closed(menu); }, 1000), "(e) Escape did not close the menu");
    CHECK(5153, button->hasActiveFocus(), "(e) focus did not return to the main menu button");
    return 0;
}

/// Over the overview of a configured Kestic (three backups), (a) the overview shows the backups,
/// not the welcome screen, and (b) a click on ☰ opens the same static menu with its two entries.
int scenario_main_menu_configured(const Ctx& c) {
    QQuickWindow* w = c.window;
    QAbstractItemModel* backups = model_named(w, "backupsModel");
    CHECK(9509, backups != nullptr, "backupsModel missing");
    CHECK(9510, wait_until([&] { return backups->rowCount() == 3; }, 5000), "(a) expected three backups");
    QQuickItem* overview = lookup(w, "overviewContent");
    CHECK(9511, overview != nullptr, "(a) overviewContent missing");
    CHECK(9512, wait_until([&] { return overview->isVisible(); }, 3000), "(a) overview with backups not shown");
    QQuickItem* welcome = lookup(w, "welcomeView");
    CHECK2(9513, welcome != nullptr, !welcome->isVisible(), "(a) welcome screen shown");
    QQuickItem* button = lookup(w, "mainMenuButton");
    QObject* menu = lookup<QObject>(w, "hamburgerMenu");
    CHECK2(9514, button, menu, "main menu button or menu missing");
    CHECK(9515, click_ui_item(w, button, "main menu button"), "(b) click on the button failed");
    CHECK(9516, wait_until([&] { return is_open(menu); }, 1000), "(b) menu did not open on click");
    if (const int rc = check_menu_entries(c); rc != 0) {
        return rc;
    }
    wait_ms(200); // quiet window: the open menu is drawn before the screenshot
    CHECK(9517, save_screenshot(w, c.out_path), "screenshot of the open menu failed");
    return 0;
}

/// Simulates a background service 9.9.9 at `commit` until `shown` holds (at most 3 s). Opening
/// the About window makes the main window inactive; the GUI reports that and fetches the real
/// state again, which may overwrite a value set only once.
bool simulate_daemon(const Ctx& c, const char* commit, const std::function<bool()>& shown) {
    return wait_until([&] {
        c.backend->setProperty("daemon_version", QStringLiteral("9.9.9"));
        c.backend->setProperty("daemon_build_commit", QString::fromLatin1(commit));
        return shown();
    }, 3000);
}

/// With a background service of another build: (a) the hint to reopen Kestic shows; (b) a
/// clean commit links to its GitHub page; (c) a `-dirty` commit shows as text without a link.
int scenario_about_builds_differ(const Ctx& c) {
    QQuickWindow* w = c.window;
    QMetaObject::invokeMethod(w, "openAboutPage");
    CHECK(9518, about_window(w) != nullptr, "About Kestic did not open");
    QObject* hint = lookup<QObject>(w, "buildsDifferMessage");
    QObject* link = lookup<QObject>(w, "daemonBuildRowCommitLink");
    QObject* text = lookup<QObject>(w, "daemonBuildRowCommitText");
    CHECK3(9519, hint, link, text, "hint, commit link or commit text missing");
    CHECK(9520, simulate_daemon(c, "0123abc", [&] { return hint->property("visible").toBool(); }),
             "(a) hint about different builds not shown");
    CHECK_EQ(9521, hint->property("text").toString(),
                loc(c.lang, "The background service runs another version. Close and reopen Kestic to load the new version.",
                    "Der Hintergrunddienst läuft in einer anderen Version. Kestic schliessen und neu öffnen, um die neue Version zu laden."),
                "(a) hint text");
    CHECK(9522, link->property("visible").toBool(), "(b) clean commit without link");
    CHECK_EQ(9523, link->property("url").toString(), QStringLiteral("https://github.com/Swissbite/kestic/commit/0123abc"), "(b) commit link");
    CHECK(9524, !text->property("visible").toBool(), "(b) clean commit also shown as text");
    // The screenshot must show the simulated build too, so it is taken while it holds.
    QQuickWindow* about = about_window(w);
    // The hint slides open; it is open once its height is above zero and stays the same.
    auto* hint_item = qobject_cast<QQuickItem*>(hint);
    qreal last_height = -1.0;
    CHECK(9525, simulate_daemon(c, "0123abc", [&] {
        const qreal height = hint_item->height();
        const bool opened = hint_item->isVisible() && height > 0.0 && height == last_height;
        last_height = height;
        return opened && save_screenshot(about, c.out_path) && hint_item->isVisible();
    }), "screenshot with the hint failed");

    CHECK(9526, simulate_daemon(c, "0123abc-dirty", [&] { return !link->property("visible").toBool(); }),
             "(c) dirty commit with link");
    CHECK(9527, text->property("visible").toBool(), "(c) dirty commit not shown");
    CHECK_EQ(9528, text->property("text").toString(), QStringLiteral("0123abc-dirty"), "(c) dirty commit text");
    return 0;
}

/// Started with `--show-about` (from the tray), Kestic opens the «About Kestic» window, in
/// German with Kirigami's own texts in German.
int scenario_show_about_on_start(const Ctx& c) {
    QQuickWindow* about = about_window(c.window);
    CHECK(9529, about != nullptr, "About Kestic not open after the start");
    CHECK(9530, about != c.window, "About Kestic opened inside the main window");
    wait_ms(300); // quiet window: the About window lays out its pages
    finish_incubation(about);
    if (const int rc = check_kirigami_texts_translated(c, about, 5304); rc != 0) {
        return rc;
    }
    CHECK(9531, save_screenshot(about, c.out_path), "screenshot failed");
    return 0;
}

const Scenario kScenarios[] = {
    {"m3-main-menu", scenario_main_menu},
    {"m3-main-menu-configured", scenario_main_menu_configured},
    {"m3-about-builds-differ", scenario_about_builds_differ},
    {"m3-show-about-on-start", scenario_show_about_on_start},
};

} // namespace

m3::ScenarioFn find_main_menu_scenario(const QString& scen) {
    return find_scenario(kScenarios, scen);
}

} // namespace kestic::smoke
