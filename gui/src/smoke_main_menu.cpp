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

QQuickItem* about_page(QQuickWindow* w) {
    wait_until([&] { return item_named(w, "mainAboutPage") != nullptr; }, 3000);
    return item_named(w, "mainAboutPage");
}

/// Back from «About Kestic» to the overview.
void close_about(QQuickWindow* w) {
    QMetaObject::invokeMethod(w, "switchTab", Q_ARG(QVariant, 0));
    wait_ms(200);
    finish_incubation(w);
}

/// The four entries of the open menu: text, enabled state and accessible name.
int check_menu_entries(const Ctx& c) {
    struct Entry {
        const char* name;
        const char* en;
        const char* de;
        bool enabled;
    };
    const Entry entries[] = {
        { "configureKesticMenuItem", "Configure Kestic…", "Kestic einrichten…", true },
        { "configureShortcutsMenuItem", "Configure Keyboard Shortcuts…", "Tastenkürzel einrichten…", false },
        { "handbookMenuItem", "Handbook", "Handbuch", false },
        { "aboutKesticMenuItem", "About Kestic", "Über Kestic", true },
    };
    int code = 5110;
    for (const Entry& e : entries) {
        QQuickItem* item = item_named(c.window, e.name);
        M3_CHECK(code, item != nullptr, e.name);
        M3_CHECK(code + 1, item->isVisible(), e.name);
        M3_CHECK_EQ(code + 2, item->property("text").toString(), loc(c.lang, e.en, e.de), e.name);
        M3_CHECK(code + 3, item->isEnabled() == e.enabled, e.name);
        M3_CHECK(code + 4, !accessible_name(item).isEmpty(), e.name);
        code += 5;
    }
    return 0;
}

/// The About page shows the running build for the GUI and the background service (the same
/// binary here, so the same build) and no hint about different builds.
int check_about_page(const Ctx& c, int code) {
    QQuickItem* page = about_page(c.window);
    M3_CHECK(code, page != nullptr, "mainAboutPage did not open");
    wait_ms(300);
    finish_incubation(c.window);
    const QString version = qEnvironmentVariable("KESTIC_SMOKE_VERSION");
    const QString commit = qEnvironmentVariable("KESTIC_SMOKE_COMMIT");
    M3_CHECK2(code + 1, !version.isEmpty(), !commit.isEmpty(), "KESTIC_SMOKE_VERSION and KESTIC_SMOKE_COMMIT must be set");
    QQuickItem* app_row = item_named(c.window, "appBuildRow");
    QQuickItem* daemon_row = item_named(c.window, "daemonBuildRow");
    M3_CHECK2(code + 2, app_row, daemon_row, "build rows missing");
    M3_CHECK(code + 3, has_visible_text(app_row, version), "GUI version not shown");
    M3_CHECK(code + 4, has_visible_text(app_row, commit), "GUI commit not shown");
    M3_CHECK(code + 5, has_visible_text(daemon_row, version), "background service version not shown");
    M3_CHECK(code + 6, has_visible_text(daemon_row, commit), "background service commit not shown");
    QObject* hint = object_named(c.window, "buildsDifferMessage");
    M3_CHECK2(code + 7, hint, !hint->property("visible").toBool(), "hint about different builds shown for the same build");
    return 0;
}

