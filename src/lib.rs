//! # dig-options — the DIG Network canonical Chia option-contract expert crate
//!
//! `dig-options` is a **pure, key-free, network-free** CoinSpend-builder for Chia covered
//! options (the CHIP-0042 option primitive). It constructs the exact
//! [`CoinSpend`](chia_protocol::CoinSpend)s for the option lifecycle — [`create`] (lock an XCH
//! underlying, mint the option singleton), [`exercise`] (pay the strike, unlock the underlying
//! to the holder), [`transfer`] (move the option ticket to a new owner), [`clawback`] (the
//! creator reclaims the underlying after expiry), and inspect ([`parse`]/[`parse_child`]) — and
//! reports the exact signatures a caller must produce ([`required_signatures`]).
//!
//! It also [`rehydrate`]s a previously-minted option: [`parse`] recovers only an option's
//! identity fields, so [`rehydrate`] reconstructs the full operable [`CreatedOption`] from
//! caller-observed terms + the launcher metadata ([`parse_metadata`]) and VERIFIES it against the
//! option's on-chain commitments, letting a caller exercise/transfer/claw back an option it did
//! not mint in the same session.
//!
//! ## The custody model (HARD invariants)
//!
//! dig-options **never holds a secret key, never signs, and never touches the network.** Every
//! builder takes only public inputs (an [`Owner`] carrying a public key or a caller-supplied
//! inner spender, plain [`Bytes32`](chia_protocol::Bytes32) puzzle hashes, coins the caller
//! already fetched) and appends unsigned coin spends to a caller-owned [`SpendContext`]. The
//! consumer signs the messages reported by [`required_signatures`], assembles the
//! `SpendBundle`, and broadcasts. Signing — and the secret key — stay entirely on the caller's
//! side of the identity boundary (#908).
//!
//! ## Scope (v0.1.0)
//!
//! The underlying is **XCH**, and the strike is **XCH-only**: [`create`] REJECTS a non-XCH strike
//! up front so create and [`exercise`] have symmetric support envelopes (no holder can acquire an
//! option it could never exercise). [`exercise`] builds BOTH settlement legs for an XCH strike —
//! the underlying is claimed to the holder and the strike is paid to the creator, in one bundle —
//! and keeps its non-XCH guard as defense-in-depth. [`clawback`] and inspection work for any strike
//! type curried into an existing option. CAT/revocable-CAT/NFT underlyings and strike are a future
//! extension. See `SPEC.md` for the normative contract.

#![forbid(unsafe_code)]

mod clawback;
mod create;
mod error;
mod exercise;
mod hydrate;
mod rehydrate;
mod sign;
mod transfer;
mod types;

pub use clawback::clawback;
pub use create::create;
pub use error::{Error, Result};
pub use exercise::{exercise, StrikePayment};
pub use hydrate::{parse, parse_child, ParsedOption};
pub use rehydrate::{parse_metadata, rehydrate, OptionMetadata, RehydratedTerms};
pub use sign::required_signatures;
pub use transfer::transfer;
pub use types::{CreatedOption, OptionSpend, OptionTerms, Owner};

// Re-exports so a consumer need not depend on the SDK directly for the common surface.
pub use chia_wallet_sdk::driver::{
    OptionContract, OptionType, OptionUnderlying, SpendContext, SpendWithConditions,
};
pub use chia_wallet_sdk::signer::RequiredSignature;

/// The crate's semantic version, surfaced so a consumer can record which builder version
/// produced a spend.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_reported() {
        assert!(!super::version().is_empty());
    }
}
