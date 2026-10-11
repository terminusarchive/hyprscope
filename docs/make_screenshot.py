#!/usr/bin/env python3
"""Makes docs/screenshot.svg, the picture of hyprscope's front page in the README.

    python3 docs/make_screenshot.py

It needs Python 3 (standard library only), tmux, `unshare` (util-linux) with unprivileged
user namespaces enabled, and the release binary (built with `cargo build --release` when
it is missing). It takes about 80 seconds.

What it does:

1. Makes a temporary directory that stands in for Hyprland: the command socket, the
   event socket, `hyprland.log` and `hyprland.lock`. All of its content is invented.
   Nothing is read from the machine it runs on, except the standard clock and /proc
   entries of the stand-in process.
2. Runs hyprscope against it in a 160x44 terminal (a tmux session on a private
   socket, with no tmux configuration read) for long enough that the sparklines fill.
3. Captures the screen with its colours and writes it as SVG. Text is placed on a
   fixed grid, so it lines up in any monospace font; box-drawing lines and block
   characters are drawn as shapes, so they join without gaps.

The "Hyprland" process in the Hyprland panel is a child of this script that runs a
little code to give the panel a CPU and memory figure and a thread count; its command
line says `--config ~/.config/hypr/hyprland.lua`. It has no graphics device open. The GPU row is
invented: hyprscope runs in a private mount namespace (`unshare -r -m`) in which a
directory of the script's own is bind-mounted over `/proc/<its pid>/fdinfo`. It holds one
DRM client file in the kernel's time form, with an invented driver, device and client id,
rewritten once a second with a rising engine counter (about 3 to 30 percent busy) and a
memory figure of a few hundred MiB. No GPU figure of the machine is read. Its
process ID, shown in the same panel, is replaced in the picture by an invented one.
"""

import argparse
import html
import json
import os
import random
import re
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COLS, ROWS = 160, 44
SEPARATOR = "\n\n\n"
SIGNATURE = "demo"

# ---------------------------------------------------------------------------
# The invented desktop.

MONITORS = [
    # id, name, make, model, width, height, refresh, x, scale, workspace, special, vrr, scanout
    (0, "DP-1", "Example Co", "EX270Q", 2560, 1440, 144.0, 0, 1.0, 1, -98, True, None),
    (1, "HDMI-A-1", "Sample Inc", "SM240", 1920, 1080, 60.0, 2560, 1.0, 4, 0, False, "mpv"),
]

WORKSPACES = [
    # id, name, monitor id
    (1, "1", 0), (2, "2", 0), (3, "3", 0), (4, "4", 1), (5, "5", 1), (-98, "special:scratchpad", 0),
]

# key, workspace id, monitor id, class, title, pid, at, size, floating, fullscreen
WINDOWS = [
    ("emacs", 1, 0, "emacs", "parser.c - Editor", 18204, (20, 50), (1250, 1370), False, 0),
    ("demo", 1, 0, "foot", "~/src/demo", 18376, (1290, 50), (1250, 680), False, 0),
    ("cargo", 1, 0, "foot", "cargo test (1/12)", 18376, (1290, 750), (1250, 670), False, 0),
    ("docs", 2, 0, "firefox", "The Rust Programming Language - Mozilla Firefox", 18520, (20, 50), (1250, 1370), False, 0),
    ("example", 2, 0, "firefox", "Example Domain - Mozilla Firefox", 18520, (1290, 50), (1250, 1370), False, 0),
    ("files", 3, 0, "thunar", "Downloads - Thunar", 18911, (20, 50), (1250, 1370), False, 0),
    ("manual", 3, 0, "org.gnome.Evince", "manual.pdf - Document Viewer", 19046, (1290, 50), (1250, 680), False, 0),
    ("htop", 3, 0, "foot", "htop", 18376, (1290, 750), (1250, 670), False, 0),
    ("mpv", 4, 1, "mpv", "Big Buck Bunny - mpv", 19377, (2560, 0), (1920, 1080), False, 2),
    ("music", 5, 1, "strawberry", "Slow Waves - Nightdrive - Strawberry", 19520, (2580, 20), (1880, 1040), False, 0),
    ("volume", 5, 1, "pavucontrol", "Volume Control", 19688, (3000, 300), (800, 500), True, 0),
    ("scratch", -98, 0, "foot", "scratch", 18376, (480, 200), (1600, 900), False, 0),
]
FOCUSED = "emacs"

