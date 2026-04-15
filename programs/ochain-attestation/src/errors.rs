use anchor_lang::prelude::*;

#[error_code]
pub enum AttestationError {
    // ── Submission ─────────────────────────────────────────────────────────
    #[msg("tee_type must be b\"TD\" (TDX) or b\"SE\" (SEV-SNP)")]
    InvalidTeeType,

    #[msg("node_pubkey must not be the default (all-zero) key")]
    InvalidNodePubkey,

    #[msg("quote_hash must be a non-zero 32-byte SHA-256 digest")]
    InvalidQuoteHash,

    #[msg("measurement_hash must be non-zero")]
    InvalidMeasurementHash,

    // ── Verification lifecycle ─────────────────────────────────────────────
    #[msg("Attestation record is already verified")]
    AlreadyVerified,

    #[msg("Attestation record has not been verified yet")]
    NotVerified,

    #[msg("Attestation record has been revoked and cannot be re-verified")]
    Revoked,

    // ── Authority ─────────────────────────────────────────────────────────
    #[msg("Caller is not the designated verifier authority")]
    NotVerifier,

    #[msg("Caller is not the operator who submitted this attestation")]
    NotSubmitter,
}
