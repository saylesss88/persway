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

Persway works with the Sway Compositor, it persuades it to do little evil
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
cargo install --git https://github.com/saylesss88/persway
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

2. **Key Bindings (Optional)**

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
