//! Reconstruct a full, operable [`CreatedOption`] from on-chain state.
//!
//! [`crate::parse`]/[`crate::parse_child`] recover only an option's *identity* fields — the
//! option singleton's puzzle does not commit to its terms (creator puzzle hash, expiry, underlying
//! amount, strike type), so those cannot be inverted from the singleton coin spend alone. But
//! [`crate::exercise`], [`crate::clawback`], and [`crate::transfer`] all need the full
//! [`OptionUnderlying`] terms. Without a way to recover them, a caller could only ever operate an
//! option it minted itself in the same session.
//!
//! [`rehydrate`] closes that gap. The caller supplies the terms it can observe off-chain — the
//! creator puzzle hash it recorded, plus the expiry + strike recovered from the launcher metadata
//! via [`parse_metadata`] — together with the parsed option and its fetched underlying coin.
//! `rehydrate` reconstructs the [`OptionUnderlying`] and **verifies it against the option's
//! on-chain commitments**: the 1-of-2 underlying path, the underlying delegated-puzzle hash, and
//! the underlying coin id all must match. A single wrong term changes one of those hashes and is
//! rejected — so a successfully rehydrated [`CreatedOption`] is guaranteed to bind to the real
//! on-chain option and produce spends the consensus will accept.

use chia_protocol::{Coin, Program};

use chia_wallet_sdk::driver::{OptionContract, OptionType, OptionUnderlying, SpendContext};
use chia_wallet_sdk::prelude::ToTreeHash;

use crate::error::{Error, Result};
use crate::types::CreatedOption;

// Re-exported so a caller need not depend on the SDK directly to name the recovered metadata.
pub use chia_wallet_sdk::driver::OptionMetadata;

/// The terms a caller supplies to [`rehydrate`] a previously-minted option.
///
/// Every field is verified against the option's on-chain commitments, so these are asserted, not
/// trusted: a wrong value is rejected rather than producing an option handle that builds an
/// unspendable bundle. `expiry_seconds` and `strike_type` are recoverable from the launcher
/// metadata ([`parse_metadata`]); `creator_puzzle_hash` is the party the caller recorded at mint
/// (it is committed only inside the underlying's clawback path, so it is supplied and verified
/// rather than inverted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RehydratedTerms {
    /// The puzzle hash the creator reclaims the underlying to on clawback.
    pub creator_puzzle_hash: chia_protocol::Bytes32,
    /// The absolute unix timestamp (seconds) at which the option expires.
    pub expiry_seconds: u64,
    /// The asset + amount the holder must pay to exercise the option.
    pub strike_type: OptionType,
}

/// Recover an option's [`OptionMetadata`] (expiry seconds + strike type) from the launcher coin's
/// solution, which the caller fetched from a node/indexer.
///
/// The launcher solution carries the option's key-value metadata; this decodes it into the
/// caller-owned [`SpendContext`]. Network-free: the caller supplies the serialized solution.
pub fn parse_metadata(
    ctx: &mut SpendContext,
    launcher_solution: &Program,
) -> Result<OptionMetadata> {
    let solution = ctx.alloc(launcher_solution)?;
    Ok(OptionContract::parse_metadata(ctx, solution)?)
}

/// Reconstruct a full [`CreatedOption`] from a parsed `option`, caller-supplied `terms`, and the
/// fetched `underlying_coin`, verifying every reconstructed field against the option's on-chain
/// commitments.
///
/// Rebuilds the [`OptionUnderlying`] from `option.info.launcher_id`, `terms`, and
/// `underlying_coin.amount`, then rejects the reconstruction unless ALL THREE of the following
/// match the on-chain option. The three are **jointly** load-bearing — no single check covers
/// every field, so none is mere defense-in-depth (verified against the chia-wallet-sdk 0.30
/// `OptionUnderlying` derivation):
/// - **1-of-2 path hash** equals `underlying_coin.puzzle_hash`. Per the SDK, the path is
///   `merkle([exercise_path(launcher_id), clawback_path(expiry, creator_ph)])`, so it binds ONLY
///   the launcher id, expiry, and creator puzzle hash — NOT the amount or strike type. A wrong
///   creator hash or expiry is caught here.
/// - **delegated-puzzle hash** equals `option.info.underlying_delegated_puzzle_hash`. The
///   delegated puzzle commits to the expiry, the underlying amount, and the strike type (settlement
///   target + requested-payment amount). A wrong **strike type** is caught ONLY here; a wrong
///   amount is caught both here and by the coin-id check below.
/// - **underlying coin id** equals `option.info.underlying_coin_id`. This binds the coin's full
///   identity (parent + puzzle hash + amount), uniquely rejecting a substituted coin of the right
///   shape but wrong parent that would slip past the two hash checks.
///
/// On success the returned [`CreatedOption`] is operable by [`crate::exercise`],
/// [`crate::clawback`], and [`crate::transfer`] exactly as one returned by [`crate::create`].
///
/// **Pure: performs no I/O and holds no key.** The caller fetches the option spend + underlying
/// coin and recovers the metadata; `rehydrate` only reconstructs + verifies.
pub fn rehydrate(
    option: &OptionContract,
    terms: &RehydratedTerms,
    underlying_coin: Coin,
) -> Result<CreatedOption> {
    let underlying = OptionUnderlying::new(
        option.info.launcher_id,
        terms.creator_puzzle_hash,
        terms.expiry_seconds,
        underlying_coin.amount,
        terms.strike_type,
    );

    // Check 1: the 1-of-2 path hash. Per the SDK it is
    // merkle([exercise_path(launcher_id), clawback_path(expiry, creator_ph)]), so it binds ONLY the
    // launcher id, expiry, and creator puzzle hash — NOT the amount or strike type. This is the sole
    // check that catches a wrong creator puzzle hash.
    let reconstructed_path: chia_protocol::Bytes32 = underlying.tree_hash().into();
    if reconstructed_path != underlying_coin.puzzle_hash {
        return Err(Error::invalid(
            "rehydrated terms do not match the underlying coin's 1-of-2 path — check creator puzzle hash and expiry",
        ));
    }

    // Check 2: the delegated-puzzle hash, which the option singleton independently commits to. It
    // binds the expiry, the underlying amount, and the STRIKE TYPE (settlement target +
    // requested-payment amount). This is the ONLY check that catches a wrong strike type — it is
    // load-bearing, not defense in depth.
    let reconstructed_delegated: chia_protocol::Bytes32 =
        underlying.delegated_puzzle().tree_hash().into();
    if reconstructed_delegated != option.info.underlying_delegated_puzzle_hash {
        return Err(Error::invalid(
            "rehydrated terms do not match the option's underlying delegated-puzzle hash — check the strike type, amount, and expiry",
        ));
    }

    // Check 3: the underlying coin id. This binds the coin's full identity (parent + puzzle hash +
    // amount), uniquely rejecting a substituted coin of the right shape but wrong parent that the
    // two hash checks above could not distinguish.
    if underlying_coin.coin_id() != option.info.underlying_coin_id {
        return Err(Error::invalid(
            "underlying coin id does not match the option's committed underlying coin id",
        ));
    }

    Ok(CreatedOption {
        option: *option,
        underlying,
        underlying_coin,
    })
}
