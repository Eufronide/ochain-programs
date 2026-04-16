use anchor_lang::prelude::*;
use crate::constants::{PAYLOAD_HASH_LEN, RESULT_HASH_LEN};

// ---------------------------------------------------------------------------
// JobStatus
// ---------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum JobStatus {
    /// Posted by client; open for any qualified operator to claim.
    Open,
    /// Claimed by an operator; awaiting result submission.
    Claimed,
    /// Operator submitted a valid result before the deadline.
    Completed,
    /// Operator failed to submit before deadline_slot; client was refunded.
    Slashed,
}

// ---------------------------------------------------------------------------
// JobAccount  (seeds: ["job", client, job_nonce])
// ---------------------------------------------------------------------------

#[account]
pub struct JobAccount {
    /// Client that posted the job.
    pub client: Pubkey,
    /// Operator that claimed the job (Pubkey::default() when Open).
    pub operator: Pubkey,
    /// SOL payment the client deposited into the vault.
    pub payment_lamports: u64,
    /// Minimum bond the client requires the operator to deposit on claim.
    pub required_bond_lamports: u64,
    /// Actual bond the operator deposited (0 before claim).
    pub claim_bond_lamports: u64,
    /// SHA-256 of the off-chain job payload.
    pub payload_hash: [u8; PAYLOAD_HASH_LEN],
    /// SHA-256 of the TEE-attested result (set on submit_result).
    pub result_hash: [u8; RESULT_HASH_LEN],
    /// Operator must submit before this slot or face slashing.
    pub deadline_slot: u64,
    /// b"TD" = Intel TDX, b"SE" = AMD SEV-SNP.
    pub required_tee_type: [u8; 2],
    pub status: JobStatus,
    pub posted_slot: u64,
    /// Slot when the operator called claim_job (0 if not yet claimed).
    pub claimed_slot: u64,
    /// Client-scoped nonce; included in PDA seeds to allow multiple concurrent jobs.
    pub job_nonce: u64,
    pub bump: u8,
    pub vault_bump: u8,
}

impl JobAccount {
    pub const SPACE: usize = 8   // discriminator
        + 32  // client
        + 32  // operator
        + 8   // payment_lamports
        + 8   // required_bond_lamports
        + 8   // claim_bond_lamports
        + 32  // payload_hash
        + 32  // result_hash
        + 8   // deadline_slot
        + 2   // required_tee_type
        + 1   // status discriminant
        + 8   // posted_slot
        + 8   // claimed_slot
        + 8   // job_nonce
        + 1   // bump
        + 1   // vault_bump
        + 32; // padding
}

// ---------------------------------------------------------------------------
// JobVault  (seeds: ["job_vault", client, job_nonce])
// ---------------------------------------------------------------------------

/// Holds the client's payment + operator's claim bond.
/// Closed to the operator on submit_result or to the client on slash_timeout.
#[account]
pub struct JobVault {}

impl JobVault {
    pub const SPACE: usize = 8; // discriminator only
}
