<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
    <img src="assets/wordmark-light.svg" alt="allock" height="140">
  </picture>
</h1>
<p align="center">A lock screen and greetd greeter for Wayland.</p>
<p align="center">
    <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue"></a>
</p>

> [!NOTE]
> This project has been developed with the assistance of AI.

## ✨ Features

- Lock screen through `ext-session-lock-v1`, one surface per monitor, hotplug included
- greetd greeter with a user picker and a session picker
- Password through PAM, fingerprint through fprintd (lock screen only)
- Wallpaper with blur and dim, frosted cards, and a clock
- Battery indicator
- Multi-monitor: choose which monitors show the controls
- Remembers the last user and session in the greeter
- Preview mode to try a theme without locking

## 🛠️ Install

```sh
scripts/install.sh             # lock screen
scripts/install.sh --greeter   # lock screen and greeter
```

The script builds allock, installs it to `/usr/local/bin` and installs the PAM
policy `pam/allock` to `/etc/pam.d/allock`. Icons need the `Symbols Nerd Font`.

See [docs/SETUP.md](docs/SETUP.md) to use it as your lock screen and as your
login screen.

## ⚙️ Configuration

allock works without a config file. To change it, copy
[`config.example.toml`](config.example.toml) to `~/.config/allock/config.toml`:
every key is optional and shown with its default.

The layout is fixed. The config chooses colours, wallpaper, clock format, which
indicators appear and which monitors show the controls.

## 🔒 Lock screen

```sh
allock             # lock
allock --preview   # draw the lock screen without locking, exits on a correct password
```

Bind `allock` to a key in your compositor, and to your idle daemon's lock and
lid actions.

## 🚪 Greeter

`allock --greeter` runs inside a compositor started by greetd. With
`scripts/install.sh --greeter` the setup is:

```toml
# /etc/greetd/config.toml
[default_session]
command = "systemd-cat -t allock-greeter niri -c /etc/greetd/allock-niri.kdl"
user = "greeter"
```

The greeter's own options go in `/etc/greetd/allock.toml` under `[greeter]`.
[docs/SETUP.md](docs/SETUP.md) has the details.

## 📄 License

GPL-3.0-or-later. See [LICENSE](LICENSE).
