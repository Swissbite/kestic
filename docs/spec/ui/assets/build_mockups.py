#!/usr/bin/env python3
"""Kestic UI mockups: erzeugt kestic-mockups.html (eigenstaendig) und optional PNGs.

Farben aus /usr/share/color-schemes/Breeze{Light,Dark}.colors, Icons aus /usr/share/icons/breeze.
Aufruf:  python3 build_mockups.py [--png]
Die Mockups sind Illustrationen zu «GUI Design.md», kein Code der App.
"""
import base64, glob, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_HTML = os.path.join(HERE, "kestic-mockups.html")
ICON_ROOT = "/usr/share/icons/breeze"

# ---------------------------------------------------------------- Farben
def read_scheme(name):
    path = f"/usr/share/color-schemes/{name}.colors"
    sec, data = None, {}
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if line.startswith("["):
            sec = line[1:-1]
            continue
        if "=" in line and sec:
            k, v = line.split("=", 1)
            data[(sec, k)] = v

    def c(sec, key):
        r, g, b = (int(x) for x in data[(f"Colors:{sec}", key)].split(",")[:3])
        return f"rgb({r},{g},{b})"

    def rgba(sec, key, a):
        r, g, b = (int(x) for x in data[(f"Colors:{sec}", key)].split(",")[:3])
        return f"rgba({r},{g},{b},{a})"

    return {
        "win-bg": c("Window", "BackgroundNormal"), "win-alt": c("Window", "BackgroundAlternate"),
        "view-bg": c("View", "BackgroundNormal"), "view-alt": c("View", "BackgroundAlternate"),
        "header-bg": c("Header", "BackgroundNormal"), "btn-bg": c("Button", "BackgroundNormal"),
        "text": c("Window", "ForegroundNormal"), "text-in": c("Window", "ForegroundInactive"),
        "sel": c("Selection", "BackgroundNormal"), "sel-text": c("Selection", "ForegroundNormal"),
        "sel-soft": rgba("Selection", "BackgroundNormal", 0.22),
        "pos": c("Window", "ForegroundPositive"), "neu": c("Window", "ForegroundNeutral"),
        "neg": c("Window", "ForegroundNegative"), "link": c("Window", "ForegroundLink"),
        "pos-soft": rgba("Window", "ForegroundPositive", 0.16), "neu-soft": rgba("Window", "ForegroundNeutral", 0.16),
        "neg-soft": rgba("Window", "ForegroundNegative", 0.14), "info-soft": rgba("Selection", "BackgroundNormal", 0.14),
        "sep": rgba("Window", "ForegroundNormal", 0.16), "sep-strong": rgba("Window", "ForegroundNormal", 0.28),
        "compl-bg": c("Complementary", "BackgroundNormal"), "compl-text": c("Complementary", "ForegroundNormal"),
        "_text_hex": data[("Colors:Window", "ForegroundNormal")],
    }

LIGHT, DARK = read_scheme("BreezeLight"), read_scheme("BreezeDark")

# ---------------------------------------------------------------- Icons
_icon_cache = {}
def _find_icon(name, sizes=("22", "16", "24", "32", "48")):
    for s in sizes:
        hits = glob.glob(f"{ICON_ROOT}/*/{s}/{name}.svg")
        if hits:
            return hits[0]
    hits = glob.glob(f"{ICON_ROOT}/**/{name}.svg", recursive=True)
    if not hits:
        raise SystemExit(f"Icon fehlt: {name}")
    return hits[0]

def _uri(svg, text_rgb):
    svg = re.sub(r"color:\s*#232629", f"color:rgb({text_rgb})", svg)
    return "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()

def ic(name, size=22, cls="", sizes=None):
    """Icon in beiden Themes (img mit Klasse il/id, per CSS umgeschaltet)."""
    key = (name, sizes)
    if key not in _icon_cache:
        path = _find_icon(name, sizes) if sizes else _find_icon(name)
        svg = open(path, encoding="utf-8").read()
        _icon_cache[key] = (_uri(svg, LIGHT["_text_hex"]), _uri(svg, DARK["_text_hex"]))
    l, d = _icon_cache[key]
    return (f'<img class="ic il {cls}" src="{l}" width="{size}" height="{size}" alt="">'
            f'<img class="ic id {cls}" src="{d}" width="{size}" height="{size}" alt="">')

# ---------------------------------------------------------------- Bausteine
def win(title, body, w=1180, h=740):
    return f'''<div class="win" style="width:{w}px;height:{h}px">
  <div class="titlebar"><span class="tb-icon">{ic("backup",16)}</span><span class="tb-title">{title}</span>
  <span class="tb-btns">{ic("window-minimize",16)}{ic("window-maximize",16)}{ic("window-close",16)}</span></div>
  <div class="client">{body}</div></div>'''

TABS = [("overview", "Übersicht", "dashboard-show"), ("restore", "Wiederherstellen", "document-revert"),
        ("backups", "Backups", "folder-sync"), ("dest", "Ziele", "network-server"), ("history", "Verlauf", "view-history")]

def tabbar(active, emblems=("backups", "dest"), bottom=False):
    items = ""
    for key, label, icon in TABS:
        badge = f'<span class="tab-badge">{ic("emblem-important",12)}</span>' if key in emblems else ""
        items += f'<div class="tab {"on" if key == active else ""}">{badge}{ic(icon,22)}<span>{label}</span></div>'
    return f'<div class="tabrow {"bottom" if bottom else ""}"><div class="tabs">{items}</div></div>'

def tbtn(text, icon=None, menu=False, primary=False, disabled=False):
    i = ic(icon, 22) if icon else ""
    m = f'<span class="caret">{ic("arrow-down",16)}</span>' if menu else ""
    cls = ("primary " if primary else "") + ("disabled" if disabled else "")
    return f'<span class="tbtn {cls}">{i}{f"<span>{text}</span>" if text else ""}{m}</span>'

def btn(text, icon=None, kind=""):
    i = ic(icon, 16) if icon else ""
    t = f"<span>{text}</span>" if text else ""
    return f'<span class="btn {kind}">{i}{t}</span>'

BURGER = "<!--BURGER-->"
def page(title, body, actions="", width=None, bg="view", breadcrumb=None, head=True):
    style = f"width:{width}px;flex:none" if width else "flex:1"
    crumbs = ""
    if breadcrumb:
        title = f'<span class="crumbtitle">{breadcrumb}</span>'
    ph = f'<div class="pagehead"><span class="ptitle">{title or ""}</span><span class="pactions">{actions}{BURGER}</span></div>' if head else ""
    return f'''<div class="page bg-{bg}" style="{style}">{ph}{crumbs}<div class="pbody">{body}</div></div>'''

def columns(*pages, burger=True):
    html = '<div class="columns">' + '<div class="colsep"></div>'.join(pages) + "</div>"
    if burger and BURGER in html:
        i = html.rfind(BURGER)
        html = html[:i] + f'<span class="tbtn">{ic("application-menu",22)}</span>' + html[i + len(BURGER):]
    return html.replace(BURGER, "")

def li(icon, title, sub="", trailing="", selected=False, arrow=False, small=False):
    t = f'<span class="li-trail">{trailing}</span>' if trailing else ""
    a = f'<span class="li-arrow">{ic("go-next",16)}</span>' if arrow else ""
    s = f'<div class="li-sub">{sub}</div>' if sub else ""
    icn = f'<span class="li-icon">{ic(icon, 22 if small else 32, sizes=("22",) if small else ("32","22"))}</span>' if icon else ""
    return f'<div class="li {"sel" if selected else ""}">{icn}<div class="li-text"><div class="li-title">{title}</div>{s}</div>{t}{a}</div>'

def section(text):
    return f'<div class="lsection">{text}</div>'

def inviewheader(title, action):
    return f'<div class="ivh"><span class="ivh-title">{title}</span>{action}</div>'

MSG_ICON = {"info": "dialog-information", "pos": "dialog-positive", "warn": "dialog-warning", "err": "dialog-error"}
def inline(kind, text, actions="", closable=True):
    c = f'<span class="im-close">{ic("window-close",16)}</span>' if closable else ""
    return f'<div class="im im-{kind}">{ic(MSG_ICON[kind],22)}<div class="im-text">{text}</div><div class="im-actions">{actions}</div>{c}</div>'

def heading(text, level=2):
    return f'<div class="h h{level}">{text}</div>'

def status(icon, text, kind=""):
    return f'<span class="status {kind}">{ic(icon,16)}<span>{text}</span></span>'

def checkbox(label, on=False, disabled=False):
    return f'<label class="cb {"on" if on else ""} {"disabled" if disabled else ""}"><span class="box"></span><span>{label}</span></label>'

def radio(label, on=False):
    return f'<label class="rb {"on" if on else ""}"><span class="dot"></span><span>{label}</span></label>'

def switch(label, on=False):
    return f'<label class="sw {"on" if on else ""}"><span class="track"><span class="knob"></span></span><span>{label}</span></label>'

def combo(text, w=None):
    st = f' style="width:{w}px"' if w else ""
    return f'<span class="combo"{st}><span>{text}</span>{ic("arrow-down",16)}</span>'

def spin(v, w=64, disabled=False):
    return f'<span class="spin {"disabled" if disabled else ""}" style="width:{w}px"><span>{v}</span><span class="spin-arrows">▴<br>▾</span></span>'

def field(v="", ph="", w=None, mono=False, pw=False):
    st = f' style="width:{w}px"' if w else ""
    txt = "•" * 10 if pw else (v or f'<span class="ph">{ph}</span>')
    return f'<span class="field {"mono" if mono else ""}"{st}>{txt}</span>'

def chip(text, on=False):
    return f'<span class="chip {"on" if on else ""}">{text}</span>'

def progress(pct):
    return f'<div class="prog"><div class="prog-fill" style="width:{pct}%"></div></div>'

def form(rows):
    """rows: (label, widget) oder ("#", Abschnittstitel)"""
    out = ""
    for lab, w in rows:
        if lab == "#":
            out += f'<div class="fsec">{w}</div>'
        else:
            out += f'<div class="flabel">{lab}</div><div class="fwidget">{w}</div>'
    return f'<div class="form">{out}</div>'

def card(body):
    return f'<div class="card">{body}</div>'

