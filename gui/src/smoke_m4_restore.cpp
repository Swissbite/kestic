// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

// M4 GUI smoke scenarios of the «Restore» area: the restore dialog.

#include "src/smoke_m4_util.h"

namespace kestic::smoke {

namespace {

using namespace m4;

// Whether the one-line path row of the browse page starts with a whole folder: scrolled to
// the start of one of its folders (or not scrolled at all), never into a folder name.
bool path_row_starts_with_whole_folder(QQuickWindow* w) {
    QObject* flick = lookup<QObject>(w, "browsePathFlick");
    QQuickItem* row = visible_item(w, "browsePathRow");
    if (!flick || !row) {
        return false;
    }
    const qreal x = flick->property("contentX").toReal();
    if (x < 0.5) {
        return true;
    }
    for (QQuickItem* crumb : row->childItems()) {
        if (crumb->width() > 0 && qAbs(crumb->x() - x) < 0.5) {
            return true;
        }
    }
    return false;
}

bool wait_for_preview(QObject* model) {
    return wait_until([&] { return model->property("state").toString() == QLatin1String("done"); }, 20000);
}

// Marks `name` in the open folder of the browse page and opens the restore dialog for it.
int open_restore_dialog_for(const Ctx& c, const QString& name, int code) {
    QQuickWindow* w = c.window;
    Browse b = browse_page(w);
    CHECK(code, b.model != nullptr, "browse page shown");
    const int row = browse_names(b.model).indexOf(name);
    CHECK(code + 1, row >= 0, "the file is listed");
    if (!role_value(b.model, row, "selected").toBool()) {
        QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, row));
    }
    QObject* action = lookup<QObject>(w, "restoreSelectionAction");
    CHECK(code + 2, action && action->property("enabled").toBool(), "«Restore Selection…» enabled");
    trigger(action);
    Restore r = restore_dialog(w);
    CHECK(code + 3, r.dialog && r.model, "restore dialog and model exist");
    CHECK(code + 4, wait_until([&] { return is_open(r.dialog); }, 3000), "the restore dialog opens");
    finish_incubation(w);
    return 0;
}

/// A selected name that is not valid UTF-8 (restic shows U+FFFD): the dialog (a) runs no preview
/// and says why, (b) offers «Restore Folder», which (c) replaces the name by its folder and runs
/// the preview for that folder.
int scenario_restore_unmatchable(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6301, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6306)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(8307, wait_for_dir(w, top), "the snapshot opens in its folder");
    // The name also holds markup: the message must show it literally.
    const QString name = QStringLiteral("<b>bad<b>") + QChar(0xFFFD) + QStringLiteral("name.txt");
    if (int rc = open_restore_dialog_for(c, name, 6310)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* error = lookup<QObject>(w, "restorePreviewError");
    QObject* instead = lookup<QObject>(w, "restoreFolderInsteadAction");
    QObject* restore_now = lookup<QObject>(w, "restoreNowAction");
    CHECK(6315, error && instead && restore_now, "dialog controls exist");
    CHECK(6316, wait_until([&] { return r.model->property("state").toString() == QLatin1String("error"); }, 3000),
        "(a) no preview runs");
    CHECK(6317, error->property("visible").toBool(), "(a) the reason is shown");
    const QString folder = top.section(QLatin1Char('/'), -1);
    CHECK(6318, error->property("text").toString().contains(
                       loc(c.lang, "'%1'", "«%1»").arg(folder)),
        "(a) the message names the folder to restore instead");
    CHECK(6319, !restore_now->property("enabled").toBool(), "«Restore Now» stays disabled");
    QQuickItem* shown = nullptr;
    QList<QQuickItem*> parts;
    collect_visual_descendants(qobject_cast<QQuickItem*>(error), parts);
    for (QQuickItem* it : parts) {
        if (it->inherits("QQuickTextEdit") && it->isVisible()) {
            shown = it;
        }
    }
    QString plain;
    if (shown) {
        QMetaObject::invokeMethod(shown, "getText", Q_RETURN_ARG(QString, plain), Q_ARG(int, 0),
            Q_ARG(int, shown->property("length").toInt()));
    }
    CHECK(6328, plain.contains(QStringLiteral("<b>bad<b>")), qPrintable(QStringLiteral(
        "the name with markup is shown literally: ") + plain));
    CHECK(6327, path_row_starts_with_whole_folder(w), "the path row starts with a whole folder");
    CHECK(6320, instead->property("visible").toBool(), "(b) «Restore Folder» offered");
    repaint_fully(w);
    CHECK(6321, save_screenshot(w, c.out_path), "screenshot failed");

    const qreal width_with_message = r.dialog->property("width").toReal();
    trigger(instead);
    CHECK(6322, wait_for_preview(r.model), "(c) the preview of the folder is done");
    CHECK(6326, qAbs(r.dialog->property("width").toReal() - width_with_message) < 1.0,
        "the dialog keeps its width for the message");
    CHECK(6323, !error->property("visible").toBool(), "(c) the message is gone");
    CHECK(6324, restore_now->property("enabled").toBool(), "(c) «Restore Now» enabled for the folder");
    CHECK_EQ(6325, r.dialog->property("title").toString(),
        loc(c.lang, "Restore 1 Item", "1 Element wiederherstellen"), "(c) one folder selected");
    return 0;
}

