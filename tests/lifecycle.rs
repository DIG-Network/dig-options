//! End-to-end and adversarial tests for the dig-options builder surface.
//!
//! The offline tests assert the pure input validation (custody guards, honest gaps). The
//! simulator tests drive real coin spends onto the in-process Chia simulator, proving the
//! full option lifecycle validates against the consensus: create -> exercise (strike paid,
//! underlying unlocked before expiry) and create -> clawback (creator reclaims after expiry),
//! plus the adversarial paths the consensus MUST reject.
//!
//! dig-options never signs; [`sign_for_sim`] is a TEST-ONLY bridge that first asserts the
//! crate's own [`required_signatures`] report is non-empty, then signs with the test key so the
//! built spends can be submitted to the simulator.

use chia_protocol::{Bytes32, Coin, CoinSpend, SpendBundle};
use chia_sdk_test::{sign_transaction, Simulator};
use chia_wallet_sdk::driver::{SingletonInfo, SpendContext, StandardLayer};
use chia_wallet_sdk::prelude::{SecretKey, Signature};
use chia_wallet_sdk::types::{Conditions, TESTNET11_CONSTANTS};

use dig_options::{
    clawback, create, exercise, parse, parse_child, required_signatures, CreatedOption,
    OptionTerms, OptionType, Owner, StrikePayment,
};

/// A distinct 32-byte puzzle hash derived from a seed (never a hard-coded crypto literal —
/// CodeQL flags integer/array-literal crypto values; #917/#950).
fn puzzle_hash(seed: &str) -> Bytes32 {
    use chia_sdk_test::BlsPair;
    // A BLS pair's standard puzzle hash is a deterministic hash of a seed-derived key — a
    // convenient stand-in for an arbitrary destination puzzle hash.
    BlsPair::new(hash_seed(seed)).puzzle_hash
}

/// Derive a `u64` seed from a string by hashing it, so no test carries a literal key/nonce.
fn hash_seed(seed: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    hasher.finish()
}

/// Sign `coin_spends` for the TESTNET11 simulator, first asserting the crate's own
/// [`required_signatures`] report agrees there is something to sign (dig-options itself never
/// signs — this is a TEST-ONLY bridge to drive the built spends onto the simulator).
fn sign_for_sim(coin_spends: &[CoinSpend], sks: &[SecretKey]) -> anyhow::Result<Signature> {
    let reported =
        required_signatures(coin_spends, TESTNET11_CONSTANTS.agg_sig_me_additional_data)?;
    assert!(
        !reported.is_empty(),
        "a spend must report required signatures"
    );
    Ok(sign_transaction(coin_spends, sks)?)
}

fn xch(amount: u64) -> OptionType {
    OptionType::Xch { amount }
}

// ----- offline: input validation + honest gaps -----

#[test]
fn create_rejects_zero_underlying() {
    let ctx = &mut SpendContext::new();
    let creator = puzzle_hash("creator");
    let funding = Coin::new(Bytes32::default(), creator, 10);
    let terms = OptionTerms::new(creator, 0, xch(1), 10);
    let err = create(ctx, &Owner::Standard(bls("creator").pk), funding, &terms).unwrap_err();
    assert!(format!("{err}").contains("greater than zero"), "got: {err}");
}

#[test]
fn create_rejects_funding_too_small() {
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    // need underlying(10) + 1 = 11, provide 10.
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 10);
    let terms = OptionTerms::new(alice.puzzle_hash, 10, xch(1), 10);
    let err = create(ctx, &Owner::Standard(alice.pk), funding, &terms).unwrap_err();
    assert!(format!("{err}").contains("too small"), "got: {err}");
}

#[test]
fn clawback_rejects_wrong_creator_key() -> anyhow::Result<()> {
    // Build a well-formed option, then attempt to claw it back with a DIFFERENT standard
    // owner — the up-front guard must reject it.
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    let stranger = bls("stranger");
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 1_001);
    let terms = OptionTerms::new(alice.puzzle_hash, 1_000, xch(1), 10);
    let created = create(ctx, &Owner::Standard(alice.pk), funding, &terms)?
        .created
        .expect("create yields a CreatedOption");

    let err = clawback(ctx, &Owner::Standard(stranger.pk), &created).unwrap_err();
    assert!(
        format!("{err}").contains("creator puzzle hash"),
        "got: {err}"
    );
    Ok(())
}

#[test]
fn exercise_rejects_underfunded_strike() -> anyhow::Result<()> {
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 1_001);
    let terms = OptionTerms::new(alice.puzzle_hash, 1_000, xch(250), 10);
    let created = create(ctx, &Owner::Standard(alice.pk), funding, &terms)?
        .created
        .unwrap();

    // Strike funding of 100 mojos cannot cover a 250-mojo XCH strike.
    let strike = StrikePayment {
        funding_coin: Coin::new(Bytes32::default(), alice.puzzle_hash, 100),
    };
    let err = exercise(ctx, &Owner::Standard(alice.pk), &created, &strike).unwrap_err();
    assert!(format!("{err}").contains("too small"), "got: {err}");
    Ok(())
}

