# wooloo-sync protocol v1 (shared Space metadata)

Status: draft for M4. Reference implementation: `crates/wooloo-sync` (Rust).
Test vectors: `docs/sync-vectors.json` (every implementation, including a later
Swift one in the Mac app, must pass them).

## 1. Goal and non-goals

Each machine runs its own Herdr server and hosts its own Spaces. Both apps are
clients of both servers. Herdr already shares the live terminals; what it does
not share is wooloo's own metadata about Spaces. This protocol replicates that
metadata between machines, both ways, with offline edits merging on reconnect.

Replicated (and nothing else, see the allow-list in section 4):

- per Space: name, category pin, colour, sidebar order, notes, pinned tabs,
  hosting machine + Herdr workspace id, deleted flag;
- the unit-state categories file (`~/.local/state/unit-space-categories.json`,
  a flat `{ "<lowercased space name>": "<category>" }` map written by
  `unit-space-purpose.sh`).

Never replicated: secrets, keys, tokens, game art, terminal scrollback or
surfaces, file paths, environment, Herdr config. Anything not in the allow-list
is rejected on send AND on receive.

No new daemon, no cloud, no new network listener. Transport is the SSH
connection both apps already use for Herdr (over Tailscale).

## 2. Data model

The replicated document is a set of **registers**. A register is one field of
one key:

```json
{"key":"space:6f1c7a52-3b0e-4d8e-9a51-2a8f1c0d9e11","field":"name",
 "value":"Study Central","hlc":{"wall":1763112000000,"counter":0},"node":"mac-3f9a21c0"}
```

| member  | type | meaning |
|---|---|---|
| `key`   | string | entity, see section 4 |
| `field` | string | attribute of that entity, see section 4 |
| `value` | JSON   | the value; `null` means "cleared" (a field tombstone) |
| `hlc`   | object | `{"wall": <u64 ms since Unix epoch>, "counter": <u64>}` |
| `node`  | string | id of the machine that wrote this value |

An **op** on the wire is exactly a register. The document is a map
`(key, field) -> register`; it holds at most one register per pair.

### 2.1 Node id

Each machine picks a random node id once and stores it in its state file:
`<os>-<8 lowercase hex>`, e.g. `mac-3f9a21c0`, `win-07be55d1`. Allowed
pattern: 1-64 chars of `[a-z0-9-]`. It never contains a hostname, user name or
anything secret.

### 2.2 Hybrid logical clock (HLC)

Each node keeps `last = (wall, counter)`, persisted in its state file.

- **tick (local write)** with physical time `now` (ms):
  if `now > last.wall` then `last = (now, 0)` else `last.counter += 1`.
  The new `last` is the write's `hlc`.
- **observe (accepted remote op)** with remote `r`:
  if `(r.wall, r.counter) > (last.wall, last.counter)` then `last = r`.

This keeps every local write strictly after everything the node has seen, even
when its wall clock is behind a peer's.

`(wall, counter)` pairs compare lexicographically.

### 2.3 Last-writer-wins order

Two registers for the same `(key, field)` are ordered by, in turn:

1. `hlc.wall` (bigger wins)
2. `hlc.counter` (bigger wins)
3. `node`, compared as UTF-8 bytes (bigger wins)
4. the value's canonical JSON text, compared as UTF-8 bytes (bigger wins)

Step 4 only matters if one node reuses an HLC for two different values, which a
correct node never does; it exists so even a buggy peer cannot make replicas
diverge. Canonical JSON text: compact (no whitespace), object keys sorted,
strings with only `"`, `\\` and characters below U+0020 escaped (`\n`, `\t`,
`\r`, `\b`, `\f`, others as `\u00xx` lowercase hex), `/` NOT escaped, non-ASCII
as raw UTF-8, numbers in shortest round-trip form. Allowed values are only null,
booleans, numbers, strings and arrays of strings, so this is easy to produce.
The test vectors exercise step 4 with string values only.

### 2.4 Merge

`apply(op)` on a replica:

1. **validate** against the allow-list (section 4). Invalid: reject, do not
   store, do not observe its clock.
2. **skew guard**: if `op.hlc.wall > now + MAX_FUTURE_MS` (600000 ms = 10 min),
   reject it as `future`. It is deferred, not lost: the sender keeps it and
   resends it on every exchange until the receiver's clock catches up.
