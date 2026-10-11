# Changelog

All notable changes to hyprscope are listed here, newest first.

## Unreleased

## 0.3.0 - 2026-10-11

- hyprscope is on crates.io: `cargo install hyprscope`.
- Twenty-six colour themes: `--theme NAME` starts with one showing, and `t` and `T` step through them while it runs. The name shows in the top border, and the `terminal` theme uses your terminal's own colours.
- The repository is public, and pull requests are merged directly.

## 0.2.1 - 2026-10-08

- The README describes the six full-screen views, and its picture shows the front page as it now is.

## 0.2.0 - 2026-10-08

- Enter opens the Windows panel full-screen with an inspector for the selected window: its state, every property Hyprland reports for it, and the process behind it.
- `1` opens the Hyprland panel full-screen: tall history graphs for the compositor, and a table of what each application with windows costs, child processes included. Panels with a zoomed view show their number.
- `4` opens the Monitors panel full-screen: a map of the monitors with their windows, and for the selected one its details, whether VRR, solitary, tearing and direct scanout are in effect (and why not, for all but VRR), its layer surfaces and its modes. Up and Down choose the monitor.
- `5` opens the Events panel full-screen: a graph of events per second, every one of the last 2,000 events on its own line with the newest at the top, and a count of each kind. Scrolling back pauses the view, Space pauses it, `/` filters the lines by text and `r` shows the events exactly as Hyprland sent them.
- `6` opens the Log panel full-screen: every warning and error Hyprland has logged, up to the newest 2,000, and the newest 5,000 lines of every other level, one per line with the newest at the top. `l` steps the lowest level shown, from warnings and up to errors only to every level; scrolling back pauses the view, Space pauses it and `/` filters the lines by text.
- `2` opens the Config panel full-screen, in four tabs that Tab and Shift+Tab move between: the options your configuration sets, with each value next to its default and Hyprland's description (`a` shows every option); your binds, grouped by submap; the animations, with every bezier curve drawn as a plot; and the input devices. `/` filters the open list by text, and the lists follow a reload within a second.
- `/` in the Windows view filters the tree by text: only the windows whose class or title contains it are shown, under their workspaces and monitors. The selected window stays selected while it is still shown, and otherwise the first match is.
- The contributing guide now says how a pull request reaches a release: it is applied by hand, then closed, and credited to its author in the changelog.

## 0.1.2 - 2026-10-07

- A README, with a generated picture of the front page.
- `--version` and `--help`; any other argument is now an error instead of being ignored.
- A contributing guide, and automatic checks for pull requests.

## 0.1.1 - 2026-10-07

- hyprscope is now under the MIT licence.

## 0.1.0 - 2026-10-07

First release: the front page.

- A live tree of monitors, workspaces and windows, with class, title, PID, size and
  position, and flags. Up and Down move a selection from window to window; the view
  follows it.
- A Hyprland panel showing the compositor's own CPU, memory and GPU use as sparklines,
  with its version, config provider, submap, process ID and uptime.
- A Monitors panel that draws each monitor where it sits in the layout, with its mode,
  scale, active workspace, and whether VRR, tearing and direct scanout are in effect.
- An Events panel that shows Hyprland's events as they happen, folds repeats into one
  line with a count, and graphs events per second.
- A Log panel with the warnings and errors from the compositor's log and their counts
  for the session.
- A Config panel with the config file Hyprland was started with, config errors, options
  set, binds and submaps, animations and curves, and the time of the last reload.
- The front page laid out as two columns over a full-width Log. It needs a terminal of
  at least 116 columns by 32 rows and says so when it is smaller.