/// AC-M4-05-16 (f) and AC-M4-05-25 (c), smoke part: at the original location of `docs` (its
/// subfolder exists there) the dialog shows the note on folder permissions; for `b.bin`, which
/// is a folder with content at the target, it shows with «Always» the translated refusal naming
/// that folder and offers «Retry».
int scenario_restore_target_texts(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6501, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6506)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(6509, wait_for_dir(w, top), "the snapshot opens in its folder");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("docs"), 6510)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* original_radio = lookup<QObject>(w, "restoreOriginalRadio");
    QObject* notes = lookup<QObject>(w, "restoreNotesMessage");
    CHECK(6515, original_radio && notes, "dialog controls exist");
    CHECK(6516, wait_for_preview(r.model), "the preview of the new folder is done");
    CHECK(6517, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    CHECK(6518, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    CHECK(6519, wait_for_preview(r.model), "the preview at the original location is done");
    CHECK(6520, notes->property("visible").toBool(), "AC-M4-05-16 (f) the note is shown");
    CHECK(6521, notes->property("text").toString().contains(loc(c.lang,
                       "Existing folders at the target get the permissions and modification time from the snapshot.",
                       "Vorhandene Ordner am Ziel erhalten Zugriffsrechte und Änderungszeit aus dem Snapshot.")),
        qPrintable(QStringLiteral("AC-M4-05-16 (f) the folder note: ") + notes->property("text").toString()));
    repaint_fully(w);
    CHECK(6522, save_screenshot(w, c.out_path), "screenshot failed");

    QMetaObject::invokeMethod(r.dialog, "reject");
    CHECK(6523, wait_until([&] { return is_closed(r.dialog); }, 3000), "the dialog closes");
    QObject* clear = lookup<QObject>(w, "clearSelectionButton");
    CHECK(6533, clear != nullptr, "clearSelectionButton missing");
    CHECK(6534, click_ui_item(w, qobject_cast<QQuickItem*>(clear), "clear selection"),
        "the selection could not be cleared");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("b.bin"), 6524)) {
        return rc;
    }
    Restore refused = restore_dialog(w);
    QObject* refused_original = lookup<QObject>(w, "restoreOriginalRadio");
    QObject* refusal = lookup<QObject>(w, "restorePreviewError");
    CHECK(6532, refused_original && refusal, "dialog controls exist");
    CHECK(6525, wait_for_preview(refused.model), "the preview of the new folder is done");
    CHECK(6526, click_ui_item(w, qobject_cast<QQuickItem*>(refused_original), "original location"),
        "the original location could not be chosen");
    // With «Never» restic leaves the folder alone; with «Always» it would fail there.
    CHECK(6538, wait_for_preview(refused.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(refused.model, "chooseOverwrite", Q_ARG(int, 3));
    CHECK(6527, wait_until([&] { return refused.model->property("state").toString() == QLatin1String("error"); },
                       30000),
        "AC-M4-05-25 the preview refuses the target");
    CHECK(6528, refusal->property("visible").toBool(), "AC-M4-05-25 (c) the refusal is shown");
    const QString text = refusal->property("text").toString();
    // The message wraps the text in a span, so markup in a path shows literally.
    const QString expected =
        loc(c.lang, "Cannot safely restore to %1. Choose 'Into a new folder' instead.",
            "Wiederherstellen nach %1 ist nicht sicher möglich. Stattdessen «In neuen Ordner» wählen.")
            .arg(top + QStringLiteral("/b.bin"));
    CHECK(6529, text.contains(expected),
        qPrintable(QStringLiteral("AC-M4-05-25 (c) translated text naming the folder: ") + text));
    CHECK(6530, !text.contains(QStringLiteral("invalid restore target")), "AC-M4-05-25 (c) no raw text");
    QObject* retry = lookup<QObject>(w, "restoreRetryAction");
    CHECK(6535, retry && !retry->property("visible").toBool(),
        "a refused target offers no «Retry», which would be refused again");
    repaint_fully(w);
    CHECK(6531, save_screenshot(w, screenshot_variant(c.out_path, "-refused")), "screenshot failed");
    // The refusal offers the new folder, which then previews without an error.
    QObject* new_folder = lookup<QObject>(w, "restoreNewFolderInsteadAction");
    CHECK(6539, new_folder && new_folder->property("visible").toBool(),
        "the refusal offers «Into a New Folder»");
    trigger(new_folder);
    CHECK(6540, wait_until([&] { return !refused.model->property("target_original").toBool(); }, 3000),
        "«Into a New Folder» chooses the new folder");
    CHECK(6541, wait_for_preview(refused.model), "the preview of the new folder is done");
    CHECK(6542, !refusal->property("visible").toBool(), "the refusal is gone");
    return 0;
}

/// AC-M4-06-1 (smoke part): the snapshot holds `/home/<user>/Dokumente/a.txt` and the home of
/// the test has `Dokumente`. The restore dialog for `a.txt` (a) offers `~/Dokumente` as the
/// original location and (b) shows the mapping «Original: /home/<user>/Dokumente (<user> →
/// <home>)»; the preview at the mapped folder runs.
int scenario_restore_mapping(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6601, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6606)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    // A snapshot of one file opens at that file; its folder holds it.
    const QString file = role_value(snaps, 0, "startDir").toString();
    const QString folder = file.section(QLatin1Char('/'), 0, -2);
    const QString user = folder.section(QLatin1Char('/'), 2, 2);
    CHECK(6609, folder == QStringLiteral("/home/") + user + QStringLiteral("/Dokumente"),
        qPrintable(QStringLiteral("the snapshot holds /home/<user>/Dokumente: ") + file));
    Browse b = browse_page(w);
    CHECK(6610, QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, folder)), "openDir failed");
    CHECK(6611, wait_for_dir(w, folder), "the folder Dokumente opens");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6612)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* original_radio = lookup<QObject>(w, "restoreOriginalRadio");
    QObject* original_dir = lookup<QObject>(w, "restoreOriginalDirLabel");
    QObject* mapping = lookup<QObject>(w, "restoreMappingLabel");
    CHECK(6617, original_radio && original_dir && mapping, "dialog controls exist");
    CHECK(6618, wait_for_preview(r.model), "the preview of the new folder is done");
    CHECK(6619, original_radio->property("enabled").toBool(), "(a) the original location can be chosen");
    CHECK_EQ(6620, original_dir->property("text").toString(), QStringLiteral("~/Dokumente"),
        "(a) the original location is Dokumente in the user's home");
    const QString home = QString::fromLocal8Bit(qgetenv("HOME")).section(QLatin1Char('/'), -1);
    CHECK(6621, mapping->property("visible").toBool(), "(b) the mapping is shown");
    CHECK_EQ(6622, mapping->property("text").toString(),
        loc(c.lang, "Original: %1 (%2 → %3)", "Original: %1 (%2 → %3)").arg(folder, user, home),
        "(b) the mapping names the original folder and old → new user");
    CHECK(6623, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    CHECK(6624, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    CHECK(6625, wait_for_preview(r.model), "the preview at the mapped folder is done");
    CHECK(6626, r.model->property("can_restore").toBool(), "the mapped folder can be restored into");
    repaint_fully(w);
    return shot(c, 6627);
}

/// AC-M4-05-1, AC-M4-05-4 (smoke part), AC-M4-05-11, AC-M4-05-15 (c, e): the restore dialog
/// (AC-M4-05-1) preselects (a) a new folder and (b) not the original location. At the original
/// location (AC-M4-05-15) (c) «Never» is preselected and (e) «Restore Now» stays disabled until
/// the preview is done; (AC-M4-05-4) the preview names the outcomes and, when files are
/// overwritten, the consequence («1 file will be overwritten. This cannot be undone.») and
/// «Overwrite and Restore». After a restore with «Always» (AC-M4-05-11) (a) a new dialog starts
/// with «Never» again.
int scenario_restore_dialog(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6201, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6206)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(8308, wait_for_dir(w, top), "the snapshot opens in its folder");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6210)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* new_radio = lookup<QObject>(w, "restoreNewFolderRadio");
    QObject* original_radio = lookup<QObject>(w, "restoreOriginalRadio");
    QObject* restore_now = lookup<QObject>(w, "restoreNowAction");
    QObject* overwrite_box = lookup<QObject>(w, "restoreOverwriteBox");
    CHECK(6215, new_radio && original_radio && restore_now && overwrite_box, "dialog controls exist");
    CHECK(6216, new_radio->property("checked").toBool(), "AC-M4-05-1 (a) a new folder is preselected");
    CHECK(6217, !original_radio->property("checked").toBool(), "AC-M4-05-1 (b) not the original location");
    CHECK(6218, wait_for_preview(r.model), "the preview of the new folder is done");
    CHECK_EQ(6219, r.dialog->property("title").toString(),
        loc(c.lang, "Restore 1 Item", "1 Element wiederherstellen"), "dialog title");
    CHECK(6220, restore_now->property("enabled").toBool(), "«Restore Now» enabled for a new folder");
    repaint_fully(w);
    CHECK(6221, save_screenshot(w, c.out_path), "screenshot failed");

    // The preview at the original location waits for its held dry run.
    CHECK(6222, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    CHECK(6223, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    CHECK(6224, r.model->property("state").toString() == QLatin1String("running"), "the preview runs");
    CHECK(6225, !restore_now->property("enabled").toBool(),
        "AC-M4-05-15 (e) «Restore Now» disabled while the preview runs");
    CHECK(8309, overwrite_box->property("currentIndex").toInt() == 0,
        "AC-M4-05-15 (c) «Never» preselected");
    const QString release = qEnvironmentVariable("KESTIC_SMOKE_RELEASE_FILE");
    CHECK(6227, !release.isEmpty() && QFile::remove(release), "the held dry run could not be released");
    CHECK(6228, wait_for_preview(r.model), "the preview at the original location is done");
    CHECK(6229, restore_now->property("enabled").toBool(), "AC-M4-05-15 (e) enabled after the preview");
    QObject* summary = lookup<QObject>(w, "restoreSummaryLabel");
    CHECK(8310, summary != nullptr, "restoreSummaryLabel missing");
    CHECK_EQ(8311, summary->property("text").toString(),
        loc(c.lang, "1 existing file will not be overwritten",
            "1 vorhandene Datei wird nicht überschrieben"),
        "AC-M4-05-4 the preview names the kept file");

    // The modes chosen the way a user does, with the keyboard on the list: the focus stays on
    // the list while previews that overwrite files come in.
    auto* box_item = qobject_cast<QQuickItem*>(overwrite_box);
    CHECK(6276, box_item != nullptr, "the list of modes is an item");
    box_item->forceActiveFocus(Qt::TabFocusReason);
    for (int mode = 1; mode <= 3; ++mode) {
        send_key(w, Qt::Key_Down);
        CHECK(6277, wait_until([&] { return r.model->property("overwrite").toInt() == mode; }, 2000),
            "a mode chosen with the keyboard");
        CHECK(6278, wait_for_preview(r.model), "the preview of the mode is done");
        CHECK(6279, w->activeFocusItem() == box_item, "the focus stays on the list of modes");
    }
    CHECK(8312, overwrite_box->property("currentIndex").toInt() == 3, "«Always» chosen");
    // Reached with the focus elsewhere, overwriting puts the focus on «Cancel».
    qobject_cast<QQuickItem*>(original_radio)->forceActiveFocus(Qt::TabFocusReason);
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 0));
    CHECK(6280, wait_for_preview(r.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    CHECK(8313, wait_for_preview(r.model), "the preview with «Always» is done");
    QObject* consequence = lookup<QObject>(w, "restoreConsequenceMessage");
    CHECK(8314, consequence && consequence->property("visible").toBool(), "AC-M4-05-4 (c) consequence shown");
    CHECK_EQ(6235, consequence->property("text").toString(),
        loc(c.lang, "1 file will be overwritten. This cannot be undone.",
            "1 Datei wird überschrieben. Das lässt sich nicht rückgängig machen."),
        "AC-M4-05-4 (c) the consequence before writing");
    CHECK_EQ(6236, restore_now->property("text").toString(),
        loc(c.lang, "Overwrite and Restore", "Überschreiben und wiederherstellen"), "overwrite button");
    QQuickItem* focused = w->activeFocusItem();
    CHECK(6261, focused != nullptr, "an item holds the focus");
    CHECK_EQ(6262, focused->property("text").toString().remove(QLatin1Char('&')),
        loc(c.lang, "Cancel", "Abbrechen"), "«Cancel» is the default while overwriting");
    QObject* list_button = lookup<QObject>(w, "restoreListButton");
    CHECK(6237, list_button != nullptr, "restoreListButton missing");
    r.dialog->setProperty("listShown", true);
    finish_incubation(w);
    auto* preview_rows = qobject_cast<QAbstractItemModel*>(r.model);
    CHECK(6238, preview_rows && preview_rows->rowCount() == 1, "AC-M4-05-4 (b) one file listed");
    CHECK_EQ(6239, role_value(preview_rows, 0, "outcomeText").toString(),
        loc(c.lang, "Overwritten", "Überschrieben"), "AC-M4-05-4 (b) a.txt overwritten");
    QMetaObject::invokeMethod(r.model, "setFilter", Q_ARG(QString, QStringLiteral("no-such-file")));
    CHECK(6263, wait_until([&] { return preview_rows->rowCount() == 0; }, 3000), "the filter hides the file");
    CHECK(6264, r.model->property("matched").toInt() == 0, "nothing matches the filter");
    QMetaObject::invokeMethod(r.model, "setFilter", Q_ARG(QString, QString()));
    CHECK(6265, wait_until([&] { return preview_rows->rowCount() == 1; }, 3000), "the file again without filter");
    wait_ms(400); // quiet window: the list redraws its row before the screenshot
    repaint_fully(w);
    CHECK(8315, save_screenshot(w, screenshot_variant(c.out_path, "-overwrite")), "screenshot failed");

    trigger(restore_now);
    CHECK(8316, wait_until([&] { return is_closed(r.dialog); }, 3000), "the dialog closes on «Overwrite and Restore»");
    CHECK(8317, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("ok"); }, 30000),
        "the restore ends");
    QObject* banner = lookup<QObject>(w, "restoreRunBanner");
    CHECK(8318, banner && banner->property("visible").toBool(), "the result of the restore is shown");
    CHECK_EQ(8319, r.model->property("run_text").toString(),
        loc(c.lang, "1 file restored", "1 Datei wiederhergestellt"), "result line");
    // The result message must not squeeze the folder list below one entry, also in the
    // narrow window at its minimum height.
    auto* list = lookup(w, "browseListView");
    auto* row = visible_item(w, "browseEntryRow");
    auto* list_header = list ? list->property("headerItem").value<QQuickItem*>() : nullptr;
    CHECK(6273, list && row && list_header, "folder list, its header and an entry exist");
    CHECK(6275, path_row_starts_with_whole_folder(w), "the path row starts with a whole folder");
    CHECK(6274, list->height() >= list_header->height() + row->height(),
        qPrintable(QStringLiteral("the folder list shows at least one entry (%1 px for %2 + %3)")
                       .arg(list->height()).arg(list_header->height()).arg(row->height())));
    repaint_fully(w);
    CHECK(8320, save_screenshot(w, screenshot_variant(c.out_path, "-done")), "screenshot failed");

    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6246)) {
        return rc;
    }
    CHECK(8321, r.model->property("overwrite").toInt() == 0, "AC-M4-05-11 (a) «Never» again");
    CHECK(8322, overwrite_box->property("currentIndex").toInt() == 0, "AC-M4-05-11 (a) list shows «Never»");
    CHECK(6253, new_radio->property("checked").toBool(), "AC-M4-05-11 a new folder again");

    // AC-M4-05-15 (c): another target starts with «Never» again.
    CHECK(6254, wait_for_preview(r.model), "the preview of the new folder is done");
    CHECK(6255, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    CHECK(6256, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    CHECK(6257, wait_until([&] { return r.model->property("overwrite").toInt() == 3; }, 2000), "«Always» chosen");
    CHECK(6258, click_ui_item(w, qobject_cast<QQuickItem*>(new_radio), "new folder"),
        "the new folder could not be chosen");
    CHECK(6259, wait_until([&] { return !r.model->property("target_original").toBool(); }, 3000),
        "the new folder is chosen");
    CHECK(6260, r.model->property("overwrite").toInt() == 0, "AC-M4-05-15 (c) «Never» for another target");
    // AC-M4-05-15 (c), (e) for an existing folder with content (the backed-up folder itself).
    CHECK(6266, wait_for_preview(r.model), "the preview of the new folder is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    CHECK(6267, wait_until([&] { return r.model->property("overwrite").toInt() == 3; }, 2000), "«Always» chosen");
    QMetaObject::invokeMethod(r.model, "chooseFolder", Q_ARG(QString, top));
    CHECK(6268, r.model->property("overwrite").toInt() == 0, "AC-M4-05-15 (c) «Never» for a folder with content");
    CHECK(6269, r.model->property("state").toString() == QLatin1String("running")
        && !restore_now->property("enabled").toBool(), "AC-M4-05-15 (e) disabled while its preview runs");
    CHECK(6270, wait_for_preview(r.model), "the preview of the folder with content is done");
    CHECK(6271, r.model->property("preview_required").toBool(), "AC-M4-05-15 (d) a folder with content needs the preview");
    CHECK(6272, restore_now->property("enabled").toBool(), "AC-M4-05-15 (e) enabled after its preview");
    // Closed without «Cancel» (Escape, a click outside), the dialog stops its running preview.
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 1));
    CHECK(6281, r.model->property("state").toString() == QLatin1String("running"), "a preview runs");
    QMetaObject::invokeMethod(r.dialog, "close");
    CHECK(6282, wait_until([&] { return is_closed(r.dialog); }, 3000), "the dialog closes");
    CHECK_EQ(6283, r.model->property("state").toString(), QStringLiteral("cancelled"),
        "the running preview stopped");
    return 0;
}

