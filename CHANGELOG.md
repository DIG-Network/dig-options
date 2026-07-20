# Changelog

All notable changes to this project are documented here.
This project adheres to [Semantic Versioning](https://semver.org) and
[Conventional Commits](https://www.conventionalcommits.org).

## [0.2.0] - 2026-07-20

### Features
- **options:** curated `transfer` builder — move the option ticket to a new owner, composing
  `OptionContract::transfer` so consumers no longer reach past dig-options into the SDK primitive (#1288).
- **options:** `rehydrate` + `parse_metadata` — reconstruct a full, operable `CreatedOption` from on-chain
  state (verified against the option's commitments), lifting the recoverable-fields limitation so a caller
  can exercise/transfer/claw back an option it did not mint in the same session (#1288).

## [0.1.0] - 2026-07-19

### Features
- **options:** V0.1.0 — key-free CoinSpend builder for Chia covered options (#1)

## [0.0.0] - 2026-07-19

### Chores
- Genesis scaffold (v0.0.0) — crate + release/quality pipeline