def table(headers, rows, widths=None):
    th = "".join(f"<th>{h}</th>" for h in headers)
    tr = "".join("<tr>" + "".join(f"<td>{c}</td>" for c in r) + "</tr>" for r in rows)
    return f'<table class="tbl"><thead><tr>{th}</tr></thead><tbody>{tr}</tbody></table>'

def dlg_box(title, body, buttons, w=520):
    return f'''<div class="dialog" style="width:{w}px">
  <div class="dlg-head"><span class="h h2">{title}</span><span class="im-close">{ic("window-close",16)}</span></div>
  <div class="dlg-body">{body}</div><div class="dlg-foot">{buttons}</div></div>'''

def dialog(title, body, buttons, w=520):
    return f'<div class="overlay">{dlg_box(title, body, buttons, w)}</div>'

def sheet(*boxes):
    return '<div class="sheet">' + "".join(f'<div class="sheet-cell">{b}</div>' for b in boxes) + '</div>'

def placeholder(icon, text, expl, action=""):
    return f'''<div class="phm">{ic(icon,64, sizes=("64","48","32","22"))}<div class="h h2">{text}</div>
  <div class="phm-expl">{expl}</div>{action}</div>'''

def stateline(kind, text, actions=""):
    return f'<div class="stateline">{inline(kind, text, actions, closable=False)}</div>'

def default_btn(text, icon=None, kind=""):
    """Knopf mit Default-Fokus (Enter)."""
    return btn(text, icon, (kind + " default").strip())

# ---------------------------------------------------------------- Referenz-Datensatz (GUI Design §0.2)
OV = "dashboard-show"
B_HOME = ("user-home", "Persönlicher Ordner", "~ → NAS")
B_FOTO = ("drive-harddisk", "Fotos", "/media/fotos → USB-Disk")
B_PROJ = ("folder", "Projekte", "~/Projekte → S3")
ST_HOME = ("state-ok", "Gesichert · vor 2 Stunden", "ok")
ST_FOTO = ("chronometer", "Wartet: Quelle nicht verfügbar", "")
ST_PROJ = ("state-error", "Fehlgeschlagen: Passwort abgelehnt", "err")
ERR_PROJ = "Backup «Projekte» fehlgeschlagen: Der S3-Speicher hat das gespeicherte Passwort abgelehnt."

def backup_list(selected=None, stand=""):
    return (inviewheader("3 Backups", btn("Neues Backup…", "list-add"))
            + li(B_HOME[0], B_HOME[1], status(ST_HOME[0], ST_HOME[1] + stand, ST_HOME[2]), selected=selected == "home")
            + li(B_FOTO[0], B_FOTO[1], status(*ST_FOTO), selected=selected == "foto")
            + li(B_PROJ[0], B_PROJ[1], status(*ST_PROJ), selected=selected == "proj"))

# ---------------------------------------------------------------- Screens
def ovcard(b, st, next_txt, button):
    return card(f'''<div class="card-head">{ic(b[0],32,sizes=("32","22"))}<div><div class="h h3">{b[1]}</div><div class="muted">{b[2]}</div></div></div>
        {st}<div class="muted small">{next_txt}</div>
        <div class="card-foot">{button}<span class="tbtn">{ic("overflow-menu",22)}</span></div>''')

def s_overview(running=False, banner=False):
    sb = stateline("warn", "<b>Zeitpläne pausiert</b> seit 13:00, bis 15:00.", btn("Fortsetzen", "media-playback-start")) if banner else ""
    disabled_now = '<span class="btn disabled">' + ic("media-playback-start", 16) + '<span>Jetzt sichern</span></span>'
    if running:
        c1 = card(f'''<div class="card-head">{ic("user-home",32,sizes=("32","22"))}<div><div class="h h3">Persönlicher Ordner</div><div class="muted">~ → NAS</div></div></div>
        {status("state-sync","Wird gesichert · 42 %")}{progress(42)}<div class="muted small">1,2 GB von 3 GB · noch etwa 4 Minuten</div>
        <div class="card-foot">{btn("Abbrechen","process-stop")}<span class="tbtn">{ic("overflow-menu",22)}</span></div>''')
        sub = "Persönlicher Ordner wird gesichert · Fotos: Wartet: Quelle nicht verfügbar"
    else:
        c1 = ovcard(B_HOME, status(*ST_HOME), "Nächstes Backup: morgen 12:00", btn("Jetzt sichern", "media-playback-start"))
        sub = "Persönlicher Ordner gesichert vor 2 Stunden · Fotos wartet"
    c2 = ovcard(B_FOTO, status(*ST_FOTO), "Nächstes Backup: sobald eingesteckt", disabled_now)
    c3 = ovcard(B_PROJ, status(*ST_PROJ), "Nächstes Backup: Mo 16.11. 20:00", btn("Gespeichertes Passwort aktualisieren…", "dialog-password"))
    head = f'{heading(ic("state-error",22)+" 1 Backup fehlgeschlagen",1)}<div class="muted">{sub}</div>'
    msgs = (inline("warn", "Für das Ziel <b>S3</b> gibt es kein Wiederherstellungs-Kit. Ohne Kit sind die Daten bei Verlust des Passworts nicht mehr lesbar.",
                     '<span class="btn disabled">' + ic("security-high",16) + '<span>Kit erzeugen…</span></span><span class="muted small reason">Zuerst gespeichertes Passwort aktualisieren</span>' + btn("Später")))
    body = f'{msgs}<div class="ov-head">{head}</div><div class="cards">{c1}{c2}{c3}</div>'
    p = page(None, body, tbtn("Alle jetzt sichern", "media-playback-start"), bg="win")
    return win("Kestic", sb + tabbar("overview") + columns(p), h=760)

def s_backups(dark=False):
    snaps = section("Snapshots") + li(None, "Heute 12:00", "312 MB neu", arrow=True, small=True) + \
        li(None, "Gestern 12:00", "45 MB neu · 2 Dateien nicht lesbar", arrow=True, small=True) + li(None, "Alle 46 Snapshots anzeigen", arrow=True, small=True)
    sett = section("Einstellungen") + "".join(
        li(None, a, b, arrow=True, small=True) for a, b in [
            ("Quelle", "~ (Ordner)"), ("Ausschluss-Regeln", "3 Ordner, 2 Vorlagen"),
            ("Zeitplan", "täglich 12:00 · am Strom, mit Netzwerk"),
            ("Aufbewahrung", "7 tägliche, 4 wöchentliche, 12 monatliche"), ("Ziel", "NAS (REST-Server)"),
            ("Gesamte Konfiguration anzeigen", "")])
    danger = f'<div class="dangerzone"><div class="h h4">Gefahrenbereich</div><div class="muted small">Löscht alle Snapshots dieses Backups endgültig vom Ziel.</div>{btn("Backup komplett löschen…","edit-delete","danger")}</div>'
    detail_head = f'<div class="detail-status">{status("state-ok","Gesichert · vor 2 Stunden · 312 MB neu","ok")}<div class="muted">Nächstes Backup morgen um 12:00 · nur am Strom und mit Netzwerk</div></div>'
    d = page("Persönlicher Ordner", detail_head + snaps + sett + danger,
             tbtn("Jetzt sichern", "media-playback-start") + tbtn("Pausieren", "media-playback-pause", menu=True) + tbtn("", "overflow-menu"), bg="win")
    l = page(None, backup_list("home"), width=360, bg="view", head=False)
    return win("Kestic", tabbar("backups") + columns(l, d), h=830)

def s_excludes():
    lst = inviewheader("Diese Ordner nicht sichern", btn("Ordner hinzufügen…", "list-add")) + "".join(
        li("folder", p, trailing=f'<span class="tbtn">{ic("edit-delete",16)}</span>', small=True)
        for p in ["~/Downloads", "~/.cache", "~/Videos/Rohmaterial"])
    tpl = section("Vorlagen") + f'''<div class="pad">{checkbox("Cache-Ordner ausschliessen (mit CACHEDIR.TAG)", True)}
      {checkbox("Papierkorb ausschliessen", True)}
      <div class="row">{checkbox("Dateien ausschliessen, grösser als")} {spin("2",56)} {combo("GB",64)}</div></div>'''
    expert = f'<div class="expert-sec"><div class="lsection">Regeln in restic-Syntax <span class="badge">{ic("configure",16)} Expertenmodus</span></div>' + f'''<div class="pad">
      <div class="code"><div><span class="ln">1</span>$HOME/.var</div><div><span class="ln">2</span>!$HOME/.var/app/org.darktable.Darktable</div><div><span class="ln">3</span>.git</div></div>
      {inline("warn","Zeile 2: Diese Ausnahme wirkt nicht, weil <code>$HOME/.var</code> ganz ausgeschlossen ist. restic kann darunter nichts wieder einschliessen.", closable=False)}
      {checkbox("Gross-/Kleinschreibung ignorieren (--iexclude)", True)}
      {checkbox("Ordner mit Datei «.nobackup» ausschliessen (--exclude-if-present)", True)}
      <div>{btn("Pfad testen…","edit-find")}</div></div></div>'''
    p = page("Ausschluss-Regeln – Persönlicher Ordner", lst + tpl + expert,
             tbtn("Vorschau…", "view-preview") + tbtn("Speichern", "document-save", primary=True), bg="win",
             breadcrumb="Backups › Persönlicher Ordner › Ausschluss-Regeln")
    return win("Kestic", tabbar("backups") + columns(p))

def s_schedule():
    days = "".join(f'<span class="daybtn {"on" if d in ("Mo","Mi","Fr") else ""}">{d}</span>' for d in ["Mo","Di","Mi","Do","Fr","Sa","So"])
    f = form([
        ("Häufigkeit:", combo("An bestimmten Wochentagen", 260)),
        ("Tage:", f'<span class="days">{days}</span>'),
        ("Uhrzeit:", field("20:00", w=80)),
        ("Nächste Termine:", '<span class="muted">Mo 16.11. 20:00 · Mi 18.11. 20:00 · Fr 20.11. 20:00</span>'),
        ("#", "Laufbedingungen"),
        ("Nur sichern, wenn:", checkbox("am Strom", True)), ("", checkbox("mit dem Netzwerk verbunden", True)),
        ("", checkbox("keine getaktete Verbindung (z. B. Handy-Hotspot)")),
        ("", f'<div class="row">{checkbox("in diesen Netzwerken:")} {chip("Heim-WLAN ✕")} {btn("Netzwerk…","list-add")}</div>'),
        ("", '<span class="muted small">Ist das Ziel nicht erreichbar, wartet Kestic und versucht es später erneut.</span>'),
        ("#", "Überwachung"),
        ("Überfällig melden nach:", f'<div class="row">{spin("2")} <span>verpassten Terminen</span></div>'),
    ])
    p = page("Zeitplan – Projekte", f'<div class="formwrap">{f}</div>', tbtn("Speichern", "document-save", primary=True), bg="win",
             breadcrumb="Backups › Projekte › Zeitplan")
    return win("Kestic", tabbar("backups") + columns(p))

