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
use chia_puzzles::SETTLEMENT_PAYMENT_HASH;
use chia_sdk_test::{sign_transaction, Simulator};
use chia_wallet_sdk::driver::{SingletonInfo, SpendContext, StandardLayer};
use chia_wallet_sdk::prelude::{SecretKey, Signature};
use chia_wallet_sdk::types::{Conditions, TESTNET11_CONSTANTS};

use dig_options::{
    clawback, create, exercise, parse, parse_child, parse_metadata, rehydrate, required_signatures,
    transfer, CreatedOption, OptionTerms, OptionType, Owner, RehydratedTerms, StrikePayment,
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
    // Defense-in-depth: even if a CAT-strike option somehow reaches `exercise` (create now
    // rejects it up front — see `create_rejects_non_xch_strike`), the exercise guard still
    // returns an honest error rather than emitting an incorrect spend. Build a well-formed XCH
    // option, then flip its strike type to CAT to drive the guard directly.
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 1_001);
    let terms = OptionTerms::new(alice.puzzle_hash, 1_000, xch(250), 10);
    let mut created = create(ctx, &Owner::Standard(alice.pk), funding, &terms)?
        .created
        .unwrap();
    created.underlying.strike_type = OptionType::Cat {
        asset_id: puzzle_hash("asset"),
        amount: 250,
    };

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
fn create_rejects_non_xch_strike() -> anyhow::Result<()> {
    // v0.1.0 create is XCH-strike-only: a non-XCH strike is rejected up front so no holder can
    // acquire an option they could never exercise (create/exercise support envelopes are
    // symmetric). CAT/NFT strikes land with the CAT/NFT follow-up.
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 1_001);
    let cat_strike = OptionType::Cat {
        asset_id: puzzle_hash("asset"),
        amount: 250,
    };
    let terms = OptionTerms::new(alice.puzzle_hash, 1_000, cat_strike, 10);
    let err = create(ctx, &Owner::Standard(alice.pk), funding, &terms).unwrap_err();
    assert!(format!("{err}").contains("CAT/NFT strike"), "got: {err}");
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

/// Create + confirm an option from arbitrary `terms` (funded + authorized by `creator`),
/// allowing a holder distinct from the creator.
fn create_confirmed_terms(
    sim: &mut Simulator,
    ctx: &mut SpendContext,
    creator: &chia_sdk_test::BlsPairWithCoin,
    terms: &OptionTerms,
) -> anyhow::Result<CreatedOption> {
    let spend = create(ctx, &Owner::Standard(creator.pk), creator.coin, terms)?;
    let created = spend.created.clone().unwrap();
    let sig = sign_for_sim(&spend.coin_spends, std::slice::from_ref(&creator.sk))?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;
    assert!(
        sim.coin_state(created.option.coin.coin_id()).is_some(),
        "the option singleton should exist after create"
    );
    Ok(created)
}

/// Total unspent value at `puzzle_hash` (hinted + unhinted coins).
fn balance(sim: &Simulator, puzzle_hash: Bytes32) -> u64 {
    sim.unspent_coins(puzzle_hash, false)
        .iter()
        .map(|c| c.amount)
        .sum()
}

#[test]
fn create_then_exercise_round_trip() -> anyhow::Result<()> {
    // A covered option minted by the CREATOR (alice) but OWNED by a DISTINCT holder (bob). On
    // exercise both legs must land, and value must be conserved for BOTH parties:
    //   - the HOLDER receives EXACTLY the underlying amount (the claimed underlying leg), and
    //   - the CREATOR receives the strike amount (the strike-settlement leg).
    // A single-key create/exercise would mask a stranded underlying (the holder never actually
    // receives it), so the two parties are kept distinct here on purpose.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let underlying_amount = 1_000u64;
    let strike_amount = 250u64;
    let expiry = 10_000u64;

    // Creator funds the create; the option is minted to a distinct holder.
    let alice = sim.bls(underlying_amount + 1);
    let bob = bls("holder");
    let terms = OptionTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        owner_puzzle_hash: bob.puzzle_hash,
        underlying_amount,
        strike_type: xch(strike_amount),
        expiry_seconds: expiry,
    };
    let created = create_confirmed_terms(&mut sim, ctx, &alice, &terms)?;

    // Exercise before expiry, authorized by the HOLDER (bob owns the option singleton). Bob
    // funds the strike from his own coin. Snapshot balances AFTER funding so the strike coin is
    // in the holder baseline — his net becomes exactly (underlying received - strike paid).
    let strike_funding = sim.new_coin(bob.puzzle_hash, strike_amount);
    let creator_before = balance(&sim, alice.puzzle_hash);
    let holder_before = balance(&sim, bob.puzzle_hash);

    let spend = exercise(
        ctx,
        &Owner::Standard(bob.pk),
        &created,
        &StrikePayment {
            funding_coin: strike_funding,
        },
    )?;
    let sig = sign_for_sim(&spend.coin_spends, &[bob.sk])?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;

    // The holder receives EXACTLY the underlying (minus the strike he funded from his own coin),
    // and the creator receives the strike — value conserved for both parties.
    let creator_gain = balance(&sim, alice.puzzle_hash) - creator_before;
    let holder_gain = balance(&sim, bob.puzzle_hash) as i128 - holder_before as i128;
    assert_eq!(
        creator_gain, strike_amount,
        "the creator should receive exactly the strike ({strike_amount})"
    );
    assert_eq!(
        holder_gain,
        underlying_amount as i128 - strike_amount as i128,
        "the holder should net exactly underlying - strike (received {underlying_amount}, paid {strike_amount})"
    );
    Ok(())
}

