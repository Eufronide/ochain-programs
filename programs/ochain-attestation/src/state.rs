use anchor_lang::prelude::*;
use crate::constants::*;

// ---------------------------------------------------------------------------
// VerifierState  (seeds: ["verifier_state"])
// ---------------------------------------------------------------------------

/// Singleton that names the off-chain verifier service's authority key.
/// Created once by the deployer; authority should be moved to the governance
/// program PDA after the verifier service is live.
#[account]
pub struct VerifierState {
    /// The key authorised to call `verify_attestation` and `revoke_attestation`.
    pub verifier_authority: Pubkey,
    pub bump:               u8,
}

impl VerifierState {
    pub const SPACE: usize = 8
        + 32  // verifier_authority
        + 1   // bump
        + 16; // padding for future fields
}

// ---------------------------------------------------------------------------
// AttestationRecord  (seeds: ["attestation", node_pubkey])
// ---------------------------------------------------------------------------

/// Lifecycle of an on-chain attestation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum AttestationStatus {
    /// Submitted but not yet inspected by the verifier.
    Pending,
    /// Verifier confirmed the quote is valid; safe to use as key-rotation proof.
    Verified,
    /// Verifier rejected or revoked the quote (measurement mismatch, replay, etc.)
    Revoked,
}

/// One record per attestation key (`node_pubkey`).  The registry V2
/// `update_attestation_pubkey` instruction will load this account and require
/// `status == AttestationStatus::Verified` before accepting the rotation.
///
/// The raw TEE quote (4–8 KB) is kept off-chain; only its SHA-256 digest is
/// stored here so it can be replayed for audit without on-chain bloat.
#[account]
pub struct AttestationRecord {
    /// Operator that submitted this attestation.
    pub operator:         Pubkey,
    /// The Ed25519 key generated inside the enclave being attested.
    /// Also used as part of the PDA seed — one record per key.
    pub node_pubkey:      Pubkey,
    /// b"TD" = Intel TDX, b"SE" = AMD SEV-SNP.
    pub tee_type:         [u8; 2],
    /// TDX MRTD or SEV-SNP measurement (zero-padded for SEV-SNP's 32-byte hash).
    /// Must match the NodeAccount.measurement_hash for the registry to accept
    /// a key-rotation CPI.
    pub measurement_hash: [u8; MEASUREMENT_HASH_LEN],
    /// SHA-256 of the raw attestation quote provided off-chain to the verifier.
    /// Stored so an auditor can re-verify the original quote at any time.
    pub quote_hash:       [u8; QUOTE_HASH_LEN],
    pub status:           AttestationStatus,
    /// Slot when the operator called `submit_attestation`.
    pub submitted_slot:   u64,
    /// Slot when `verify_attestation` was called (0 if still pending/revoked).
    pub verified_slot:    u64,
    pub bump:             u8,
}

impl AttestationRecord {
    pub const SPACE: usize = 8
        + 32                      // operator
        + 32                      // node_pubkey
        + 2                       // tee_type
        + MEASUREMENT_HASH_LEN    // measurement_hash (48)
        + QUOTE_HASH_LEN          // quote_hash (32)
        + 1                       // status discriminant
        + 8                       // submitted_slot
        + 8                       // verified_slot
        + 1                       // bump
        + 16;                     // padding

    /// Convenience accessor mirroring the registry V2 check:
    ///   `attestation_record.verified == true`
    pub fn is_verified(&self) -> bool {
        self.status == AttestationStatus::Verified
    }
}