3. **observe** the op's hlc (section 2.2).
4. if there is no register for `(key, field)`, or the op is bigger in the order
   of 2.3, store the op. If it is equal, it is a duplicate (no-op). If smaller,
   ignore it.

This merge is commutative, associative and idempotent per register, so replicas
that have applied the same set of accepted ops hold identical documents in any
order (the convergence property the tests check).

## 3. Identity of Spaces across machines

Herdr workspace ids (`w3`, ...) are per server, so they cannot identify a Space
across machines. Every Space gets a **UUID** (lowercase v4, 8-4-4-4-12 hex),
and the document key is `space:<uuid>`.

The UUID is bound to the Herdr workspace through two replicated fields:
`host` (node id of the machine whose Herdr server runs the Space) and
`herdr_id` (the workspace id on that server).

Rules (this avoids two clients minting two UUIDs for one workspace):

- **Only the host mints.** A node creates `space:<uuid>` records only for
  workspaces of its own Herdr server.
- A client viewing a peer's server maps workspace `w` on peer node `P` to the
  live Space with `host == P` and `herdr_id == w`. If none exists yet (the host
  has not synced), the client shows the workspace with Herdr's label and no
  metadata, and does not write metadata for it until the mapping exists.
- **Host binding pass** (on start and whenever its workspace list changes),
  for each local workspace `(id, label)`:
  1. a live Space with `host == me` and `herdr_id == id`: bound;
  2. else a live Space with `host == me` whose `herdr_id` is not a current
     workspace and whose `name == label` (Herdr restarted and renumbered):
     rebind by writing `herdr_id`;
  3. else mint a new UUID and write `host`, `herdr_id`, `name`.
  Step 1 runs over all workspaces before step 2 starts. Workspaces are taken in
  Herdr's order, and when several Spaces qualify the one with the smallest UUID
  (byte order) is used. Each Space binds to at most one workspace.
  Live Spaces with `host == me` left unbound are returned as **orphans**; the
  app tombstones them once it knows the workspace was closed (not merely that
  Herdr is still starting).
- Herdr is the source of truth for a hosted Space's label. The host writes a
  `name` op when Herdr reports a rename, and renames the workspace in Herdr when
  a newer `name` arrives by sync.

## 4. Allow-list

Anything else is invalid. Lengths are in Unicode scalar values unless marked
bytes. "Text" means a string with no control characters (U+0000-U+001F,
U+007F).

### 4.1 `space:<uuid>`

| field | value |
|---|---|
| `name` | text, 1-200 |
| `category` | `null` or text, 1-64 (wooloo category pin, e.g. `claude`, `school`, `creative`, `finance`, `misc`; null = automatic) |
| `color` | `null` or `"#rrggbb"` (lowercase hex) |
| `order` | finite number (sidebar position; fractional values allowed so a move rewrites one field) |
| `notes` | `null` or string, at most 65536 UTF-8 bytes; the only control characters allowed are `\n`, `\r`, `\t` |
| `pinned_tabs` | `null` or array of at most 64 texts, each 1-200 (tab labels, not ids) |
| `host` | node id (2.1) |
| `herdr_id` | 1-64 chars of `[A-Za-z0-9_.:-]` |
| `deleted` | boolean (Space tombstone) |

### 4.2 `unitcat:<name>`

`<name>` is the lowercased Space name exactly as in the unit-state file, text
1-200.

| field | value |
|---|---|
| `category` | `null` (entry removed) or text, 1-64 |

## 5. Deletes and conflict policy

- **Field clear**: write `null`. It is a normal register and wins or loses by
  2.3 like any other value.
- **Space delete**: write `deleted = true`, and in the same batch `notes = null`
  and `pinned_tabs = null` so deleted content is not kept. A Space whose
  `deleted` register is `true` is hidden. **Delete wins** over concurrent or
  later field edits (they are stored but stay hidden). Only an explicit
  `deleted = false` with a bigger HLC restores the Space.
- **Unit category removal**: `unitcat:<name>.category = null`.
- Tombstones are kept forever in v1 (they are a few dozen bytes each).
- Concurrent edits of the same field: last writer wins (2.3); the other edit
  is lost. Notes and `pinned_tabs` are whole-value registers, so two machines
  editing the same Space's notes or pins while offline keep only the later
  edit. Different fields of the same Space merge independently.
