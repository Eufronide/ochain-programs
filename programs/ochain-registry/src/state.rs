use anchor_lang::prelude::*;
use crate::constants::*;

// ---------------------------------------------------------------------------
// ProtocolState  (seeds: ["protocol_state"])
// ---------------------------------------------------------------------------

/// Global protocol parameters.  Created once by the deployer; authority
/// is transferred to the governance program PDA after deployment.
#[account]
pub struct ProtocolState {
    /// Current authority (governance program PDA after initialisation).
    pub authority: Pubkey,
    /// Treasury PDA that accumulates protocol fees.
    pub treasury: Pubkey,
    /// Minimum lamports an operator must stake to register.
    pub min_stake_lamports: u64,
    /// Fraction of stake burned on a governance-ordered slash (bps, 10_000 = 100 %).
    pub slash_basis_points: u16,
    /// Monotonically increasing counter — NOT used as PDA seed (operators use
    /// their authority key), but useful for analytics.
    pub operator_count: u64,
    /// Length of one SLA epoch in slots.
    pub epoch_duration_slots: u64,
    /// Anti-centralisation cap: max distinct nodes per operator.
    pub max_nodes_per_operator: u8,
    /// Reserved for V2 OCHAIN SPL-token staking (set to Pubkey::default() in V1).
    pub ochain_token_mint: Pubkey,
    pub bump: u8,
}

impl ProtocolState {
    /// Anchor discriminator (8) + all field sizes.
    pub const SPACE: usize = 8
        + 32  // authority
        + 32  // treasury
        + 8   // min_stake_lamports
        + 2   // slash_basis_points
        + 8   // operator_count
        + 8   // epoch_duration_slots
        + 1   // max_nodes_per_operator
        + 32  // ochain_token_mint
        + 1   // bump
        + 32; // padding for future fields
}

// ---------------------------------------------------------------------------
// OperatorAccount  (seeds: ["operator", authority.key()])
// ---------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum OperatorStatus {
    /// Normal operation.
    Active,
    /// Temporarily blocked from claiming jobs due to SLA violations.
    Suspended,
    /// Unbonding period started; stake not yet returned.
    Exiting,
    /// Governance-ordered permanent ban; stake slashed.
    Slashed,
}

/// Per-operator on-chain state.  One account per hot-wallet authority.
#[account]
pub struct OperatorAccount {
    /// Hot wallet that controls this operator (signs heartbeats, claim_job, etc.)
    pub authority: Pubkey,
    /// Amount of lamports currently staked (mirrors the vault balance, stored
    /// here so we can slash without needing the vault account every time).
    pub stake_amount: u64,
    /// Slot at which the operator registered.
    pub registration_slot: u64,
    /// 0 – 10_000 reputation score (starts at INITIAL_REPUTATION_SCORE).
    pub reputation_score: u16,
    pub status: OperatorStatus,
    /// b"TD" = Intel TDX, b"SE" = AMD SEV-SNP.
    pub tee_type: [u8; 2],
    /// Ed25519 public key of the *primary* node's enclave attestation key.
    /// Updated via `update_attestation_pubkey`.
    pub attestation_pubkey: Pubkey,
    pub jobs_completed: u64,
    pub jobs_failed: u64,
    /// Number of SLA violations incurred lifetime.
    pub sla_violations: u8,
    /// Number of active NodeAccount sub-accounts.
    pub node_count: u8,
    /// Set when `begin_exit` is called; 0 means not exiting.
    pub exit_initiated_slot: u64,
    /// Length of the valid UTF-8 data in `endpoint_url`.
    pub endpoint_url_len: u32,
    /// HTTPS endpoint the operator exposes for job receipt (max 200 bytes).
    pub endpoint_url: [u8; MAX_ENDPOINT_URL_LEN],
    /// Bump seeds stored to avoid recomputation in CPIs.
    pub bump: u8,
    pub stake_vault_bump: u8,
}

impl OperatorAccount {
    pub const SPACE: usize = 8
        + 32  // authority
        + 8   // stake_amount
        + 8   // registration_slot
        + 2   // reputation_score
        + 1   // status (u8 discriminant)
        + 2   // tee_type
        + 32  // attestation_pubkey
        + 8   // jobs_completed
        + 8   // jobs_failed
        + 1   // sla_violations
        + 1   // node_count
        + 8   // exit_initiated_slot
        + 4   // endpoint_url_len
        + MAX_ENDPOINT_URL_LEN // endpoint_url
        + 1   // bump
        + 1   // stake_vault_bump
        + 16; // padding
}

// ---------------------------------------------------------------------------
// StakeVault  (seeds: ["stake_vault", authority.key()])
// ---------------------------------------------------------------------------

/// Empty account owned by the registry program that holds the operator's
/// staked lamports.  We use a separate PDA so the stake is isolated from the
/// OperatorAccount rent lamports and can be drained cleanly on exit/slash.
#[account]
pub struct StakeVault {}

impl StakeVault {
    pub const SPACE: usize = 8; // discriminator only
}

// ---------------------------------------------------------------------------
// NodeAccount  (seeds: ["operator_node", operator.key(), &[node_index]])
// ---------------------------------------------------------------------------

/// One per physical TEE machine an operator runs.
#[account]
pub struct NodeAccount {
    /// Parent operator account.
    pub operator: Pubkey,
    /// Ed25519 pubkey generated inside *this* enclave (unique per node).
    pub attestation_pubkey: Pubkey,
    /// TDX MRTD or SEV-SNP measurement of the OchainRuntime binary.
    /// 48 bytes (TDX MRTD size; zero-padded for SEV which uses 32 bytes).
    pub measurement_hash: [u8; 48],
    /// Slot of the last accepted remote attestation update.
    pub last_attestation_slot: u64,
    /// Slot of the last successful heartbeat transaction.
    pub last_heartbeat_slot: u64,
    /// Slot of the last SLA violation check (prevents double-penalising
    /// within the same epoch).
    pub last_sla_check_slot: u64,
    /// Sequential 0-based index within the operator's node set.
    pub node_index: u8,
    pub bump: u8,
}

impl NodeAccount {
    pub const SPACE: usize = 8
        + 32  // operator
        + 32  // attestation_pubkey
        + 48  // measurement_hash
        + 8   // last_attestation_slot
        + 8   // last_heartbeat_slot
        + 8   // last_sla_check_slot
        + 1   // node_index
        + 1   // bump
        + 8;  // padding
}
