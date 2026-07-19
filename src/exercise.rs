//! Exercise an option — pay the strike and unlock the underlying to the holder.
//!
//! [`exercise`] builds the complete exercise in one bundle: it spends the option singleton
//! through its exercise path, unlocks the locked XCH underlying to the holder, and — for an
//! XCH strike — pays the strike into the settlement puzzle and settles it to the creator's
//! requested payment. The holder authorizes both the singleton spend and the strike-funding
//! spend through the caller's [`Owner`] layer. The exercise is valid only strictly before
//! `expiry_seconds` (enforced by the consensus).

use chia_protocol::Coin;
use chia_puzzle_types::offer::{NotarizedPayment, Payment, SettlementPaymentsSolution};
use chia_puzzle_types::Memos;
use chia_puzzles::SETTLEMENT_PAYMENT_HASH;
use chia_wallet_sdk::driver::{
    Layer, OptionType, SettlementLayer, SingletonInfo, SpendContext, SpendWithConditions,
};
use chia_wallet_sdk::types::Conditions;

use crate::error::{Error, Result};
use crate::types::{CreatedOption, OptionSpend, Owner};

/// The strike payment funding a [`exercise`]: the caller-supplied XCH coin the holder spends
/// to pay the strike into the settlement puzzle.
///
/// It must hold at least the strike amount (`created.underlying.strike_type.amount()`); any
/// excess is left as an implicit fee.
#[derive(Clone, Copy, Debug)]
pub struct StrikePayment {
    /// The XCH coin the holder spends to fund the strike payment.
    pub funding_coin: Coin,
}

/// Build the unsigned coin spends that EXERCISE `created` by its `holder`, paying `strike`.
///
/// Spends the option singleton through its exercise path, unlocks the locked XCH underlying to
/// the holder, and pays the XCH strike into the settlement puzzle — settled to the creator's
/// requested payment — all in one bundle. Rejects a `strike.funding_coin` smaller than the
/// strike amount.
///
/// **v0.1.0 scope:** only an XCH strike can be exercised. A CAT/NFT strike returns
/// [`Error::InvalidInput`] rather than emitting an incorrect spend — building the CAT/NFT
/// settlement leg is a documented follow-up.
///
/// **Pure: does NOT sign or broadcast.** Returns [`OptionSpend`] with `created: None`.
pub fn exercise(
    ctx: &mut SpendContext,
    holder: &Owner,
    created: &CreatedOption,
    strike: &StrikePayment,
) -> Result<OptionSpend> {
    let OptionType::Xch {
        amount: strike_amount,
    } = created.underlying.strike_type
    else {
        return Err(Error::invalid(
            "CAT/NFT strike exercise not yet supported — see dig-options CAT/NFT follow-up",
        ));
    };

    if strike.funding_coin.amount < strike_amount {
        return Err(Error::invalid(format!(
            "strike funding coin amount {} is too small: need {strike_amount} for the XCH strike",
            strike.funding_coin.amount
        )));
    }

    // Spend the option singleton through its exercise path (the holder authorizes it).
    // `OptionContract` is `Copy`, so this copies rather than moves `created.option`.
    created.option.exercise(ctx, holder, Conditions::new())?;

    // Unlock the locked underlying. The underlying's exercise-path delegated puzzle emits
    // `CreateCoin(SETTLEMENT_PAYMENT_HASH, underlying_amount)` — the unlocked XCH lands on a BARE
    // settlement/offer coin (spendable by anyone with a `SettlementPaymentsSolution`, no key). The
    // `inner_puzzle_hash` argument here is ONLY the singleton co-spend proof (a `SingletonMember`
    // check), NOT a payout destination. We MUST claim that settlement coin to the holder in this
    // same bundle — otherwise the underlying is stranded and any mempool watcher steals it while
    // the holder has paid the strike and received nothing.
    created.underlying.exercise_coin_spend(
        ctx,
        created.underlying_coin,
        created.option.info.inner_puzzle_hash().into(),
        created.option.coin.amount,
    )?;

    // Claim the unlocked-underlying settlement coin to the holder (the option's current p2 owner),
    // paying the full underlying amount, in the same bundle. Mirrors the strike leg below.
    let underlying_settlement_coin = Coin::new(
        created.underlying_coin.coin_id(),
        SETTLEMENT_PAYMENT_HASH.into(),
        created.underlying.amount,
    );
    let holder_payment = NotarizedPayment::new(
        created.option.info.launcher_id,
        vec![Payment::new(
            created.option.info.p2_puzzle_hash,
            created.underlying.amount,
            Memos::None,
        )],
    );
    let underlying_claim = SettlementLayer.construct_coin_spend(
        ctx,
        underlying_settlement_coin,
        SettlementPaymentsSolution::new(vec![holder_payment]),
    )?;
    ctx.insert(underlying_claim);

    // Pay the XCH strike into the settlement puzzle, then settle it to the creator's
    // requested payment. The holder authorizes the strike-funding spend.
    let strike_inner = holder.spend_with_conditions(
        ctx,
        Conditions::new().create_coin(SETTLEMENT_PAYMENT_HASH.into(), strike_amount, Memos::None),
    )?;
    ctx.spend(strike.funding_coin, strike_inner)?;

    let settlement_coin = Coin::new(
        strike.funding_coin.coin_id(),
        SETTLEMENT_PAYMENT_HASH.into(),
        strike_amount,
    );
    let payment = created.underlying.requested_payment(&mut **ctx)?;
    let coin_spend = SettlementLayer.construct_coin_spend(
        ctx,
        settlement_coin,
        SettlementPaymentsSolution::new(vec![payment]),
    )?;
    ctx.insert(coin_spend);

    Ok(OptionSpend {
        coin_spends: ctx.take(),
        created: None,
    })
}
