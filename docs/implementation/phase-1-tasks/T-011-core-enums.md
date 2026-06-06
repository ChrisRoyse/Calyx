# T-011 — calyx-core: enums

**Phase:** PH03 · **Dep:** T-008 · **Sudo:** no

## Objective
The closed, typed vocabulary of modalities, slot shapes, quant policies, anchor
kinds, and states — the shared enums every engine matches on.

## Preconditions
- T-008 (workspace).

## Steps
1. `crates/calyx-core/src/enums.rs` (split if >500 lines):
   - `Modality { Text, Code, Image, Audio, Video, Structured, Mixed }`
   - `SlotShape { Dense(u32), Sparse(u32), Multi { token_dim: u32 } }`
   - `Asymmetry { None, Dual { a: SlotId, b: SlotId } }`
   - `QuantPolicy { None, Pq{m,nbits}, Float8, Binary }` (TurboQuant added in
     Stage 2 as a runtime policy; keep this enum aligned)
   - `AnchorKind { TestPass, TieFormed, Thumbs, Label(String), Reward,
     SpeakerMatch, StyleHold, Recurrence }` (incl. the identity + recurrence
     kinds, PRD `18 §2`/`26 §9`)
   - `SlotState { Active, Parked, Retired }`
   - `AbsentReason` (for explicit `SlotVector::Absent` — never zero-fill, A16)
2. Derive serde + `PartialEq`/`Eq`/`Hash` where needed; stable serialization.
3. Round-trip tests for each enum; exhaustive-match test to lock the variant set.

## Deliverables
- `enums.rs` with all core enums + round-trip tests.

## FSV gate
`cargo test -p calyx-core` green; serde round-trip is byte-stable for each enum
(assert on bytes); the `AnchorKind` set includes `SpeakerMatch`/`StyleHold`/
`Recurrence` (matched against PRD `18 §2`).

## Done
Enums implemented, serialization stable, variant sets locked by test.

## Refs
PRD `03 §3/§4`, `18 §2`, A16, `26 §9`.