def s_retention():
    def kv(lbl, v, unl=False):
        x = f' {checkbox("alle", disabled=True)}' if unl else ""
        return f'<span class="kv"><span class="kv-l">{lbl}</span>{spin(v,64,disabled=True)}{x}</span>'
    f = form([
        ("Vorlage:", combo("Standard", 260)),
        ("", '<span class="muted">7 tägliche, 4 wöchentliche, 12 monatliche Snapshots. Felder sind schreibgeschützt; Ändern eines Werts wechselt zu «Eigene Regeln».</span>'),
        ("#", "Nach Anzahl: «die letzten n … behalten»"),
        ("", f'<div class="grid3">{kv("Snapshots","")}{kv("stündliche","")}{kv("tägliche","7")}{kv("wöchentliche","4")}{kv("monatliche","12")}{kv("jährliche","",True)}</div>'),
        ("#", "Nach Zeitraum: «alle … der letzten Dauer behalten»"),
        ("", f'<div class="grid3"><span class="kv"><span class="kv-l">alle</span>{spin("",56,disabled=True)}{combo("Tage",90)}</span><span class="kv"><span class="kv-l">tägliche</span>{spin("",56,disabled=True)}{combo("Monate",90)}</span></div>'),
        ("", inline("info", "«Die letzten 7 täglichen» = die 7 jüngsten Tage mit Snapshot, auch mit Lücken. «Tägliche der letzten 7 Tage» = nur Snapshots aus den letzten 7 Tagen.", closable=False)),
    ])
    p = page("Aufbewahrung – Persönlicher Ordner", f'<div class="formwrap">{f}</div>',
             tbtn("Vorschau…", "view-preview") + tbtn("Speichern", "document-save", primary=True), bg="win",
             breadcrumb="Backups › Persönlicher Ordner › Aufbewahrung")
    return win("Kestic", tabbar("backups") + columns(p))

def _restore_browse():
    rows = [
        (checkbox("", True), f'{ic("folder",16)} Steuern', "–", "03.11.2026"),
        (checkbox(""), f'{ic("folder",16)} Rechnungen', "–", "28.10.2026"),
        (checkbox("", True), f'{ic("document-save",16)} Lebenslauf.odt', "48 KB", "12.10.2026"),
        (checkbox(""), f'{ic("document-save",16)} Notizen.md', "3 KB", "14.11.2026"),
    ]
    browse = f'''<div class="crumbbar">{ic("go-home",16)} › home › user › Dokumente <span class="spacer"></span>{field(ph="Filtern", w=180)}</div>
      {table(["", "Name", "Grösse", "Geändert"], rows)}
      <div class="selbar">3 Elemente ausgewählt (2 hier, 1 in «Bilder») · <a>Auswahl anzeigen</a> · <a>Auswahl aufheben</a></div>'''
    return page(None, browse, tbtn("Auswahl wiederherstellen…", "document-revert", primary=True), bg="view", breadcrumb="Wiederherstellen › Persönlicher Ordner › 15.11.2026 12:00")

def s_restore(overwrite=False):
    p = _restore_browse()
    if not overwrite:
        dlg_body = f'''<div class="dlgcol"><div class="h h4">Wohin</div>
          {radio("In neuen Ordner", True)}<div class="indent"><div class="row">{field("~/Wiederhergestellt/Persönlicher Ordner 2026-11-15", w=380)}{btn("Wählen…","folder-open")}</div></div>
          {radio("An ursprünglichen Ort (/home/user/…)")}</div>'''
        d = dialog("3 Elemente wiederherstellen", dlg_body, btn("Abbrechen", "dialog-cancel") + default_btn("Jetzt wiederherstellen", "document-revert", "primary"), w=560)
    else:
        dlg_body = f'''<div class="dlgcol"><div class="h h4">Wohin</div>
          {radio("In neuen Ordner")}
          {radio("An ursprünglichen Ort (/home/user/…)", True)}
          <div class="indent"><div class="row"><span>Vorhandene Dateien:</span> {combo("Immer überschreiben",200)}</div>
          <div class="preview">{ic("view-preview",16)} <b>Vorschau</b> (läuft automatisch)<br>117 Dateien neu · <b>3 Dateien werden überschrieben</b> · 0 unverändert · <a>Liste anzeigen</a></div>
          {inline("warn","Überschriebene Dateien lassen sich nicht wiederherstellen. Nicht rückgängig zu machen.", closable=False)}</div></div>'''
        d = dialog("3 Elemente wiederherstellen", dlg_body, default_btn("Abbrechen", "dialog-cancel") + btn("Überschreiben und wiederherstellen", "edit-delete", "danger"), w=560)
    return win("Kestic", tabbar("restore") + columns(p) + d)

def s_restore_select():
    lst = (section("Aus welchem Backup?")
           + li(B_HOME[0], B_HOME[1], "46 Snapshots", selected=True, arrow=True)
           + li(B_FOTO[0], B_FOTO[1], "12 Snapshots · Ziel «USB-Disk» nicht eingesteckt", arrow=True)
           + li(B_PROJ[0], B_PROJ[1], "30 Snapshots", arrow=True)
           + section("Weitere Snapshots")
           + li("state-information", "Nicht zugeordnet", "8 Snapshots ohne Kestic-Backup", arrow=True, small=True)
           + li("emblem-important", "Verwaist", "2 Backups ohne Beschreibung", arrow=True, small=True))
    snaps = (f'<div class="pad">{field(ph="In Snapshots dieses Backups suchen", w=420)}</div>'
             + section("Heute") + li(None, "12:00", "312 MB neu", arrow=True, small=True)
             + section("Gestern") + li(None, "12:00", "45 MB neu · 2 Dateien nicht lesbar", arrow=True, small=True)
             + section("Diese Woche") + li(None, "Fr 13.11. 12:00", "18 MB neu", arrow=True, small=True)
             + li(None, "Do 12.11. 12:00", "2 MB neu", arrow=True, small=True))
    d = page("Persönlicher Ordner", snaps, tbtn("Vergleichen…", "vcs-diff"), bg="view")
    return win("Kestic", tabbar("restore") + columns(page(None, lst, width=380, bg="view", head=False), d))

def s_compare():
    rows = [
        (f'{ic("list-add",16)} neu', "Dokumente/Notizen.md", "3 KB"),
        (f'{ic("document-edit",16)} geändert', "Dokumente/Steuern/2026.ods", "212 KB"),
        (f'{ic("document-edit",16)} geändert', "Projekte-Liste.txt", "1 KB"),
        (f'{ic("edit-delete",16)} entfernt', "Downloads/alt.iso", "1,2 GB"),
    ]
    body = f'''<div class="filters">{chip("Neu (1)", True)} {chip("Geändert (2)", True)} {chip("Entfernt (1)", True)}<span class="spacer"></span><span class="muted">Gestern 12:00 → Heute 12:00</span></div>
      {table(["Änderung", "Pfad", "Grösse"], rows)}'''
    p = page("Unterschiede – Persönlicher Ordner", body, "", bg="view", breadcrumb="Wiederherstellen › Persönlicher Ordner › Vergleichen")
    return win("Kestic", tabbar("restore") + columns(p))

def s_dest():
    lst = (inviewheader("3 Ziele", btn("Ziel hinzufügen…", "list-add"))
           + li("network-server", "NAS", status("network-connect", "Erreichbar · 1 Backup", "ok"), selected=True)
           + li("drive-removable-media", "USB-Disk", status("drive-removable-media", "Laufwerk nicht eingesteckt · 1 Backup"))
           + li("folder-cloud", "S3", status("state-error", "Gespeichertes Passwort passt nicht · kein Kit", "err")))
    body = f'''<div class="detail-status">{status("network-connect","Erreichbar · REST-Server · rest:https://nas.example.org:8000/","ok")}<div class="muted">zuletzt geprüft vor 5 Minuten</div></div>
      {section("Backups auf diesem Ziel")}{li("user-home","Persönlicher Ordner",status("state-ok","Gesichert","ok"),arrow=True,small=True)}
      {section("Wartung")}{li(None,"Aufräumen","sonntags 12:00 · zuletzt heute 12:30",arrow=True,small=True)}{li(None,"Prüfen","am ersten Sonntag im Monat 12:00 · zuletzt 01.11.2026",arrow=True,small=True)}
      <div class="pad">{btn("Wartung jetzt ausführen…","view-refresh")}</div>
      {section("Sicherheit")}{li("security-high","Wiederherstellungs-Kit",status("security-high","erzeugt am 01.11.2026","ok"),arrow=True,small=True)}
      {section("Weitere")}{li(None,"Backups von diesem Ziel übernehmen","",arrow=True,small=True)}'''
    d = page("NAS", body, tbtn("Verbindung prüfen", "view-refresh") + tbtn("", "overflow-menu"), bg="win")
    return win("Kestic", tabbar("dest") + columns(page(None, lst, width=360, bg="view", head=False), d))

def s_history():
    lst = f'<div class="filters">{combo("Alle Backups",150)} {chip("Probleme")} {chip("Wartung")}</div>' + section("Heute") + \
        li(None, f'{ic("state-ok",16)} 12:30 Wartung NAS', "18 Minuten", small=True) + \
        li(None, f'{ic("state-ok",16)} 12:00 Persönlicher Ordner', "3 Minuten", small=True) + section("Gestern") + \
        li(None, f'{ic("state-warning",16)} 12:00 Persönlicher Ordner', "2 Dateien nicht lesbar", selected=True, small=True) + \
        section("Freitag, 13.11.") + li(None, f'{ic("state-error",16)} 20:05 Projekte', "Passwort abgelehnt", small=True)
    body = f'''<div class="detail-status">{status("state-warning","Gesichert, 2 Dateien nicht lesbar · 4 Minuten 12 Sekunden","warn")}
      <div class="muted">Persönlicher Ordner · 45 MB hinzugefügt</div></div>
      <div class="qtabs"><span class="on">Ergebnis</span><span>Log</span><span>Befehle</span></div>
      <div class="pad"><div class="muted">1 248 neu · 312 geändert · 9 830 unverändert</div></div>
      {section("Nicht lesbare Dateien")}
      {li(None,"~/.local/share/app/lock",trailing=btn("Zu Ausschluss-Regeln hinzufügen…"),small=True)}
      {li(None,"~/VM/disk.qcow2",trailing=btn("Zu Ausschluss-Regeln hinzufügen…"),small=True)}'''
    d = page("Lauf vom 14.11.2026, 12:00", body, tbtn("", "overflow-menu"), bg="win")
    return win("Kestic", tabbar("history") + columns(page(None, lst, width=360, bg="view", head=False), d))

