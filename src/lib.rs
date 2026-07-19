//! # dig-options — the DIG Network canonical Chia option-contract expert crate
//!
//! `dig-options` is a **pure, key-free, network-free** CoinSpend-builder for Chia covered
//! options (the CHIP-0042 option primitive). It constructs the exact
//! [`CoinSpend`](chia_protocol::CoinSpend)s for the option lifecycle — [`create`] (lock an XCH
//! underlying, mint the option singleton), [`exercise`] (pay the strike, unlock the underlying
//! to the holder), [`clawback`] (the creator reclaims the underlying after expiry), and
//! inspect ([`parse`]/[`parse_child`]) — and reports the exact signatures a caller must
//! produce ([`required_signatures`]).
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
//! The underlying is **XCH**. The strike may be any [`OptionType`] for [`create`], [`clawback`],
//! and inspection (it is curried into the option puzzle), while [`exercise`] builds the full
//! settlement leg for an **XCH strike**; a CAT/NFT strike exercise returns an honest
//! [`Error::InvalidInput`] rather than an incorrect spend. CAT/NFT underlyings and strike
//! exercise are a future extension. See `SPEC.md` for the normative contract.

#![forbid(unsafe_code)]

mod clawback;
mod create;
mod error;
mod exercise;
mod hydrate;
mod sign;
mod types;

pub use clawback::clawback;
pub use create::create;
pub use error::{Error, Result};
pub use exercise::{exercise, StrikePayment};
pub use hydrate::{parse, parse_child, ParsedOption};
pub use sign::required_signatures;
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