/// The preview at the original location of `a.txt` (changed locally), `b.bin` (unchanged) and
/// `docs` (`docs/x.txt` deleted locally) with «Always»: the list of files shows (a) one filter
/// chip per outcome, new, overwritten and unchanged, and (b) a chip with the keyboard focus
/// draws the focus ring.
int scenario_restore_chips(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 6550, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 6555)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(6558, wait_for_dir(w, top), "the snapshot opens in its folder");
    Browse b = browse_page(w);
    for (const char* name : { "b.bin", "docs" }) {
        const int row = browse_names(b.model).indexOf(QLatin1String(name));
        CHECK(6559, row >= 0, "the entry is listed");
        QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, row));
    }
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6560)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    CHECK(6565, wait_for_preview(r.model), "the preview of the new folder is done");
    QObject* original_radio = lookup<QObject>(w, "restoreOriginalRadio");
    CHECK(6566, original_radio != nullptr, "restoreOriginalRadio missing");
    CHECK(6567, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    CHECK(6568, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    CHECK(6569, wait_for_preview(r.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    CHECK(6570, wait_for_preview(r.model), "the preview with «Always» is done");
    r.dialog->setProperty("listShown", true);
    finish_incubation(w);
    QQuickItem* chips = visible_item(w, "restoreKindChips");
    CHECK(6571, chips != nullptr, "(a) the row of chips is shown");
    for (const char* kind : { "new", "overwritten", "identical" }) {
        const QByteArray name = QByteArray("restoreKindChip_") + kind;
        CHECK(6572, visible_item(w, name.constData()) != nullptr,
            qPrintable(QStringLiteral("(a) a chip for the outcome ") + QLatin1String(kind)));
    }
    QQuickItem* chip = visible_item(w, "restoreKindChip_overwritten");
    chip->forceActiveFocus(Qt::TabFocusReason);
    CHECK(6573, wait_until([&] { return chip->hasActiveFocus(); }, 2000), "(b) the chip holds the focus");
    bool ring_shown = false;
    for (QQuickItem* child : chip->childItems()) {
        if (QByteArray(child->metaObject()->className()).startsWith("FocusRing") && child->isVisible()) {
            ring_shown = true;
        }
    }
    CHECK(6574, ring_shown, "(b) the focused chip draws the focus ring");
    scroll_into_view(chips);
    CHECK(6576, wait_until([&] { return shown_in_its_flickables(chips); }, 400), "the chips are scrolled into view for the screenshot");
    repaint_fully(w);
    return shot(c, 6575);
}

// The content of the file at `path` (empty when it cannot be read).
QString file_text(const QString& path) {
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? QString::fromUtf8(f.readAll()) : QString();
}

/// AC-M4-07-2 (a), GUI part: a snapshot without a Kestic tag («Unassigned») of a small folder
/// whose `a.txt` changed locally afterwards. Browsing it shows (AC-M4-03-1 (a)) the folder list
/// with its folders and files and a path row that draws each folder once. Its restore dialog
/// (AC-M4-05-1) preselects (a) a new folder and (b) not the original location. At the original
/// location with «Always» (AC-M4-05-4 (c)) it names the consequence «1 file will be overwritten.
/// This cannot be undone.» and writes nothing before «Overwrite and Restore».
int scenario_foreign_restore(const Ctx& c) {
    QQuickWindow* w = c.window;
    QObject* tab = lookup<QObject>(w, "restoreTabAction");
    CHECK(6701, tab != nullptr, "restoreTabAction missing");
    trigger(tab);
    CHECK(6702, wait_until([&] {
        return w->property("activeTab").toInt() == 4
            && !c.backend->property("restore_overview_loading").toBool()
            && model_rows(w, "restoreGroupsModel") == 2;
    }, 20000), "the restore area lists the unassigned and the orphaned snapshots");
    finish_incubation(w);
    QAbstractItemModel* groups = model_named(w, "restoreGroupsModel");
    CHECK_EQ(6703, role_value(groups, 0, "group").toString(), QStringLiteral("unassigned"), "first group");
    CHECK_EQ(6737, role_value(groups, 0, "subtitle").toString(),
        loc(c.lang, "1 snapshot without a unique Kestic tag", "1 Snapshot ohne eindeutigen Kestic-Tag"),
        "the snapshot without a Kestic tag is unassigned");
    QQuickItem* group_row = tab_to(w, "restoreGroupListItem", 40);
    CHECK(6704, group_row != nullptr, "Tab reaches the further groups");
    send_key(w, Qt::Key_Return);
    CHECK(6705, wait_for_group(w, c.backend, loc(c.lang, "Unassigned", "Nicht zugeordnet"), 1),
        "Return opens the unassigned snapshot");
    finish_incubation(w);
    if (int rc = open_browser_at(c, 0, 6706)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(6709, top.endsWith(QLatin1String("/trees/src")), "the snapshot is of the folder src");
    CHECK(6710, wait_for_dir(w, top), "the snapshot opens in its folder");
    finish_incubation(w);
    Browse b = browse_page(w);
    CHECK_EQ(6711, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("a.txt,b.bin,docs,empty"),
        "AC-M4-07-2 (a) / AC-M4-03-1 (a) the folder list shows the folders and files");
    QStringList names;
    QMetaObject::invokeMethod(b.model, "segmentNames", Q_RETURN_ARG(QStringList, names), Q_ARG(QString, top));
    CHECK(6712, names.size() >= 4, "AC-M4-07-2 (a) / AC-M4-03-1 (a) path row from the top down to «src»");
    if (int rc = check_path_row_drawn_once(c, names)) {
        return rc;
    }
    QMetaObject::invokeMethod(lookup<QObject>(w, "browsePathFlick"), "showEnd");
    CHECK(6713, save_screenshot(w, c.out_path), "screenshot failed");

    const QString changed = top + QStringLiteral("/a.txt");
    CHECK_EQ(6714, file_text(changed), QStringLiteral("changed locally"), "a.txt differs locally");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 6715)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    QObject* new_radio = lookup<QObject>(w, "restoreNewFolderRadio");
    QObject* original_radio = lookup<QObject>(w, "restoreOriginalRadio");
    QObject* restore_now = lookup<QObject>(w, "restoreNowAction");
    CHECK(6720, new_radio && original_radio && restore_now, "dialog controls exist");
    CHECK(6721, new_radio->property("checked").toBool(),
        "AC-M4-07-2 (a) / AC-M4-05-1 (a) a new folder is preselected");
    CHECK(6722, !original_radio->property("checked").toBool(),
        "AC-M4-07-2 (a) / AC-M4-05-1 (b) not the original location");
    CHECK(6723, wait_for_preview(r.model), "the preview of the new folder is done");
    CHECK(6724, click_ui_item(w, qobject_cast<QQuickItem*>(original_radio), "original location"),
        "the original location could not be chosen");
    CHECK(6725, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000),
        "the original location is chosen");
    CHECK(6726, wait_for_preview(r.model), "the preview with «Never» is done");
    QMetaObject::invokeMethod(r.model, "chooseOverwrite", Q_ARG(int, 3));
    CHECK(6727, wait_until([&] { return r.model->property("overwrite").toInt() == 3; }, 2000), "«Always» chosen");
    CHECK(6728, wait_for_preview(r.model), "the preview with «Always» is done");
    QObject* consequence = lookup<QObject>(w, "restoreConsequenceMessage");
    CHECK(6729, consequence && consequence->property("visible").toBool(),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) the consequence is shown");
    CHECK_EQ(6730, consequence->property("text").toString(),
        loc(c.lang, "1 file will be overwritten. This cannot be undone.",
            "1 Datei wird überschrieben. Das lässt sich nicht rückgängig machen."),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) the consequence before writing");
    CHECK_EQ(6731, restore_now->property("text").toString(),
        loc(c.lang, "Overwrite and Restore", "Überschreiben und wiederherstellen"),
        "AC-M4-05-4 (c) the confirmation names overwriting");
    CHECK_EQ(6738, r.model->property("run_state").toString(), QString(),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) no restore runs before the confirmation");
    CHECK_EQ(6732, file_text(changed), QStringLiteral("changed locally"),
        "AC-M4-07-2 (a) / AC-M4-05-4 (c) nothing is written before the confirmation");
    repaint_fully(w);
    CHECK(6733, save_screenshot(w, screenshot_variant(c.out_path, "-overwrite")), "screenshot failed");

    trigger(restore_now);
    CHECK(6734, wait_until([&] { return is_closed(r.dialog); }, 3000),
        "the dialog closes on «Overwrite and Restore»");
    CHECK(6735, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("ok"); }, 30000),
        "the restore ends");
    CHECK_EQ(6736, file_text(changed), QStringLiteral("hello"),
        "AC-M4-05-4 (c) the confirmation writes the file from the snapshot");
    return 0;
}

