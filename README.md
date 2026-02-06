[![Nix Flake](https://img.shields.io/badge/Nix_Flake-Geared-dddd00?logo=nixos&logoColor=white)](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake.html)

[![Nix](https://img.shields.io/badge/Nix-5277C3?style=flat&logo=nixos&logoColor=white)](https://nixos.org)

# Persway-Tokio

[Original Video Demo](https://user-images.githubusercontent.com/28332/223278211-ba3943ee-becc-45e5-ae0e-4f1a121a6f17.mp4)

> _Parental Advisory - Explicit Content. Unmute the video for the full
> experience._

**Persway-Tokio** is a modernized fork of
[Persway](https://github.com/johnae/persway), a Sway IPC daemon.

This version (v0.7.0+) replaces the `async-std` runtime with **Tokio**,
resolving long-standing dependency conflicts, modernizing the codebase, and
ensuring compatibility with the latest Rust ecosystem. It serves as a drop-in
replacement for the original Persway v0.6.2.

---

Persway works with the Sway Compositor, it persuades it to do little "evil"
things. It features window focus handlers that can be used to adjust the opacity
of focused and non-focused windows among many other things. Persway currently
supports two layouts: `spiral` and `stack_main`.

- **Spiral**: Alternates between horizontal and vertical splits based on window
  geometry.
- **Stack Main**: Keeps a stack of windows on the side of a larger main area
  (sometimes referred to as master-stack).

Persway talks to itself through a socket and listens to sway events through the
sway socket making it a flexible tool for manipulating the Sway Compositor.

---

## Installation

### From Source (Cargo)

```bash
# Latest Changes
cargo install --git https://github.com/saylesss88/persway
# crates.io
cargo install persway-tokio
```

---

## Nix Flake

Nix Flake If you are on NixOS or use the Nix Package Manager with flakes
enabled, you can use the flake directly from this repository:

```nix
inputs.persway.url = "github:saylesss88/persway";
```

And in `environment.systemPackages`:

```nix
environment.systemPackages = [
  inputs.persway.packages.{pkgs.stdenv.hostPlatform.system}.default
];
```

- Pass `inputs` through `specialArgs` in your `flake.nix`

---

## Setup & Configuration

To set up Persway, you need to run the daemon using the `persway daemon`
subcommand. Once the daemon is running, you can use the client portion of
Persway to communicate with it (e.g., binding keys to layout movement).

1. **Start the Daemon**

Add this to your sway config or autostart script:

```bash
# Example: Auto-renaming workspaces, handling opacity focus, and marking windows
exec persway daemon \
  -w \
  -e '[tiling] opacity 1' \
  -f '[tiling] opacity 0.95; opacity 1' \
  -l 'mark --add _prev' \
  --default-layout spiral
```

**Or for the Master Stack layout**:

```bash
# Example: Auto-renaming workspaces, handling opacity focus, and marking windows
exec persway daemon \
  -w \
  -e '[tiling] opacity 1' \
  -f '[tiling] opacity 0.95; opacity 1' \
  -l 'mark --add _prev' \
  --default-layout stack_main
```

**Declarative Setup for NixOS**

```nix
# sway.nix
startup = [
  {
    command = "exec persway daemon -w -e '[tiling] opacity 1' -f '[tiling] opacity 0.95; opacity 1' -l 'mark --add _prev' --default-layout spiral";
  }
];
```

- You can set this and forget it and you will have spiral spawning windows
  (First spawn is vertical, next is horizontal), and different shading for
  focused and unfocused windows.

You can also use an `exec` if you prefer.

## Key Bindings (Optional)

> Stack-\* commands only do something when the current workspace is in **Stack
> Main** layout; otherwise Persway will return an error.

| Command                                | Works in Spiral | Works in Stack Main | What it does                                                        |
| :------------------------------------- | :-------------: | :-----------------: | :------------------------------------------------------------------ |
| `persway change-layout spiral`         |       Yes       |         Yes         | Sets the focused workspace’s layout preference to Spiral.           |
| `persway change-layout stack-main ...` |       Yes       |         Yes         | Sets the focused workspace’s layout preference to Stack Main.       |
| `persway change-layout manual`         |       Yes       |         Yes         | Sets focused workspace to “manual” (Persway stops rearranging).     |
| `persway stack-focus-next/prev`        |       No        |         Yes         | Changes focus within the stack of the main‑stack layout.            |
| `persway stack-swap-main`              |       No        |         Yes         | Swaps the focused stacked window with the main window.              |
| `persway stack-main-rotate-next/prev`  |       No        |         Yes         | Rotates the main area and the stack (e.g., brings top of stack in). |

2. **Key Bindings**

Bind keys to control layout and focus. Add these to your
`~/.config/sway/config`:

```text
# Layout Rotation & Focus
bindsym Mod4+Control+space exec persway stack-main-rotate-next
bindsym Mod4+Shift+Tab     exec persway stack-focus-prev
bindsym Mod4+Tab           exec persway stack-focus-next
bindsym Mod4+space         exec persway stack-swap-main

# Switching Layouts
bindsym Mod4+c exec persway change-layout stack-main --size 70 --stack-layout tiled
bindsym Mod4+v exec persway change-layout manual
bindsym Mod4+x exec persway change-layout stack-main --size 70
bindsym Mod4+z exec persway change-layout spiral
```

These key bindings are **optional**. If you want to rely purely on
`persway change-layout` interactively, you can skip them. They're mainly useful
if you want to integrate Persway into your Sway key-chords or mouse bindings.

---

**CLI Reference**

Main Command

```text
Usage: persway [OPTIONS] <COMMAND>

Commands:
  daemon                  Starts the persway daemon
  stack-focus-next        Focuses the next stacked window (stack_main)
  stack-focus-prev        Focuses the previous stacked window (stack_main)
  stack-swap-main         Swaps the current stacked window with the main window
  stack-main-rotate-next  Pops top of stack into main, pushes old main to bottom
  change-layout           Changes the layout of the focused workspace
  help                    Print help

Options:
  -s, --socket-path <PATH>  Path to control socket. Defaults to XDG_RUNTIME_DIR
  -h, --help                Print help
  -V, --version             Print version
```

---

**Daemon Options**

```text
Usage: persway daemon [OPTIONS]

Options:
  -d, --default-layout <LAYOUT>
          Default layout (manual, spiral, stack_main) [default: manual]

  -w, --workspace-renaming
          Enable automatic workspace renaming (e.g. based on app name)

  -f, --on-window-focus <CMD>
          Sway command to run when window gains focus.
          Example: '[tiling] opacity 0.8; opacity 1'

  -l, --on-window-focus-leave <CMD>
          Sway command to run when window loses focus.
          Example: 'mark --add _prev'

  -e, --on-exit <CMD>
          Sway command to run when persway exits (cleanup).
          Example: '[tiling] opacity 1'
```

---

**Change Layout**

```text
Usage: persway change-layout <COMMAND>

Commands:
  spiral      Spiral autotiling (Golden Ratio / Fibonacci style)
  stack-main  Master-Stack layout
  manual      Standard Sway manual tiling
```

---

**Stack Main Options**

```text
Usage: persway change-layout stack-main [OPTIONS]

Options:
  -s, --size <PERCENT>      Size of the main area [default: 70]
  -l, --stack-layout <TYPE> Layout of the stack: tabbed, tiled, stacked [default: stacked]
```

---

## License

Persway-Tokio is released under the
[MIT license](https://github.com/saylesss88/persway/blob/main/LICENSE).
