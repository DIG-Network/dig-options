//! Claw back an expired option — the creator reclaims the locked XCH underlying.
//!
//! After expiry the holder can no longer exercise, so [`clawback`] lets the creator recover
//! the locked underlying to `created.underlying.creator_puzzle_hash` via the underlying's
//! clawback path (valid only strictly after `expiry_seconds`, enforced by the consensus).
//! The creator's inner spend is authorized through the caller's [`Owner`] layer.

use chia_wallet_sdk::driver::{SpendContext, SpendWithConditions};
use chia_puzzle_types::Memos;
use chia_wallet_sdk::types::Conditions;

use crate::error::{Error, Result};
use crate::types::{CreatedOption, OptionSpend, Owner};

/// Build the unsigned coin spends that CLAW BACK `created`'s locked XCH underlying to its
/// creator, authorized by `creator`.
///
/// Recovers exactly `created.underlying_coin.amount` mojos to
/// `created.underlying.creator_puzzle_hash` through the underlying's clawback path — valid
/// only AFTER `created.underlying.seconds` (the holder had until expiry to exercise). A
/// [`Owner::Standard`] `creator` whose puzzle hash does not match the option's creator is
/// rejected up front; a [`Owner::Custom`] creator cannot be checked here and relies on the
/// consensus to reject a wrong-party spend.
///
/// **Pure: does NOT sign or broadcast.** Returns [`OptionSpend`] with `created: None`.
pub fn clawback(
    ctx: &mut SpendContext,
    creator: &Owner,
    created: &CreatedOption,
) -> Result<OptionSpend> {
    if let Some(puzzle_hash) = creator.standard_puzzle_hash() {
        if puzzle_hash != created.underlying.creator_puzzle_hash {
            return Err(Error::invalid(
                "clawback owner does not match the option's creator puzzle hash",
            ));
        }
    }

    // The creator recovers the locked underlying to its own (creator) puzzle hash.
    let inner = creator.spend_with_conditions(
        ctx,
        Conditions::new().create_coin(
            created.underlying.creator_puzzle_hash,
            created.underlying_coin.amount,
            Memos::None,
        ),
    )?;

    created
        .underlying
        .clawback_coin_spend(ctx, created.underlying_coin, inner)?;

    Ok(OptionSpend {
        coin_spends: ctx.take(),
        created: None,
    })
}
