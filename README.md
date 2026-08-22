# Argent

Argent is an actor-based language and compiler for building stateful,
multi-contract and multi-app applications on covenant-native UTXO rails.

Applications are expressed as transaction-wide state transitions over covenant
UTXOs. Actors own typed state, entries consume and emit actors, and `become`
defines the successor actors created by one atomic transaction. Inter-Covenant
Communication (ICC) extends the same model across independently compiled apps.

## Compiler foundation

Argent is built on [Silverscript](https://github.com/kaspanet/silverscript),
the core foundation of this compiler. Silverscript provides the complete
single-contract language and compiler stack, from typed contract source to
Kaspa Script, together with the low-level builtins that make Argent's complex
multi-contract and multi-app work possible.

Argent adds the application layer above that foundation. It turns `.ag` source
into plain, auditable Silverscript contracts and portable artifacts consumed by
`argent-runtime`. Together the layers handle state layouts, template
commitments, routing, output validation, cross-covenant observation, virtual
state expansion, and hidden witness material.

The design targets programmable UTXO systems with script-composition primitives
such as `OP_CAT` and `OP_SUBSTR`, transaction introspection, and
consensus-supported covenant identities (see
[Kaspa’s KIP-20](https://github.com/kaspanet/kips/blob/master/kip-0020.md) for a
reference model).

Kaspa is the native target: `.ag` programs compile to Silverscript and
ultimately execute on the native Kaspa Script engine. The language, artifact,
and runtime layers keep Kaspa-specific assumptions explicit and separated where
practical.

## Project status

> The project is still under active development and is not yet release-ready.
Once Silverscript completes its audit and is released, advanced users who can
review the generated `.sil` contracts will have a viable path to careful early
production use. This requires understanding Argent's route semantics and
compiler model well enough to verify that the generated contracts match the
intended application. Argent itself will still need further audit and hardening
before general production use.

The main pieces are present: compiler, generated Silverscript, portable
artifacts, runtime transaction building, multi-actor routing, cross-app linking,
constrained covenant spawning, actor enums, closed and open ICC, and
virtual-slot state expansion.

```text
.ag source
    |
    v
Argent compiler
    |
    +-- plain .sil contracts
    |
    +-- portable artifact
              |
              v
       argent-runtime
              |
              v
   atomic multi-actor Kaspa tx
```

## Quick start

Run the standard local check loop:

```sh
./check.sh
```

Regenerate tracked example outputs and run the full check loop:

```sh
./check.sh --full
```

Build one app manually:

```sh
cargo run -- build examples/tickets.ag --out examples/build/tickets
cargo run -- build examples/stones/app.ag --out examples/build/stones
cargo run -- build examples/icc/kcc20_asset.ag --out examples/build/icc_kcc20_asset
cargo run -- build examples/icc/minter.ag --out examples/build/icc_minter
cargo run -- build examples/open_icc/agent.ag --out examples/build/open_icc_agent
cargo run -- build examples/open_icc/core.ag --out examples/build/open_icc_core
```

When one source file declares multiple apps, select the app to build by name:

```sh
cargo run -- build contracts.ag --app DexCore --out build/dex-core
```

Generated outputs include:

- `artifact.json`: the portable Argent artifact
- `manifest.json`: build metadata
- `sil/*.sil`: generated Silverscript contracts

Inspect the compiled artifact without rebuilding it:

```sh
cargo run --bin argentc -- inspect examples/build/tickets
```

The report summarizes actor script, state, and template sizes, static opcode
counts, entry arguments and generated witnesses, route metadata, and
signature-script size estimates.

Generated `.sil` files compile as ordinary Silverscript. Within each contract,
Silverscript provides the types, expressions, functions, arrays, loops, and
control flow, then performs the final type checking and Kaspa Script
compilation. Argent does not use Silverscript covenant macros.

## Transaction tests

Argent transaction tests stay at the authored level: actor inputs and source
state, entry names and arguments, actor outputs and source state, and an
`accept` or `reject` expectation. `TxBuilder` still supplies constructor
arguments, generated fields, selectors, hidden witnesses, and raw scripts.

Place the test file beside the source using the same stem, then run it with:

```sh
argentc test examples/tickets.ag
argentc test examples/tickets.ag --filter redeem
```

The filter is a substring match on test names. For `tickets.ag`, the CLI reads
`tickets.test.json`. Its schema is:

```json
{
  "keys": {
    "owner": "0x0000000000000000000000000000000000000000000000000000000000000001"
  },
  "tests": [
    {
      "name": "redeem works",
      "inputs": [
        {
          "actor": "Ticket",
          "state": { "owner": { "key_hash": "owner" }, "serial": 7, "redeemed": 0 },
          "entry": "redeem",
          "args": [{ "sign": "owner" }, { "pubkey": "owner" }]
        }
      ],
      "outputs": [
        {
          "actor": "Ticket",
          "state": { "owner": { "key_hash": "owner" }, "serial": 7, "redeemed": 1 }
        }
      ],
      "expect": "accept"
    }
  ]
}
```

`keys` is optional and maps names to 32-byte secp256k1 secret
keys. These keys are test fixtures only and must never control real funds.
Values may refer to them with `{ "sign": "owner" }`,
`{ "pubkey": "owner" }`, or `{ "key_hash": "owner" }`; signing is performed
against the concrete transaction input assembled by the runner.

Each test has `name`, `inputs`, `outputs`, and `expect`. An input has `actor`,
`state`, `entry`, and optional `args`, `value`, and `covenant`; an output has
`actor`, `state`, and optional `value`.

Each input and output value defaults to `1000`. An input without `covenant`
gets its own deterministic covenant; inputs with the same covenant label belong
to the same covenant group. Deterministic outpoints and output covenant bindings
are generated by the runner, so test files do not contain raw transaction data.
This first sidecar cut covers transitions between existing covenants; genesis
and spawned-output cases remain available through programmatic `TxContext`s.
If identical actor outputs could be authorized by more than one input, the
sidecar reports the ambiguity as a definition error instead of guessing;
programmatic contexts can express that binding explicitly.

`expect` accepts `"accept"`, `"reject"`, or a stable authored rejection target:

```json
"expect": { "reject": "Ticket::redeem" }
```

An accepted transaction passes `accept`; a script rejection passes `reject`
(and must match the target when one is given). Acceptance under `reject`, or
rejection under `accept`, is a test failure. Builder setup, artifact, shape, or
encoding errors are test errors and cannot satisfy `reject`.

The same expectations are available programmatically:

```rust
use argent::testing::ArgentTestRunner;

let runner = ArgentTestRunner::from_build_dir("build/tickets")?;
let builder = runner.builder()?;

runner.expect_accept("redeem works", &builder, &valid_context)?;
runner.expect_reject("cannot redeem twice", &builder, &invalid_context)?;
runner.expect_reject_at("cannot redeem twice", "Ticket::redeem", &builder, &invalid_context)?;
```

SilverScript is used only to explain an unexpected actor rejection: Argent
recompiles the generated `.sil` with the executed state, verifies that its
bytecode matches the redeem script, and formats the debugger report. Silver's
low-level JSON test schema is not part of the Argent test format.

## Language at a glance

```rust
state TicketState {
    byte[32] owner;
    int units;
}

actor Ticket owns TicketState {
    entry transfer(byte[32] next_owner, sig owner_sig, pubkey owner_pk) emits next: Ticket {
        require(blake2b(byte[](owner_pk)) == owner);
        require(checkSig(owner_sig, owner_pk));
        require(next.value == self.value);

        TicketState new_state = {
            owner: next_owner,
            units: units,
        };

        become next <- Ticket(new_state);
    }
}

app Tickets {
    actor Ticket;
}
```

Argent uses type-first syntax for declarations and callable parameters.
Bindings put the local name on the left. See
[Surface syntax conventions](docs/argent-design.md#surface-syntax-conventions)
for the rules and examples.

Argent actors are not async actors with mailboxes or message queues. They are
covenant objects that get consumed and recreated by transactions. The shared
idea with actor models is state ownership: an actor's code is the only
authority that can consume and mutate that actor's state.

Core terms:

- `state` defines a persistent covenant state layout.
- `actor` defines one contract template that owns a state layout.
- `entry` defines a callable transition path.
- `delegate` defines a non-leading check in a coordinated transition.
- `consumes` names peer covenant inputs in the same transaction.
- `emits` declares the authorized output handles for an entrypoint.
- `become` is the terminal transition into successor actor state.
- `observes` declares a foreign covenant view for ICC.
- `spawns` declares a genesis covenant output group and binds its generated
  covenant id. A spawn target can be an actor in the selected app or an
  `actor_type<State>` value.
- `actor_type<State>` identifies a runtime-selected actor implementation
  compatible with `State`.
- `actor enum` defines a closed set of runtime-selected actor targets.
- `virtual` slots and `state X expands Base` let concrete actors bind private
  digest-backed memory while preserving a shared base state layout.

## Examples

- [examples/tickets.ag](examples/tickets.ag): tiny single-file issuer/ticket app,
  with [transaction tests](examples/tickets.test.json)
- [examples/spawns.ag](examples/spawns.ag): constrained genesis covenant launch
  with a complete two-output group
- [examples/stones](examples/stones): small coordinated game with league,
  player, game, and settle actors
- [examples/toy_chess](examples/toy_chess/app.ag): actor enums and
  route-family selector lowering
- [examples/icc](examples/icc): closed ICC between a minter and asset app
- [examples/open_icc](examples/open_icc): open observed actors and virtual-slot
  agent state

For client-side examples, see
[argent-playground](https://github.com/argent-lang/argent-playground). It is a
separate Rust project that depends on a neighboring Argent checkout and shows
complete app compilation and transaction-building flows through
`argent-runtime`.

## Runtime

`argent-runtime` is the artifact-only consumer surface. It has no compiler
dependency. It loads compiled artifacts, fills hidden witness material, builds
covenant UTXOs, composes artifact bundles, and builds complete transactions
from concrete actor inputs and outputs.

Classic single-app flow:

```rust
let builder = TxBuilder::new(&artifact)?;

let input_state = state! { count: 2 };
let output_state = state! { count: 5 };

// The covenant UTXO being spent.
let input_utxo = builder.covenant_utxo(
    "Counter",
    input_state.clone(),
    value,
    0,
    false,
    Some(covenant_id),
)?;

let context = TxContext::new()
    .actor_input(
        "Counter",
        input_state,
        EntryCall::new("bump").args(args![3]),
        outpoint,
        input_utxo,
        0, // sequence
    )
    .actor_output(
        "Counter",
        output_state,
        CovenantBinding::new(0, covenant_id),
        value,
    );

let tx = builder.build(&context)?;
```

Each input declares its sequence. Lock time, lane and gas, and payload can be
set fluently on `TxContext`; their defaults produce a native transaction.

The runtime API is Argent-specific while the language settles. The lower-level
Silverscript ABI and artifact boundaries are split into small crates so they can
be kept portable. Multi-app ICC uses `ArtifactBundle`; the transaction context
is otherwise the same for single- and multi-app transactions.

## Why Argent

Kaspa covenants make it possible to build applications from several stateful
UTXOs whose transitions compose atomically in one transaction. But hand-written
multi-contract systems quickly accumulate mechanical obligations: state
serialization, template hashes, route commitments, prefix/suffix witnesses,
output ordering, observed covenant ids, and cross-contract state reads.

Argent makes the application graph source-level. Actors own state. Entries
declare the peer actors they consume, the outputs they emit, the foreign
covenants they observe, and the successor actors those outputs become. The
compiler checks the declared state-machine edges and emits the Silverscript that
performs the low-level validation.

Generated contracts stay as plain `.sil` files, and the artifact records the
runtime recipe needed to build transactions against them.

## How it works

The compiler parses `.ag` source into an actor/state model and lowers each actor
to one Silverscript contract. Source state fields become the contract state
layout. Compiler-generated fields and hidden entry arguments carry template
receipts, route-family tables, observed-covenant witnesses, and expanded-state
preimages.

`become` routes lower to output validation. Exact continuations can use cheaper
script-public-key checks. Foreign or runtime-selected actors use template
prefix/suffix witnesses or route-family tables. `observes` lowers to covenant
input/output checks against another app. `virtual` slots lower to fixed digest
fields, with concrete actors providing hidden preimages when they expand those
slots into structured memory.

The portable artifact records the runtime recipe for all of this: script bytes,
state layouts, type descriptors, route receipts, observed covenant metadata,
hidden witness recipes, artifact ids, and interface fingerprints.
`argent-runtime` consumes that artifact directly; it does not depend on compiler
AST types.

Each app artifact also records the exact artifact ID of each direct app
dependency. Runtime bundles reject missing or different dependency artifacts
before they build a transaction.

## Current status

What is useful today:

- compiling `.ag` apps to auditable `.sil`
- building tracked example transactions through `argent-runtime`
- closed and open ICC examples
- route-family and actor-enum examples
- virtual-slot expanded state for open-agent style apps

What is still being built:

- broader launch and bootstrap tooling
- richer package and dependency tooling
- stronger diagnostics and typechecking
- generated app-specific builder APIs
- broader hardening and negative-test coverage

Design notes can be found in [docs/argent-design.md](docs/argent-design.md).
ICC semantics can be found in [docs/icc-semantics.md](docs/icc-semantics.md).
Subtle generated-code security arguments are documented in [SECURITY.md](SECURITY.md).

## Contributing

Run `./check.sh --full` before submitting changes. Open design questions and
implementation sketches are collected in [docs/followups.md](docs/followups.md);
they are useful starting points for discussion and contributions.
