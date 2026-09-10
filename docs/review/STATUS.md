# Oxidize Hardening and Compatibility Status

Status legend:
- `[OPEN]`: Reproduced or identified, pending implementation.
- `[IN_PROGRESS]`: Currently being implemented and verified.
- `[FIXED]`: Narrow fix and regression tests passing.
- `[VERIFIED]`: Comprehensive gates and test suite passing.

## Phase 1 — Safety Baseline

| Finding | Severity | Description | Status |
|---|---|---|---|
| **F01** | P0 | Path traversal in checkout / tree materialization writes outside repo or into `.git` | [FIXED] |
| **F04** | P1 | Index and ref locks truncate existing locks (`File::create`); CAS outside lock | [FIXED] |
| **F02** | P1 | Checkout overwrites uncommitted local modifications and untracked files | [FIXED] |
| **F03** | P1 | `rm` ignores force check on modified files; recursive `rm` wipes untracked files | [FIXED] |
| **F05** | P1 | Mutating v4 index writes v2 records under v4 header, corrupting index | [FIXED] |
| **F11** | P1 | SSH argument injection, path parsing strips leading slash, spaces in executable | [FIXED] |
| **F13** | P1 | Delta parser panics on truncated copy instructions; pkt-line accepts truncated prefix | [FIXED] |

## Phase 2 — Storage & Operation Correctness

| Finding | Severity | Description | Status |
|---|---|---|---|
| **F08** | P1 | Essential commands fail on packed repos (`LooseObjectStore` used); `REF_DELTA` fails | [FIXED] |
| **F16** | P2 | Empty index commit rejected when deleting last file; repo discovery gaps (gitfile/bare) | [FIXED] |
| **F06** | P1 | Unresolved conflicts in index allowed in `write-tree` / `commit`; merge state missing | [FIXED] |
| **F14** | P1 | Tree merge treats binary data as UTF-8 text and mangles line endings/newlines | [FIXED] |
| **F07** | P1 | Stash pop/drop loses stack entries; TUI divergent stash implementation | [FIXED] |
| **F09** | P1 | Ref namespace traversal, unmerged branch deletion without check, packed ref shadows | [FIXED] |
| **F15** | P2 | Status ignores file mode changes; deletion staging missing; ignore bugs | [FIXED] |

## Phase 3 — Transport Correctness & Boundedness

| Finding | Severity | Description | Status |
|---|---|---|---|
| **F12** | P1 | Pkt-line reads to EOF deadlocking SSH; `SidebandDemuxer` contaminates pack with NAK | [FIXED] |
| **F10** | P1 | Push sends all objects instead of reachable closure; force overwrites remote/checked-out | [FIXED] |

## Phase 4 — Shared Architecture & TUI Robustness

| Finding | Severity | Description | Status |
|---|---|---|---|
| **F17** | P2 | TUI duplicates CLI logic; synchronous git execution; terminal state left dirty on panic | [FIXED] |

## Phase 5 — Quality, Support & MSRV

| Finding | Severity | Description | Status |
|---|---|---|---|
| **F18** | P2 | MSRV 1.80 claimed but dependencies require 1.85+; docs overclaim 100% parity & zero panic | [FIXED] |

## Phase 6 — Measured Performance

| Focus | Description | Status |
|---|---|---|
| Index & Rename | Bulk index sort/merge; OID-indexed rename pairing; bounded caches | [COMPLETED] |