def wizard(title, step, total, label, pct, content, next_label="Weiter", next_icon="go-next", next_disabled=False, back=True):
    nb = (f'<span class="btn disabled">{ic(next_icon,16)}<span>{next_label}</span></span>' if next_disabled
          else default_btn(next_label, next_icon, "primary"))
    body = f'''<div class="wiz"><div class="wiz-top"><span class="muted">Schritt {step} von {total}</span>{progress(pct)}</div>
      {heading(label,1)}{content}</div>
      <div class="wiz-foot">{btn("Abbrechen")}<span class="spacer"></span>{btn("Zurück","go-previous") if back else ""}{nb}</div>'''
    return win("Kestic", columns(page(title, body, "", bg="win"), burger=False))

def s_wiz1():
    c = f'''<div class="boxlist">{li("user-home","Persönlicher Ordner","/home/user · empfohlen", selected=True)}</div>
      <div>{btn("Anderen Ordner wählen…","folder-open")}</div>
      <div class="muted small">Ein ganzes Laufwerk (z. B. unter /media) wird nur gesichert, wenn es eingesteckt ist.</div>'''
    return wizard("Erstes Backup einrichten", "1", "5", "Was soll gesichert werden?", 10, c, back=False)

def s_wiz2():
    ready = (li("drive-removable-media", "Externes Laufwerk / USB", "Ein Laufwerk ist eingesteckt: «USB-Disk»", selected=True)
             + li("folder", "Ordner / anderer Datenträger", "Lokaler Ordner, z. B. eine zweite Festplatte")
             + li("network-server", "NAS / Server per SSH", "SFTP")
             + li("network-server", "REST-Server", "z. B. auf dem NAS")
             + li("folder-cloud", "Cloud-Speicher", "S3-kompatibel, Backblaze B2, Azure, Google Cloud, Swift"))
    need = li("folder-cloud", "Cloud-Dienste über rclone", "Google Drive, OneDrive, Dropbox … · braucht das Programm «rclone»",
              trailing=btn("In Discover öffnen", "folder-download"))
    c = f'<div class="boxlist">{section("Bereit")}{ready}{section("Braucht Installation")}{need}</div>'
    return wizard("Erstes Backup einrichten", "2", "5", "Wohin soll gesichert werden?", 30, c)

def s_wiz3():
    f = form([
        ("Adresse:", field("https://nas.example.org:8000/", w=340)), ("Benutzername:", field("user", w=340)),
        ("Server-Passwort:", field(pw=True, w=340)),
        ("", f'<div class="row">{btn("Verbindung prüfen","view-refresh")} {status("network-connect","Erreichbar · kein bestehendes Backup gefunden","ok")}</div>'),
        ("", '<span class="muted small">Kestic legt das Ziel erst beim Abschluss an. Abbrechen hinterlässt nichts auf dem Server.</span>'),
    ])
    return wizard("Erstes Backup einrichten", "3", "5", "Wo liegt der REST-Server?", 50, f'<div class="formwrap">{f}</div>')

def s_wiz4a():
    f = form([
        ("Backup-Passwort:", field(pw=True, w=320)), ("Wiederholen:", field(pw=True, w=320)),
        ("", btn("Sicheres Passwort erzeugen", "dialog-password")),
        ("", status("security-high", "Stark", "ok")),
    ])
    c = f'''<div class="muted">Mit diesem Passwort wird das Backup verschlüsselt. Kestic merkt es sich im Passwortspeicher (KWallet).</div>
      <div class="formwrap">{f}</div>'''
    return wizard("Erstes Backup einrichten", "4", "5", "Passwort für das Backup", 65, c)

def kit_content(target="NAS", addr="rest:https://nas.example.org:8000/", access="Benutzername und Passwort des REST-Servers (nicht im Kit)"):
    kitbox = f'''<div class="kit"><div class="h h3">{ic("security-high",22)} Wiederherstellungs-Kit für «{target}»</div>
      <div class="form"><div class="flabel">Adresse:</div><div class="fwidget mono">{addr}</div>
      <div class="flabel">Kit-Passphrase:</div><div class="fwidget mono kitpw">vortex-amber-ledger-quiet-orbit-71-pine-saffron</div>
      <div class="flabel">Zugang zum Server:</div><div class="fwidget">{access}</div></div></div>'''
    return f'''<div class="muted">Ohne dieses Gerät oder bei vergessenem Passwort kommen Sie nur mit dem Kit an die Daten. Kestic speichert die Kit-Passphrase nicht.</div>
      {kitbox}<div class="row">{btn("Drucken…","document-print")}{btn("Als Datei speichern…","document-export")}</div>
      {inline("warn","Das Kit enthält das Passwort im Klartext. Ausdrucken oder auf einem getrennten, sicheren Medium aufbewahren – nicht im gesicherten Ordner.", closable=False)}
      {checkbox("Wiederherstellungs-Kit sicher aufbewahrt", False)}'''

def s_wiz4b():
    return wizard("Erstes Backup einrichten", "4", "5", "Wiederherstellungs-Kit sichern", 80, kit_content(), next_disabled=True)

def s_wiz5():
    rows = [("Quelle", "Persönlicher Ordner (~)"), ("Ziel", "NAS (REST-Server)"),
            ("Zeitplan", "täglich 12:00 · nur am Strom und mit Netzwerk"),
            ("Ausschlüsse", "Cache-Ordner, Papierkorb, Download-Ordner, Cache-markierte Ordner"),
            ("Aufbewahrung", "7 tägliche, 4 wöchentliche, 12 monatliche Snapshots")]
    lst = "".join(li(None, a, b, trailing=btn("Ändern…"), small=True) for a, b in rows)
    c = f'''<div class="boxlist">{lst}</div>
      {inline("info","Geschätzte Grösse: 41 GB. Das erste Backup dauert am längsten (etwa 2 Stunden); der Rechner kann weiter genutzt oder ausgeschaltet werden – Kestic setzt beim nächsten Mal fort.", closable=False)}'''
    body = f'''<div class="wiz"><div class="wiz-top"><span class="muted">Schritt 5 von 5</span>{progress(100)}</div>
      {heading("Zusammenfassung",1)}{c}</div>
      <div class="wiz-foot">{btn("Abbrechen")}<span class="spacer"></span>{btn("Zurück","go-previous")}{btn("Speichern, später sichern")}{default_btn("Jetzt sichern","media-playback-start","primary")}</div>'''
    return win("Kestic", columns(page("Erstes Backup einrichten", body, "", bg="win"), burger=False))

def s_welcome():
    appicon = '<div class="appicon">' + ic("backup", 64, sizes=("22",)) + '</div><div class="muted small">(Platzhalter App-Icon)</div>'
    body = f'''<div class="phm">{appicon}<div class="h h1">Willkommen bei Kestic</div>
      <div class="phm-expl">Kestic sichert Dateien regelmässig und verschlüsselt auf ein Laufwerk, einen Server oder in die Cloud.</div>
      {default_btn("Erstes Backup einrichten","list-add","primary")}<a class="lnk">Vorhandenes Backup verbinden…</a></div>'''
    return win("Kestic", columns(page(None, body, bg="win", head=False), burger=False), w=900, h=560)

def s_tray():
    menu = f'''<div class="menu">
      <div class="mi dis">{ic("state-error",16)} 1 Backup fehlgeschlagen</div>
      <div class="mi dis">{ic("state-sync",16)} Persönlicher Ordner wird gesichert · 42 %</div>
      <div class="mi dis">{ic("state-pause",16)} Pausiert seit 13:00 (bis 15:00)</div>
      <div class="mi dis">&nbsp;&nbsp;&nbsp;&nbsp; Nächstes Backup: morgen 12:00</div><div class="msep"></div>
      <div class="mi">{ic("media-playback-start",16)} Jetzt sichern <span class="sub">▸</span></div>
      <div class="mi">{ic("media-playback-start",16)} Zeitpläne fortsetzen</div>
      <div class="mi">{ic("process-stop",16)} Laufendes Backup abbrechen</div><div class="msep"></div>
      <div class="mi dis">{ic("security-high",16)} Wiederherstellungs-Kit erzeugen… (zuerst Passwort aktualisieren)</div>
      <div class="mi dis">{ic("update-none",16)} restic aktualisieren…</div><div class="msep"></div>
      <div class="mi">{ic("backup",16)} Kestic öffnen</div>
      <div class="mi">{ic("document-revert",16)} Dateien wiederherstellen…</div><div class="msep"></div>
      <div class="mi">{ic("window-close",16)} Kestic beenden…</div></div>'''
    sub = f'''<div class="menu sub2"><div class="mi">Alle Backups</div><div class="msep"></div>
      <div class="mi dis">{ic("state-sync",16)} Persönlicher Ordner (läuft)</div>
      <div class="mi dis">Fotos (Quelle nicht verfügbar)</div>
      <div class="mi">Projekte</div></div>'''
    job = f'''<div class="notif"><div class="n-head">{ic("backup",16)} <span>Kestic</span><span class="spacer"></span><span class="muted small">jetzt</span></div>
      <div class="n-title">Persönlicher Ordner wird gesichert</div>{progress(42)}
      <div class="muted small">42 % · 1,2 GB von 3 GB · 7 MB/s · noch etwa 4 Minuten</div>
      <div class="n-actions">{btn("Abbrechen","process-stop")}</div></div>'''
    err = f'''<div class="notif"><div class="n-head">{ic("backup",16)} <span>Kestic</span><span class="spacer"></span><span class="muted small">Fr 20:05</span></div>
      <div class="n-title">{ic("state-error",16)} Projekte: Backup fehlgeschlagen</div>
      <div>Der S3-Speicher hat das gespeicherte Passwort abgelehnt.</div>
      <div class="n-actions">{btn("Details")}{btn("Gespeichertes Passwort aktualisieren…")}</div></div>'''
    panel = f'''<div class="panel"><span class="spacer"></span><span class="trayicons">{ic("network-wireless",22)}{ic("battery-full",22)}
      <span class="trayhl">{ic("state-error",22)}</span></span><span class="clock">14:32<br><span class="small">15.11.2026</span></span></div>'''
    note = '<div class="desk-note">Tray-Symbol = Platzhalter mit Overlay «Fehler» (schlechtester Zustand, §10.1); SNI-Status NeedsAttention</div>'
    body = f'<div class="desk"><div class="notifs">{err}{job}</div>{menu}{sub}{panel}{note}</div>'
    return f'<div class="desk-frame">{body}</div>'