- Clock skew: up to 10 min ahead is accepted (the HLC absorbs it). Beyond that,
  ops are deferred (2.4). A peer whose clock is behind loses ties it would have
  won by real time, which is accepted LWW behaviour.

## 6. State file

Path: `~/.config/wooloo-sync/state.json` on both macOS and Windows (`~` is
`HOME`, or `USERPROFILE` on Windows). `WOOLOO_SYNC_STATE` overrides it. It is
not inside the Herdr config dir, so Herdr never reads or rewrites it.

```json
{
  "format": 1,
  "node": "win-07be55d1",
  "clock": {"wall": 1763112000000, "counter": 2},
  "registers": [ <register>, ... ],
  "local": {"unitcat_snapshot": {"study central": "school"}}
}
```

- `registers` are written sorted by `(key, field)` (byte order).
- On load, `clock` is raised to the largest hlc among the registers.
- `local` is never sent to peers. `unitcat_snapshot` is the unit-state file as
  last imported or exported, used for a three-way diff (section 7).
- Writers take the lock file `state.json.lock` (create-exclusive; a lock older
  than 30 s is stale and removed), re-read the file, apply their change,
  write `state.json.tmp`, rename it over `state.json`, then remove the lock.

## 7. Unit-state categories file

The app (or the `wooloo-sync` CLI around an exchange) does:

1. **import**: read the local unit-state file. If it is missing, unreadable or
   not a JSON object, do nothing. Otherwise drop the entries that would not be
   valid ops (name not text 1-200, value not text 1-64); call the rest the
   file's *valid entries*. Diff the valid entries against `unitcat_snapshot`:
   entries added or changed become `category` ops, entries removed become
   `null` ops. Then set the snapshot to the valid entries (never to the raw
   file), so an invalid entry is never diffed again. (A machine without unit
   scripts therefore never deletes the other machine's entries.)
2. **exchange** with the peer.
3. **export**: build the map of all `unitcat:*` registers with non-null
   values. If the file's directory does not exist, or the file exists but is
   unreadable or not a JSON object, do nothing (snapshot unchanged). If the
   map differs from the file's valid entries, or the file is missing and the
   map is not empty, write the map to the file (temp + rename, sorted keys,
   one-space indent like `unit-space-purpose.sh`). Then set the snapshot to
   the map. When nothing is written, invalid entries already in the file stay
   there untouched.

The responder does the same import before applying the received ops and the
same export after, so both machines' unit files take part.

## 8. Exchange protocol

Run as `ssh <peer> wooloo-sync exchange` (the peer runs the responder on
stdin/stdout). Line-delimited JSON, UTF-8, one message per `\n`-terminated line,
each message an object with a `t` member. Strictly ping-pong, so neither side
writes a large payload while the other is also writing (no pipe deadlock).

```
initiator -> {"t":"hello","proto":1,"node":"win-07be55d1"[,"herdr_version":"0.9.5"]}
responder -> {"t":"hello","proto":1,"node":"mac-3f9a21c0"[,"herdr_version":"0.9.5"]}
initiator -> {"t":"op", <register members>}      (all registers, any order)
initiator -> {"t":"end","count":<n ops sent>}
responder    applies them, saves its state
responder -> {"t":"op", ...}                      (its registers that differ from
                                                   or are missing in what it received)
responder -> {"t":"end","count":<n>,"accepted":<a>,"rejected":<r>}
initiator    applies them, saves its state
```

- Either side may send `{"t":"error","msg":"..."}` instead of its next message
  and close; the other side then commits nothing from this exchange.
- `proto` mismatch: the responder answers with an error. Unknown members are
  ignored; unknown `t` is an error.
- `hello` may carry an optional `herdr_version`: the version of the Herdr
  server on the sender's machine, a string of 1 to 32 bytes of ASCII letters,
  digits or `+.-` (the same rule as pairing). It is informational, for a
  "versions differ" notice; nothing in the merge depends on it. A sender that
  does not know a valid version leaves the member out (never sends `null`,
  `""` or a number). A receiver that finds any other value (wrong type, empty,
  too long, other characters) treats it as absent, never as an error. Peers
  that predate it ignore it as an unknown member, so `proto` stays 1. The
  responder learns its own version without running anything at exchange
  time (section 9): from configuration, or not at all.
- A side commits (saves) only after it has received a complete `end` whose
  `count` matches the ops read. An interrupted exchange is harmless: the next
  one resends everything.
- v1 sends the full document (it is small: tens of Spaces, a few KB, worst case
  a few MB of notes). A later version may add a digest in `hello` to skip
  unchanged documents; it must stay optional.
- Responses are not echoed back further: one exchange makes both replicas equal
  in their accepted ops (except ops deferred by the skew guard).
- Max line length: 1 MiB (an op with 64 KB of notes fits easily). Longer lines
  are an error.
- A side may refuse an exchange of more than 200 000 ops (an error).
- A peer whose `node` equals the receiver's own node id is refused (two
  machines must never share a node id).