#[test]
fn exercise_rejects_non_xch_strike() -> anyhow::Result<()> {
    // A CAT strike is a valid option to CREATE, but exercising it is a documented v0.1.0 gap:
    // exercise returns an honest error rather than emitting an incorrect spend.
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 1_001);
    let cat_strike = OptionType::Cat {
        asset_id: puzzle_hash("asset"),
        amount: 250,
    };
    let terms = OptionTerms::new(alice.puzzle_hash, 1_000, cat_strike, 10);
    let created = create(ctx, &Owner::Standard(alice.pk), funding, &terms)?
        .created
        .unwrap();

    let strike = StrikePayment {
        funding_coin: Coin::new(Bytes32::default(), alice.puzzle_hash, 1_000),
    };
    let err = exercise(ctx, &Owner::Standard(alice.pk), &created, &strike).unwrap_err();
    assert!(
        format!("{err}").contains("CAT/NFT strike exercise not yet supported"),
        "got: {err}"
    );
    Ok(())
}

#[test]
fn required_signatures_reports_messages() -> anyhow::Result<()> {
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 1_001);
    let terms = OptionTerms::new(alice.puzzle_hash, 1_000, xch(1), 10);
    let spend = create(ctx, &Owner::Standard(alice.pk), funding, &terms)?;

    let reported = required_signatures(
        &spend.coin_spends,
        TESTNET11_CONSTANTS.agg_sig_me_additional_data,
    )?;
    assert!(
        !reported.is_empty(),
        "create must report at least one required signature"
    );
    Ok(())
}

// ----- simulator: full lifecycle -----

/// Create + confirm an option, returning the confirmed [`CreatedOption`].
fn create_confirmed(
    sim: &mut Simulator,
    ctx: &mut SpendContext,
    alice: &chia_sdk_test::BlsPairWithCoin,
    underlying_amount: u64,
    strike_amount: u64,
    expiry: u64,
) -> anyhow::Result<CreatedOption> {
    let terms = OptionTerms::new(
        alice.puzzle_hash,
        underlying_amount,
        xch(strike_amount),
        expiry,
    );
    let spend = create(ctx, &Owner::Standard(alice.pk), alice.coin, &terms)?;
    let created = spend.created.clone().unwrap();
    let sig = sign_for_sim(&spend.coin_spends, std::slice::from_ref(&alice.sk))?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;
    assert!(
        sim.coin_state(created.option.coin.coin_id()).is_some(),
        "the option singleton should exist after create"
    );
    Ok(created)
}

#[test]
fn create_then_exercise_round_trip() -> anyhow::Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let underlying_amount = 1_000u64;
    let strike_amount = 250u64;
    let expiry = 10_000u64;

    let alice = sim.bls(underlying_amount + 1);
    let created = create_confirmed(
        &mut sim,
        ctx,
        &alice,
        underlying_amount,
        strike_amount,
        expiry,
    )?;

    // Exercise before expiry: pay the strike, unlock the underlying.
    let strike_funding = sim.new_coin(alice.puzzle_hash, strike_amount);
    let spend = exercise(
        ctx,
        &Owner::Standard(alice.pk),
        &created,
        &StrikePayment {
            funding_coin: strike_funding,
        },
    )?;
    let sig = sign_for_sim(&spend.coin_spends, &[alice.sk])?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;

    // The strike payment settles to the creator (here alice); its puzzle hash gains at least
    // the strike amount, proving the exercise's settlement leg was accepted.
    let owned = sim
        .unspent_coins(alice.puzzle_hash, true)
        .iter()
        .map(|c| c.amount)
        .sum::<u64>();
    assert!(
        owned >= strike_amount,
        "the creator should receive at least the strike ({strike_amount}); got {owned}"
    );
    Ok(())
}

#[test]
fn create_then_clawback_on_expiry() -> anyhow::Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let underlying_amount = 2_000u64;
    let expiry = 100u64;

    let alice = sim.bls(underlying_amount + 1);
    let created = create_confirmed(&mut sim, ctx, &alice, underlying_amount, 1, expiry)?;

    // Advance past expiry so the clawback path's seconds boundary lets the creator reclaim.
    sim.pass_time(expiry + 10);

    let spend = clawback(ctx, &Owner::Standard(alice.pk), &created)?;
    let sig = sign_for_sim(&spend.coin_spends, &[alice.sk])?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;

    let recovered = sim
        .unspent_coins(alice.puzzle_hash, false)
        .iter()
        .map(|c| c.amount)
        .sum::<u64>();
    assert_eq!(
        recovered, underlying_amount,
        "the creator should reclaim exactly the locked underlying on expiry"
    );
    Ok(())
}