/// AC-M4-05-26: with every entry of the snapshot's top folder selected, (a) the action reads
/// «Restore All…» and restores the whole snapshot (empty selection, the dialog «Restore
/// Snapshot»), also while a subfolder is shown; (b) an entry outside the top folder keeps
/// «Restore Selection…»; (c) so do all entries of a subfolder. The test checks the restic call.
int scenario_restore_all(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7301, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7306)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(7309, wait_for_dir(w, top), "the snapshot opens in its folder");
    finish_incubation(w);
    Browse b = browse_page(w);
    QObject* action = lookup<QObject>(w, "restoreSelectionAction");
    CHECK(7310, action != nullptr, "restoreSelectionAction missing");
    const QString selection_text = loc(c.lang, "Restore Selection…", "Auswahl wiederherstellen…");
    const QString all_text = loc(c.lang, "Restore All…", "Alles wiederherstellen…");
    CHECK_EQ(7311, browse_names(b.model).join(QLatin1Char(',')), QStringLiteral("a.txt,b.bin,docs,empty"),
        "four entries in the top folder");
    for (int r = 0; r < 3; ++r) {
        QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, r));
    }
    CHECK_EQ(7312, action->property("text").toString(), selection_text, "three of four: «Restore Selection…»");
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, 3));
    CHECK_EQ(7313, action->property("text").toString(), all_text, "(a) all four: «Restore All…»");
    CHECK(7314, b.model->property("whole_snapshot").toBool(), "(a) the selection is the whole snapshot");

    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, top + QStringLiteral("/docs")));
    CHECK(7315, wait_for_dir(w, top + QStringLiteral("/docs")), "«docs» opens");
    CHECK_EQ(7316, action->property("text").toString(), all_text, "(a) also while a subfolder is shown");
    QMetaObject::invokeMethod(b.model, "clearSelection");
    QMetaObject::invokeMethod(b.model, "selectAll");
    CHECK(7317, wait_until([&] { return b.model->property("selection_count").toInt() == 2; }, 5000),
        "both entries of «docs» selected");
    CHECK_EQ(7318, action->property("text").toString(), selection_text, "(c) all of a subfolder: «Restore Selection…»");

    const int cut = top.lastIndexOf(QLatin1Char('/'));
    const QString above = top.left(cut);
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, above));
    CHECK(7319, wait_for_dir(w, above), "the folder above the top folder opens");
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, top));
    CHECK(7320, wait_for_dir(w, top), "the top folder opens again");
    QMetaObject::invokeMethod(b.model, "selectAll");
    CHECK(7321, wait_until([&] { return b.model->property("selection_count").toInt() == 6; }, 5000),
        "the top folder and «docs» selected");
    CHECK_EQ(7322, action->property("text").toString(), all_text,
        "(a) entries below a selected one change nothing");
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, above));
    CHECK(7323, wait_for_dir(w, above), "the folder above opens again");
    const int src_row = browse_names(b.model).indexOf(top.mid(cut + 1));
    CHECK(7324, src_row >= 0, "the top folder is listed in the folder above");
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, src_row));
    CHECK_EQ(7325, action->property("text").toString(), selection_text,
        "(b) the top folder itself selected from above: «Restore Selection…»");
    QMetaObject::invokeMethod(b.model, "toggleSelected", Q_ARG(int, src_row));
    CHECK_EQ(7326, action->property("text").toString(), all_text, "back to every entry of the top folder");

    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, top));
    CHECK(7327, wait_for_dir(w, top), "the top folder opens for the screenshot");
    finish_incubation(w);
    CHECK(7328, save_screenshot(w, c.out_path), "screenshot failed");
    trigger(action);
    Restore r = restore_dialog(w);
    CHECK(7329, r.dialog && wait_until([&] { return is_open(r.dialog); }, 3000), "«Restore All…» opens the dialog");
    CHECK_EQ(7330, r.dialog->property("title").toString(),
        loc(c.lang, "Restore Snapshot", "Snapshot wiederherstellen"),
        "(a) the dialog restores the whole snapshot, like «Restore…» in the context menu");
    CHECK(7331, wait_for_preview(r.model), "the check of the new folder is done");
    QObject* restore_now = lookup<QObject>(w, "restoreNowAction");
    CHECK(7332, restore_now && restore_now->property("enabled").toBool(), "«Restore Now» enabled");
    trigger(restore_now);
    CHECK(7333, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("ok"); }, 60000),
        qPrintable(QStringLiteral("the restore ends ok: ") + r.model->property("run_state").toString()));
    return 0;
}