### 8.1 Wire bytes and implementation pitfalls

Every message is one compact JSON object followed by a single `\n` (a reader
also accepts `\r\n`). No pretty-printing, no BOM, no blank lines. Member order
does not matter to a reader, and no implementation may depend on it. Receivers must accept any member order on every message (vector section `key_order`; tests in the Rust crate and the Swift app). The Rust
crate currently writes members sorted by name, so a complete exchange where
the Windows app (`win-07be55d1`) pushes one op the Mac (`mac-3f9a21c0`)
already has is, byte for byte:

```
> {"node":"win-07be55d1","proto":1,"t":"hello"}
< {"node":"mac-3f9a21c0","proto":1,"t":"hello"}
> {"field":"name","hlc":{"counter":0,"wall":1763112000000},"key":"space:6f1c7a52-3b0e-4d8e-9a51-2a8f1c0d9e11","node":"mac-3f9a21c0","t":"op","value":"Study Central"}
> {"count":1,"t":"end"}
< {"accepted":1,"count":0,"rejected":0,"t":"end"}
```

With `herdr_version` (8), the same hellos are:

```
> {"herdr_version":"0.9.5","node":"win-07be55d1","proto":1,"t":"hello"}
< {"herdr_version":"0.9.4","node":"mac-3f9a21c0","proto":1,"t":"hello"}
```

(`>` initiator to responder, `<` back; the markers are not on the wire.) An
error is `{"msg":"<text>","t":"error"}`. `proto`, `count`, `accepted`,
`rejected`, `hlc.wall` and `hlc.counter` are JSON integers with no fraction or
exponent (`1`, never `1.0` or `1e0`). A reader treats an `hlc` member that is
not two non-negative integers as a malformed op (counted as rejected), and a
non-integer `count` in `end` as a protocol error.

Pitfalls for the Swift (Mac) side, each of which makes replicas disagree:

- **Slash escaping.** `JSONSerialization` writes `/` as `\/` unless given
  `.withoutEscapingSlashes`. That is still valid on the wire, but the
  canonical text (2.3 step 4, and anything hashed or compared) must not escape
  `/`. Do not build canonical text with `JSONSerialization` or `JSONEncoder`;
  write the small canonical encoder by hand and check it against the
  `canonical` vectors.
- **Bool vs NSNumber.** `JSONSerialization` decodes `true` and `1` both as
  `NSNumber`, and `as? Bool` / `as? Int` succeed for either. Validation must
  tell them apart, e.g. `CFGetTypeID(n) == CFBooleanGetTypeID()`: `deleted:1`
  and `order:true` are invalid. The `validate` vectors cover both.
- **Integers vs floats.** Write `wall`/`counter` from `UInt64`/`Int64`,
  never from a `Double` (e.g. `Date().timeIntervalSince1970 * 1000` must be
  converted first). When reading, reject an `hlc` given as `1.0`, even though
  `NSNumber` will happily bridge it to an integer. For `order`, keep the
  value's kind: integer `2` has canonical text `2`, float `2.5` has `2.5`.
  Floats use the shortest round-trip form (Swift `"\(double)"` gives `0.1`,
  not `0.10000000000000001`). `JSONSerialization` on older macOS prints up to
  17 significant digits, which is another reason not to use it for canonical
  text. `2.0` and `2` are different canonical texts, so a node should never
  rewrite one as the other.
