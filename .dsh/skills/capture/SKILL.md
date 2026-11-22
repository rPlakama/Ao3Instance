---
name: window-screenshot
description: Capture a window with niri to get a visual reference. Use when you need to see the current state of a UI, app, or terminal.
---

# Window screenshot (niri)

Capture the focused window:

    niri msg action screenshot-window

Capture a specific window:

    niri msg --json windows        # find the window id
    niri msg action screenshot-window --id <ID>

The image is copied to the clipboard and saved to niri's `screenshot-path` (default `~/Pictures/Screenshots/`). Find the newest file and view it:

    ls -t ~/Pictures/Screenshots/* | head -1

Run it after each UI change to verify the result. Use `--write-to-disk true` if nothing was saved.
