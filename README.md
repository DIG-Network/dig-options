# dig-options

The DIG Network canonical Chia **option-contract** expert crate — a pure, key-free, network-free
`CoinSpend`-builder for Chia covered options (the CHIP-0042 option primitive).

`dig-options` constructs the exact `CoinSpend`s for the option lifecycle and reports the signatures a
caller must produce. It **never holds a secret key, never signs, and never touches the network** — the
consumer signs the reported messages, assembles the `SpendBundle`, and broadcasts.

## What it builds

- **create** — lock an XCH underlying and mint the transferable option singleton (the "ticket"),
  exercisable for a configured strike until an expiry.
- **exercise** — the holder pays the strike into the settlement puzzle and unlocks the underlying to
  itself (rejected by consensus after expiry).
- **clawback / cancel** — after expiry, the creator reclaims the locked underlying.
- **inspect** — reconstruct a spendable option (its recoverable identity fields) from an on-chain coin
  spend.

## Scope (v0.1.0)

The underlying is **XCH** and the strike is **XCH-only**: `create` rejects a non-XCH strike up front so
create and exercise stay symmetric (no unexercisable option can be minted). **exercise** builds both
settlement legs for an XCH strike — the underlying is claimed to the holder and the strike is paid to
the creator, in one bundle. `clawback`/inspect work for any strike curried into an existing option.
CAT/NFT underlyings and strike are a future extension. See `SPEC.md`.

## Custody model (HARD invariants)

1. **Key-free** — no function accepts, holds, derives, or stores a secret key.
2. **Never signs** — the crate reports the BLS messages a caller must sign; the caller signs and
   aggregates.
3. **Network-free** — no I/O; chain data a builder needs is fetched by the caller and passed in.

## Install

```toml
[dependencies]
dig-options = "0.1"
```

## Documentation

`SPEC.md` is the normative contract — an independent reimplementation can be built against it.

## License

Licensed under either of Apache-2.0 or MIT at your option.