#[test]
fn exercise_leaves_no_orphan_underlying_settlement_coin() -> anyhow::Result<()> {
    // A third party MUST NOT be able to strand-then-steal the underlying: after a complete
    // exercise, the underlying must already be claimed to the holder in the SAME bundle, leaving
    // NO unspent bare settlement coin (SETTLEMENT_PAYMENT_HASH) holding the underlying that a
    // mempool watcher could claim key-free.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let underlying_amount = 1_000u64;
    let strike_amount = 250u64;

    let alice = sim.bls(underlying_amount + 1);
    let bob = bls("holder");
    let terms = OptionTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        owner_puzzle_hash: bob.puzzle_hash,
        underlying_amount,
        strike_type: xch(strike_amount),
        expiry_seconds: 10_000,
    };
    let created = create_confirmed_terms(&mut sim, ctx, &alice, &terms)?;

    let strike_funding = sim.new_coin(bob.puzzle_hash, strike_amount);
    let spend = exercise(
        ctx,
        &Owner::Standard(bob.pk),
        &created,
        &StrikePayment {
            funding_coin: strike_funding,
        },
    )?;
    let sig = sign_for_sim(&spend.coin_spends, &[bob.sk])?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;

    // No unspent coin sits at the bare settlement puzzle hash: both settlement legs were claimed
    // in the exercise bundle, so nothing is left for a key-free thief.
    let orphan = sim
        .unspent_coins(SETTLEMENT_PAYMENT_HASH.into(), false)
        .iter()
        .any(|c| c.amount == underlying_amount || c.amount == strike_amount);
    assert!(
        !orphan,
        "no bare settlement coin holding the underlying/strike may survive a complete exercise"
    );
    // The holder actually holds the underlying value.
    assert!(
        balance(&sim, bob.puzzle_hash) >= underlying_amount - strike_amount,
        "the holder must hold the claimed underlying after exercise"
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

// ----- transfer -----

#[test]
fn transfer_moves_option_then_new_owner_exercises() -> anyhow::Result<()> {
    // Mint an option owned by bob, transfer it to carol, then have CAROL exercise it. The
    // transfer must actually re-home the singleton (new p2 puzzle hash + a confirmed coin) and
    // the transferred handle must be fully operable: carol receives the underlying, and the
    // ORIGINAL creator (alice) still receives the strike. This proves transfer moves only the
    // ticket, not the underlying terms.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let underlying_amount = 1_000u64;
    let strike_amount = 250u64;

    let alice = sim.bls(underlying_amount + 1);
    let bob = bls("holder-bob");
    let carol = bls("holder-carol");
    let terms = OptionTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        owner_puzzle_hash: bob.puzzle_hash,
        underlying_amount,
        strike_type: xch(strike_amount),
        expiry_seconds: 10_000,
    };
    let created = create_confirmed_terms(&mut sim, ctx, &alice, &terms)?;

    // Transfer bob -> carol. The spend must report a signature (bob authorizes it) and produce a
    // handle whose option lives at carol's puzzle hash.
    let spend = transfer(ctx, &Owner::Standard(bob.pk), &created, carol.puzzle_hash)?;
    let transferred = spend
        .created
        .clone()
        .expect("transfer yields the re-homed option");
    assert_eq!(
        transferred.option.info.p2_puzzle_hash, carol.puzzle_hash,
        "the transferred option must live at the new owner's puzzle hash"
    );
    assert_eq!(
        transferred.underlying, created.underlying,
        "transfer must not change the underlying terms"
    );
    let sig = sign_for_sim(&spend.coin_spends, &[bob.sk])?;
    sim.new_transaction(SpendBundle::new(spend.coin_spends, sig))?;
    assert!(
        sim.coin_state(transferred.option.coin.coin_id()).is_some(),
        "the re-homed option singleton must exist after transfer"
    );

    // Carol now exercises the transferred option and receives the underlying; alice gets the strike.
    let strike_funding = sim.new_coin(carol.puzzle_hash, strike_amount);
    let creator_before = balance(&sim, alice.puzzle_hash);
    let holder_before = balance(&sim, carol.puzzle_hash);
    let ex = exercise(
        ctx,
        &Owner::Standard(carol.pk),
        &transferred,
        &StrikePayment {
            funding_coin: strike_funding,
        },
    )?;
    let sig = sign_for_sim(&ex.coin_spends, &[carol.sk])?;
    sim.new_transaction(SpendBundle::new(ex.coin_spends, sig))?;

    assert_eq!(
        balance(&sim, alice.puzzle_hash) - creator_before,
        strike_amount,
        "the original creator receives the strike after a transferred exercise"
    );
    assert_eq!(
        balance(&sim, carol.puzzle_hash) as i128 - holder_before as i128,
        underlying_amount as i128 - strike_amount as i128,
        "the NEW owner nets underlying - strike after exercising the transferred option"
    );
    Ok(())
}