ADDRESS = {w[0]: "0x55a3c7e0%04x" % (0x1000 + 16 * i) for i, w in enumerate(WINDOWS)}


def bare(key):
    return ADDRESS[key][2:]


def window_json(i, w):
    key, ws, mon, cls, title, pid, at, size, floating, fullscreen = w
    name = next(n for (i_, n, _) in WORKSPACES if i_ == ws)
    return {
        "address": ADDRESS[key], "mapped": True, "hidden": False, "visible": True,
        "acceptsInput": True, "at": list(at), "size": list(size),
        "workspace": {"id": ws, "name": name}, "floating": floating, "monitor": mon,
        "class": cls, "title": title, "initialClass": cls, "initialTitle": title,
        "pid": pid, "xwayland": False, "pinned": False, "pinFullscreened": False,
        "fullscreen": fullscreen, "fullscreenClient": 0, "fullscreenHandler": "default",
        "allowedOverFullscreen": True, "grouped": [], "tags": [], "swallowing": "0x0",
        "focusHistoryID": i, "inhibitingIdle": False, "xdgTag": "", "xdgDescription": "",
        "contentType": "none", "tearingHint": False, "stableId": "%08x" % (0x18000000 + i),
    }


def monitor_json(m):
    mid, name, make, model, w, h, hz, x, scale, ws, special, vrr, scanout = m
    ws_name = next(n for (i, n, _) in WORKSPACES if i == ws)
    sp_name = next((n for (i, n, _) in WORKSPACES if i == special), "")
    return {
        "id": mid, "name": name, "description": f"{make} {model} SERIAL000{mid}", "make": make,
        "model": model, "serial": f"SERIAL000{mid}", "width": w, "height": h,
        "physicalWidth": 600, "physicalHeight": 340, "refreshRate": hz, "x": x, "y": 0,
        "activeWorkspace": {"id": ws, "name": ws_name},
        "specialWorkspace": {"id": special, "name": sp_name},
        "reserved": [0, 30, 0, 0], "scale": scale, "transform": 0, "focused": mid == 0,
        "dpmsStatus": True, "vrr": vrr, "solitary": "0", "solitaryBlockedBy": [],
        "activelyTearing": False, "tearingBlockedBy": ["NOT_TORN", "USER"],
        "directScanoutTo": ADDRESS[scanout] if scanout else "0", "directScanoutBlockedBy": [],
        "disabled": False, "currentFormat": "XRGB8888", "mirrorOf": "none",
        "availableModes": [f"{w}x{h}@{hz:.2f}Hz"], "colorManagementPreset": "srgb",
        "sdrBrightness": 1, "sdrSaturation": 1, "sdrMinLuminance": 0.2, "sdrMaxLuminance": 80,
        "hardwareCursorsInUse": True,
    }


def workspace_json(ws):
    wid, name, mon = ws
    inside = [w for w in WINDOWS if w[1] == wid]
    last = inside[-1] if inside else None
    return {
        "id": wid, "name": name, "monitor": MONITORS[mon][1], "monitorID": mon,
        "windows": len(inside), "hasfullscreen": any(w[9] for w in inside),
        "lastwindow": ADDRESS[last[0]] if last else "0x0",
        "lastwindowtitle": last[4] if last else "", "ispersistent": False, "tiledLayout": "dwindle",
    }


def pretty(value):
    return json.dumps(value, indent=4)


