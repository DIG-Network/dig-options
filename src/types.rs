//! The public value types every dig-options builder speaks in.
//!
//! These types are deliberately **key-free**: an [`Owner`] carries a public key (or an
//! arbitrary caller-supplied inner spender), never a secret; the parties an option is created
//! for are named by plain [`Bytes32`] puzzle hashes carried in [`OptionTerms`]. A builder
//! consumes these, appends `CoinSpend`s to a caller-owned [`SpendContext`], and returns an
//! [`OptionSpend`] — the built spends plus, for `create`, the [`CreatedOption`] the caller
//! keeps to later exercise or claw back the option.

use chia_protocol::{Bytes32, Coin, CoinSpend};
use chia_puzzle_types::standard::StandardArgs;
use chia_wallet_sdk::driver::{
    DriverError, OptionContract, OptionType, OptionUnderlying, Spend, SpendContext,
    SpendWithConditions, StandardLayer,
};
use chia_wallet_sdk::prelude::PublicKey;
use chia_wallet_sdk::types::Conditions;

/// The p2 (owner) layer that authorizes an option's inner spend, expressed WITHOUT any secret.
///
/// dig-options never signs; it only builds the spend and reports the signature the caller must
/// produce. [`Owner::Standard`] is the common case — the standard
/// `p2_delegated_puzzle_or_hidden_puzzle` layer identified by its public key.
/// [`Owner::Custom`] is the escape hatch: any layer implementing the SDK's
/// [`SpendWithConditions`] (a multisig, a custom p2, a settlement layer) borrowed for the
/// build, so a non-standard owner is fully supported through every builder.
pub enum Owner<'a> {
    /// A standard-layer owner identified by its BLS public key.
    Standard(PublicKey),
    /// An arbitrary owner layer that knows how to emit a set of conditions.
    Custom(&'a dyn SpendWithConditions),
}

impl Owner<'_> {
    /// The standard p2 puzzle hash of a [`Owner::Standard`] owner.
    ///
    /// Returns `None` for [`Owner::Custom`], whose puzzle hash dig-options cannot derive
    /// without knowing the concrete layer. Used to reject a wrong-party clawback up front
    /// (a Custom owner skips that check — the consensus still enforces the real path).
    pub fn standard_puzzle_hash(&self) -> Option<Bytes32> {
        match self {
            Owner::Standard(public_key) => {
                Some(StandardArgs::curry_tree_hash(*public_key).into())
            }
            Owner::Custom(_) => None,
        }
    }
}

impl SpendWithConditions for Owner<'_> {
    /// Route an option's output conditions through the concrete owner layer, producing the
    /// inner [`Spend`]. Neither variant holds or uses a secret key.
    fn spend_with_conditions(
        &self,
        ctx: &mut SpendContext,
        conditions: Conditions,
    ) -> std::result::Result<Spend, DriverError> {
        match self {
            Owner::Standard(public_key) => {
                StandardLayer::new(*public_key).spend_with_conditions(ctx, conditions)
            }
            Owner::Custom(inner) => inner.spend_with_conditions(ctx, conditions),
        }
    }
}

/// The terms of a covered option: who created it, who owns (may exercise) it, how much XCH it
/// locks as the underlying, what strike the holder must pay, and when it expires.
///
/// Key-free: the parties are named by their [`Bytes32`] puzzle hashes, never by keys. The
/// underlying is XCH (v0.1.0 scope); `strike_type` may be any [`OptionType`] (it is curried
/// into the option puzzle), though only an XCH strike can be *exercised* in v0.1.0 (see
/// [`crate::exercise`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionTerms {
    /// The puzzle hash the creator reclaims the underlying to on clawback (after expiry).
    pub creator_puzzle_hash: Bytes32,
    /// The puzzle hash the option singleton is minted to — its initial holder/owner.
    pub owner_puzzle_hash: Bytes32,
    /// The amount of XCH (in mojos) locked as the underlying.
    pub underlying_amount: u64,
    /// The asset + amount the holder must pay to exercise the option.
    pub strike_type: OptionType,
    /// The absolute unix timestamp (seconds) at which the option expires: exercise is valid
    /// strictly before it, clawback strictly after it.
    pub expiry_seconds: u64,
}

impl OptionTerms {
    /// Terms with the creator as the initial owner (the common self-minted case).
    ///
    /// The option is minted to `creator_puzzle_hash` and, on clawback, reclaimed to it. Use
    /// the struct literal directly to mint an option owned by a different party.
    #[must_use]
    pub fn new(
        creator_puzzle_hash: Bytes32,
        underlying_amount: u64,
        strike_type: OptionType,
        expiry_seconds: u64,
    ) -> Self {
        Self {
            creator_puzzle_hash,
            owner_puzzle_hash: creator_puzzle_hash,
            underlying_amount,
            strike_type,
            expiry_seconds,
        }
    }
}

/// A created option, returned by [`crate::create`] so the caller can later exercise or claw it
/// back — both need the option singleton, the underlying terms, and the locked-underlying
/// coin, which only exist once the create spend is confirmed.
///
/// The caller must retain this (or the equivalent [`OptionTerms`] plus the confirmed coins) to
/// operate the option: the terms are not recoverable from the option singleton coin alone
/// (see [`crate::parse`]).
#[derive(Clone, Debug)]
pub struct CreatedOption {
    /// The option singleton (the transferable "ticket"); its holder may exercise it.
    pub option: OptionContract,
    /// The underlying terms (launcher id, creator ph, expiry seconds, locked amount, strike
    /// type) — needed to build the exercise / clawback spend.
    pub underlying: OptionUnderlying,
    /// The locked-underlying XCH coin (parent = the funding coin, puzzle hash = the
    /// underlying's 1-of-2 path, amount = the locked amount).
    pub underlying_coin: Coin,
}

/// The result of a dig-options builder: the unsigned coin spends it produced, and — for
/// [`crate::create`] — the [`CreatedOption`] the caller keeps to operate the option later.
///
/// `created` is `Some` for `create` and `None` for `exercise`/`clawback` (which consume an
/// existing option rather than producing one).
#[derive(Clone, Debug)]
pub struct OptionSpend {
    /// The unsigned coin spends this operation produced.
    pub coin_spends: Vec<CoinSpend>,
    /// The created option — `Some` only for `create`.
    pub created: Option<CreatedOption>,
}