def s_blocked():
    sb = stateline("err", "<b>Der Passwortspeicher (KWallet) ist gesperrt.</b> Kestic kann nicht auf die Ziele zugreifen.", btn("Entsperren…", "dialog-password"))
    stale = (inline("info", "Letzter bekannter Stand (vor 3 Minuten). Aktuelle Snapshots erscheinen nach dem Entsperren.", closable=False)
             + '<div class="stale">' + section("Snapshots") + li(None, "Heute 12:00", "312 MB neu", arrow=True, small=True)
             + li(None, "Gestern 12:00", "45 MB neu · 2 Dateien nicht lesbar", arrow=True, small=True) + '</div>')
    d = page("Persönlicher Ordner", stale,
             tbtn("Jetzt sichern", "media-playback-start", disabled=True), bg="win")
    l = page(None, backup_list("home", " (Stand: vor 3 Minuten)"), width=360, bg="view", head=False)
    return win("Kestic", sb + tabbar("backups") + columns(l, d))

def s_maintenance():
    f = form([
        ("#", "Aufräumen <span class='muted small'>(entfernt nicht mehr benötigte Daten)</span>"),
        ("Häufigkeit:", f'<div class="row">{combo("Wöchentlich",160)} <span>am</span> {combo("Sonntag",130)} <span>um</span> {field("12:00",w=70)}</div>'),
        ("#", "Prüfen <span class='muted small'>(kontrolliert die Integrität)</span>"),
        ("Häufigkeit:", f'<div class="row">{combo("Monatlich",160)} <span>am</span> {combo("ersten Sonntag",150)} <span>um</span> {field("12:00",w=70)}</div>'),
        ("", inline("info", "Wartung läuft nur am Strom, ohne getaktete Verbindung und wenn kein Backup auf diesem Ziel läuft.", closable=False)),
    ])
    p = page("Wartung – NAS", f'<div class="formwrap">{f}</div>', tbtn("Speichern", "document-save", primary=True), bg="win",
             breadcrumb="Ziele › NAS › Wartung")
    body = ('<div class="dlgcol"><div>Beim Aufräumen werden Snapshots, die keine Aufbewahrungsregel mehr behält, <b>endgültig</b> entfernt.</div>'
            + table(["Backup", "Wird entfernt"], [("Persönlicher Ordner", "3 Snapshots")])
            + '<div class="muted small">Gilt ab dem nächsten Aufräumen (Sonntag 12:00). Nicht rückgängig zu machen.</div></div>')
    d = dialog("Aufräum-Zeitplan aktivieren?", body, default_btn("Abbrechen", "dialog-cancel") + btn("Aktivieren", "edit-delete", "danger"), w=500)
    return win("Kestic", tabbar("dest") + columns(p) + d)

def s_kit():
    c = kit_content("S3", "s3:https://s3.example.com/kestic-backup", "Access-Key und Secret-Key des S3-Speichers (nicht im Kit)")
    return wizard("Wiederherstellungs-Kit erzeugen", "2", "3", "Kit sicher aufbewahren", 66, c, next_label="Kit aktivieren", next_icon="security-high", next_disabled=True)

def s_adopt():
    body = f'''<div class="pad"><div>Auf diesem Ziel liegen Backups, die auf diesem Rechner nicht eingerichtet sind.</div></div>
      <div class="adopt">{checkbox("", True)}<div>{ic("folder",22)} <b>Dokumente</b><div class="muted">Quelle /home/owner/Dokumente → wird zu /home/user/Dokumente · letzter Snapshot vor 2 Tagen von Rechner «notebook-alt»</div>
      {inline("warn","Jünger als 7 Tage und von einem anderen Rechner: Übernehmen heisst, dass der andere Rechner dieses Backup nicht mehr ausführen soll.", closable=False)}</div></div>
      <div class="adopt">{checkbox("")}<div>{ic("folder",22)} <b>Musik</b><div class="muted">Quelle /home/owner/Musik → wird zu /home/user/Musik · letzter Snapshot vor 40 Tagen von «notebook-alt»</div></div></div>
      {section("Verwaist")}<div class="pad"><div class="muted">2 Backups ohne Beschreibung – unter «Wiederherstellen › Verwaist» durchsuchbar.</div></div>'''
    p = page("Backups von NAS übernehmen", body, tbtn("Ausgewählte übernehmen", "list-add", primary=True), bg="win",
             breadcrumb="Ziele › NAS › Backups übernehmen")
    return win("Kestic", tabbar("dest") + columns(p))

def s_dialogs_a():
    pw = dlg_box("Gespeichertes Passwort aktualisieren",
        f'<div class="dlgcol"><div>Das im Passwortspeicher hinterlegte Passwort für «S3» passt nicht (mehr). Das Passwort des Ziels selbst ändert sich dadurch nicht.</div>'
        f'<div class="row"><span>Passwort:</span>{field(pw=True,w=300)}</div></div>',
        btn("Abbrechen","dialog-cancel") + default_btn("Prüfen und speichern","dialog-ok","primary"), w=500)
    lock = dlg_box("Sperre aufheben?",
        '<div>Ein unterbrochener Lauf (z. B. Absturz oder Ausschalten) hat das Ziel «NAS» gesperrt (von diesem Rechner, der Lauf ist beendet). Solange die Sperre besteht, sind keine Backups möglich.</div>',
        default_btn("Abbrechen","dialog-cancel") + btn("Sperre aufheben","object-locked"), w=500)
    host = dlg_box("Diesem Server vertrauen?",
        f'<div class="dlgcol"><div class="form"><div class="flabel">Server:</div><div class="fwidget">backup.example.org (Port 22)</div>'
        f'<div class="flabel">Schlüsseltyp:</div><div class="fwidget">ED25519</div><div class="flabel">Fingerprint:</div>'
        f'<div class="fwidget mono">SHA256:Qm4f…J8sY2k</div></div>'
        f'<div class="muted small">Vergleichen Sie den Fingerprint mit dem des Servers. Danach wird er in ~/.ssh/known_hosts gespeichert.</div></div>',
        default_btn("Abbrechen","dialog-cancel") + btn("Server vertrauen","security-high"), w=500)
    exp = dlg_box("Konfiguration als Script exportieren",
        f'<div class="dlgcol"><div class="h h4">Backups</div>{checkbox("Persönlicher Ordner",True)}{checkbox("Fotos",True)}{checkbox("Projekte",True)}'
        f'{inline("info","Das Script enthält keine Passwörter; es fragt sie beim Ausführen ab. Es sichert und räumt gemäss Aufbewahrung auf – ohne Kestic.",closable=False)}</div>',
        btn("Abbrechen","dialog-cancel") + default_btn("Speichern unter…","document-export","primary"), w=500)
    return f'<div class="sheetwrap">{sheet(pw, lock, host, exp)}</div>'

def s_dialogs_b():
    force = dlg_box("Trotzdem jetzt sichern?",
        '<div>Bedingung nicht erfüllt: <b>am Strom</b> (Akku 34 %). Das Backup kann den Akku stärker belasten.</div>',
        default_btn("Abbrechen","dialog-cancel") + btn("Trotzdem sichern","media-playback-start"), w=500)
    rmb = dlg_box("«Fotos» aus Kestic entfernen?",
        '<div>Die Konfiguration wird entfernt, die 12 Snapshots bleiben auf «USB-Disk». Das Backup lässt sich über «Backups von diesem Ziel übernehmen» zurückholen.</div>',
        default_btn("Abbrechen","dialog-cancel") + btn("Aus Kestic entfernen","edit-delete","danger"), w=500)
    rm = dlg_box("Ziel «Alte-Disk» entfernen?",
        '<div>Das gespeicherte Passwort wird aus dem Passwortspeicher gelöscht. Die Daten bleiben auf dem Ziel und sind danach nur mit Passwort oder Wiederherstellungs-Kit lesbar.</div>',
        default_btn("Abbrechen","dialog-cancel") + btn("Ziel entfernen","edit-delete","danger"), w=500)
    exc = dlg_box("Zu Ausschluss-Regeln hinzufügen?",
        f'<div class="dlgcol"><div>Neue Regel für «Persönlicher Ordner»:</div><div class="code">/home/user/VM/disk.qcow2</div>'
        f'<div class="muted small">Datei → Regel in restic-Syntax (im einfachen Modus als «weitere Regel» sichtbar). Wird ab dem nächsten Backup nicht mehr gesichert; bestehende Snapshots bleiben unverändert.</div></div>',
        btn("Abbrechen","dialog-cancel") + default_btn("Hinzufügen und speichern","list-add","primary"), w=500)
    exp = dlg_box("Expertenmodus einschalten?",
        '<div>Zeigt zusätzlich Ausschluss-Regeln in restic-Syntax und Expertenoptionen. Der Modus lässt sich in den Einstellungen jederzeit wieder ausschalten.</div>',
        btn("Abbrechen","dialog-cancel") + default_btn("Einschalten","configure","primary"), w=500)
    return f'<div class="sheetwrap">{sheet(force, rmb, rm, exc, exp)}</div>'