- **Comparing strings.** Swift `String` `==` and `<` use Unicode canonical
  equivalence (`"é"` precomposed equals `"e\u{301}"`). Every comparison in
  this spec (node ids, canonical value text, sorting by name bytes) is on
  UTF-8 bytes: compare `Array(s.utf8)`. Lengths are Unicode scalar counts
  (`s.unicodeScalars.count`), not `s.count` (grapheme clusters); `notes` is
  limited in UTF-8 bytes (`s.utf8.count`).
- **Control characters.** "Text" excludes U+0000-U+001F and U+007F; check
  `unicodeScalars`, not `Character.isNewline` or similar.
- **Framing.** Flush stdout after each side's `end`. Read with a 1 MiB cap
  per line, and never read the peer's reply before sending your own `end`
  (2-way ping-pong; see above).
- **Optional `herdr_version`.** Decode it with `as? String` and apply the
  1-32 byte `[A-Za-z0-9+.-]` rule; on any failure treat it as absent and
  carry on (do not fail the hello, do not show the raw value). Omit the
  member when the local version is unknown; `JSONSerialization` turns an
  optional `nil` into `NSNull` (`null`) if you put it in the dictionary. The
  `hello` vectors cover both forms and the ignored values.
- **Member order and duplicates.** `JSONSerialization` dictionaries have no
  order; pass `.sortedKeys` if you want output byte-identical to the Rust
  crate (useful in tests, not required). A message with a duplicated member
  name is undefined; never send one.

## 9. Security notes

- The responder only reads stdin, writes stdout, and touches its own state file
  and the unit-state categories file (section 7). It takes no arguments from the
  peer, executes nothing, and touches no other path.
- Validation on receive means a compromised or buggy peer can at worst write
  allow-listed metadata (names, notes, colours) into the document.
- Notes are user text and can contain anything the user types; they are synced
  by design. Do not paste secrets into Space notes.

## 10. Test plan

- `cargo test -p wooloo-sync` in CI (windows-latest and macos-latest):
  HLC tick/observe, LWW order, validation, merge convergence over all
  permutations of op sets, commutativity, idempotence, offline edits on two
  replicas merged both ways, clock skew deferral, deletes and resurrection,
  host binding, unit-category three-way import/export, exchange over in-memory
  pipes (including error, proto mismatch and truncated stream), state file
  round trip, and `docs/sync-vectors.json`.
- A Swift implementation loads the same `docs/sync-vectors.json` and must
  produce identical results.
- Manual (after both apps ship): edit a Space's colour on the Mac offline,
  its notes on Windows offline, reconnect, run one exchange, both show both
  edits.

## Interop fixtures

The Swift app and this crate must put identical bytes on the wire. Both run one scripted scenario
(`crates/wooloo-sync/tests/fixtures/interop-scenario.json`, mirrored in wooloo as
`woolooTests/Fixtures/interop-scenario.json`) and write a transcript: one wire line per line, prefixed `> `
(initiator to responder) or `< ` (responder to initiator), hello (with `herdr_version`), ops, end.

- `interop-rust.transcript` is emitted by `crates/wooloo-sync/tests/interop.rs`; `interop-swift.transcript` by wooloo's `PeerSyncInteropTests`.
- Normal runs assert each side's output equals its own committed transcript byte for byte, and replay the other side's transcript through its own responder and initiator, comparing messages as parsed JSON (member order is not significant, 8.1) and checking the converged state. Swift writes its `end` line with members unsorted, so the two transcripts differ in bytes but not in meaning.
- Regenerate (needs `WOOLOO_INTEROP_REGENERATE=1`): Rust via `scripts/sync-interop.sh regenerate`; Swift by running the test with the same variable (CI prints the transcript between `INTEROP-TRANSCRIPT-BEGIN`/`END` markers in the log). Copy each file into the other repo's fixtures folder (`scripts/sync-interop.sh pull-swift <file>` for the Swift one). Any change to the scenario or wire format means regenerating both.
- CI (`vectors` job) regenerates the Rust transcript and uploads it as artifact `interop-rust-transcript`; fetch it with `gh run download <run> -n interop-rust-transcript` into an empty directory and commit it as the fixture in both repos.
- A missing transcript is skipped with a note, so the first CI run on each side only prints it.
