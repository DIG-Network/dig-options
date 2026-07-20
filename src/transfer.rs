//! Transfer an option — move the option singleton to a new owner.
//!
//! [`transfer`] spends the option singleton through the current owner's p2 layer and recreates
//! it at `new_owner_puzzle_hash` (hinted, so the new owner's wallet discovers it). Only the
//! option ticket moves; the locked underlying and its terms are untouched. The returned
//! [`OptionSpend`] carries the transferred option as its `created` handle so the caller can
//! immediately track — and later exercise or claw back — the option in its new-owner state.

use chia_protocol::Bytes32;
use chia_wallet_sdk::driver::SpendContext;
use chia_wallet_sdk::types::Conditions;

use crate::error::{Error, Result};
use crate::types::{CreatedOption, OptionSpend, Owner};

/// Build the unsigned coin spend that TRANSFERS `created`'s option singleton to
/// `new_owner_puzzle_hash`, authorized by its current `owner`.
///
/// Spends the option singleton through `owner`'s p2 layer and recreates it (same launcher id,
/// same underlying, same amount) at `new_owner_puzzle_hash` with a hint so the recipient's
/// wallet discovers it. The locked underlying coin and the option's terms are unchanged — only
/// the ticket's `p2_puzzle_hash` moves.
///
/// A [`Owner::Standard`] `owner` whose puzzle hash does not match the option's current
/// `p2_puzzle_hash` is rejected up front; a [`Owner::Custom`] owner cannot be checked here and
/// relies on the consensus to reject a wrong-party spend.
///
/// **Pure: does NOT sign or broadcast.** Returns [`OptionSpend`] with `created: Some(..)` — the
/// option in its NEW-owner state (its coin now lives at `new_owner_puzzle_hash`), so the caller
/// can chain further transfers or an exercise/clawback against the transferred singleton once
/// this spend is confirmed.
pub fn transfer(
    ctx: &mut SpendContext,
    owner: &Owner,
    created: &CreatedOption,
    new_owner_puzzle_hash: Bytes32,
) -> Result<OptionSpend> {
    if let Some(puzzle_hash) = owner.standard_puzzle_hash() {
        if puzzle_hash != created.option.info.p2_puzzle_hash {
            return Err(Error::invalid(
                "transfer owner does not match the option's current owner puzzle hash",
            ));
        }
    }

    // `OptionContract` is `Copy`; `transfer` consumes it and returns the recreated child at the
    // new p2 puzzle hash (hinted for wallet discovery via the SDK). Only the singleton moves —
    // the underlying stays locked under the same terms.
    let transferred =
        created
            .option
            .transfer(ctx, owner, new_owner_puzzle_hash, Conditions::new())?;

    Ok(OptionSpend {
        coin_spends: ctx.take(),
        created: Some(CreatedOption {
            option: transferred,
            underlying: created.underlying,
            underlying_coin: created.underlying_coin,
        }),
    })
}
