# The handoff with a bar

allock and a bar that draws an island (ashell) are two processes, and the
compositor switches between them with a cut no client can animate: the session
disappears when the lock takes the screen, and reappears in one frame when it is
undone. The handoff puts one thing in the same place on both sides of the cut:
the **seed**, the island's smallest state, a dot of `[island.seed] size` logical
pixels whose top is `[island.seed] top` pixels below the output's top edge, centred
horizontally. Both processes read the same `[island.seed]` table, so the dot is
the same pixels in each.

allock works without a bar. Everything below is what changes when one is there.

## The interface

The bar owns `org.ashell.Island` on the **session bus** and exports
`/org/ashell/Island` with:

```
interface org.ashell.Island1

method PrepareHandoff(s reason) -> (b folded)
method ResumeAfterHandoff()
```

`PrepareHandoff("lock")` folds the island into the seed: whatever is open
closes, toasts and OSDs are queued, and the island springs to the seed. The
reply is sent **once the island is the seed** on screen, about 400 ms with
ashell's island spring. `folded` is `false` when the bar will not fold (it is
already the seed, or does not want to); allock locks either way.

`ResumeAfterHandoff()` lets the island grow back out of the seed, and show
what was queued.

Nothing else travels over it: never a credential, an attempt or a verdict.

## A lock

```
allock                                        bar
  NameHasOwner(org.ashell.Island)? ───────────▶ (bus)
  PrepareHandoff("lock") ─────────────────────▶ folds into the seed (~400 ms)
                         ◀───────────────────── folded
  [snapshot of every output, if configured]
  ext-session-lock: lock, first frame = seed     stays the seed
  SetLockedHint(true) on logind
  …                                              …
  card folds into the seed
  unlock_and_destroy                             (the session reappears: the
                                                  bar is the seed it was left)
  SetLockedHint(false) on logind
  ResumeAfterHandoff() ───────────────────────▶ grows back
  exit
```

## Without a bar, or with a slow one

| Case | allock |
|---|---|
| Nobody owns `org.ashell.Island` | Locks at once: one round trip to the bus, no wait. The island **pops out** of the seed instead of starting as a dot, and at the end **shrinks away** into it instead of leaving a dot at the cut. |
| The bar does not answer within `[handoff] timeout_ms` (600) | Locks anyway: the lock is never held back for an animation. |
| `[handoff] enabled = false` | Never asks; behaves as without a bar. |
| The bar appears or quits while locked | Followed through `NameOwnerChanged`: at the unlock the seed is left only if a bar is there to take it. |
| `ResumeAfterHandoff` is lost | The bar also grows back on `LockedHint = false` (below). |

## What the bar does on its own

- **Starts while the session is locked** (a crash, a restart): reads logind's
  `LockedHint` for its session; if `true`, it comes up as the seed and waits for
  `ResumeAfterHandoff` or `LockedHint = false`.
- **`LockedHint` turns `false`**: grows back, as `ResumeAfterHandoff` would.
- **A lock that did not go through allock's call** (another locker, or one
  started before the bar was on the bus): the bar may fold on logind's `Lock`
  signal by itself.
- **While it is the seed for a lock**, notifications, OSDs and reshapes are
  queued, not shown; they come out after the island has grown back.

## The greeter

The greeter and the session run in two compositors, and greetd switches between
them; there is no bus between the two. The greeter folds into the seed over the
wallpaper, sharpened, and the session's bar starts as the seed and grows once
its services answer, so the only cut is the compositor switch. The greeter
always leaves its seed, since it cannot know what the session will start.

## Shared values

| Value | Where |
|---|---|
| Seed size and position | `[island.seed]` in each one's config (same keys, same defaults: 30 and 6) |
| The spring every fold and growth uses | `ashell_ui::motion::ISLAND_SPRING` (mass 1, stiffness 230, damping 21) |
| How long a fold takes | ~400 ms, from that spring |