def snapshot_reply():
    version = {"branch": "v0.56.2", "version": "0.56.2", "dirty": False, "tag": "v0.56.2", "flags": []}
    status = {"configProvider": "lua", "backend": "drm"}
    focused = window_json(0, next(w for w in WINDOWS if w[0] == FOCUSED))
    replies = [
        pretty(version), pretty(status), json.dumps("default"),
        pretty([monitor_json(m) for m in MONITORS]),
        pretty([workspace_json(w) for w in WORKSPACES]),
        pretty([window_json(i, w) for i, w in enumerate(WINDOWS)]),
        pretty(focused),
    ]
    return SEPARATOR.join(replies)


OPTION_COUNT, OPTIONS_SET = 284, 52
BINDS, SUBMAPS = 71, 2
ANIMATIONS, OVERRIDDEN, CURVES = 30, 9, 4


def option_names():
    return [f"section{i // 12}:option{i % 12}" for i in range(OPTION_COUNT)]


def request_reply(command):
    if command == "[[BATCH]]j/version;j/status;j/submap;j/monitors;j/workspaces;j/clients;j/activewindow":
        return snapshot_reply()
    if command == "j/descriptions":
        return pretty([{"name": n, "description": "invented", "type": 0} for n in option_names()])
    if command == "[[BATCH]]j/configerrors;j/binds;j/animations":
        binds = [{"submap": "resize" if i < 6 else "", "key": f"k{i}"} for i in range(BINDS)]
        animations = [{"name": f"anim{i}", "overridden": i < OVERRIDDEN} for i in range(ANIMATIONS)]
        curves = [{"name": f"curve{i}"} for i in range(CURVES)]
        return SEPARATOR.join([json.dumps([""]), pretty(binds), pretty([animations, curves])])
    if command.startswith("[[BATCH]]j/getoption "):
        names = command[len("[[BATCH]]"):].split(";")
        replies = []
        for i, _ in enumerate(names):
            replies.append(json.dumps({"option": f"o{i}", "int": 1, "set": i < OPTIONS_SET}))
        return SEPARATOR.join(replies)
    return "unknown request"


# Event lines as (seconds after hyprscope connects, line).
def events():
    out = []

    def at(t, *lines):
        out.extend((t, line) for line in lines)

    def focus(t, key):
        cls, title = next((w[3], w[4]) for w in WINDOWS if w[0] == key)
        at(t, f"activewindow>>{cls},{title}", f"activewindowv2>>{bare(key)}")

    def retitle(t, key, title):
        at(t, f"windowtitle>>{bare(key)}", f"windowtitlev2>>{bare(key)},{title}")

    focus(2, "emacs")
    retitle(4, "emacs", "parser.c [+] - Editor")
    focus(6, "cargo")
    at(8, "openlayer>>rofi")
    at(9.5, "closelayer>>rofi")
    new = "55a3c7e02000"
    at(11, f"openwindow>>{new},2,firefox,Mozilla Firefox")
    at(11.2, f"activewindow>>firefox,Mozilla Firefox", f"activewindowv2>>{new}")
    retitle(13, "emacs", "parser.c - Editor")
    for k in range(1, 13):
        retitle(8 + k * 5, "cargo", f"cargo test ({k}/12)")
    at(15, "submap>>resize")
    focus(17, "demo")
    at(20, "submap>>")
    at(23, "workspacev2>>2,2", "workspace>>2", "focusedmonv2>>DP-1,2", "focusedmon>>DP-1,2")
    focus(23.4, "docs")
    focus(26, "example")
    focus(28.5, "docs")
    at(31, "configreloaded>>")
    at(33, "workspacev2>>1,1", "workspace>>1", "focusedmonv2>>DP-1,1", "focusedmon>>DP-1,1")
    focus(33.4, "emacs")
    at(36, f"closewindow>>{new}")
    focus(38, "demo")
    focus(40, "emacs")
    retitle(42, "demo", "~/src/demo/parser")
    focus(44, "demo")
    at(47, f"changefloatingmode>>{bare('volume')},1")
    at(49, f"urgent>>{bare('music')}")
    at(51, "fullscreen>>1")
    at(53, "fullscreen>>0")
    at(55, "activelayout>>example-keyboard,English (US)")
    focus(57, "emacs")
    at(60, f"movewindowv2>>{bare('htop')},3,3", "movewindow>>" + bare("htop") + ",3")
    focus(62, "cargo")
    at(64, "openlayer>>rofi")
    at(65, "closelayer>>rofi")
    at(66.5, f"changefloatingmode>>{bare('volume')},0")
    at(68, f"changefloatingmode>>{bare('volume')},1")
    focus(70, "emacs")
    retitle(71.2, "emacs", "parser.c [+] - Editor")
    focus(72.4, "demo")
    focus(73.1, "cargo")
    retitle(73.5, "cargo", "cargo test (done)")
    focus(74.2, "emacs")
    return sorted(out, key=lambda e: e[0])