// AC-M4-05-26 (a), browser opened from the search hit for `text` (a folder hit when `is_dir`) in
// a snapshot of one file, whose tree does not hold its common folder as a folder: the whole
// snapshot is restored from `/`; with every entry of `/` selected the action reads «Restore
// All…» and restores the whole snapshot.
int restore_all_from_hit(const Ctx& c, const QString& text, bool is_dir, int code) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, code, 1)) {
        return rc;
    }
    if (int rc = search_with_keyboard(c, text, code + 5)) {
        return rc;
    }
    Search s = search_page(w);
    int hit = -1;
    for (int r = 0; r < s.model->rowCount(); ++r) {
        if (role_value(s.model, r, "path").toString().endsWith(QLatin1Char('/') + text)
            && role_value(s.model, r, "isDir").toBool() == is_dir) {
            hit = r;
        }
    }
    CHECK(code + 10, hit >= 0, is_dir ? "the folder is a hit" : "the file is a hit");
    const QString path = role_value(s.model, hit, "path").toString();
    const QString folder = role_value(s.model, hit, "folder").toString();
    CHECK(code + 11, path.startsWith(QLatin1String("/home/")), qPrintable(QStringLiteral("the hit below /home: ") + path));
    CHECK(code + 12, folder == path.section(QLatin1Char('/'), 0, -2), "the folder of the hit holds it");
    QMetaObject::invokeMethod(s.page, "showHit", Q_ARG(QVariant, hit));
    CHECK(code + 13, wait_for_dir(w, folder), "the folder of the hit opens");
    finish_incubation(w);
    Browse b = browse_page(w);
    CHECK_EQ(code + 14, b.model->property("top_dir").toString(), QStringLiteral("/"),
        "(a) a snapshot of one file is restored from `/`");
    QMetaObject::invokeMethod(b.page, "openDir", Q_ARG(QVariant, QStringLiteral("/")));
    CHECK(code + 15, wait_for_dir(w, QStringLiteral("/")), "`/` opens");
    QMetaObject::invokeMethod(b.model, "selectAll");
    CHECK(code + 16, wait_until([&] {
            return b.model->property("selection_count").toInt() == b.model->property("total").toInt()
                && b.model->property("selection_count").toInt() > 0;
        }, 5000),
        "every entry of `/` selected");
    QObject* action = lookup<QObject>(w, "restoreSelectionAction");
    CHECK(code + 17, action != nullptr, "restoreSelectionAction missing");
    CHECK_EQ(code + 18, action->property("text").toString(), loc(c.lang, "Restore All…", "Alles wiederherstellen…"),
        "(a) every entry of `/`: «Restore All…»");
    CHECK(code + 19, b.model->property("whole_snapshot").toBool(), "(a) the selection is the whole snapshot");
    QStringList selection;
    QMetaObject::invokeMethod(b.model, "restoreSelection", Q_RETURN_ARG(QStringList, selection));
    CHECK(code + 20, selection.isEmpty(), "(a) the restore request has an empty selection");
    finish_incubation(w);
    CHECK(code + 21, save_screenshot(w, c.out_path), "screenshot failed");
    return 0;
}

