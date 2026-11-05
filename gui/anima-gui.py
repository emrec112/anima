#!/usr/bin/env python3
"""Anima: pick the desktop character and start/stop the daemon."""

import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gio, GLib, Gtk  # noqa: E402

CONFIG = Path.home() / ".config/anima/config.toml"
DATA = Path.home() / ".local/share/anima"
UMA = DATA / "uma"
# 3D models (.glb), e.g. Blue Archive chibis, one folder per character
BA = DATA / "ba"
LOG = DATA / "animad.log"


def find_bin(name):
    here = Path(__file__).resolve().parent.parent / "target/release" / name
    return str(here) if here.exists() else shutil.which(name) or name


ANIMAD = find_bin("animad")


# --- config ---------------------------------------------------------------

def read_char_config():
    try:
        return tomllib.loads(CONFIG.read_text()).get("character", {})
    except (OSError, tomllib.TOMLDecodeError):
        return {}


def toml_value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    if isinstance(v, (int, float)):
        return repr(float(v)) if isinstance(v, float) else str(v)
    return '"' + str(v).replace("\\", "\\\\").replace('"', '\\"') + '"'


def write_char_config(**values):
    """Set keys inside [character], keeping the rest of the file as is."""
    text = CONFIG.read_text() if CONFIG.exists() else "[character]\n"
    lines = text.splitlines()
    start = next((i for i, l in enumerate(lines) if l.strip() == "[character]"), None)
    if start is None:
        lines += ["", "[character]"]
        start = len(lines) - 1
    end = next((i for i in range(start + 1, len(lines)) if lines[i].lstrip().startswith("[")), len(lines))
    for key, val in values.items():
        new = f"{key} = {toml_value(val)}"
        for i in range(start + 1, end):
            if re.match(rf"\s*{re.escape(key)}\s*=", lines[i]):
                lines[i] = new
                break
        else:
            lines.insert(end, new)
            end += 1
    CONFIG.parent.mkdir(parents=True, exist_ok=True)
    CONFIG.write_text("\n".join(lines) + "\n")


# --- daemon ---------------------------------------------------------------

def daemon_running():
    return subprocess.run(["pgrep", "-x", "animad"], capture_output=True).returncode == 0


def start_daemon():
    DATA.mkdir(parents=True, exist_ok=True)
    log = open(LOG, "w")
    subprocess.Popen([ANIMAD], stdout=log, stderr=log, start_new_session=True)


def stop_daemon():
    subprocess.run(["pkill", "-x", "animad"])


def restart_daemon():
    if daemon_running():
        stop_daemon()
        for _ in range(20):
            if not daemon_running():
                break
            GLib.usleep(50_000)
    start_daemon()


# --- sprites --------------------------------------------------------------

def outfits(char_dir):
    """[(name, calm_path, happy_path or None)] for one character folder."""
    out = []
    for a in sorted(char_dir.glob("*_0010.png")):
        b = a.with_name(a.name.replace("[A]", "[B]").replace("_0010.", "_0011."))
        name = re.sub(r"\s*-\s*Chibi.*$", "", a.name).replace("_", "")
        out.append((name, a, b if b.exists() else None))
    for m in sorted(char_dir.glob("*.glb")):
        out.append((m.stem, m, None))
    return out


