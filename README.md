# dig-options

The DIG Network canonical Chia **option-contract** expert crate — a pure, key-free, network-free
`CoinSpend`-builder for Chia covered options (the CHIP-0042 option primitive).

`dig-options` constructs the exact `CoinSpend`s for the option lifecycle and reports the signatures a
caller must produce. It **never holds a secret key, never signs, and never touches the network** — the
consumer signs the reported messages, assembles the `SpendBundle`, and broadcasts.

## What it builds

- **create** — lock an underlying asset and mint the transferable option singleton (the "ticket"),
  exercisable for a configured strike asset until an expiry.
- **exercise** — the holder pays the strike into the settlement puzzle and unlocks the underlying to
  itself (rejected by consensus after expiry).
- **clawback / cancel** — after expiry, the creator reclaims the locked underlying.
- **inspect** — reconstruct an option (its terms + spendable coin) from an on-chain coin spend.

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