/// AC-M4-05-26 (a): «Restore All…» in a snapshot of one file opened from the hit of that file.
int scenario_restore_all_from_search(const Ctx& c) {
    return restore_all_from_hit(c, QStringLiteral("a.txt"), false, 7801);
}

/// AC-M4-05-26 (a): «Restore All…» in a snapshot of one file opened from a folder hit above the
/// file; the browser lists the snapshot's common folder first to learn that it is none.
int scenario_restore_all_from_folder_hit(const Ctx& c) {
    return restore_all_from_hit(c, QStringLiteral("Dokumente"), true, 7841);
}

// The name of the control that holds the keyboard focus in the restore dialog: the object name
// of the nearest named restore control, «close» for the close button of the dialog's header,
// otherwise the text of the button.
QString dialog_focus_id(QQuickWindow* w) {
    QQuickItem* f = w->activeFocusItem();
    for (QQuickItem* it = f; it; it = it->parentItem()) {
        if (it->objectName().startsWith(QLatin1String("restore"))) {
            return it->objectName();
        }
    }
    if (f && QQmlProperty::read(f, QStringLiteral("icon.name")).toString().startsWith(QLatin1String("window-close"))) {
        return QStringLiteral("close");
    }
    return f ? f->property("text").toString().remove(QLatin1Char('&')) : QStringLiteral("-");
}

// The Tab order of the restore dialog from the control in focus, `n` steps forward (or with
// Shift+Tab backward), as names.
QStringList dialog_tab_order(QQuickWindow* w, int n, bool forward) {
    QStringList out;
    for (int i = 0; i < n; ++i) {
        if (forward) {
            send_key(w, Qt::Key_Tab, Qt::NoModifier, QStringLiteral("\t"));
        } else {
            send_key(w, Qt::Key_Backtab, Qt::ShiftModifier);
        }
        out << dialog_focus_id(w);
    }
    return out;
}