def s_settings():
    f = form([
        ("#", "Darstellung"),
        ("", switch("Expertenmodus", False)),
        ("", '<span class="muted small">Zeigt Ausschluss-Regeln in restic-Syntax und Expertenoptionen. «Befehl anzeigen» gibt es immer.</span>'),
        ("#", "Hintergrunddienst"),
        ("Status:", status("media-playback-start", "läuft · startet mit der Sitzung", "ok")),
        ("", '<span class="muted small">Fenster schliessen beendet Kestic nicht; Backups laufen weiter. Beenden über das Kestic-Symbol im Systemabschnitt.</span>'),
        ("", btn("Autostart in den Systemeinstellungen…", "configure")),
        ("#", "Zeitpläne"),
        ("Überfällig melden nach:", f'<div class="row">{spin("2")} <span>verpassten Terminen (Standard für neue Backups)</span></div>'),
        ("#", "Benachrichtigungen"),
        ("Benachrichtigen bei:", checkbox("Fehlern", True)), ("", checkbox("Warnungen", True)), ("", checkbox("überfälligen Backups", True)),
        ("", checkbox("Problemen bei der Wartung", True)), ("", checkbox("neuen restic-Versionen", True)),
        ("", btn("Systemeinstellungen für Benachrichtigungen…", "configure")),
        ("#", "restic"),
        ("restic-Quelle:", f'<div class="row"><span>Automatisch (System-restic 0.18.1)</span>{btn("Ändern…","configure")}</div>'),
        ("", '<span class="muted small">Kestic braucht mindestens restic 0.18.0 (Fehlermeldungen als JSON, eindeutige Exit-Codes).</span>'),
        ("", checkbox("Wöchentlich nach neuen Versionen suchen", True)),
    ])
    body = f'<div class="formwrap settings">{f}</div>'
    return win("Kestic einrichten", columns(page(None, body, bg="win", head=False), burger=False), w=820, h=860)

def s_newbackup():
    lst = (li("network-server", "NAS", "REST-Server · 1 Backup · erreichbar", selected=True)
           + li("drive-removable-media", "USB-Disk", "Laufwerk · 1 Backup · nicht eingesteckt")
           + li("folder-cloud", "S3", "S3-kompatibel · 1 Backup · gespeichertes Passwort passt nicht")
           + li("list-add", "Neues Ziel hinzufügen…", "Laufwerk, Server oder Cloud", small=True))
    c = f'''<div class="boxlist">{lst}</div>
      {inline("warn","«~/Dokumente» liegt in «~», das schon auf «NAS» gesichert wird. Die Dateien würden doppelt verarbeitet (kein doppelter Speicherplatz).", closable=False)}'''
    return wizard("Neues Backup", "2", "3", "Wohin soll «Dokumente» gesichert werden?", 66, c)

def s_narrow():
    p = page(None, backup_list(), "", bg="view")
    return win("Kestic", columns(p) + tabbar("backups", bottom=True), w=468, h=720)

def s_reassign():
    body = (section("Aktuelles Ziel")
            + li("network-server", "NAS", "REST-Server · 46 Snapshots", trailing=ic("state-ok",16), small=True)
            + section("Anderes Ziel wählen")
            + li("drive-removable-media", "USB-Disk", "Laufwerk · nicht eingesteckt", selected=True, small=True)
            + li("folder-cloud", "S3", "S3-kompatibel · gespeichertes Passwort passt nicht", small=True)
            + f'<div class="pad"><div>{btn("Neues Ziel hinzufügen…","list-add")}</div><div class="muted small">Ohne Vorauswahl; hier hat der Benutzer «USB-Disk» gewählt.</div></div>')
    p = page("Ziel – Persönlicher Ordner", body, tbtn("Ziel wechseln…", "go-next", primary=True), bg="win",
             breadcrumb="Backups › Persönlicher Ordner › Ziel")
    dlg = ('<div>Die Snapshot-Reihe beginnt auf «USB-Disk» neu; das nächste Backup sichert alles noch einmal. '
           'Die 46 Snapshots bleiben auf «NAS» und lassen sich dort weiter wiederherstellen.</div>')
    d = dialog("«Persönlicher Ordner» auf «USB-Disk» sichern?", dlg, default_btn("Abbrechen", "dialog-cancel") + btn("Ziel wechseln", "go-next"), w=520)
    return win("Kestic", tabbar("backups") + columns(p) + d)

def s_edit_dest():
    f = form([
        ("Name:", field("NAS", w=340)),
        ("Adresse:", field("https://nas.example.org:8443/", w=340)),
        ("Benutzername:", field("user", w=340)),
        ("Server-Passwort:", field(pw=True, w=340)),
        ("", f'<div class="row">{btn("Verbindung prüfen","view-refresh")} <span class="muted">Adresse geändert – bitte prüfen</span></div>'),
        ("", inline("info", "Speichern ist erst nach erfolgreicher Prüfung möglich. Kestic prüft dabei, ob unter der neuen Adresse dasselbe oder ein umgezogenes Backup-Ziel liegt.", closable=False)),
    ])
    p = page("Verbindung bearbeiten – NAS", f'<div class="formwrap">{f}</div>', tbtn("Speichern", "document-save", disabled=True), bg="win",
             breadcrumb="Ziele › NAS › Verbindung bearbeiten")
    return win("Kestic", tabbar("dest") + columns(p))

def s_restore_states():
    run = inline("info", f'<b>Wird wiederhergestellt · 64 %</b> · 77 von 120 Dateien · noch etwa 1 Minute{progress(64)}', btn("Abbrechen","process-stop"), closable=False)
    ok = inline("pos", "<b>120 Dateien wiederhergestellt</b> nach ~/Wiederhergestellt/Persönlicher Ordner 2026-11-15.", btn("Ordner öffnen","folder-open"))
    err = inline("err", "<b>Wiederherstellen fehlgeschlagen:</b> Auf dem Datenträger ist nicht genug Platz (restic meldet «no space left on device»). 31 von 120 Dateien wurden wiederhergestellt.",
                 btn("Anderen Ordner wählen…","folder-open") + btn("Details"))
    body = f'<div class="statesheet"><div class="h h4">Läuft</div>{run}<div class="h h4">Fertig</div>{ok}<div class="h h4">Fehler</div>{err}</div>'
    return f'<div class="sheetwrap t-sheet">{body}</div>'

def s_restore_search():
    res = (section("3 Treffer für «steuer» in 46 Snapshots")
           + li("folder", "Dokumente/Steuern", "in 46 Snapshots · zuletzt geändert 03.11.2026", trailing=btn("Anzeigen","go-next"), small=True)
           + li("document-save", "Dokumente/Steuern/2026.ods", "in 12 Snapshots · zuletzt geändert 14.11.2026", trailing=btn("Anzeigen","go-next"), small=True)
           + li("document-save", "Dokumente/Steuern/2025.ods", "in 46 Snapshots · seit 20.04.2026 unverändert", trailing=btn("Anzeigen","go-next"), small=True))
    body = f'<div class="pad">{field("steuer", w=420)}</div>' + res
    p = page(None, body, "", bg="view", breadcrumb="Wiederherstellen › Persönlicher Ordner › Suche")
    return win("Kestic", tabbar("restore") + columns(p))

def s_unassigned():
    body = (section("Nicht zugeordnet auf NAS · 8 Snapshots")
            + li("state-information", "08.11.2026 21:14", "Host notebook · /home/user · ohne Kestic-Tag", trailing=btn("Durchsuchen","go-next"), small=True)
            + li("state-information", "01.11.2026 20:55", "Host notebook · /home/user · ohne Kestic-Tag", trailing=btn("Durchsuchen","go-next"), small=True)
            + li(None, "6 weitere anzeigen", "", arrow=True, small=True)
            + section("Verwaist auf NAS · 2 Backups ohne Beschreibung")
            + li("emblem-important", "Backup ohne Beschreibung", "14 Snapshots · zuletzt 02.10.2026 von notebook-alt", trailing=btn("Durchsuchen","go-next"), small=True)
            + li("emblem-important", "Backup ohne Beschreibung", "3 Snapshots · zuletzt 12.06.2026 von notebook-alt", trailing=btn("Durchsuchen","go-next"), small=True))
    p = page(None, body, "", bg="view", breadcrumb="Wiederherstellen › Nicht zugeordnet und verwaist")
    return win("Kestic", tabbar("restore") + columns(p))

SIZES = {"welcome": (900, 560), "backups": (1180, 830), "tray": (1180, 700), "settings": (820, 860),
         "dialogs-a": (1100, 560), "dialogs-b": (1100, 700), "narrow": (468, 720), "restore-states": (1100, 360), "overview": (1180, 760),
         "overview-running": (1180, 760)}