def thumb(path, size):
    """Sprite preview, or a generic icon for 3D models."""
    if str(path).endswith(".glb"):
        return Gtk.Image(icon_name="applications-graphics-symbolic", pixel_size=size // 2)
    pic = Gtk.Picture.new_for_filename(str(path))
    pic.set_size_request(size, size)
    pic.set_can_shrink(True)
    return pic


class Window(Adw.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Anima", default_width=900, default_height=620)
        self.cfg = read_char_config()
        self.pending_restart = 0

        # header: on/off switch
        header = Adw.HeaderBar()
        self.power = Gtk.Switch(valign=Gtk.Align.CENTER, tooltip_text="Anima'yı aç/kapat")
        self.power.set_active(daemon_running())
        self.power.connect("notify::active", self.on_power)
        power_box = Gtk.Box(spacing=8)
        power_box.append(Gtk.Label(label="Çalışıyor"))
        power_box.append(self.power)
        header.pack_end(power_box)

        # left: character list with search
        self.search = Gtk.SearchEntry(placeholder_text="Karakter ara")
        self.search.connect("search-changed", lambda *_: self.chars.invalidate_filter())
        self.chars = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
        self.chars.add_css_class("navigation-sidebar")
        self.chars.set_filter_func(self.filter_char)
        self.chars.connect("row-selected", self.on_char)
        dirs = [p for root in (UMA, BA) if root.exists() for p in root.iterdir() if p.is_dir()]
        for d in sorted(dirs, key=lambda p: p.name.lower()):
            row = Gtk.ListBoxRow()
            row.char_dir = d
            row.set_child(Gtk.Label(label=d.name, xalign=0, margin_start=8, margin_top=6, margin_bottom=6))
            self.chars.append(row)
        left = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6, margin_start=6, margin_end=6, margin_top=6)
        left.append(self.search)
        left.append(Gtk.ScrolledWindow(child=self.chars, vexpand=True, width_request=220))

        # right: outfits of the selected character
        self.grid = Gtk.FlowBox(
            selection_mode=Gtk.SelectionMode.NONE, max_children_per_line=4, min_children_per_line=1,
            row_spacing=12, column_spacing=12, margin_start=12, margin_end=12, margin_top=12, margin_bottom=12,
            valign=Gtk.Align.START,
        )
        self.current = Gtk.Label(xalign=0, ellipsize=3, margin_start=12, margin_top=8)
        self.current.add_css_class("dim-label")
        # active characters: main first, then extras (click × to remove)
        self.team = Gtk.Box(spacing=8, margin_start=12, margin_end=12, margin_top=6)

        # bottom: size + walking
        self.scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, 0.2, 1.5, 0.05)
        self.scale.set_hexpand(True)
        self.scale.set_value(float(self.cfg.get("scale", 0.45)))
        self.scale.connect("value-changed", self.on_scale)
        self.walk = Gtk.Switch(valign=Gtk.Align.CENTER, active=bool(self.cfg.get("walk", True)))
        self.walk.connect("notify::active", lambda s, _: self.apply(walk=s.get_active()))
        self.follow = Gtk.Switch(valign=Gtk.Align.CENTER, active=bool(self.cfg.get("look_at_mouse", True)))
        self.follow.connect("notify::active", lambda s, _: self.apply(look_at_mouse=s.get_active()))
        controls = Gtk.Box(spacing=10, margin_start=12, margin_end=12, margin_top=8, margin_bottom=12)
        controls.append(Gtk.Label(label="Boyut"))
        controls.append(self.scale)
        controls.append(Gtk.Label(label="Gezinme"))
        controls.append(self.walk)
        controls.append(Gtk.Label(label="Fare takibi"))
        controls.append(self.follow)

        right = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        right.append(self.current)
        right.append(Gtk.ScrolledWindow(child=self.team, vscrollbar_policy=Gtk.PolicyType.NEVER, min_content_height=96))
        right.append(Gtk.Separator())
        right.append(Gtk.ScrolledWindow(child=self.grid, vexpand=True, hexpand=True))
        right.append(Gtk.Separator())
        right.append(controls)

        body = Gtk.Box()
        body.append(left)
        body.append(Gtk.Separator(orientation=Gtk.Orientation.VERTICAL))
        body.append(right)

        view = Adw.ToolbarView(content=body)
        view.add_top_bar(header)
        self.toast = Adw.ToastOverlay(child=view)
        self.set_content(self.toast)
        self.update_current()

        # keep the switch honest if the daemon dies or is started elsewhere
        GLib.timeout_add_seconds(2, self.poll_daemon)

        # open on the character in use
        src = Path(self.cfg.get("source", ""))
        row = self.chars.get_first_child()
        while row:
            if getattr(row, "char_dir", None) == src.parent:
                self.chars.select_row(row)
                break
            row = row.get_next_sibling()

    # --- handlers ---

    def filter_char(self, row):
        q = self.search.get_text().strip().lower()
        return not q or q in row.char_dir.name.lower()

    def on_char(self, _box, row):
        while (c := self.grid.get_first_child()) is not None:
            self.grid.remove(c)
        if row is None:
            return
        for name, calm, happy in outfits(row.char_dir):
            pics = Gtk.Box(spacing=4, halign=Gtk.Align.CENTER)
            for p in (calm, happy):
                if p:
                    pics.append(thumb(p, 96))
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
            box.append(pics)
            box.append(Gtk.Label(label=name, wrap=True, max_width_chars=18, justify=Gtk.Justification.CENTER))
            btn = Gtk.Button(child=box, tooltip_text="Ana karakter yap\nSol poz: durma/yürüme · Sağ: zıplama/yakalama")
            btn.add_css_class("flat")
            btn.connect("clicked", lambda _b, c=calm, n=name: self.pick(c, n))
            add = Gtk.Button(icon_name="list-add-symbolic", tooltip_text="Ekibe ekle", halign=Gtk.Align.CENTER)
            add.add_css_class("circular")
            add.connect("clicked", lambda _b, c=calm, n=name: self.add_extra(c, n))
            card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            card.append(btn)
            card.append(add)
            self.grid.append(card)

    def pick(self, calm, name):
        self.apply(source=str(calm), source_happy="", enabled=True)
        self.update_current()
        self.toast.add_toast(Adw.Toast(title=f"{calm.parent.name} · {name}", timeout=2))

    def add_extra(self, calm, name):
        cfg = read_char_config()
        extra = list(cfg.get("extra", []))
        if not cfg.get("source"):
            return self.pick(calm, name)
        extra.append(str(calm))
        self.apply(extra=extra, enabled=True)
        self.update_current()
        self.toast.add_toast(Adw.Toast(title=f"Eklendi: {calm.parent.name} · {name}", timeout=2))

    def remove(self, index):
        cfg = read_char_config()
        extra = list(cfg.get("extra", []))
        if index == 0:
            # removing the main one promotes the first extra
            if not extra:
                return
            self.apply(source=extra.pop(0), source_happy="", extra=extra)
        else:
            extra.pop(index - 1)
            self.apply(extra=extra)
        self.update_current()

    def on_scale(self, scale):
        # wait until the slider settles before restarting
        self.pending_restart += 1
        n = self.pending_restart
        GLib.timeout_add(400, lambda: n == self.pending_restart and self.apply(scale=round(scale.get_value(), 2)) and False)

    def apply(self, **values):
        write_char_config(**values)
        if self.power.get_active():
            restart_daemon()
        return False

    def on_power(self, sw, _):
        if sw.get_active() and not daemon_running():
            start_daemon()
        elif not sw.get_active() and daemon_running():
            stop_daemon()

    def poll_daemon(self):
        running = daemon_running()
        if running != self.power.get_active():
            self.power.handler_block_by_func(self.on_power)
            self.power.set_active(running)
            self.power.handler_unblock_by_func(self.on_power)
        return True

    def update_current(self):
        cfg = read_char_config()
        src = cfg.get("source", "")
        team = ([src] if src else []) + list(cfg.get("extra", []))
        self.current.set_label(f"Ekip: {len(team)} karakter · ilki ana karakter" if team else "Karakter seçilmedi")
        while (c := self.team.get_first_child()) is not None:
            self.team.remove(c)
        for i, path in enumerate(team):
            pic = thumb(path, 64)
            rm = Gtk.Button(icon_name="window-close-symbolic", tooltip_text="Çıkar", halign=Gtk.Align.CENTER)
            rm.add_css_class("circular")
            rm.add_css_class("flat")
            rm.connect("clicked", lambda _b, i=i: self.remove(i))
            rm.set_sensitive(len(team) > 1)
            col = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, tooltip_text=Path(path).parent.name)
            col.append(pic)
            col.append(rm)
            self.team.append(col)


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id="dev.anima.Anima", flags=Gio.ApplicationFlags.DEFAULT_FLAGS)

    def do_activate(self):
        win = self.props.active_window or Window(self)
        win.present()


if __name__ == "__main__":
    sys.exit(App().run(sys.argv))
