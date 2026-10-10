# SOW 2.2.4 split: 2.2.4a / 2.2.4b

**Why:** 2.2.4 (de-SSA: phi elimination + liveness coalescing) was a very "heavy" task,
estimated at ~14h. Splitting the task makes it easier to manage and it also has two
separable halves: one needed for correctness, one that is cleanup. Splitting prevents
the objective from appearing too overwhelming at once.

Priority: High (SOW 2.2.4). Sprint: 2.

| ID | Title | Est. |
|---|---|---|
| 2.2.4a | Lower phi nodes to per-edge copies in flat HIR | ~8h |
| 2.2.4b | Coalesce phi/temporary variables via liveness | ~6h |

## 2.2.4a: phi lowering (correctness)

`-O0` IR still emits `phi` for `&&`, `||` and ternaries used as values, which the flat
HIR cannot currently round-trip.

**Scope**
- Lower each phi to a named local; emit a copy at the end of each predecessor block,
  just before its terminator.
- Handle swap / lost-copy cases (phis that read each other) by copying through temporaries.
- Emit through the existing `c_goto` backend; no new types.
- Unsupported phi types (pointer, aggregate) return a clean `Err`, not a panic.

## 2.2.4b: liveness coalescing (cleanup)

Reduces the noise 2.2.4a leaves behind: merge phi variables and temporaries whose live
ranges do not overlap, so emitted C has fewer copies and locals.

**Scope**
- Compute per-function liveness over the flat HIR.
- Merge non-interfering phi/temporary variables; drop copies that become `x = x`.
- Behavior must be unchanged: same round-trip results as before the pass.

## Notes

File can be removed upon merging with "master" or starting PR.