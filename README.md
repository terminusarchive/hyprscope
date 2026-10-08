# hyprscope

hyprscope is a terminal program that shows what a Hyprland session is doing. It puts
your windows, monitors, the compositor's own resource use, its event stream, its log
warnings and the size of your config on one screen, and updates them live. It is for
people who run Hyprland and want to see what it is up to without running several
commands.

![hyprscope's front page](docs/screenshot.svg)

The picture was made from invented content, not from a real desktop. Its GPU row shows
`n/a` because the stand-in process has no GPU counters; on a real session with a
supported driver it is a third sparkline.

## What it shows

The front page has six panels.

| Panel | What it shows |
| --- | --- |
| Windows | A tree of monitors, workspaces and windows. Each window shows its class, title and flags (`float`, `full`, `pin`, `xwl`); its PID, and then its size and position, are added as the panel gets wider (all of them from a terminal about 160 columns wide). One window is selected, and the tree scrolls to keep it in view. |
| Hyprland | The compositor process: PID and uptime; CPU, resident memory and GPU use as sparklines with the current value, and the peak when the panel is wide enough; the thread count and graphics memory. The title has Hyprland's version, its config provider and the current submap. |
| Config | A summary of the running configuration: whether it has errors (and the first one), how many options your config sets out of all that exist, binds and submaps, animations (and how many your config sets) and curves, and the time of the last reload, once hyprscope has seen one. The title shows the config file: the one given to Hyprland with `--config` or `-c`, otherwise `hyprland.lua` or `hyprland.conf` from your Hyprland config directory if it exists. |
| Monitors | Each monitor as a box placed where it sits in your layout: name, mode, scale, active workspace and special workspace, and whether VRR, tearing and direct scanout are in effect. When the boxes would be too small, or monitors overlap in the layout, it shows one line per monitor instead (name, mode, scale, workspace). |
| Events | Events from Hyprland's event socket, with the time each arrived. A new event folds into an existing line, raising its count (`×3`), when it is the same kind of event and, for events about a window, the same window. The line then shows the latest payload and time, and keeps its place, so the list is in order of first appearance, not of time. The title graphs events per second over the last 16 seconds. |
| Log | The newest warnings and errors from Hyprland's log, with a count for each distinct line and the totals in the title. |

## Requirements

- Linux, with a running Hyprland session. hyprscope reads Hyprland's IPC sockets and
  `/proc`, and the log file Hyprland writes. It was built and run on Hyprland 0.56.
  Other versions are untried.
- A terminal with truecolour support, at least 116 columns by 32 rows. hyprscope does not
  check for truecolour and always uses 24-bit colours, so a terminal without it shows
  wrong colours. In a smaller terminal it shows a message that says so.
- A Rust toolchain to build it: Rust 1.88 or newer; built with 1.98.
- For the GPU row: a driver that gives the kernel's per-process DRM counters. Intel
  (i915, xe) and AMD drivers do. With other drivers, including NVIDIA's proprietary
  one, the GPU row shows `n/a`. Only the Intel xe driver has been tried on real
  hardware.

hyprscope only reads from Hyprland. It never changes windows, workspaces or
configuration.

## Install

Straight from GitHub:

```sh
cargo install --git https://github.com/terminusarchive/hyprscope
```

This puts `hyprscope` in `~/.cargo/bin`.

Or from a clone:

```sh
git clone https://github.com/terminusarchive/hyprscope
cd hyprscope
./build.sh
```

`./build.sh` runs `cargo build --release` and prints where the binary is:
`target/release/hyprscope`. `./build.sh run` builds and then starts it.

## Run

Start `hyprscope` from a terminal inside your Hyprland session (it needs the
`XDG_RUNTIME_DIR` and `HYPRLAND_INSTANCE_SIGNATURE` variables that Hyprland sets).
These two variables are how hyprscope finds the Hyprland instance, so outside the
session (over SSH, from a service) both must be set. If either is missing, or
Hyprland's socket cannot be reached, it prints one line saying so and exits with
status 1. If Hyprland exits or stops answering while hyprscope runs, it restores the
terminal, prints one line and exits with status 1.

| Option | Action |
| --- | --- |
| `-V`, `--version` | Print the version and exit. |
| `-h`, `--help` | Print a short help text and exit. |

Any other argument is an error: hyprscope names it on standard error and exits with
status 2.

| Key | Action |
| --- | --- |
| `Up`, `k` | Select the previous window. From the first window it goes to the last. In the Monitors view it selects the previous monitor and stops at the first. In the Events and Log views it scrolls one line toward newer lines. In the Config view it scrolls the list one row up. |
| `Down`, `j` | Select the next window. From the last window it goes to the first. In the Monitors view it selects the next monitor and stops at the last. In the Events and Log views it scrolls one line toward older lines. In the Config view it scrolls the list one row down. |
| `PageUp` | Select the window one page up. It stops at the first window. In the Events and Log views it scrolls one page toward newer lines. In the Config view it scrolls the list one page up. |
| `PageDown` | Select the window one page down. It stops at the last window. In the Events and Log views it scrolls one page toward older lines. In the Config view it scrolls the list one page down. |
| `Enter` | On the front page, open the Windows view with the inspector. |
| `1`, `2`, `3`, `4`, `5`, `6` | Open the Hyprland, Config, Windows, Monitors, Events or Log view full-screen, from the front page or another view. |
| `Tab`, `Right` | In the Config view, open the next tab: Options, Binds, Animations, Devices. `Shift+Tab` and `Left` open the previous one. |
| `a` | In the Config view's Options tab, switch between the options your configuration sets and every option. |
| `Space` | In the Events and Log views, pause the list or resume it. |
| `/` | In the Windows, Events, Log and Config views, filter the list by text (in the Windows view, the tree by class or title): type, then `Enter` keeps the filter and `Esc` clears it. |
| `l` | In the Log view, step the lowest level shown: warnings and up, errors only, every level. |
| `r` | In the Events view, show the events as Hyprland sent them, and back. |
| `Home` | In the Events and Log views, go back to the newest line. In the Config view, go to the first row of the list. |
| `End` | In the Events and Log views, go to the oldest line, which pauses the view. In the Config view, go to the last row of the list. |
| `Esc` | From a full-screen view, back to the front page. In the Windows, Events, Log and Config views it first closes the filter prompt, or clears a kept filter. |
| `q`, `Ctrl-C` | Quit. |

## Planned 

- tbd

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

MIT. See [LICENSE](LICENSE).