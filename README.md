# allock

A lock screen and greetd greeter for [ashell](https://github.com/MalpenZibo/ashell)
users, built on [guido](https://github.com/MalpenZibo/guido).

One binary, two modes:

- **lock** (default) — takes an `ext-session-lock-v1` lock, puts a surface on
  every output, authenticates through PAM, and exits once the compositor
  confirms the session is unlocked.
- **greeter** (`--greeter`) — a fullscreen overlay layer surface per output
  that authenticates through greetd and starts the chosen session.

Unlike hyprlock, the layout is not yours to rearrange. allock decides where
things go; the config picks colours, wallpaper, formats and which indicators
appear. That is ashell's bargain, kept — and it is why the `[appearance]`
table is *imported from ashell's own config*, so retheming ashell retints the
lock screen with it.

## Install

```sh
cargo build --release
install -Dm755 target/release/allock /usr/local/bin/allock
install -Dm644 pam/allock /etc/pam.d/allock   # recommended, see below
```

Icons come from a system `Symbols Nerd Font`; without one they render as
tofu, but nothing stops working.

### About the PAM policy

Every locker needs one — with no `/etc/pam.d/<service>`, PAM falls back to the
`other` policy, which is `pam_deny` on essentially every distribution, so the
screen would lock and then reject every password. You have never had to think
about this because the *packages* ship it: `/etc/pam.d/hyprlock` belongs to
the hyprlock package, `/etc/pam.d/greetd` to greetd.

allock is usually built from source, so if it finds no policy of its own it
borrows a stock one — `system-auth`, `common-auth`, then `login` — and logs a
warning saying so. It refuses to lock only when none of those exist either.
That keeps `cargo build && allock` honest without requiring root first.

Installing `pam/allock` is still worth it, because it is the only way to
choose the stack deliberately. It includes `system-auth` rather than `login`,
which is what hyprlock and swaylock use. Their chain (`login` →
`system-local-login` → `system-login` → `system-auth`) drags in `pam_shells`
and `pam_nologin`: an `/etc/nologin` file, or a shell missing from
`/etc/shells`, then makes the screen impossible to unlock. Those are
login-on-a-tty checks — a lock screen is re-proving the identity of a session
that is already running, so it has no business enforcing whether that session
should have been allowed to start. What is left is what matters: `pam_unix`
for the password and `pam_faillock` to rate-limit guessing.

## Configure

Copy `config.example.toml` to `~/.config/allock/config.toml`. Every key is
optional. The `[appearance]` table is read from
`~/.config/ashell/config.toml` and merged key by key with any `[appearance]`
you put in allock's own file, so overriding one colour keeps the rest.

## Lock mode

```sh
allock
```

Bind it in your compositor. Under niri:

```kdl
binds {
    Mod+Shift+L { spawn "allock"; }
}
```

### Trying it out first

```sh
allock --preview
```

Draws the same screen without taking the lock, with the real PAM conversation
behind it, and exits when the password is accepted instead of unlocking. It
takes keyboard focus on demand rather than exclusively, so it can never trap
your keyboard — kill it whenever. Use it for theming: iterating on a lock
screen by locking the session you are working in is how you end up at a TTY,
because a lock client that dies while holding the lock leaves the compositor
locked with nobody to unlock it. The protocol guarantees that on purpose.

The password field takes keystrokes the instant the screen appears — there is
no field to click and no cursor to find. `Esc` clears what you typed; pressing
it on an empty field restarts the PAM conversation, which is the way out of a
stack stuck waiting on something other than the keyboard.

### Fingerprint

With `[auth] fingerprint = true` (the default), allock verifies against
fprintd *alongside* the password rather than through `pam_fprintd`. Driving
it separately is what keeps the password field usable while the reader waits
— PAM's single conversation channel cannot do both at once.

The trade is explicit: a `verify-match` from fprintd is accepted as proof of
identity, the same trust model hyprlock and the GNOME/KDE lock screens use.
Set `fingerprint = false` to require the password. No reader, no enrolled
finger, or no fprintd all mean the same thing — the password is the only way
in, and the hint disappears.

## Greeter mode

Point greetd at it in `/etc/greetd/config.toml`, inside a compositor:

```toml
[default_session]
command = "niri --config /etc/greetd/niri.conf"
user = "greeter"
```

…with the greeter compositor spawning `allock --greeter -c /etc/greetd/allock.toml`.
allock runs as the `greeter` user there, so it cannot read your home — give it
its own config with `-c`.

greetd runs the PAM stack itself, so the greeter always requires the password:
a fingerprint match would prove identity without producing a session greetd
would accept.

## What it shows

User (GECOS name, `~/.face` avatar if present), clock and date, the password
field with the backend's own prompt, why the last attempt failed, the
fingerprint hint, and the battery. In greeter mode, a session picker.

## Notes on the internals

`src/services/` and `src/components/icons.rs` are copied verbatim from the
ashell/guido port. They are written against iced-shaped service traits that
`services/compat.rs` mimics; when ashell publishes them as a crate this
becomes a `use` line. Do not refactor them in place — divergence is what
would make that swap expensive.

Authentication is modelled as one conversation (`src/auth/`): PAM, greetd and
fprintd all reduce to a stream of `AuthEvent` driven by `AuthCommand`, which
is why the lock screen and the greeter share a UI and a state machine.
`authenticated` is set in exactly one place, on `AuthEvent::Success` — a
backend that crashes, a channel that closes or a daemon that vanishes all
leave the screen shut.

### Frosted cards

The cards show a blurred cut-out of the wallpaper behind them. guido does the
work — `container().backdrop_blur(radius)` can filter both what the surface
drew and what the compositor composites below it, and allock asks for the
first source only.

That is not a limitation, it is the whole shape of the problem:
`ext-background-effect-v1` blurs what the *compositor* has under the surface,
and a session lock surface has nothing under it — the protocol requires
everything beneath to be blanked. The wallpaper is drawn by allock, inside its
own surface, so it is guido's in-surface pass that softens it, and asking the
compositor for a blur region as well would be a request that can never do
anything.

Set `[background] blur = 0` for plain translucent cards, and `card_opacity` for
how much tint covers the blur.

### Known limitations

- **No caps-lock indicator.** guido's `Modifiers` exposes ctrl/alt/shift/logo
  but not the lock group, so there is nothing to read. A one-field addition
  to guido would enable it.
- **The password is not scrubbed from memory.** It lives in a `String` that
  reallocates as you type, and PAM copies it again. Doing this properly needs
  a zeroing buffer down in guido's input handling.
- The greeter does not remember the last user or session between boots.

## Licence

GPL-3.0-or-later.
