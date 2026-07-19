//! Create a covered option — lock an XCH underlying and mint the option singleton.
//!
//! [`create`] funds two coins from one `funding_coin` spend: the locked-underlying XCH coin
//! (at the option's 1-of-2 exercise/clawback path) and the option singleton launcher. The
//! funding coin is spent through the caller's [`Owner`] layer; dig-options never holds the
//! key that authorizes it. The returned [`CreatedOption`] is what the caller keeps to later
//! exercise or claw back the option.

use chia_protocol::Coin;
use chia_puzzle_types::Memos;
use chia_wallet_sdk::driver::{
    OptionLauncher, OptionLauncherInfo, OptionType, SpendContext, SpendWithConditions,
};

use crate::error::{Error, Result};
use crate::types::{CreatedOption, OptionSpend, OptionTerms, Owner};

/// Build the unsigned coin spends that CREATE an option per `terms`, funded from
/// `funding_coin` and authorized by `creator`.
///
/// Locks `terms.underlying_amount` mojos of XCH as the underlying and mints the option
/// singleton to `terms.owner_puzzle_hash`, exercisable for `terms.strike_type` until
/// `terms.expiry_seconds`. `funding_coin` must hold at least `underlying_amount + 1` mojos
/// (the underlying plus the 1-mojo singleton); any excess is left as an implicit fee.
///
/// The returned [`OptionSpend::created`] is `Some`: retain it to exercise or claw back the
/// option once this spend is confirmed. **Pure: does NOT sign or broadcast** — `creator`
/// authorizes the funding-coin spend; the caller signs the reported messages
/// ([`crate::required_signatures`]).
pub fn create(
    ctx: &mut SpendContext,
    creator: &Owner,
    funding_coin: Coin,
    terms: &OptionTerms,
) -> Result<OptionSpend> {
    if terms.underlying_amount == 0 {
        return Err(Error::invalid(
            "option underlying amount must be greater than zero",
        ));
    }
    // v0.1.0 exercise supports only an XCH strike, so minting a non-XCH-strike option would
    // create an option no holder could ever exercise (an asymmetric loss — the creator claws it
    // back after expiry). Keep create/exercise support symmetric by rejecting it up front, with
    // the same message shape as the exercise guard. CAT/NFT strike lands with the follow-up.
    if !matches!(terms.strike_type, OptionType::Xch { .. }) {
        return Err(Error::invalid(
            "CAT/NFT strike exercise not yet supported — see dig-options CAT/NFT follow-up",
        ));
    }
    let needed = terms.underlying_amount.checked_add(1).ok_or_else(|| {
        Error::invalid("underlying amount overflows the 1-mojo singleton addition")
    })?;
    if funding_coin.amount < needed {
        return Err(Error::invalid(format!(
            "funding coin amount {} is too small: need {needed} (underlying {} + 1 mojo singleton)",
            funding_coin.amount, terms.underlying_amount
        )));
    }

    // Build the launcher off the funding coin. The launcher's 1-mojo coin becomes the option
    // singleton; the terms name the creator (clawback path) and owner (holder) puzzle hashes.
    let launcher = OptionLauncher::new(
        ctx,
        funding_coin.coin_id(),
        OptionLauncherInfo::new(
            terms.creator_puzzle_hash,
            terms.owner_puzzle_hash,
            terms.expiry_seconds,
            terms.underlying_amount,
            terms.strike_type,
        ),
        1,
    )?;

    let underlying = launcher.underlying();
    let p2_option = launcher.p2_puzzle_hash();

    // Lock the underlying XCH at the option's 1-of-2 path AND create the launcher coin, both
    // funded by the single funding-coin spend.
    let underlying_coin = Coin::new(funding_coin.coin_id(), p2_option, terms.underlying_amount);
    let launcher = launcher.with_underlying(underlying_coin.coin_id());
    let (mint_conditions, option) = launcher.mint(ctx)?;

    let conditions = mint_conditions.create_coin(p2_option, terms.underlying_amount, Memos::None);
    let inner_spend = creator.spend_with_conditions(ctx, conditions)?;
    ctx.spend(funding_coin, inner_spend)?;

    Ok(OptionSpend {
        coin_spends: ctx.take(),
        created: Some(CreatedOption {
            option,
            underlying,
            underlying_coin,
        }),
    })
}