SCREENS = [
    ("welcome", "Willkommen (Startansicht ohne Konfiguration, §9.1)", s_welcome, ("light",)),
    ("wizard-1", "Einrichtungsassistent, Schritt 1: Quelle (§9.1)", s_wiz1, ("light",)),
    ("wizard-2", "Einrichtungsassistent, Schritt 2: Ziel-Typ (§9.1)", s_wiz2, ("light",)),
    ("wizard", "Einrichtungsassistent, Schritt 3: Ziel-Details (§9.1)", s_wiz3, ("light",)),
    ("wizard-4a", "Einrichtungsassistent, Schritt 4a: Passwort (§9.1)", s_wiz4a, ("light",)),
    ("wizard-4b", "Einrichtungsassistent, Schritt 4b: Wiederherstellungs-Kit (§9.1)", s_wiz4b, ("light",)),
    ("wizard-5", "Einrichtungsassistent, Schritt 5: Zusammenfassung (§9.1)", s_wiz5, ("light",)),
    ("overview", "Übersicht (§7.1)", lambda: s_overview(), ("light", "dark")),
    ("overview-running", "Übersicht während eines Laufs, globale Pause in der Zustandsleiste (§6.1, §7.1)", lambda: s_overview(True, True), ("dark",)),
    ("backups", "Backups: Liste und Detail (§7.2)", s_backups, ("light", "dark")),
    ("excludes", "Ausschluss-Regeln mit Expertenmodus (§7.3)", s_excludes, ("light",)),
    ("schedule", "Zeitplan und Laufbedingungen (§7.4)", s_schedule, ("light",)),
    ("retention", "Aufbewahrung (§7.5)", s_retention, ("light",)),
    ("restore-select", "Wiederherstellen: Backup und Snapshot wählen (§7.6)", s_restore_select, ("light",)),
    ("restore", "Wiederherstellen: Durchsuchen + Dialog, neuer Ordner (§7.6)", lambda: s_restore(False), ("light",)),
    ("restore-overwrite", "Wiederherstellen: Originalort mit Überschreiben (§7.6, §12.3)", lambda: s_restore(True), ("dark",)),
    ("compare", "Wiederherstellen: Snapshots vergleichen (§7.6)", s_compare, ("light",)),
    ("restore-search", "Wiederherstellen: Suchergebnisse (§7.6, P18)", s_restore_search, ("light",)),
    ("restore-states", "Wiederherstellen: läuft, fertig, Fehler (§7.6)", s_restore_states, ("light",)),
    ("unassigned", "Nicht zugeordnete und verwaiste Snapshots (§8.14)", s_unassigned, ("light",)),
    ("destinations", "Ziele (§7.7)", s_dest, ("light",)),
    ("history", "Verlauf mit Befehlen (§7.8)", s_history, ("light",)),
    ("blocked", "Blockierender Zustand: Passwortspeicher gesperrt (§6.1, §6.5)", s_blocked, ("light",)),
    ("tray", "Tray-Menü, Fortschrittsanzeige und Fehler-Benachrichtigung (§10)", s_tray, ("dark",)),
    ("maintenance", "Wartung mit Bestätigung des Aufräum-Zeitplans (§8.1, P10)", s_maintenance, ("light",)),
    ("kit", "Wiederherstellungs-Kit erzeugen, Schritt 2 (§8.2)", s_kit, ("light",)),
    ("adopt", "Backups von einem Ziel übernehmen (§8.3)", s_adopt, ("light",)),
    ("reassign", "Backup einem anderen Ziel zuweisen (§8.8)", s_reassign, ("light",)),
    ("edit-destination", "Verbindung bearbeiten mit Prüfung vor dem Speichern (§8.9)", s_edit_dest, ("light",)),
    ("dialogs-a", "Dialoge: Passwort aktualisieren, Sperre, Host-Schlüssel, Export (§8.4–8.7)", s_dialogs_a, ("light",)),
    ("dialogs-b", "Dialoge: Trotzdem sichern, Backup entfernen, Ziel entfernen, Ausschluss-Regel, Expertenmodus (§7.2, §7.8, §12.3, K2)", s_dialogs_b, ("light",)),
    ("settings", "Einstellungen (§9.6)", s_settings, ("light",)),
    ("newbackup", "Assistent «Neues Backup», Schritt 2 (§9.2)", s_newbackup, ("light",)),
    ("narrow", "Schmales Fenster, 26 gridUnit (§4.3)", s_narrow, ("light", "dark")),
]

# ---------------------------------------------------------------- CSS
def theme_css(sel, t):
    return sel + "{" + ";".join(f"--{k}:{v}" for k, v in t.items() if not k.startswith("_")) + "}"

