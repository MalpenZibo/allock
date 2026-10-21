# The handoff with a bar

allock and a bar that draws an island (ashell) are two processes, and the
compositor switches between them with a cut no client can animate: the session
disappears when the lock takes the screen, and reappears in one frame when it is
undone. The handoff makes sure no island is on screen at the cut. The bar's
island goes up past the output's top edge before the lock, and allock's drops in
from that edge once the lock is drawn; at the unlock allock's goes back up, and
the bar's drops back in. The eye sees one island leave into the edge and one
come out of it, never a shape that jumps.

Both islands go up as the **seed**, the dot of `[island.seed] size` logical
pixels, and rest `[island.seed] top` pixels below the edge. The two configs share
the keys but need not agree: neither island is on screen at the cut.

allock works without a bar. Everything below is what changes when one is there.

## The interface

The bar owns `org.ashell.Island` on the **session bus** and exports
`/org/ashell/Island` with:

```
interface org.ashell.Island1

method PrepareHandoff(s reason) -> (b gone)
method ResumeAfterHandoff()
```

`PrepareHandoff("lock")` sends the island away: whatever is open closes,
toasts and OSDs are queued, and the island shrinks into the seed as it springs
up past the top edge. The reply is sent **once the island is off screen**, about
400 ms with ashell's island spring. `gone` is `false` when the bar will not go
(it is already away, or does not want to); allock locks either way.

`ResumeAfterHandoff()` lets the island drop back in, and show what was queued.

Nothing else travels over it: never a credential, an attempt or a verdict.

## A lock

```
allock                                        bar
  NameHasOwner(org.ashell.Island)? ───────────▶ (bus)
  PrepareHandoff("lock") ─────────────────────▶ goes up past the edge (~400 ms)
                         ◀───────────────────── gone
  [snapshot of every output, if configured:
   no island in it]
  ext-session-lock: lock, first frame = the     stays away
  background alone
  SetLockedHint(true) on logind
  the island drops in from the edge
  …                                              …
  the card goes up into the edge
  unlock_and_destroy                             (the session reappears, with
                                                  no island in it)
  SetLockedHint(false) on logind
  ResumeAfterHandoff() ───────────────────────▶ drops back in
  exit
```

## Without a bar, or with a slow one

| Case | allock |
|---|---|
| Nobody owns `org.ashell.Island` | Locks at once: one round trip to the bus, no wait. The island comes and goes as it does with a bar. |
| The bar does not answer within `[handoff] timeout_ms` (600) | Locks anyway: the lock is never held back for an animation. Its island may then be in the snapshot. |
| `[handoff] enabled = false` | Never asks; behaves as without a bar. |
| `ResumeAfterHandoff` is lost | The bar also drops back in on `LockedHint = false` (below). |

## What the bar does on its own

- **Starts**: comes up away, past the top edge, and drops in once its services
  answer.
- **Starts while the session is locked** (a crash, a restart): reads logind's
  `LockedHint` for its session; if `true`, it stays away and waits for
  `ResumeAfterHandoff` or `LockedHint = false`.
- **`LockedHint` turns `false`**: drops back in, as `ResumeAfterHandoff` would.
- **A lock that did not go through allock's call** (another locker, or one
  started before the bar was on the bus): the bar may go on logind's `Lock`
  signal by itself.
- **While it is away for a lock**, notifications, OSDs and reshapes are queued,
  not shown; they come out after the island has dropped back in.

## The greeter

The greeter and the session run in two compositors, and greetd switches between
them; there is no bus between the two. The greeter's island goes up into the
edge over the wallpaper, sharpened, and the session's bar starts away and drops
in once its services answer, so the only cut is the compositor switch, with no
island on either side of it.

## Shared values

| Value | Where |
|---|---|
| Where each island rests, and the seed it goes as | `[island.seed]` in each one's config (same keys, same defaults: 30 and 6; they need not match) |
| The spring every fold and growth uses | `ashell_ds::motion::ISLAND_SPRING` (mass 1, stiffness 230, damping 21) |
| How long going away takes | ~400 ms, from that spring |
