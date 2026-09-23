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
scripts/install.sh             # binary and PAM policy
scripts/install.sh --greeter   # and the greeter's config, wallpaper and niri config
```

It builds, installs `allock` to `/usr/local/bin` and `pam/allock` to
`/etc/pam.d/allock` (recommended, see below). [docs/SETUP.md](docs/SETUP.md)
covers the rest: binding the lock screen, idle and lid, and making it your
greetd greeter.

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

### Which monitor gets the controls

Every monitor is always covered and always shows the background — the
session-lock protocol requires the surface, and a screen left uncovered while
the session is locked is a screen showing what you locked. What `main_output`
decides is which of them draw the clock, the login card and the status row on
top of it:

```toml
main_output = "All"                    # every screen (the default)
main_output = { Targets = ["DP-2"] }   # connector name, or an EDID substring
```

Name more than one and each match mirrors the form, sharing the same input
buffer — whichever screen holds the keyboard is the one typing. Name a monitor
that is not connected, or unplug it while the screen is locked, and the
controls move to the first output rather than staying on a screen that is no
longer there: a password field nobody can see is a session nobody can unlock.
It moves back when the monitor returns.

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

The password field takes keystrokes the instant the screen appears — it asks for
the keyboard itself, so there is nothing to click first and no cursor to find on
a surface with no pointer. `Esc` clears what you typed; pressing it on an empty
field restarts the PAM conversation, which is the way out of a stack stuck
waiting on something other than the keyboard.

It draws no caret. On a masked field every character looks the same and you are
always at the end, so the caret says nothing — and it would be the one thing a
still lock screen redraws on its own, twice a second, all night. What says where
the keys are going is the border: it lights up while the field holds the
keyboard, and steps aside for the colour of a failed attempt, which has more to
say. With two monitors both drawing the form — the default `main_output`, see
[above](#which-monitor-gets-the-controls) — that is the whole answer to which
screen is listening: both draw it, one has the focus.

Pasting *into* the field works; nothing can copy out of it. Blocking paste is
what banking sites are mocked for — it stops password managers, not attackers —
while copy, cut and the primary selection are refused, because a lock screen is
exactly where a password must not end up in a buffer that outlives the unlock.

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

A refused attempt also shakes the card — a small rotation, about a third of a
second, decaying. Both backends count towards it: a wrong password and a finger
that was read and not recognised are the same news to the screen.

Every scan answers. A finger that is read and not recognised says so and
counts down the tries left — `[auth] fingerprint_retries`, five by default —
and when they run out the line says the reader has stopped rather than going
quiet, because a screen that says nothing looks like a broken sensor.

## Greeter mode

Point greetd at it in `/etc/greetd/config.toml`, inside a compositor:

```toml
[default_session]
command = "systemd-cat -t allock-greeter niri -c /etc/greetd/allock-niri.kdl"
user = "greeter"
```

`systemd-cat` keeps niri's log off the console, where greetd would otherwise
print it on the greeter's VT.

…with the greeter compositor spawning `allock --greeter -c /etc/greetd/allock.toml`
and quitting once allock exits: greetd starts the session only after the
greeter process — the compositor — has terminated, and allock exits as soon as
greetd accepts the session. allock runs as the `greeter` user there, so it
cannot read your home — give it its own config with `-c`.
`scripts/install.sh --greeter` writes both files; see
[docs/SETUP.md](docs/SETUP.md).

greetd runs the PAM stack itself, so the greeter always requires the password:
a fingerprint match would prove identity without producing a session greetd
would accept.

### What it remembers

The account is chosen in a user picker, like the session: the accounts people
log in to (a uid in `UID_MIN..UID_MAX` and a login shell), then "Other…", which
asks for the name. So the password field only ever receives a password — typed
into a user-name field, a password ends up in the system journal as a "bad
username". `[greeter] other_user = false` drops "Other…" where every account is
listed; with nobody listed the greeter asks for the name regardless.

After a successful login the greeter notes the account and the session in
`$XDG_CACHE_HOME/allock/last-login.toml` (the `greeter` user's own cache, since
that is who it runs as) and starts there next time. Both are suggestions the
pickers can change, so a greeter can never remember its way into keeping
somebody else out. The session is stored by name, so installing a new one does
not shift the choice. `[greeter] default_session = "Niri"` preselects a session
every time instead, and `[greeter] user` an account; `remember = false` leaves
nothing on disk to read or to write.

The session picker is not drawn with only one session: there is nothing to
choose.

## What it shows

User (GECOS name; `~/.face` avatar if present, or
`/var/lib/AccountsService/icons/<user>` where the greeter cannot read your
home), clock and date, the password
field with the backend's own prompt, why the last attempt failed, the
fingerprint hint, and the battery. In greeter mode, a session picker.

A ⇪ appears inside the field while caps lock is on. It comes from the latched
modifier state rather than from a key event, so it is already there when the
screen appears with caps lock on — not one keystroke later.

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

### A clock made of glass

The clock is the only text sitting directly on the wallpaper, and it asks for
the same effect with the shape of its own glyphs: the numerals are a hole onto
the picture behind them, blurred. Where a card frosts a box, the clock frosts
the letters.

The hour has no fill left at all — an opaque one would hide the very thing
being blurred — so what draws its edge is a black contour rather than its
colour, and the date keeps a little tint because at its size a pure hole reads
as a smudge. Without a wallpaper none of it applies: a blur of a flat colour is
that same flat colour, and the clock goes back to being ordinary text.

### Known limitations

- **The password is not scrubbed from memory.** It lives in a `String` — guido's
  text input, and the signal behind it — that reallocates as you type, and PAM
  copies it again. Doing this properly needs a zeroing buffer down in guido's
  input handling.

## Licence

GPL-3.0-or-later.