/// AC-M4-05-27: the Tab order of the restore dialog is «Into a new folder», «Choose…»,
/// «To the original location», «Existing files» (when enabled), the preview list with its
/// controls (when shown), «Cancel», «Restore Now», the close button; (a) forward and (b)
/// backward, (c) Tab never leaves the dialog, and (d) Esc closes it and the focus returns to the
/// folder list.
int scenario_restore_dialog_tab(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7401, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7406)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(7409, wait_for_dir(w, top), "the snapshot opens in its folder");
    finish_incubation(w);
    CHECK(7410, tab_into(w, visible_item(w, "browseListView"), 60), "Tab reaches the folder list");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7411)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    CHECK(7420, wait_for_preview(r.model), "the check of the new folder is done");
    wait_ms(300); // quiet window: the dialog settles its focus after the check
    const QString cancel = loc(c.lang, "Cancel", "Abbrechen");
    const QString now = loc(c.lang, "Restore Now", "Jetzt wiederherstellen");
    const QString close = QStringLiteral("close");
    CHECK_EQ(7421, dialog_focus_id(w), QStringLiteral("restoreNewFolderRadio"), "the dialog opens on «Into a new folder»");
    const QStringList new_folder = {QStringLiteral("restoreChooseFolderButton"), QStringLiteral("restoreOriginalRadio"),
        cancel, now, close, QStringLiteral("restoreNewFolderRadio")};
    CHECK_EQ(7422, dialog_tab_order(w, 6, true).join(QLatin1String(" > ")), new_folder.join(QLatin1String(" > ")),
        "(a, c) Tab order into a new folder");
    QStringList back = new_folder;
    std::reverse(back.begin(), back.end());
    back.removeFirst();
    back << QStringLiteral("restoreNewFolderRadio");
    CHECK_EQ(7423, dialog_tab_order(w, 6, false).join(QLatin1String(" > ")), back.join(QLatin1String(" > ")),
        "(b, c) Shift+Tab order into a new folder");

    QObject* original = lookup<QObject>(w, "restoreOriginalRadio");
    CHECK(7424, click_ui_item(w, qobject_cast<QQuickItem*>(original), "original location"), "original location");
    CHECK(7425, wait_until([&] { return r.model->property("target_original").toBool(); }, 3000), "original chosen");
    CHECK(7426, wait_for_preview(r.model), "the preview at the original location is done");
    QObject* list_button = lookup<QObject>(w, "restoreListButton");
    CHECK(7427, list_button && click_ui_item(w, qobject_cast<QQuickItem*>(list_button), "list"), "the list opens");
    CHECK(7428, wait_until([&] { return visible_item(w, "restorePreviewList") != nullptr; }, 3000), "the preview list is shown");
    finish_incubation(w);
    qobject_cast<QQuickItem*>(original)->forceActiveFocus(Qt::TabFocusReason);
    QStringList preview;
    preview << QStringLiteral("restoreOverwriteBox") << QStringLiteral("restoreListButton");
    if (visible_item(w, "restorePreviewFilter")) {
        preview << QStringLiteral("restorePreviewFilter");
    }
    preview << QStringLiteral("restorePreviewList") << cancel;
    const QString restore_text = r.model->property("overwrites").toBool()
        ? loc(c.lang, "Overwrite and Restore", "Überschreiben und wiederherstellen") : now;
    preview << restore_text << close << QStringLiteral("restoreNewFolderRadio") << QStringLiteral("restoreChooseFolderButton")
            << QStringLiteral("restoreOriginalRadio");
    CHECK_EQ(7429, dialog_tab_order(w, preview.size(), true).join(QLatin1String(" > ")), preview.join(QLatin1String(" > ")),
        "(a, c) Tab order at the original location with the list shown");
    QStringList preview_back = preview;
    std::reverse(preview_back.begin(), preview_back.end());
    preview_back.removeFirst();
    preview_back << QStringLiteral("restoreOriginalRadio");
    CHECK_EQ(7430, dialog_tab_order(w, preview_back.size(), false).join(QLatin1String(" > ")),
        preview_back.join(QLatin1String(" > ")), "(b, c) Shift+Tab order at the original location");
    repaint_fully(w);
    CHECK(7431, save_screenshot(w, c.out_path), "screenshot failed");

    send_key(w, Qt::Key_Escape);
    CHECK(7432, wait_until([&] { return is_closed(r.dialog); }, 3000), "(d) Esc closes the dialog");
    CHECK(7433, wait_until([&] { return focused_entry_row(w) != nullptr; }, 2000),
        qPrintable(QStringLiteral("(d) the focus returns to the folder list: ") + focus_name(w)));
    return 0;
}

// The icon `message` (an InlineMessage) shows: its visible Kirigami icon, or nullptr.
QQuickItem* message_icon(QObject* message) {
    QList<QQuickItem*> items;
    collect_visual_descendants(qobject_cast<QQuickItem*>(message), items);
    for (QQuickItem* it : items) {
        if (it->isVisible() && it->property("valid").isValid() && it->property("source").isValid()
            && it->width() > 0) {
            return it;
        }
    }
    return nullptr;
}

/// AC-M4-05-29: the restore banner (a) shows a running restore with the icon `document-revert`,
/// which the icon theme has, (b) a cancelled restore as neutral information with the icon
/// `process-stop` and «Restore cancelled», and an interrupted one as neutral information with
/// the icon `state-offline`, and (c) comes back for the next restore after its close button.
int scenario_restore_banner(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7601, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7606)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    CHECK(7609, wait_for_dir(w, role_value(snaps, 0, "startDir").toString()), "the snapshot opens");
    finish_incubation(w);
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7610)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    CHECK(7620, wait_for_preview(r.model), "the check of the new folder is done");
    trigger(lookup<QObject>(w, "restoreNowAction"));
    CHECK(7621, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("running"); }, 10000),
        "the restore runs (restic held)");
    QObject* banner = lookup<QObject>(w, "restoreRunBanner");
    CHECK(7622, banner && banner->property("visible").toBool(), "the banner shows the running restore");
    QQuickItem* running_icon = nullptr;
    CHECK(7623, wait_until([&] { return (running_icon = message_icon(banner)) != nullptr; }, 400),
        "the banner shows an icon");
    CHECK_EQ(7624, running_icon->property("source").toString(), QStringLiteral("document-revert"),
        "(a) the running restore shows the restore icon");
    CHECK(7625, running_icon->property("valid").toBool(), "(a) the icon theme has the icon");
    // AC-M4-05-30 (d): the test advances the daemon's clock and the bytes restored.
    CHECK(7634, wait_until([&] {
        return r.model->property("run_text").toString().contains(QStringLiteral("/s"));
    }, 20000), qPrintable(QStringLiteral("AC-M4-05-30 (d) the message names the speed: ")
        + r.model->property("run_text").toString()));
    repaint_fully(w);
    CHECK(7626, save_screenshot(w, screenshot_variant(c.out_path, "-running")), "screenshot failed");

    trigger(lookup<QObject>(w, "restoreRunCancelAction"));
    CHECK(7627, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("cancelled"); }, 30000),
        qPrintable(QStringLiteral("(b) the run ends as cancelled: ") + r.model->property("run_state").toString()));
    CHECK(7628, banner->property("type").toInt() == 0, "(b) neutral information, no error");
    CHECK_EQ(7629, r.model->property("run_text").toString(),
        loc(c.lang, "Restore cancelled", "Wiederherstellen abgebrochen"), "(b) the text names the run kind");
    QQuickItem* cancelled_icon = nullptr;
    CHECK(7630, wait_until([&] {
        cancelled_icon = message_icon(banner);
        return cancelled_icon && cancelled_icon->property("source").toString() == QLatin1String("process-stop");
    }, 300), "the banner shows an icon");
    CHECK_EQ(7631, cancelled_icon->property("source").toString(), QStringLiteral("process-stop"),
        "(b) the icon of «cancelled»");
    CHECK(7632, cancelled_icon->property("valid").toBool(), "(b) the icon theme has the icon");
    repaint_fully(w);
    CHECK(7633, save_screenshot(w, c.out_path), "screenshot failed");

    // (c) Closed with its close button, the message comes back for the next restore.
    QQuickItem* close = nullptr;
    QList<QQuickItem*> items;
    collect_visual_descendants(qobject_cast<QQuickItem*>(banner), items);
    for (QQuickItem* it : items) {
        // A cancelled restore offers no action: the only button is the close button.
        if (it->isVisible() && it->inherits("QQuickAbstractButton")) {
            CHECK(7647, close == nullptr, "one button in the message of a cancelled restore");
            close = it;
        }
    }
    CHECK(7635, close != nullptr, "the message has a close button");
    CHECK(7636, click_ui_item(w, close, "close message"), "the close button could not be clicked");
    CHECK(7637, wait_until([&] { return !banner->property("visible").toBool(); }, 3000), "the message is closed");
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7638)) {
        return rc;
    }
    CHECK(7642, wait_for_preview(r.model), "the check of the new folder is done");
    trigger(lookup<QObject>(w, "restoreNowAction"));
    CHECK(7643, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("running"); }, 10000),
        "the second restore runs");
    CHECK(7644, wait_until([&] { return banner->property("visible").toBool(); }, 3000),
        "(c) the message shows the second restore");
    QObject* cancel_action = lookup<QObject>(w, "restoreRunCancelAction");
    CHECK(7645, cancel_action && cancel_action->property("visible").toBool(), "(c) with «Cancel»");
    trigger(cancel_action);
    CHECK(7646, wait_until([&] { return r.model->property("run_state").toString() == QLatin1String("cancelled"); }, 30000),
        "the second restore ends");

    // (b) An interrupted restore (sleep, daemon start): the model maps the run status to the state
    // «interrupted» (unit test); here the banner shows that state.
    r.model->setProperty("run_state", QStringLiteral("interrupted"));
    CHECK(7648, banner->property("visible").toBool(), "(b) the banner shows the interrupted restore");
    CHECK(7649, banner->property("type").toInt() == 0, "(b) interrupted: neutral information, no error");
    QQuickItem* interrupted_icon = nullptr;
    CHECK(7650, wait_until([&] {
        interrupted_icon = message_icon(banner);
        return interrupted_icon && interrupted_icon->property("source").toString() == QLatin1String("state-offline");
    }, 300), "the banner shows an icon");
    CHECK_EQ(7651, interrupted_icon->property("source").toString(), QStringLiteral("state-offline"),
        "(b) the icon of «interrupted»");
    CHECK(7652, interrupted_icon->property("valid").toBool(), "(b) the icon theme has the icon");
    return 0;
}

