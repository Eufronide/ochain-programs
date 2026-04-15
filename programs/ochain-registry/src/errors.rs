use anchor_lang::prelude::*;

#[error_code]
pub enum RegistryError {
    // ── Registration ──────────────────────────────────────────────────────
    #[msg("Stake amount is below the protocol minimum")]
    StakeTooLow,

    #[msg("tee_type must be b\"TD\" (TDX) or b\"SE\" (SEV-SNP)")]
    InvalidTeeType,

    #[msg("endpoint_url exceeds 200 bytes or is not valid UTF-8")]
    InvalidEndpointUrl,

    #[msg("attestation_pubkey must not be the default (all-zero) key")]
    InvalidAttestationPubkey,

    // ── Status guards ─────────────────────────────────────────────────────
    #[msg("Operator is not in Active status")]
    NotActive,

    #[msg("Operator is already in the requested state")]
    AlreadyInState,

    // ── Node management ───────────────────────────────────────────────────
    #[msg("Operator has reached the maximum number of nodes")]
    TooManyNodes,

    #[msg("node_index must be sequential (equal to current node_count)")]
    NonSequentialNodeIndex,

    // ── SLA ───────────────────────────────────────────────────────────────
    #[msg("Node's last heartbeat is within the allowed SLA window")]
    NoSlaViolation,

    #[msg("SLA violation already checked this epoch; wait for next epoch")]
    ViolationAlreadyChecked,

    // ── Exit ──────────────────────────────────────────────────────────────
    #[msg("Unbonding period has not elapsed yet")]
    UnbondingNotComplete,

    #[msg("Operator must be in Exiting status to finalise exit")]
    NotExiting,

    // ── Arithmetic ────────────────────────────────────────────────────────
    #[msg("Arithmetic overflow in reputation calculation")]
    ReputationOverflow,
}
