//! # dig-options — the DIG Network canonical Chia option-contract expert crate
//!
//! `dig-options` is a **pure, key-free, network-free** CoinSpend-builder for Chia covered options
//! (the CHIP-0042 option primitive). It constructs the exact
//! [`CoinSpend`](chia_protocol::CoinSpend)s for the option lifecycle — create (lock an underlying,
//! mint the option singleton), exercise (pay the strike, unlock the underlying to the holder),
//! clawback/cancel (the creator reclaims the underlying after expiry), and inspect (reconstruct an
//! option from its coin) — and reports the exact signatures a caller must produce.
//!
//! ## The custody model (HARD invariants)
//!
//! dig-options **never holds a secret key, never signs, and never touches the network.** Every
//! builder takes only public inputs (a creator/holder expressed as a public key or a borrowed inner
//! spender, coins the caller already fetched) and appends unsigned coin spends to a caller-owned
//! `SpendContext`. The consumer signs the messages reported by the crate's required-signature
//! reporter, assembles the `SpendBundle`, and broadcasts. Signing — and the secret key — stay
//! entirely on the caller's side of the identity boundary (#908).
//!
//! ## Status
//!
//! This is the v0.0.0 bootstrap: it establishes the crate + the release/quality pipeline and proves
//! the pinned chia-wallet-sdk 0.30 dependency tree resolves. The full option builder surface lands
//! as v0.1.0. See `SPEC.md` for the normative contract.

#![forbid(unsafe_code)]

/// The crate's semantic version, surfaced so a consumer can record which builder version produced a
/// spend.
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
