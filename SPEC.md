# dig-options — normative specification

`dig-options` is the DIG Network canonical builder library for Chia covered-option contracts. It
constructs the exact `CoinSpend`s for the option lifecycle and reports the signatures a caller must
produce. This document is the authoritative contract; an independent reimplementation can be built
against it.

> **Status: v0.0.0 bootstrap.** This scaffold establishes the crate + the release/quality pipeline.
> The full normative specification (public types, operation semantics, conformance) lands with the
> v0.1.0 feature PR. The custody invariants in §2 are binding from commit 1.

## 1. Scope

dig-options covers the Chia covered-option primitive (CHIP-0042): a singleton "option ticket" plus a
separate underlying coin that holds a locked asset under a 1-of-2 path — **exercise** (the ticket
holder pays the strike asset into the settlement puzzle and unlocks the underlying, valid until
expiry) and **clawback** (after expiry, the creator reclaims the locked underlying).

## 2. Custody invariants (HARD)

These are the crate's defining properties and MUST hold for every operation:

1. **Key-free.** No function accepts, holds, derives, or stores a secret key. A creator/holder is
   expressed as a public key or a borrowed inner spender, never a secret.
2. **Never signs.** No function produces a signature. The crate REPORTS the BLS messages a caller must
   sign; the caller signs and aggregates.
3. **Network-free.** No function performs I/O. Chain data a builder needs is fetched by the caller and
   passed in.

A build produces unsigned `CoinSpend`s appended to a caller-owned `SpendContext`. The caller signs the
reported messages, assembles a `SpendBundle`, and broadcasts.

## 3. The identity boundary (#908)

dig-options is identity-agnostic. It references parties purely by public key and puzzle hash — it
NEVER constructs, spends, or holds a DID coin or key, and depends on NO DIG identity crate.

## 4. Conformance

The option puzzles (`OptionLauncher` / `OptionContract` / `OptionUnderlying`) are the canonical
chia-wallet-sdk puzzles — dig-options NEVER hand-rolls a puzzle. Every builder's output is validated on
the in-process Chia simulator (`chia-sdk-test`), including a create → exercise round-trip and a
create → clawback-on-expiry flow.