#[test]
fn transfer_rejects_wrong_owner() -> anyhow::Result<()> {
    // A standard owner that is not the option's current holder is rejected up front.
    let ctx = &mut SpendContext::new();
    let alice = bls("alice");
    let bob = bls("holder-bob");
    let stranger = bls("stranger");
    let funding = Coin::new(Bytes32::default(), alice.puzzle_hash, 1_001);
    let terms = OptionTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        owner_puzzle_hash: bob.puzzle_hash,
        underlying_amount: 1_000,
        strike_type: xch(1),
        expiry_seconds: 10,
    };
    let created = create(ctx, &Owner::Standard(alice.pk), funding, &terms)?
        .created
        .unwrap();

    let err = transfer(
        ctx,
        &Owner::Standard(stranger.pk),
        &created,
        alice.puzzle_hash,
    )
    .unwrap_err();
    assert!(format!("{err}").contains("current owner"), "got: {err}");
    Ok(())
}

// ----- rehydrate -----

#[test]
fn rehydrate_recovers_operable_option() -> anyhow::Result<()> {
    // Reconstruct a full CreatedOption purely from on-chain state (as a wallet that did not mint
    // the option would): parse the singleton, recover the metadata from the launcher solution,
    // fetch the underlying coin, rehydrate, and prove the result matches AND is operable by
    // exercising it. Nothing from the original `created` handle feeds the rehydrated terms.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let underlying_amount = 1_000u64;
    let strike_amount = 250u64;
    let expiry = 10_000u64;

    let alice = sim.bls(underlying_amount + 1);
    let bob = bls("holder");
    let terms = OptionTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        owner_puzzle_hash: bob.puzzle_hash,
        underlying_amount,
        strike_type: xch(strike_amount),
        expiry_seconds: expiry,
    };
    let created = create_confirmed_terms(&mut sim, ctx, &alice, &terms)?;

    // 1. Parse the current option singleton from the eve spend (on-chain identity fields only).
    let eve_coin_id = created.option.coin.parent_coin_info;
    let eve_coin = sim.coin_state(eve_coin_id).unwrap().coin;
    let (puzzle, solution) = sim.puzzle_and_solution(eve_coin_id).unwrap();
    let parsed = parse_child(ctx, eve_coin, &puzzle, &solution)?.expect("an option child");

    // 2. Recover expiry + strike from the launcher solution.
    let launcher_id = parsed.launcher_id;
    let (l_puzzle, l_solution) = sim.puzzle_and_solution(launcher_id).unwrap();
    let _ = l_puzzle;
    let metadata = parse_metadata(ctx, &l_solution)?;
    assert_eq!(metadata.expiration_seconds, expiry);
    assert_eq!(metadata.strike_type, xch(strike_amount));

    // 3. Fetch the underlying coin the option commits to.
    let underlying_coin = sim.coin_state(parsed.underlying_coin_id).unwrap().coin;

    // 4. Rehydrate — verified against the on-chain commitments.
    let rehydrated_terms = RehydratedTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        expiry_seconds: metadata.expiration_seconds,
        strike_type: metadata.strike_type,
    };
    let rehydrated = rehydrate(&parsed.option, &rehydrated_terms, underlying_coin)?;
    assert_eq!(
        rehydrated.underlying, created.underlying,
        "rehydrated underlying terms must match the minted option"
    );
    assert_eq!(
        rehydrated.underlying_coin, created.underlying_coin,
        "rehydrated underlying coin must match"
    );

    // 5. Prove it is operable: bob exercises the rehydrated option.
    let strike_funding = sim.new_coin(bob.puzzle_hash, strike_amount);
    let creator_before = balance(&sim, alice.puzzle_hash);
    let ex = exercise(
        ctx,
        &Owner::Standard(bob.pk),
        &rehydrated,
        &StrikePayment {
            funding_coin: strike_funding,
        },
    )?;
    let sig = sign_for_sim(&ex.coin_spends, &[bob.sk])?;
    sim.new_transaction(SpendBundle::new(ex.coin_spends, sig))?;
    assert_eq!(
        balance(&sim, alice.puzzle_hash) - creator_before,
        strike_amount,
        "exercising a rehydrated option pays the creator the strike"
    );
    Ok(())
}

