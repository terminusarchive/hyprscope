# Changelog

All notable changes to hyprscope are listed here, newest first.

## Unreleased

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