CSS = theme_css(".t-light", LIGHT) + theme_css(".t-dark", DARK) + r"""
*{box-sizing:border-box}
body{margin:0;font-family:"Noto Sans",sans-serif;font-size:13.3px;background:#7f8c8d}
.t-light .id,.t-dark .il{display:none}
.ic{vertical-align:middle;flex:none}
.shot{padding:28px;display:inline-block;color:var(--text)}
.win{background:var(--win-bg);color:var(--text);border-radius:6px 6px 0 0;box-shadow:0 8px 28px rgba(0,0,0,.35);display:flex;flex-direction:column;overflow:hidden;position:relative;border:1px solid var(--sep-strong)}
.titlebar{height:30px;background:var(--header-bg);display:flex;align-items:center;padding:0 8px;flex:none}
.tb-title{flex:1;text-align:center;font-weight:400}
.tb-btns{display:flex;gap:10px}
.client{flex:1;display:flex;flex-direction:column;min-height:0;position:relative}
.tabrow{display:flex;align-items:stretch;background:var(--header-bg);border-bottom:1px solid var(--sep);flex:none}
.tabs{flex:1;display:flex}
.tab{flex:1;display:flex;flex-direction:column;align-items:center;gap:2px;padding:6px 4px 5px;position:relative;border-bottom:2px solid transparent}
.tab span{font-size:12.5px}
.tab.on{color:var(--text);border-bottom:3px solid var(--sel);background:var(--sel-soft);font-weight:700}
.tab-badge{position:absolute;top:4px;margin-left:30px}
.burger{align-self:center;margin:0 8px}
.columns{flex:1;display:flex;min-height:0}
.colsep{width:1px;background:var(--sep);flex:none}
.page{display:flex;flex-direction:column;min-width:0}
.bg-view{background:var(--view-bg)}.bg-win{background:var(--win-bg)}
.pagehead{height:42px;display:flex;align-items:center;padding:0 10px 0 14px;background:var(--header-bg);border-bottom:1px solid var(--sep);flex:none;gap:8px}
.ptitle{font-size:16px;font-weight:400;flex:1;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.pactions{display:flex;gap:4px;align-items:center}
.crumbs{padding:4px 14px;font-size:12px;color:var(--text-in);border-bottom:1px solid var(--sep);background:var(--win-bg)}
.pbody{flex:1;overflow:hidden;position:relative}
.tbtn{display:inline-flex;align-items:center;gap:5px;padding:4px 7px;border-radius:3px;border:1px solid transparent;white-space:nowrap}
.tbtn.primary{border-color:var(--sep-strong);background:var(--btn-bg)}
.tbtn.disabled{opacity:.45}
.caret{margin-left:-2px}
.btn{display:inline-flex;align-items:center;gap:6px;padding:4px 10px;min-height:28px;border-radius:3px;border:1px solid var(--sep-strong);background:var(--btn-bg);white-space:nowrap;margin-right:6px}
.btn.primary,.btn.default{border-color:var(--sel);box-shadow:inset 0 0 0 1px var(--sel)}
.btn.disabled{opacity:.45}
.cb.disabled,.spin.disabled{opacity:.5}
.badge{display:inline-flex;align-items:center;gap:4px;margin-left:8px;padding:1px 8px;border-radius:10px;border:1px solid var(--sep-strong);font-weight:400;font-size:12px}
.expert-sec{border-left:3px solid var(--sel);margin:8px 14px 0 0}
.wiz-top{display:flex;align-items:center;gap:14px}.wiz-top .prog{flex:1}
.menu.sub2{right:420px;bottom:330px;width:290px}
.desk-note{position:absolute;left:30px;top:20px;color:#fff;opacity:.85;font-size:12.5px}
.btn.danger{color:var(--neg);border-color:var(--neg)}
.li{display:flex;align-items:center;gap:10px;padding:7px 12px;margin:2px 6px;border-radius:3px}
.li.sel{background:var(--sel-soft);box-shadow:inset 0 0 0 1px var(--sel)}
.li-text{flex:1;min-width:0}.li-title{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.li-sub{font-size:12.5px;color:var(--text-in);margin-top:1px}
.li-trail{flex:none}.li-arrow{opacity:.6}
.lsection{font-weight:700;font-size:12.5px;color:var(--text-in);padding:12px 18px 4px}
.ivh{display:flex;align-items:center;padding:8px 10px 8px 18px;border-bottom:1px solid var(--sep)}
.ivh-title{flex:1;font-weight:700}
.im{display:flex;align-items:center;gap:10px;margin:10px 14px;padding:8px 10px;border-radius:5px;border:1px solid}
.im-info{border-color:var(--sel);background:var(--info-soft)}.im-pos{border-color:var(--pos);background:var(--pos-soft)}
.im-warn{border-color:var(--neu);background:var(--neu-soft)}.im-err{border-color:var(--neg);background:var(--neg-soft)}
.im-text{flex:1}.im-actions{display:flex}.im-close{opacity:.7;margin-left:4px}
.stateline{background:var(--win-bg);flex:none}.stateline .im{margin:6px 8px}
.h{font-weight:400}.h1{font-size:18px}.h2{font-size:16px}.h3{font-size:15.3px}.h4{font-size:14.6px;font-weight:700}
.muted{color:var(--text-in)}.small{font-size:12px}
.status{display:inline-flex;align-items:center;gap:5px}
.status .ic{filter:none}
.ov-head{padding:6px 18px 4px}.ov-head .h1{display:flex;align-items:center;gap:8px}
.cards{display:flex;gap:14px;padding:12px 18px;flex-wrap:wrap}
.card{background:var(--view-bg);border:1px solid var(--sep);border-radius:5px;padding:12px 14px;width:362px;display:flex;flex-direction:column;gap:7px}
.card-head{display:flex;gap:10px;align-items:center}
.card-foot{display:flex;align-items:center;justify-content:space-between;margin-top:4px}
.prog{height:6px;border-radius:3px;background:var(--sep);overflow:hidden}.prog-fill{height:100%;background:var(--sel)}
.detail-status{padding:12px 18px 4px;display:flex;flex-direction:column;gap:3px}
.dangerzone{margin:16px 14px;padding:10px 12px;border:1px solid var(--neg);border-radius:5px;background:var(--neg-soft);display:flex;flex-direction:column;gap:6px;align-items:flex-start}
.pad{padding:4px 18px;display:flex;flex-direction:column;gap:7px}
.row{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.spacer{flex:1}
.cb,.rb,.sw{display:inline-flex;align-items:center;gap:7px}
.cb .box{width:16px;height:16px;border:1px solid var(--sep-strong);border-radius:3px;background:var(--btn-bg);flex:none;position:relative}
.cb.on .box{background:var(--sel);border-color:var(--sel)}
.cb.on .box::after{content:"";position:absolute;left:5px;top:1px;width:4px;height:9px;border:solid #fff;border-width:0 2px 2px 0;transform:rotate(45deg)}
.rb .dot{width:16px;height:16px;border:1px solid var(--sep-strong);border-radius:50%;background:var(--btn-bg);position:relative;flex:none}
.rb.on .dot{border-color:var(--sel)}.rb.on .dot::after{content:"";position:absolute;inset:3px;border-radius:50%;background:var(--sel)}
.sw .track{width:32px;height:16px;border-radius:8px;background:var(--sep-strong);position:relative}
.sw.on .track{background:var(--sel)}.sw .knob{position:absolute;top:1px;left:1px;width:14px;height:14px;border-radius:50%;background:#fff}.sw.on .knob{left:17px}
.combo,.spin,.field{display:inline-flex;align-items:center;justify-content:space-between;gap:6px;min-height:28px;padding:3px 8px;border:1px solid var(--sep-strong);border-radius:3px}
.combo{background:var(--btn-bg)}.field,.spin{background:var(--view-bg)}
.field{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.dlgcol{display:flex;flex-direction:column;gap:8px}
.field.mono,.code{font-family:"Noto Sans Mono","Hack",monospace;font-size:12.5px}
.ph{color:var(--text-in)}
.spin-arrows{font-size:8px;line-height:7px;opacity:.7}
.chip{display:inline-flex;align-items:center;padding:2px 9px;border-radius:12px;border:1px solid var(--sep-strong);font-size:12.5px}
.chip.on{background:var(--sel-soft);border-color:var(--sel)}
.formwrap{padding:14px 20px}
.form{display:grid;grid-template-columns:minmax(170px,max-content) 1fr;gap:8px 10px;align-items:center}
.flabel{text-align:right}
.fsec{grid-column:1/-1;font-weight:700;margin-top:12px;padding-top:8px;border-top:1px solid var(--sep)}
.fwidget .im{margin:4px 0}
.days{display:inline-flex;gap:3px}.daybtn{padding:3px 8px;border:1px solid var(--sep-strong);border-radius:3px;background:var(--btn-bg)}
.daybtn.on{background:var(--sel);color:var(--sel-text);border-color:var(--sel)}
.grid3{display:grid;grid-template-columns:repeat(3,max-content);gap:8px 22px}
.kv{display:inline-flex;align-items:center;gap:6px}.kv-l{min-width:88px;text-align:right}
.code{background:var(--view-bg);border:1px solid var(--sep);border-radius:3px;padding:8px 10px;line-height:1.6}
.ln{display:inline-block;width:22px;color:var(--text-in)}
code{font-family:"Noto Sans Mono",monospace;font-size:12px}
.crumbbar{display:flex;align-items:center;gap:6px;padding:8px 14px;border-bottom:1px solid var(--sep)}
.tbl{width:100%;border-collapse:collapse}
.tbl th{text-align:left;font-weight:400;color:var(--text-in);padding:6px 10px;border-bottom:1px solid var(--sep);background:var(--win-alt)}
.tbl td{padding:6px 10px;border-bottom:1px solid var(--sep)}
.tbl tr:nth-child(even) td{background:var(--view-alt)}
.selbar{padding:8px 14px;color:var(--text-in)}a{color:var(--link)}
.overlay{position:absolute;inset:0;background:rgba(0,0,0,.38);display:flex;align-items:center;justify-content:center}
.dialog{background:var(--win-bg);border-radius:6px;box-shadow:0 10px 32px rgba(0,0,0,.45);border:1px solid var(--sep-strong)}
.dlg-head{display:flex;align-items:center;justify-content:space-between;padding:12px 14px;border-bottom:1px solid var(--sep)}
.dlg-body{padding:14px}.dlg-foot{display:flex;justify-content:flex-end;padding:10px 14px;border-top:1px solid var(--sep)}
.indent{margin:4px 0 8px 23px;display:flex;flex-direction:column;gap:6px}
.preview{border:1px solid var(--sep);border-radius:4px;padding:8px 10px;background:var(--view-bg);line-height:1.6}
.phm{height:100%;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:10px;text-align:center;padding:30px}
.phm>.ic{opacity:.55}.phm-expl{max-width:420px;color:var(--text-in)}
.appicon{width:96px;height:96px;border-radius:22px;background:linear-gradient(160deg,#3daee9,#1d6fa5);display:flex;align-items:center;justify-content:center;box-shadow:0 3px 8px rgba(0,0,0,.25)}
.appicon .ic{filter:brightness(0) invert(1)}
.lnk{color:var(--link)}
.wiz{padding:16px 24px;display:flex;flex-direction:column;gap:12px;height:calc(100% - 52px)}
.wiz-steps{display:flex;gap:18px;color:var(--text-in)}.wiz-steps .on{color:var(--sel);font-weight:700}.wiz-steps .done{color:var(--text)}
.wiz-foot{display:flex;align-items:center;padding:10px 14px;border-top:1px solid var(--sep);height:52px}
.qtabs{display:flex;gap:2px;margin:14px 18px 6px;border-bottom:1px solid var(--sep)}
.qtabs span{padding:6px 12px;border:1px solid transparent;border-bottom:none;border-radius:3px 3px 0 0}
.qtabs .on{border-color:var(--sep);background:var(--view-bg);margin-bottom:-1px}
.filters{display:flex;gap:6px;align-items:center;padding:10px 12px;border-bottom:1px solid var(--sep)}
.desk-frame{width:1180px;height:700px;border-radius:6px;overflow:hidden;box-shadow:0 8px 28px rgba(0,0,0,.35)}
.desk{width:100%;height:100%;position:relative;background:linear-gradient(135deg,#1d3557 0%,#2a6f97 55%,#61a5c2 100%);color:var(--text)}
.panel{position:absolute;left:0;right:0;bottom:0;height:44px;background:var(--compl-bg);color:var(--compl-text);display:flex;align-items:center;padding:0 12px;gap:10px}
.trayicons{display:flex;gap:12px;align-items:center}.trayhl{padding:4px;border-radius:4px;background:rgba(61,174,233,.35)}
.clock{text-align:center;font-size:13px;line-height:1.1;margin-left:10px}
.menu{position:absolute;right:70px;bottom:52px;width:340px;background:var(--win-bg);border:1px solid var(--sep-strong);border-radius:5px;box-shadow:0 6px 20px rgba(0,0,0,.4);padding:4px 0}
.mi{padding:5px 14px;display:flex;align-items:center;gap:8px}.mi.dis{color:var(--text-in)}.mi .sub{margin-left:auto}
.msep{height:1px;background:var(--sep);margin:4px 0}
.notifs{position:absolute;left:30px;bottom:52px;display:flex;flex-direction:column;gap:10px;width:380px}
.notif{background:var(--win-bg);border:1px solid var(--sep-strong);border-radius:6px;box-shadow:0 6px 20px rgba(0,0,0,.4);padding:10px 12px;display:flex;flex-direction:column;gap:6px}
.n-head{display:flex;align-items:center;gap:6px;font-size:12.5px;color:var(--text-in)}
.n-title{font-weight:700;display:flex;align-items:center;gap:6px}.n-actions{display:flex;margin-top:4px}
.sheetwrap{background:var(--win-alt);padding:24px;border-radius:6px;width:1100px}
.sheet{display:grid;grid-template-columns:repeat(2,max-content);gap:24px;justify-content:center}
.sheet .dialog{position:static}
.kit{border:1px dashed var(--sep-strong);border-radius:5px;padding:12px 14px;background:var(--view-bg);display:flex;flex-direction:column;gap:10px}
.kit .h3{display:flex;align-items:center;gap:8px}.mono{font-family:"Noto Sans Mono","Hack",monospace}.kitpw{font-size:15px;font-weight:700;letter-spacing:.3px}
.adopt{display:flex;gap:10px;align-items:flex-start;margin:8px 18px;padding:10px 12px;border:1px solid var(--sep);border-radius:5px;background:var(--view-bg)}
.adopt .im{margin:8px 0 0}
.boxlist{background:var(--view-bg);border:1px solid var(--sep);border-radius:5px;padding:4px 0}
.tabrow.bottom{border-top:1px solid var(--sep);border-bottom:none}
.settings .form{grid-template-columns:minmax(150px,max-content) 1fr}
.crumbtitle{font-size:14px;color:var(--text)}
.reason{margin:0 8px 0 -2px}
.stale{opacity:.6}
.statesheet{display:flex;flex-direction:column;gap:4px}.statesheet .im{margin:4px 0 12px}.statesheet .prog{margin-top:6px}
.t-sheet{color:var(--text)}
.gallery{padding:24px 28px;color:#fff}
.single{background:transparent!important}.single .gallery{padding:0}.single .shot{padding:20px}
.gallery h1{font-weight:400;margin:0 0 6px}.gallery .note{max-width:900px;line-height:1.5;opacity:.9}
.gallery .cap{font-size:15px;margin:26px 0 0 28px}
"""

# ---------------------------------------------------------------- HTML
def build_html():
    shots = []
    for sid, cap, fn, themes in SCREENS:
        markup = fn()
        for t in themes:
            shots.append(f'<section class="shotwrap" id="{sid}-{t}" data-id="{sid}-{t}"><div class="cap">{cap} – {"hell" if t=="light" else "dunkel"}</div>'
                         f'<div class="shot t-{t}">{markup}</div></section>')
    js = r"""
const id = location.hash.slice(1);
if (id) { document.querySelectorAll('section.shotwrap').forEach(s => { if (s.dataset.id !== id) s.remove(); else s.querySelector('.cap').remove(); });
          document.querySelector('.gallery-head')?.remove(); document.body.classList.add('single'); }
"""
    head = ('<div class="gallery gallery-head"><h1>Kestic – UI-Mockups</h1><div class="note">Illustrationen zu «GUI Design» (Entwurf v2, 2026-11-15). '
            'Farben aus den Plasma-Farbschemata Breeze und Breeze Dunkel, Icons aus dem Breeze-Icon-Thema (LGPL). '
            'Verbindlich ist die Spezifikation, nicht die Pixel; App-Icon und Tray-Symbol sind Platzhalter. Beispielnamen sind erfunden.</div></div>')
    return f'''<!doctype html><html lang="de"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Kestic Mockups</title><style>{CSS}</style></head><body><div class="gallery">{head}{"".join(shots)}</div><script>{js}</script></body></html>'''

def render_pngs():
    chrome = "/usr/bin/google-chrome"
    tmp = tempfile.mkdtemp(prefix="kestic-chrome-")
    for sid, cap, fn, themes in SCREENS:
        for t in themes:
            out = os.path.join(HERE, f"kestic-{sid}-{t}.png")
            w, h = SIZES.get(sid, (1180, 740)); w += 40; h += 40
            subprocess.run([chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", f"--user-data-dir={tmp}",
                            f"--window-size={w},{h}", "--force-device-scale-factor=1.5", "--default-background-color=00000000",
                            f"--screenshot={out}", f"file://{OUT_HTML}#{sid}-{t}"],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            print("PNG", os.path.basename(out))

if __name__ == "__main__":
    with open(OUT_HTML, "w", encoding="utf-8") as fh:
        fh.write(build_html())
    print("HTML", OUT_HTML, os.path.getsize(OUT_HTML) // 1024, "KB")
    if "--png" in sys.argv:
        render_pngs()