/// (a) The ☰ button is visible at the right end of the header and has an accessible name;
/// (b) a mouse click opens the menu with its four entries; (c) a click on «About Kestic» shows
/// the About page; (d) Tab reaches the button, Space opens the menu, the arrow keys reach
/// «About Kestic» and Return opens it; (e) Escape closes the open menu and nothing else;
/// (f) Ctrl+Shift+,
/// opens the settings while the menu is closed.
int scenario_main_menu(const Ctx& c) {
    QQuickWindow* w = c.window;
    QQuickItem* button = item_named(w, "mainMenuButton");
    QObject* menu = object_named(w, "hamburgerMenu");
    M3_CHECK2(5101, button, menu, "main menu button or menu missing");
    M3_CHECK(5102, button->isVisible(), "(a) button not visible");
    M3_CHECK2(5103, button->width() > 0.0, button->height() > 0.0, "(a) button has no size");
    const QPointF top_left = button->mapToScene(QPointF(0.0, 0.0));
    const qreal right_gap = w->width() - (top_left.x() + button->width());
    M3_CHECK2(5104, right_gap >= 0.0, right_gap <= 24.0, "(a) button not at the right end of the window");
    auto* header = w->property("header").value<QQuickItem*>();
    M3_CHECK(5105, header != nullptr, "window header missing");
    M3_CHECK(5106, top_left.y() + button->height() <= header->mapToScene(QPointF(0.0, 0.0)).y() + header->height(),
             "(a) button not in the window header");
    M3_CHECK_EQ(5107, accessible_name(button), loc(c.lang, "Main Menu", "Hauptmenü"), "(a) accessible name");

    // (f) before the menu was ever opened: its actions carry the shortcut from the start.
    auto* settings = qobject_cast<QQuickWindow*>(object_named(w, "settingsWindow"));
    M3_CHECK2(5151, settings, !settings->isVisible(), "settings window missing or already open");
    w->requestActivate();
    wait_ms(100);
    send_key(w, Qt::Key_Comma, Qt::ControlModifier | Qt::ShiftModifier);
    M3_CHECK(5152, wait_until([&] { return settings->isVisible(); }, 2000), "(f) Ctrl+Shift+, did not open the settings");
    settings->close();
    wait_ms(150);

    M3_CHECK(5108, click_ui_item(w, button, "main menu button"), "(b) click on the button failed");
    M3_CHECK(5109, wait_until([&] { return is_open(menu); }, 1000), "(b) menu did not open on click");
    if (const int rc = check_menu_entries(c); rc != 0) {
        return rc;
    }
    M3_CHECK(5130, save_screenshot(w, c.out_path), "screenshot of the open menu failed");

    QQuickItem* about_item = item_named(w, "aboutKesticMenuItem");
    M3_CHECK(5131, click_ui_item(w, about_item, "About Kestic"), "(c) click on About Kestic failed");
    M3_CHECK(5132, wait_until([&] { return is_closed(menu); }, 1000), "(c) menu stayed open");
    if (const int rc = check_about_page(c, 5133); rc != 0) {
        return rc;
    }
    M3_CHECK(5141, save_screenshot(w, sibling_shot(c, "about")), "screenshot of the About page failed");

    close_about(w);
    M3_CHECK(5142, item_named(w, "mainAboutPage") == nullptr, "About page still open");
    QQuickItem* focused = tab_to(w, "mainMenuButton", 60);
    M3_CHECK(5143, focused == button, "(d) Tab does not reach the main menu button");
    press_and_wait(w, Qt::Key_Space, 200);
    M3_CHECK(5144, wait_until([&] { return is_open(menu); }, 1000), "(d) Space did not open the menu");
    for (int i = 0; i < 6 && !about_item->hasActiveFocus(); ++i) {
        press_and_wait(w, Qt::Key_Down, 80);
    }
    M3_CHECK(5145, about_item->hasActiveFocus(), "(d) arrow keys do not reach About Kestic");
    press_and_wait(w, Qt::Key_Return, 200);
    M3_CHECK(5146, wait_until([&] { return is_closed(menu); }, 1000), "(d) menu stayed open after Return");
    M3_CHECK(5147, about_page(w) != nullptr, "(d) Return did not open About Kestic");

    // (e) Escape closes only the menu: About Kestic below it stays open.
    focused = tab_to(w, "mainMenuButton", 60);
    M3_CHECK(5148, focused == button, "(e) Tab does not reach the main menu button");
    press_and_wait(w, Qt::Key_Space, 200);
    M3_CHECK(5149, wait_until([&] { return is_open(menu); }, 1000), "(e) Space did not open the menu");
    press_and_wait(w, Qt::Key_Escape, 200);
    M3_CHECK(5150, wait_until([&] { return is_closed(menu); }, 1000), "(e) Escape did not close the menu");
    M3_CHECK(5153, item_named(w, "mainAboutPage") != nullptr, "(e) Escape on the menu also closed About Kestic");
    close_about(w);
    return 0;
}

/// With a background service of another build: (a) the hint to reopen Kestic shows; (b) a
/// clean commit links to its GitHub page; (c) a `-dirty` commit shows as text without a link.
int scenario_about_builds_differ(const Ctx& c) {
    QQuickWindow* w = c.window;
    QMetaObject::invokeMethod(w, "openAboutPage");
    M3_CHECK(5201, about_page(w) != nullptr, "mainAboutPage did not open");
    c.backend->setProperty("daemon_version", QStringLiteral("9.9.9"));
    c.backend->setProperty("daemon_build_commit", QStringLiteral("0123abc"));
    wait_ms(200);
    finish_incubation(w);
    QObject* hint = object_named(w, "buildsDifferMessage");
    M3_CHECK2(5202, hint, hint->property("visible").toBool(), "(a) hint about different builds not shown");
    M3_CHECK_EQ(5203, hint->property("text").toString(),
                loc(c.lang, "The background service runs another version. Close and reopen Kestic to load the new version.",
                    "Der Hintergrunddienst läuft in einer anderen Version. Kestic schliessen und neu öffnen, um die neue Version zu laden."),
                "(a) hint text");
    QObject* link = object_named(w, "daemonBuildRowCommitLink");
    QObject* text = object_named(w, "daemonBuildRowCommitText");
    M3_CHECK2(5204, link, text, "commit link or text missing");
    M3_CHECK(5205, link->property("visible").toBool(), "(b) clean commit without link");
    M3_CHECK_EQ(5206, link->property("url").toString(), QStringLiteral("https://github.com/Swissbite/kestic/commit/0123abc"), "(b) commit link");
    M3_CHECK(5207, !text->property("visible").toBool(), "(b) clean commit also shown as text");
    M3_CHECK(5208, save_screenshot(w, c.out_path), "screenshot failed");

    c.backend->setProperty("daemon_build_commit", QStringLiteral("0123abc-dirty"));
    wait_ms(200);
    M3_CHECK(5209, !link->property("visible").toBool(), "(c) dirty commit with link");
    M3_CHECK(5210, text->property("visible").toBool(), "(c) dirty commit not shown");
    M3_CHECK_EQ(5211, text->property("text").toString(), QStringLiteral("0123abc-dirty"), "(c) dirty commit text");
    return 0;
}

/// Started with `--show-about` (from the tray), the window opens on «About Kestic».
int scenario_show_about_on_start(const Ctx& c) {
    M3_CHECK(5301, about_page(c.window) != nullptr, "About Kestic not open after the start");
    M3_CHECK(5302, save_screenshot(c.window, c.out_path), "screenshot failed");
    return 0;
}

} // namespace

int run_main_menu_scenario(const m3::Ctx& ctx, const QString& scen, bool* handled) {
    *handled = true;
    if (scen == QLatin1String("m3-main-menu")) {
        return scenario_main_menu(ctx);
    }
    if (scen == QLatin1String("m3-about-builds-differ")) {
        return scenario_about_builds_differ(ctx);
    }
    if (scen == QLatin1String("m3-show-about-on-start")) {
        return scenario_show_about_on_start(ctx);
    }
    *handled = false;
    return 0;
}

} // namespace kestic::smoke