/// AC-M4-05-28: a network folder (`smb://`, `sftp://`) from the folder chooser (a) leaves the
/// target unchanged, (b) shows that only local folders can be chosen, and (c) asks the daemon
/// nothing; a local folder after it is taken.
int scenario_restore_network_folder(const Ctx& c) {
    QQuickWindow* w = c.window;
    if (int rc = open_restore_snapshots(c, 7701, 1)) {
        return rc;
    }
    if (int rc = open_browser_at(c, 0, 7706)) {
        return rc;
    }
    QAbstractItemModel* snaps = model_named(w, "restoreSnapshotsModel");
    const QString top = role_value(snaps, 0, "startDir").toString();
    CHECK(7709, wait_for_dir(w, top), "the snapshot opens");
    finish_incubation(w);
    if (int rc = open_restore_dialog_for(c, QStringLiteral("a.txt"), 7710)) {
        return rc;
    }
    Restore r = restore_dialog(w);
    CHECK(7720, wait_for_preview(r.model), "the check of the new folder is done");
    QObject* message = lookup<QObject>(w, "restoreNetworkFolderMessage");
    CHECK(7721, message && !message->property("visible").toBool(), "no message before a network folder");
    const QString target = r.model->property("target_dir").toString();
    // A chooser closed without a folder changes nothing and is no network folder.
    QMetaObject::invokeMethod(r.dialog, "chooseFolderUrl", Q_ARG(QVariant, QUrl()));
    CHECK(7731, !message->property("visible").toBool(), "no message for an empty choice");
    CHECK_EQ(7732, r.model->property("target_dir").toString(), target, "an empty choice keeps the target");
    const QString text = loc(c.lang,
        "Network folders such as smb:// cannot be chosen directly. Mount the folder first or choose a local folder.",
        "Netzwerkordner wie smb:// lassen sich nicht direkt wählen. Den Ordner zuerst einbinden oder einen lokalen Ordner wählen.");
    for (const char* url : {"smb://ds1525.local/home/upload/restore", "sftp://nas.local/srv/restore",
             "file://nas.local/srv/restore"}) {
        QMetaObject::invokeMethod(r.dialog, "chooseFolderUrl", Q_ARG(QVariant, QUrl(QString::fromLatin1(url))));
        // quiet window: a check the network folder must not start would have begun by now.
        wait_ms(500);
        CHECK_EQ(7722, r.model->property("target_dir").toString(), target, "(a) the target stays");
        CHECK_EQ(7723, r.model->property("state").toString(), QStringLiteral("done"),
            "(c) no new check: the daemon is not asked");
        CHECK(7724, message->property("visible").toBool(), "(b) the message is shown");
        CHECK(7725, message->property("text").toString().contains(text.toHtmlEscaped()),
            qPrintable(QStringLiteral("(b) the message: ") + message->property("text").toString()));
    }
    repaint_fully(w);
    CHECK(7726, save_screenshot(w, c.out_path), "screenshot failed");
    QObject* new_radio = lookup<QObject>(w, "restoreNewFolderRadio");
    CHECK(7729, click_ui_item(w, qobject_cast<QQuickItem*>(new_radio), "new folder"), "the new folder is chosen");
    CHECK(7730, wait_until([&] { return !message->property("visible").toBool(); }, 3000),
        "the message goes with another target");
    QMetaObject::invokeMethod(r.dialog, "chooseFolderUrl",
        Q_ARG(QVariant, QUrl(QStringLiteral("file://localhost") + top)));
    CHECK(7727, wait_until([&] { return r.model->property("target_dir").toString() == top; }, 5000),
        "a local folder written with the host «localhost» is taken");
    CHECK(7728, !message->property("visible").toBool(), "no message with a local folder");
    return 0;
}

const Scenario kScenarios[] = {
    {"m4-restore-dialog", scenario_restore_dialog},
    {"m4-restore-chips", scenario_restore_chips},
    {"m4-restore-unmatchable", scenario_restore_unmatchable},
    {"m4-restore-target-texts", scenario_restore_target_texts},
    {"m4-restore-mapping", scenario_restore_mapping},
    {"m4-foreign-restore", scenario_foreign_restore},
    {"m4-restore-all", scenario_restore_all},
    {"m4-restore-all-from-search", scenario_restore_all_from_search},
    {"m4-restore-all-from-folder-hit", scenario_restore_all_from_folder_hit},
    {"m4-restore-dialog-tab", scenario_restore_dialog_tab},
    {"m4-restore-banner", scenario_restore_banner},
    {"m4-restore-network-folder", scenario_restore_network_folder},
};

} // namespace

m3::ScenarioFn find_m4_restore_scenario(const QString& scen) {
    return find_scenario(kScenarios, scen);
}

} // namespace kestic::smoke
