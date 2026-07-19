# dig-options — normative specification

`dig-options` is the DIG Network canonical builder library for Chia covered-option contracts. It
constructs the exact `CoinSpend`s for the option lifecycle and reports the signatures a caller must
produce. This document is the authoritative contract; an independent reimplementation can be built
against it.

## 1. Scope

dig-options covers the Chia covered-option primitive (CHIP-0042): a singleton "option ticket" plus a
separate underlying coin that holds a locked asset under a 1-of-2 path — **exercise** (the ticket
holder pays the strike asset into the settlement puzzle and unlocks the underlying, valid strictly
before expiry) and **clawback** (strictly after expiry, the creator reclaims the locked underlying).

**v0.1.0 scope:**

- **Underlying: XCH.** The locked asset is XCH; the underlying coin is a plain XCH coin at the option's
  1-of-2 path.
- **Strike: XCH only (v0.1.0).** `create` REJECTS a non-XCH strike up front (§6), so create and
  exercise have symmetric support envelopes — a holder can never acquire an option it could never
  exercise. `parse`/`parse_child` still inspect any strike type curried into an existing option.
  **Exercise builds the full settlement legs for an XCH strike** (both the underlying-claim leg to the
  holder and the strike-payment leg to the creator); its non-XCH guard remains as defense-in-depth.
  CAT/revocable-CAT/NFT strike (create + exercise) lands with the CAT/NFT follow-up (#1254).
- **Future extension (stated positively):** CAT / revocable-CAT / NFT underlyings and CAT/NFT strike
  exercise use the same `OptionUnderlying::exercise_spend` / `clawback_spend` primitives wrapped for the
  asset; they are additive and land in a later minor version.

## 2. Custody invariants (HARD)

These are the crate's defining properties and MUST hold for every operation:

1. **Key-free.** No function accepts, holds, derives, or stores a secret key. A creator/holder is
   expressed as an `Owner` (a public key or a borrowed inner spender) and explicit `Bytes32` puzzle
   hashes, never a secret. No `IndexedKeys`, no synthetic secret key.
2. **Never signs.** No function produces a `Signature`. The only signing-adjacent surface is
   `required_signatures`, which REPORTS the BLS messages a caller must sign; the caller signs and
   aggregates.
3. **Network-free.** No function performs I/O. Every coin and parent spend a builder needs is fetched by
   the caller and passed in.

A build produces unsigned `CoinSpend`s appended to a caller-owned `SpendContext`. The caller signs the
reported messages, assembles a `SpendBundle`, and broadcasts.

## 3. The identity boundary (#908)

dig-options is identity-agnostic. It references parties purely by public key and puzzle hash — it NEVER
constructs, spends, or holds a DID coin or key, and depends on NO DIG identity crate. The user key stays
entirely on the caller's side of the boundary.

## 4. Public types

### `Owner<'a>`
The p2 layer that authorizes an inner spend, without a secret.
- `Standard(PublicKey)` — the standard `p2_delegated_puzzle_or_hidden_puzzle` layer, identified by its
  BLS public key.
- `Custom(&'a dyn SpendWithConditions)` — any layer implementing `SpendWithConditions` (multisig, custom
  p2), borrowed for the build. Supported by every builder (create, exercise, clawback).
- `standard_puzzle_hash() -> Option<Bytes32>` — the standard p2 puzzle hash for `Standard`; `None` for
  `Custom` (used by the clawback guard, §5.3).
- Implements `SpendWithConditions` by routing to the concrete layer.

### `OptionTerms`
- `creator_puzzle_hash: Bytes32` — where the creator reclaims the underlying on clawback.
- `owner_puzzle_hash: Bytes32` — the option singleton's initial holder/owner.
- `underlying_amount: u64` — XCH mojos locked as the underlying.
- `strike_type: OptionType` — the asset + amount the holder must pay to exercise.
- `expiry_seconds: u64` — absolute unix seconds; exercise valid strictly before, clawback strictly after.
- `new(creator_puzzle_hash, underlying_amount, strike_type, expiry_seconds)` — sets `owner_puzzle_hash`
  = `creator_puzzle_hash` (the self-minted case). Use the struct literal to mint to a different owner.

### `CreatedOption`
The confirmed-option handle the caller retains to operate the option later.
- `option: OptionContract` — the option singleton.
- `underlying: OptionUnderlying` — the underlying terms (launcher id, creator ph, seconds, amount,
  strike type).
- `underlying_coin: Coin` — the locked-underlying XCH coin.

### `StrikePayment`
- `funding_coin: Coin` — the caller-supplied XCH coin the holder spends to fund the strike; must hold at
  least the strike amount.

### `OptionSpend`
- `coin_spends: Vec<CoinSpend>` — the unsigned spends produced.
- `created: Option<CreatedOption>` — `Some` for `create`; `None` for `exercise`/`clawback`.

### `ParsedOption`
The identity fields recoverable from an option coin spend (§5.4): `option`, `launcher_id`, `coin_id`,
`underlying_coin_id`, `underlying_delegated_puzzle_hash`, `p2_puzzle_hash`.

## 5. Operations

### 5.1 `create(ctx, creator, funding_coin, terms) -> OptionSpend`
Locks `terms.underlying_amount` XCH and mints the option singleton to `terms.owner_puzzle_hash`.
- **Emitted spends:** one `funding_coin` spend (through `creator`) that creates the launcher coin and the
  locked-underlying coin, plus the launcher/eve option spends.
- **Enforced invariants:** `terms.strike_type` is `Xch` (v0.1.0; else error, §6, same shape as the
  exercise guard); `underlying_amount > 0`; `funding_coin.amount >= underlying_amount + 1`
  (underlying + the 1-mojo singleton), computed with a checked add (overflow → error). Excess is an
  implicit fee.
- **Returns** `created: Some(..)` — the handle for exercise/clawback.

### 5.2 `exercise(ctx, holder, created, strike) -> OptionSpend`
Spends the option singleton through its exercise path and builds BOTH settlement legs in one bundle:
the unlocked underlying — which the exercise-path puzzle emits onto a bare settlement coin — is claimed
to the holder (the option's current `p2_puzzle_hash`) via a `SettlementLayer` spend paying the full
underlying amount, and the XCH strike is paid into the settlement puzzle and settled to the creator's
requested payment.
- **Enforced invariants:** `created.underlying.strike_type` is `Xch` (else error, §6); `strike.funding_coin.amount`
  ≥ the strike amount; the exercise's `AssertBeforeSecondsAbsolute(expiry)` boundary is enforced by the
  consensus (valid strictly before expiry).
- **Value conservation:** the underlying is CLAIMED to the holder in the same bundle (no bare settlement
  coin holding the underlying survives — nothing is left for a key-free thief); the strike is paid to the
  creator's requested payment. No value is created.
- **Returns** `created: None`.

### 5.3 `clawback(ctx, creator, created) -> OptionSpend`
The creator reclaims the locked underlying to `created.underlying.creator_puzzle_hash` via the
underlying's clawback path, valid strictly after expiry (consensus-enforced `AssertSecondsAbsolute`).
- **Enforced invariants:** a `Standard` `creator` whose `standard_puzzle_hash()` ≠
  `created.underlying.creator_puzzle_hash` is rejected up front; a `Custom` creator cannot be checked
  here and relies on the consensus to reject a wrong-party spend.
- **Value conservation:** exactly `underlying_coin.amount` is recovered.
- **Returns** `created: None`.

### 5.4 `parse(ctx, coin, puzzle_reveal, solution)` / `parse_child(ctx, parent_coin, parent_puzzle_reveal, parent_solution)`
Reconstruct an option from a fetched coin spend. `parse` decodes an option from its own spend;
`parse_child` walks a parent option spend to the option child. Both return `Ok(None)` when the puzzle is
not an option contract.

**Recoverable-fields limitation (normative):** an option singleton's on-chain puzzle commits only to its
identity fields (launcher id, underlying coin id, underlying delegated-puzzle hash, current p2 puzzle
hash — the SDK's `OptionInfo`). The option's *terms* (creator puzzle hash, expiry seconds, underlying
amount, strike type) live in the launcher metadata and the underlying coin and are NOT invertible from an
option singleton coin spend. `ParsedOption` therefore carries only the identity fields; a caller that
needs the terms retains the `CreatedOption` / `OptionTerms` from `create`.

### 5.5 `required_signatures(coin_spends, agg_sig_me) -> Vec<RequiredSignature>`
Runs each spend's puzzle to collect its `AGG_SIG_*` conditions and reports the BLS messages the caller
must sign, given the network's `agg_sig_me` additional data. Performs NO signing.

## 6. Error taxonomy

`Error` (`thiserror`), `Result<T> = std::result::Result<T, Error>`:
- `Driver(#[from] DriverError)` — a chia-wallet-sdk driver failure (allocation, currying, launcher mint).
- `Signer(#[from] SignerError)` — a failure computing required signatures.
- `InvalidInput(String)` — caller input that cannot produce a valid spend: zero underlying, overflow,
  underfunded funding/strike coin, wrong-party clawback, or an unsupported (CAT/NFT) strike (rejected at
  BOTH create and exercise). The message states the precise violation. A non-XCH strike returns this
  rather than minting an unexercisable option or emitting an incorrect spend (an honest gap, never a
  silent/incorrect settlement).

## 7. Lifecycle state machine

```
                 create
   (funding) ───────────────▶ CREATED ──────────────────────────────▶ terminal
                              │  │
             exercise (before │  │ clawback (after expiry):
             expiry): strike  │  │ creator reclaims underlying
             paid to creator, │  │
             underlying       │  │
             claimed to holder▼  ▼
                           EXERCISED   CLAWED-BACK
```
An option is created, then reaches exactly one terminal state: **exercised** (strictly before expiry) or
**clawed-back** (strictly after expiry). Both exits are always reachable — the option is never
locked-forever (§8.6).

## 8. Security properties (guarantees)

1. **No theft of the underlying without the strike.** Exercise unlocks the underlying only in a bundle
   that also pays the strike (the underlying's delegated puzzle asserts the settlement payment);
   consensus-gated. (Test: `exercise_without_strike_leg_is_rejected`.)
2. **No exercise after expiry.** The exercise path asserts `AssertBeforeSecondsAbsolute(expiry)`. (Test:
   `exercise_after_expiry_is_rejected`.)
3. **No clawback before expiry.** The clawback path asserts `AssertSecondsAbsolute(expiry)`. (Test:
   `clawback_before_expiry_is_rejected`.)
4. **Terms immutable after create.** The terms are curried into the option/underlying puzzles, so any
   change yields a different coin id.
5. **Never signs / no key leak.** No `SecretKey` appears in any type; no function returns a `Signature`.
6. **No locked-forever option.** Both exits (exercise, clawback) are reachable across the expiry
   boundary. (Tests: the round-trip + clawback-on-expiry.)
7. **Wrong-party clawback rejected.** A `Standard` clawback owner mismatching the creator ph is rejected
   up front; the consensus enforces it regardless. (Test: `clawback_rejects_wrong_creator_key`.)
8. **Value conservation.** Create requires `funding ≥ underlying + 1`; exercise conserves value for
   both parties — the underlying is CLAIMED to the holder and the strike is paid to the creator in the
   same bundle, leaving no orphan settlement coin a third party could claim key-free; clawback recovers
   exactly the locked amount. (Tests: `create_then_exercise_round_trip` asserts the holder nets exactly
   `underlying − strike` and the creator nets the strike; `exercise_leaves_no_orphan_underlying_settlement_coin`
   asserts no bare settlement coin survives.)

## 9. Conformance

The option puzzles (`OptionLauncher` / `OptionContract` / `OptionUnderlying` / `SettlementLayer`) are the
canonical chia-wallet-sdk 0.30 puzzles — the byte-source-of-truth; dig-options NEVER hand-rolls a puzzle.
Every builder's output is validated on the in-process Chia simulator (`chia-sdk-test`), including a
create → exercise round-trip, a create → clawback-on-expiry flow, the adversarial negatives above, and a
parse identity round-trip.
