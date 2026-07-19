//! Reconstruct an option from an already-fetched coin spend.
//!
//! dig-options is network-free: the CALLER fetches a coin's puzzle reveal + solution (from a
//! node/indexer) and passes the serialized programs here. [`parse`] decodes an option directly
//! from its own coin spend; [`parse_child`] walks a parent option spend to the option child it
//! created. Both reconstruct into the caller-provided [`SpendContext`].
//!
//! ## Recoverable fields (an honest SDK limitation)
//!
//! An option singleton's on-chain puzzle commits only to its identity fields — the launcher
//! id, the underlying coin id, the underlying delegated-puzzle hash, and the current p2 puzzle
//! hash (via the SDK's `OptionInfo`). The option's *terms* (creator puzzle hash, expiry,
//! underlying amount, strike type) live in the launcher metadata and the underlying coin and
//! are NOT invertible from an option singleton coin spend. [`ParsedOption`] therefore carries
//! the recoverable identity fields; a caller that needs the terms retains the
//! [`crate::CreatedOption`]/[`crate::OptionTerms`] from [`crate::create`].

use chia_protocol::{Bytes32, Coin, Program};
use chia_wallet_sdk::driver::{OptionContract, Puzzle, SpendContext};

use crate::error::Result;

/// A reconstructed option plus the identity fields recoverable from its coin spend.
///
/// The [`ParsedOption::option`] is a spendable [`OptionContract`] in the [`SpendContext`] it
/// was parsed into. See the module docs for why the option's terms are not included.
#[derive(Clone, Debug)]
pub struct ParsedOption {
    /// The reconstructed option singleton.
    pub option: OptionContract,
    /// The launcher coin id — the option's stable identity.
    pub launcher_id: Bytes32,
    /// The current coin id of the unspent option singleton.
    pub coin_id: Bytes32,
    /// The coin id of the locked underlying this option unlocks on exercise.
    pub underlying_coin_id: Bytes32,
    /// The tree hash of the underlying's delegated (settlement) puzzle.
    pub underlying_delegated_puzzle_hash: Bytes32,
    /// The current p2 (owner) puzzle hash — where the option singleton lives.
    pub p2_puzzle_hash: Bytes32,
}

impl ParsedOption {
    fn from_option(option: OptionContract) -> Self {
        Self {
            launcher_id: option.info.launcher_id,
            coin_id: option.coin.coin_id(),
            underlying_coin_id: option.info.underlying_coin_id,
            underlying_delegated_puzzle_hash: option.info.underlying_delegated_puzzle_hash,
            p2_puzzle_hash: option.info.p2_puzzle_hash,
            option,
        }
    }
}

/// Decode an option directly from its own coin spend (its coin, serialized puzzle reveal, and
/// solution). Returns `Ok(None)` when the puzzle is not an option contract.
pub fn parse(
    ctx: &mut SpendContext,
    coin: Coin,
    puzzle_reveal: &Program,
    solution: &Program,
) -> Result<Option<ParsedOption>> {
    let puzzle_ptr = ctx.alloc(puzzle_reveal)?;
    let puzzle = Puzzle::parse(ctx, puzzle_ptr);
    let solution = ctx.alloc(solution)?;
    let parsed = OptionContract::parse(ctx, coin, puzzle, solution)?;
    Ok(parsed.map(|(option, _p2_puzzle, _p2_solution)| ParsedOption::from_option(option)))
}

/// Reconstruct the option child created by spending `parent_coin`, given that parent's
/// serialized puzzle reveal and solution (both fetched by the caller).
///
/// Returns `Ok(None)` when the parent spend did not produce an option child (its puzzle is not
/// an option contract).
pub fn parse_child(
    ctx: &mut SpendContext,
    parent_coin: Coin,
    parent_puzzle_reveal: &Program,
    parent_solution: &Program,
) -> Result<Option<ParsedOption>> {
    let puzzle_ptr = ctx.alloc(parent_puzzle_reveal)?;
    let puzzle = Puzzle::parse(ctx, puzzle_ptr);
    let solution = ctx.alloc(parent_solution)?;
    let child = OptionContract::parse_child(ctx, parent_coin, puzzle, solution)?;
    Ok(child.map(ParsedOption::from_option))
}