LOG_LINES = """\
DEBUG ]: Hyprland init finished
LOG ]: Creating the HookSystem
LOG ]: Using the lua configuration provider
DEBUG ]: Monitor DP-1: 2560x1440@144.00 scale 1.00 at 0x0
WARN ]: Cursor theme "Adwaita-dark" was not found, using the default theme
LOG ]: Monitor HDMI-A-1: 1920x1080@60.00 scale 1.00 at 2560x0
DEBUG ]: Using the DRM backend with 2 outputs
WARN ]: Monitor HDMI-A-1: no EDID colour data, assuming sRGB
ERR from aquamarine ]: Failed to set the gamma table on DP-1: Invalid argument
DEBUG ]: Loading 4 bezier curves and 30 animations
WARN ]: Layer namespace "waybar" asked for an unsupported keyboard mode
ERR ]: Could not open the shader "blur_extra.frag": No such file or directory
DEBUG ]: Started the event loop
WARN ]: Window rule "float,class:^(pavucontrol)$" matched 0 windows at load
LOG ]: Config loaded in 12 ms
""".splitlines()

LOG_LATER = [
    (14, "WARN ]: Monitor HDMI-A-1: no EDID colour data, assuming sRGB"),
    (22, "ERR from aquamarine ]: Failed to set the gamma table on DP-1: Invalid argument"),
    (31, "LOG ]: Config reloaded in 9 ms"),
    (38, "WARN ]: Client \"firefox\" did not answer a ping within 5 s"),
    (52, "ERR from aquamarine ]: Failed to set the gamma table on DP-1: Invalid argument"),
    (63, "WARN ]: Monitor HDMI-A-1: no EDID colour data, assuming sRGB"),
]


# ---------------------------------------------------------------------------
# The stand-in for Hyprland.

CHILD = r'''
import random, sys, threading, time
random.seed(7)
for _ in range(11):
    threading.Thread(target=time.sleep, args=(10**6,), daemon=True).start()
held = [b"x" * (1 << 20) for _ in range(90)]
level = 0.05
while True:
    second = time.perf_counter()
    level = min(0.16, max(0.02, level + random.uniform(-0.04, 0.045)))
    if random.random() < 0.08:
        level = 0.2
    busy_until = second + level
    while time.perf_counter() < busy_until:
        pass
    if random.random() < 0.4:
        held.extend(b"x" * (1 << 20) for _ in range(random.randint(2, 9)))
    elif len(held) > 100 and random.random() < 0.5:
        del held[-random.randint(2, 12):]
    time.sleep(max(0, second + 1 - time.perf_counter()))
'''