#[test]
fn rehydrate_rejects_wrong_creator() -> anyhow::Result<()> {
    // A wrong creator puzzle hash changes the underlying's 1-of-2 path hash, so it no longer
    // matches the underlying coin — rehydrate rejects it rather than yielding an unspendable handle.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let created = create_confirmed(&mut sim, ctx, &alice, 1_000, 250, 10_000)?;

    let eve_coin_id = created.option.coin.parent_coin_info;
    let eve_coin = sim.coin_state(eve_coin_id).unwrap().coin;
    let (puzzle, solution) = sim.puzzle_and_solution(eve_coin_id).unwrap();
    let parsed = parse_child(ctx, eve_coin, &puzzle, &solution)?.unwrap();
    let underlying_coin = sim.coin_state(parsed.underlying_coin_id).unwrap().coin;

    let wrong = RehydratedTerms {
        creator_puzzle_hash: puzzle_hash("not-the-creator"),
        expiry_seconds: 10_000,
        strike_type: xch(250),
    };
    let err = rehydrate(&parsed.option, &wrong, underlying_coin).unwrap_err();
    assert!(format!("{err}").contains("1-of-2 path"), "got: {err}");
    Ok(())
}

#[test]
fn rehydrate_rejects_wrong_strike() -> anyhow::Result<()> {
    // A wrong strike type is NOT bound by the 1-of-2 path hash (which commits only launcher id +
    // expiry + creator ph); it is caught solely by the delegated-puzzle-hash check. This test pins
    // that check so a refactor cannot silently drop it.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let created = create_confirmed(&mut sim, ctx, &alice, 1_000, 250, 10_000)?;

    let eve_coin_id = created.option.coin.parent_coin_info;
    let eve_coin = sim.coin_state(eve_coin_id).unwrap().coin;
    let (puzzle, solution) = sim.puzzle_and_solution(eve_coin_id).unwrap();
    let parsed = parse_child(ctx, eve_coin, &puzzle, &solution)?.unwrap();
    let underlying_coin = sim.coin_state(parsed.underlying_coin_id).unwrap().coin;

    let wrong = RehydratedTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        expiry_seconds: 10_000,
        strike_type: xch(251), // correct creator + expiry, wrong strike amount
    };
    let err = rehydrate(&parsed.option, &wrong, underlying_coin).unwrap_err();
    assert!(
        format!("{err}").contains("delegated-puzzle hash"),
        "a wrong strike must be caught by the delegated-puzzle-hash check; got: {err}"
    );
    Ok(())
}

#[test]
fn rehydrate_rejects_wrong_amount() -> anyhow::Result<()> {
    // The underlying amount is taken from the supplied coin, so a wrong amount means a coin whose
    // amount does not match the option's committed underlying. The path hash ignores amount, so the
    // reconstruction is rejected by the delegated-puzzle-hash check (which commits the amount) — never
    // silently accepted. Pins that a wrong-amount coin cannot rehydrate.
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();

    let alice = sim.bls(1_001);
    let created = create_confirmed(&mut sim, ctx, &alice, 1_000, 250, 10_000)?;

    let eve_coin_id = created.option.coin.parent_coin_info;
    let eve_coin = sim.coin_state(eve_coin_id).unwrap().coin;
    let (puzzle, solution) = sim.puzzle_and_solution(eve_coin_id).unwrap();
    let parsed = parse_child(ctx, eve_coin, &puzzle, &solution)?.unwrap();
    let real_coin = sim.coin_state(parsed.underlying_coin_id).unwrap().coin;

    // Same parent + puzzle hash as the real underlying, but a different amount.
    let wrong_amount_coin = Coin::new(real_coin.parent_coin_info, real_coin.puzzle_hash, 999);
    let terms = RehydratedTerms {
        creator_puzzle_hash: alice.puzzle_hash,
        expiry_seconds: 10_000,
        strike_type: xch(250),
    };
    let err = rehydrate(&parsed.option, &terms, wrong_amount_coin).unwrap_err();
    assert!(
        format!("{err}").contains("delegated-puzzle hash") || format!("{err}").contains("coin id"),
        "a wrong underlying amount must be rejected; got: {err}"
    );
    Ok(())
}

// ----- test helpers -----

/// A deterministic BLS pair (with coin) for a seed, so tests carry no literal keys.
fn bls(seed: &str) -> chia_sdk_test::BlsPair {
    chia_sdk_test::BlsPair::new(hash_seed(seed))
}