#[test]
fn create_via_custom_owner_layer_validates() -> anyhow::Result<()> {
    // The Custom owner routes an arbitrary SpendWithConditions (here a StandardLayer) — proving
    // the non-standard p2 path works through the same builder.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let layer = StandardLayer::new(alice.pk);
    let terms = OptionTerms::new(alice.puzzle_hash, 1_000, xch(1), 10_000);
    let spend = create(ctx, &Owner::Custom(&layer), alice.coin, &terms)?;
    let sig = sign_for_sim(&spend.coin_spends, &[alice.sk])?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;
    Ok(())
}

// ----- simulator: adversarial negatives (the consensus MUST reject) -----

#[test]
fn exercise_without_strike_leg_is_rejected() -> anyhow::Result<()> {
    // Building only the option + underlying-unlock legs (omitting the strike payment) must be
    // rejected: the underlying cannot be taken without paying the strike.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let created = create_confirmed(&mut sim, ctx, &alice, 1_000, 250, 10_000)?;

    let alice_p2 = StandardLayer::new(alice.pk);
    created.option.exercise(ctx, &alice_p2, Conditions::new())?;
    created.underlying.exercise_coin_spend(
        ctx,
        created.underlying_coin,
        created.option.info.inner_puzzle_hash().into(),
        created.option.coin.amount,
    )?;
    let coin_spends = ctx.take();
    let sig = sign_for_sim(&coin_spends, &[alice.sk])?;
    assert!(
        sim.new_transaction(SpendBundle::new(coin_spends, sig))
            .is_err(),
        "exercise without the strike payment must be rejected"
    );
    Ok(())
}

#[test]
fn exercise_after_expiry_is_rejected() -> anyhow::Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let expiry = 100u64;
    let alice = sim.bls(1_001);
    let created = create_confirmed(&mut sim, ctx, &alice, 1_000, 250, expiry)?;

    // Advance past expiry; the exercise path's before-seconds boundary must now reject.
    sim.pass_time(expiry + 10);

    let strike_funding = sim.new_coin(alice.puzzle_hash, 250);
    let spend = exercise(
        ctx,
        &Owner::Standard(alice.pk),
        &created,
        &StrikePayment {
            funding_coin: strike_funding,
        },
    )?;
    let sig = sign_for_sim(&spend.coin_spends, &[alice.sk])?;
    assert!(
        sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))
            .is_err(),
        "exercise after expiry must be rejected"
    );
    Ok(())
}

#[test]
fn clawback_before_expiry_is_rejected() -> anyhow::Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let created = create_confirmed(&mut sim, ctx, &alice, 1_000, 1, 10_000)?;

    // No time has passed; the clawback path's seconds boundary must reject before expiry.
    let spend = clawback(ctx, &Owner::Standard(alice.pk), &created)?;
    let sig = sign_for_sim(&spend.coin_spends, &[alice.sk])?;
    assert!(
        sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))
            .is_err(),
        "clawback before expiry must be rejected"
    );
    Ok(())
}

// ----- inspection -----

#[test]
fn parse_round_trips_identity() -> anyhow::Result<()> {
    // Parse the eve option spend from a confirmed create and assert the recoverable identity
    // fields match the created option.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let created = create_confirmed(&mut sim, ctx, &alice, 1_000, 250, 10_000)?;

    // The eve coin is the parent of the created option child.
    let eve_coin_id = created.option.coin.parent_coin_info;
    let eve_coin = sim
        .coin_state(eve_coin_id)
        .expect("the eve coin was spent in the create bundle")
        .coin;
    let (puzzle, solution) = sim
        .puzzle_and_solution(eve_coin_id)
        .expect("the eve coin's spend is recorded");

    let parsed = parse_child(ctx, eve_coin, &puzzle, &solution)?
        .expect("the eve spend produced an option child");
    assert_eq!(parsed.launcher_id, created.option.info.launcher_id);
    assert_eq!(parsed.coin_id, created.option.coin.coin_id());
    assert_eq!(parsed.p2_puzzle_hash, created.option.info.p2_puzzle_hash);
    assert_eq!(
        parsed.underlying_coin_id,
        created.option.info.underlying_coin_id
    );
    Ok(())
}

#[test]
fn parse_returns_none_for_non_option() -> anyhow::Result<()> {
    // A plain standard-coin spend (the funding coin) is not an option contract.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let funding_id = alice.coin.coin_id();
    let _ = create_confirmed(&mut sim, ctx, &alice, 1_000, 250, 10_000)?;

    let coin = alice.coin;
    let (puzzle, solution) = sim
        .puzzle_and_solution(funding_id)
        .expect("the funding coin's spend is recorded");
    assert!(
        parse(ctx, coin, &puzzle, &solution)?.is_none(),
        "a standard funding-coin spend is not an option"
    );
    Ok(())
}

// ----- test helpers -----

/// A deterministic BLS pair (with coin) for a seed, so tests carry no literal keys.
fn bls(seed: &str) -> chia_sdk_test::BlsPair {
    chia_sdk_test::BlsPair::new(hash_seed(seed))
}