class Standin:
    def __init__(self, base, seconds):
        self.base = base
        self.dir = os.path.join(base, "hypr", SIGNATURE)
        os.makedirs(self.dir)
        self.stop = threading.Event()
        self.threads = []
        self.child = None
        self.listeners = []
        script = os.path.join(base, "hyprland-process.py")
        with open(script, "w") as f:
            f.write(CHILD)
        # The command line is what the Config panel reads its file name from.
        self.child = subprocess.Popen(
            [sys.executable, script, "--config", "~/.config/hypr/hyprland.lua"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        with open(os.path.join(self.dir, "hyprland.lock"), "w") as f:
            f.write(f"{self.child.pid}\n{SIGNATURE}\n")
        # The invented DRM client file. The temporary copy is kept outside the directory that
        # is mounted over fdinfo, so a rescan never sees it as a second client.
        self.fdinfo = os.path.join(base, "fdinfo")
        self.fdtmp = os.path.join(base, "fdtmp")
        os.makedirs(self.fdinfo)
        os.makedirs(self.fdtmp)
        self.gpu_busy = 0
        self.gpu_level = 0.1
        self.gpu_random = random.Random(11)
        self.write_fdinfo(250)
        self.log_path = os.path.join(self.dir, "hyprland.log")
        with open(self.log_path, "w") as f:
            f.write("\n".join(LOG_LINES) + "\n")
        self.serve(".socket.sock", self.commands)
        self.serve(".socket2.sock", self.stream)

    def listen(self, name):
        path = os.path.join(self.dir, name)
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(path)
        server.listen(8)
        server.settimeout(0.2)
        self.listeners.append(server)
        return server

    def serve(self, name, handler):
        server = self.listen(name)

        def loop():
            while not self.stop.is_set():
                try:
                    conn, _ = server.accept()
                except socket.timeout:
                    continue
                except OSError:
                    return
                threading.Thread(target=handler, args=(conn,), daemon=True).start()

        thread = threading.Thread(target=loop, daemon=True)
        thread.start()
        self.threads.append(thread)

    def commands(self, conn):
        with conn:
            command = conn.recv(1 << 20).decode()
            try:
                conn.sendall(request_reply(command).encode())
            except OSError:
                pass

    def stream(self, conn):
        start = time.monotonic()
        with conn:
            try:
                for when, line in events():
                    wait = start + when - time.monotonic()
                    if self.stop.wait(max(0, wait)):
                        return
                    conn.sendall((line + "\n").encode())
                # Hyprland never closes the stream; hyprscope stops if it is closed.
                self.stop.wait()
            except OSError:
                pass

    def write_fdinfo(self, mib):
        text = (
            "pos:\t0\nflags:\t02100002\nmnt_id:\t27\nino:\t1\n"
            "drm-driver:\texampledrm\ndrm-pdev:\t0000:0a:00.0\ndrm-client-id:\t5\n"
            f"drm-engine-gfx:\t{self.gpu_busy} ns\ndrm-resident-vram:\t{mib} MiB\n"
        )
        tmp = os.path.join(self.fdtmp, "client")
        with open(tmp, "w") as f:
            f.write(text)
        os.replace(tmp, os.path.join(self.fdinfo, "7"))

    def gpu_writer(self):
        rnd = self.gpu_random
        mib = 250
        while not self.stop.wait(1):
            self.gpu_level = min(0.3, max(0.03, self.gpu_level + rnd.uniform(-0.06, 0.06)))
            self.gpu_busy += int(self.gpu_level * 1e9)
            mib = min(420, max(200, mib + rnd.randint(-12, 15)))
            self.write_fdinfo(mib)

    def start_gpu_writer(self):
        thread = threading.Thread(target=self.gpu_writer, daemon=True)
        thread.start()
        self.threads.append(thread)

    def append_log(self):
        start = time.monotonic()
        for when, line in LOG_LATER:
            if self.stop.wait(max(0, start + when - time.monotonic())):
                return
            with open(self.log_path, "a") as f:
                f.write(line + "\n")

    def start_log_writer(self):
        thread = threading.Thread(target=self.append_log, daemon=True)
        thread.start()
        self.threads.append(thread)

    def close(self):
        self.stop.set()
        for server in self.listeners:
            server.close()
        if self.child:
            self.child.kill()
            self.child.wait()


# ---------------------------------------------------------------------------
# Running hyprscope and capturing its screen.

def tmux(socket_path, *args, check=True):
    return subprocess.run(
        ["tmux", "-S", socket_path, "-f", "/dev/null", *args],
        capture_output=True, text=True, check=check,
    )


def capture(binary, base, seconds, pid, fdinfo):
    sock = os.path.join(base, "tmux.sock")
    env = f"XDG_RUNTIME_DIR={base} HYPRLAND_INSTANCE_SIGNATURE={SIGNATURE} TZ=UTC TERM=tmux-256color"
    # Mounted over the stand-in's fdinfo in a private namespace, so hyprscope reads the
    # invented client file there; everything else in /proc stays as it is.
    inner = f"mount --bind {shlex.quote(fdinfo)} /proc/{pid}/fdinfo && exec env {env} {shlex.quote(binary)}"
    command = f"unshare -r -m sh -c {shlex.quote(inner)}"
    try:
        tmux(sock, "new-session", "-d", "-s", "shot", "-x", str(COLS), "-y", str(ROWS), command)
        tmux(sock, "set-option", "-g", "status", "off")
        time.sleep(seconds)
        out = tmux(sock, "capture-pane", "-e", "-p", "-N", "-t", "shot")
        alive = tmux(sock, "has-session", "-t", "shot", check=False).returncode == 0
        if not alive:
            raise SystemExit("hyprscope exited before the capture")
        return out.stdout
    finally:
        tmux(sock, "kill-server", check=False)


# ---------------------------------------------------------------------------
# From the captured screen to SVG.

BACKGROUND = (0x0e, 0x11, 0x16)
FOREGROUND = (0xd5, 0xda, 0xe3)
CW, CH = 9, 18          # one cell, in SVG units
FONT = 15
BASELINE = 13.5
PAD = 12

SGR = re.compile(r"\x1b\[([0-9;:]*)m")
LIGHT = {  # box drawing: (left, right, up, down)
    "─": (1, 1, 0, 0), "│": (0, 0, 1, 1), "┌": (0, 1, 0, 1), "┐": (1, 0, 0, 1),
    "└": (0, 1, 1, 0), "┘": (1, 0, 1, 0), "├": (0, 1, 1, 1), "┤": (1, 0, 1, 1),
    "┬": (1, 1, 0, 1), "┴": (1, 1, 1, 0), "┼": (1, 1, 1, 1),
}
# Block elements as (x, y, width, height) fractions of the cell.
BLOCKS = {chr(0x2580): (0, 0, 1, .5), chr(0x2588): (0, 0, 1, 1), chr(0x258c): (0, 0, .5, 1), chr(0x2590): (.5, 0, .5, 1)}
for n in range(1, 8):
    BLOCKS[chr(0x2580 + n)] = (0, 1 - n / 8, 1, n / 8)


def xterm(n):
    base = [(0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16), (36, 114, 200), (188, 63, 188),
            (17, 168, 205), (229, 229, 229), (102, 102, 102), (241, 76, 76), (35, 209, 139),
            (245, 245, 67), (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255)]
    if n < 16:
        return base[n]
    if n < 232:
        n -= 16
        level = lambda v: 0 if v == 0 else 55 + 40 * v
        return level(n // 36), level(n // 6 % 6), level(n % 6)
    v = 8 + 10 * (n - 232)
    return v, v, v


def parse_screen(text):
    """Lines of cells (char, fg, bg, bold); a colour of None is the default."""
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    fg = bg = None
    bold = False
    grid = []
    for line in lines:
        row = []
        pos = 0
        for m in SGR.finditer(line):
            for ch in line[pos:m.start()]:
                row.append((ch, fg, bg, bold))
            pos = m.end()
            codes = [int(c) if c else 0 for c in re.split(r"[;:]", m.group(1))] or [0]
            i = 0
            while i < len(codes):
                c = codes[i]
                if c == 0:
                    fg = bg = None
                    bold = False
                elif c == 1:
                    bold = True
                elif c == 22:
                    bold = False
                elif c == 39:
                    fg = None
                elif c == 49:
                    bg = None
                elif 30 <= c <= 37:
                    fg = xterm(c - 30)
                elif 90 <= c <= 97:
                    fg = xterm(c - 90 + 8)
                elif 40 <= c <= 47:
                    bg = xterm(c - 40)
                elif 100 <= c <= 107:
                    bg = xterm(c - 100 + 8)
                elif c in (38, 48):
                    if codes[i + 1] == 2:
                        colour = tuple(codes[i + 2:i + 5])
                        i += 4
                    else:
                        colour = xterm(codes[i + 2])
                        i += 2
                    if c == 38:
                        fg = colour
                    else:
                        bg = colour
                i += 1
        for ch in line[pos:]:
            row.append((ch, fg, bg, bold))
        row += [(" ", None, None, False)] * (COLS - len(row))
        grid.append(row)
    grid += [[(" ", None, None, False)] * COLS for _ in range(ROWS - len(grid))]
    if len(grid) != ROWS or any(len(r) != COLS for r in grid):
        raise SystemExit(f"captured screen is not {COLS}x{ROWS}")
    return grid


def hexcolour(c):
    return "#%02x%02x%02x" % c


def number(v):
    return ("%.2f" % v).rstrip("0").rstrip(".")


def to_svg(grid):
    classes = {}

    def cls(colour, bold):
        key = (colour or FOREGROUND, bold)
        if key not in classes:
            classes[key] = "c%d" % len(classes)
        return classes[key]

    out = []
    # Backgrounds: runs of cells with a background of their own.
    rects = []
    for y, row in enumerate(grid):
        x = 0
        while x < COLS:
            bg = row[x][2]
            if bg is None or bg == BACKGROUND:
                x += 1
                continue
            start = x
            while x < COLS and row[x][2] == bg:
                x += 1
            rects.append((bg, start * CW, y * CH, (x - start) * CW, CH))
    for bg, x, y, w, h in rects:
        out.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{hexcolour(bg)}"/>')

    # Block elements, merged along a row when alike.
    blocks = []
    for y, row in enumerate(grid):
        x = 0
        while x < COLS:
            ch, fg, _, _ = row[x]
            if ch not in BLOCKS:
                x += 1
                continue
            start = x
            fx, fy, fw, fh = BLOCKS[ch]
            x += 1
            while fw == 1 and x < COLS and row[x][0] == ch and row[x][1] == fg:
                x += 1
            blocks.append((fg or FOREGROUND, (start + fx) * CW, y * CH + fy * CH, (x - start) * CW * fw, fh * CH))
    for fg, x, y, w, h in blocks:
        out.append(f'<rect x="{number(x)}" y="{number(y)}" width="{number(w)}" height="{number(h)}" fill="{hexcolour(fg)}"/>')

    # Lines: horizontal and vertical runs by colour, from centre to centre.
    paths = {}

    def add(colour, d):
        paths.setdefault(colour, []).append(d)

    for y, row in enumerate(grid):
        cy = y * CH + CH / 2
        for x, (ch, fg, _, _) in enumerate(row):
            if ch not in LIGHT:
                continue
            l, r, u, d = LIGHT[ch]
            cx = x * CW + CW / 2
            colour = fg or FOREGROUND
            if l:
                add(colour, f"M{number(x * CW)} {number(cy)}H{number(cx)}")
            if r:
                add(colour, f"M{number(cx)} {number(cy)}H{number((x + 1) * CW)}")
            if u:
                add(colour, f"M{number(cx)} {number(y * CH)}V{number(cy)}")
            if d:
                add(colour, f"M{number(cx)} {number(cy)}V{number((y + 1) * CH)}")
    line_elements = [
        f'<path stroke="{hexcolour(colour)}" d="{"".join(ds)}"/>' for colour, ds in paths.items()
    ]
    if line_elements:
        out.append('<g fill="none" stroke-width="1.2" stroke-linecap="square" shape-rendering="geometricPrecision">')
        out.extend(line_elements)
        out.append("</g>")

    # Text: runs of the same style, split at two spaces and at every non-ASCII character.
    texts = []
    for y, row in enumerate(grid):
        x = 0
        while x < COLS:
            ch, fg, _, bold = row[x]
            if ch == " " or ch in BLOCKS or ch in LIGHT:
                x += 1
                continue
            style = (fg, bold)
            if ord(ch) > 126:
                texts.append((y, x, 1, ch, style, False))
                x += 1
                continue
            start = x
            chars = []
            while x < COLS:
                c, f, _, b = row[x]
                if (f, b) != style or c in BLOCKS or c in LIGHT or ord(c) > 126:
                    break
                if c == " " and (x + 1 >= COLS or row[x + 1][0] == " " or (row[x + 1][1], row[x + 1][3]) != style):
                    break
                chars.append(c)
                x += 1
            texts.append((y, start, len(chars), "".join(chars), style, len(chars) > 1))
    for y, x, n, s, (fg, bold), spread in texts:
        attrs = f'class="{cls(fg, bold)}" x="{x * CW}" y="{number(y * CH + BASELINE)}"'
        if spread:
            attrs += f' textLength="{n * CW}"'
        out.append(f"<text {attrs}>{html.escape(s, quote=False)}</text>")

    css = [
        f"text{{font-family:'DejaVu Sans Mono','Menlo','Consolas','Liberation Mono','Courier New',monospace;"
        f"font-size:{FONT}px;white-space:pre}}",
    ]
    for (colour, bold), name in classes.items():
        css.append(f".{name}{{fill:{hexcolour(colour)}" + (";font-weight:700" if bold else "") + "}")
    width, height = COLS * CW + 2 * PAD, ROWS * CH + 2 * PAD
    head = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" '
        f'role="img" aria-label="hyprscope\'s front page: Hyprland, Config, Windows, Monitors, Events and Log panels">\n'
        f"<style>{''.join(css)}</style>\n"
        f'<rect width="{width}" height="{height}" rx="8" fill="{hexcolour(BACKGROUND)}"/>\n'
        f'<g transform="translate({PAD} {PAD})">\n'
    )
    return head + "\n".join(out) + "\n</g>\n</svg>\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--binary", default=os.path.join(ROOT, "target/release/hyprscope"))
    parser.add_argument("--output", default=os.path.join(ROOT, "docs/screenshot.svg"))
    parser.add_argument("--seconds", type=float, default=75, help="how long hyprscope runs before the capture")
    parser.add_argument("--text", help="write the captured screen (with escapes) to this file too")
    args = parser.parse_args()

    if not os.path.exists(args.binary):
        subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    base = tempfile.mkdtemp(prefix="hs-", dir="/tmp")
    standin = None
    try:
        standin = Standin(base, args.seconds)
        standin.start_log_writer()
        standin.start_gpu_writer()
        screen = capture(args.binary, base, args.seconds, standin.child.pid, standin.fdinfo)
    finally:
        if standin:
            standin.close()
        subprocess.run(["tmux", "-S", os.path.join(base, "tmux.sock"), "kill-server"], capture_output=True)
        shutil.rmtree(base, ignore_errors=True)
    if args.text:
        with open(args.text, "w") as f:
            f.write(screen)
    # The process ID of the stand-in is real, if short-lived; show an invented one of the
    # same width, so the picture holds no process ID from this machine.
    screen = re.sub(r"(pid (?:\x1b\[[0-9;]*m)*)(\d+)", lambda m: m.group(1) + ("2417" * 3)[:len(m.group(2))], screen)
    svg = to_svg(parse_screen(screen))
    with open(args.output, "w") as f:
        f.write(svg)
    print(f"wrote {args.output} ({len(svg)} bytes)")


if __name__ == "__main__":
    main()
