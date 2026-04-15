use anchor_lang::prelude::*;
use anchor_lang::system_program;

pub mod constants;
pub mod errors;
pub mod events;
pub mod state;

use constants::*;
use errors::RegistryError;
use events::*;
use state::*;

declare_id!("11111111111111111111111111111112");
// ↑ Placeholder — replace with the output of:
//   anchor build && solana-keygen pubkey target/deploy/ochain_registry-keypair.json

// ============================================================================
//  Program
// ============================================================================

#[program]
pub mod ochain_registry {
    use super::*;

    // ------------------------------------------------------------------------
    // initialize_protocol
    //
    // One-time call by the deployer.  Authority should be transferred to the
    // governance program PDA after deployment via a subsequent transaction.
    // ------------------------------------------------------------------------
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        min_stake_lamports: u64,
        slash_basis_points: u16,
        epoch_duration_slots: u64,
        max_nodes_per_operator: u8,
    ) -> Result<()> {
        require!(slash_basis_points <= 10_000, RegistryError::StakeTooLow); // reuse for bps check
        require!(epoch_duration_slots > 0, RegistryError::NoSlaViolation);

        let ps = &mut ctx.accounts.protocol_state;
        ps.authority             = ctx.accounts.authority.key();
        ps.treasury              = ctx.accounts.treasury.key();
        ps.min_stake_lamports    = min_stake_lamports;
        ps.slash_basis_points    = slash_basis_points;
        ps.operator_count        = 0;
        ps.epoch_duration_slots  = epoch_duration_slots;
        ps.max_nodes_per_operator = max_nodes_per_operator;
        ps.ochain_token_mint     = Pubkey::default(); // V2: set to real mint
        ps.bump                  = ctx.bumps.protocol_state;

        emit!(ProtocolInitialised {
            authority:            ps.authority,
            min_stake_lamports:   ps.min_stake_lamports,
            epoch_duration_slots: ps.epoch_duration_slots,
            slot: Clock::get()?.slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // register_operator
    //
    // Creates OperatorAccount + StakeVault PDA and transfers stake lamports
    // into the vault.  The operator's first node key is set as the primary
    // attestation_pubkey; subsequent nodes are added via add_node.
    // ------------------------------------------------------------------------
    pub fn register_operator(
        ctx: Context<RegisterOperator>,
        tee_type: [u8; 2],
        attestation_pubkey: Pubkey,
        endpoint_url: Vec<u8>,
        stake_amount: u64,
    ) -> Result<()> {
        let ps = &ctx.accounts.protocol_state;

        // ── Validation ──────────────────────────────────────────────────────
        require!(stake_amount >= ps.min_stake_lamports, RegistryError::StakeTooLow);
        require!(
            tee_type == *b"TD" || tee_type == *b"SE",
            RegistryError::InvalidTeeType
        );
        require!(
            !endpoint_url.is_empty()
                && endpoint_url.len() <= MAX_ENDPOINT_URL_LEN
                && std::str::from_utf8(&endpoint_url).is_ok(),
            RegistryError::InvalidEndpointUrl
        );
        require!(
            attestation_pubkey != Pubkey::default(),
            RegistryError::InvalidAttestationPubkey
        );

        // ── Transfer stake lamports into the vault ───────────────────────────
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.operator_authority.to_account_info(),
                    to:   ctx.accounts.stake_vault.to_account_info(),
                },
            ),
            stake_amount,
        )?;

        // ── Populate OperatorAccount ─────────────────────────────────────────
        let op = &mut ctx.accounts.operator_account;
        op.authority             = ctx.accounts.operator_authority.key();
        op.stake_amount          = stake_amount;
        op.registration_slot     = Clock::get()?.slot;
        op.reputation_score      = INITIAL_REPUTATION_SCORE;
        op.status                = OperatorStatus::Active;
        op.tee_type              = tee_type;
        op.attestation_pubkey    = attestation_pubkey;
        op.jobs_completed        = 0;
        op.jobs_failed           = 0;
        op.sla_violations        = 0;
        op.node_count            = 0;
        op.exit_initiated_slot   = 0;
        op.endpoint_url_len      = endpoint_url.len() as u32;
        op.bump                  = ctx.bumps.operator_account;
        op.stake_vault_bump      = ctx.bumps.stake_vault;

        let url_len = endpoint_url.len();
        op.endpoint_url[..url_len].copy_from_slice(&endpoint_url);

        // ── Update global counter ────────────────────────────────────────────
        ctx.accounts.protocol_state.operator_count += 1;

        emit!(OperatorRegistered {
            operator:  op.key(),
            authority: op.authority,
            stake:     stake_amount,
            tee_type,
            slot:      op.registration_slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // add_node
    //
    // Registers an additional TEE node under an existing operator.  node_index
    // must equal operator_account.node_count (sequential).
    // ------------------------------------------------------------------------
    pub fn add_node(
        ctx: Context<AddNode>,
        node_index: u8,
        attestation_pubkey: Pubkey,
        measurement_hash: [u8; 48],
    ) -> Result<()> {
        let op = &mut ctx.accounts.operator_account;

        require!(op.status == OperatorStatus::Active, RegistryError::NotActive);
        require!(
            op.node_count < ctx.accounts.protocol_state.max_nodes_per_operator,
            RegistryError::TooManyNodes
        );
        require!(node_index == op.node_count, RegistryError::NonSequentialNodeIndex);
        require!(
            attestation_pubkey != Pubkey::default(),
            RegistryError::InvalidAttestationPubkey
        );

        let slot = Clock::get()?.slot;
        let node = &mut ctx.accounts.node_account;
        node.operator               = op.key();
        node.attestation_pubkey     = attestation_pubkey;
        node.measurement_hash       = measurement_hash;
        node.last_attestation_slot  = slot;
        node.last_heartbeat_slot    = slot;
        node.last_sla_check_slot    = 0;
        node.node_index             = node_index;
        node.bump                   = ctx.bumps.node_account;

        op.node_count += 1;

        emit!(NodeAdded {
            operator: op.key(),
            node_pubkey: attestation_pubkey,
            node_index,
            measurement_hash,
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // heartbeat
    //
    // Operator calls once per epoch per node to prove liveness.
    // Can be called by the operator's hot wallet or, for automation, by the
    // ochain-relay host process which holds the operator's signing key.
    // ------------------------------------------------------------------------
    pub fn heartbeat(ctx: Context<Heartbeat>, node_index: u8) -> Result<()> {
        let op = &ctx.accounts.operator_account;
        require!(op.status == OperatorStatus::Active, RegistryError::NotActive);

        let slot = Clock::get()?.slot;
        let node = &mut ctx.accounts.node_account;
        node.last_heartbeat_slot = slot;

        emit!(HeartbeatReceived {
            operator:    op.key(),
            node_pubkey: node.attestation_pubkey,
            node_index,
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // check_sla_violation  (permissionless)
    //
    // Anyone may call this to report a missed heartbeat.  The caller pays the
    // transaction fee but earns a small SOL reward from the treasury (V2).
    // Prevents double-penalising within the same epoch via last_sla_check_slot.
    // ------------------------------------------------------------------------
    pub fn check_sla_violation(
        ctx: Context<CheckSlaViolation>,
        node_index: u8,
    ) -> Result<()> {
        let ps   = &ctx.accounts.protocol_state;
        let slot = Clock::get()?.slot;

        let op   = &mut ctx.accounts.operator_account;
        let node = &mut ctx.accounts.node_account;

        // ── Guard: must be Active ─────────────────────────────────────────
        require!(op.status == OperatorStatus::Active, RegistryError::NotActive);

        // ── Guard: not already checked this epoch ─────────────────────────
        require!(
            slot > node.last_sla_check_slot + ps.epoch_duration_slots,
            RegistryError::ViolationAlreadyChecked
        );

        // ── Guard: heartbeat actually overdue ─────────────────────────────
        let miss_threshold = ps.epoch_duration_slots * SLA_MISS_MULTIPLIER;
        require!(
            slot > node.last_heartbeat_slot + miss_threshold,
            RegistryError::NoSlaViolation
        );

        // ── Apply penalty ────────────────────────────────────────────────
        node.last_sla_check_slot = slot;
        op.sla_violations        = op.sla_violations.saturating_add(1);
        op.reputation_score      = op.reputation_score.saturating_sub(REPUTATION_PENALTY_SLA);

        let suspended = op.sla_violations >= SUSPENSION_VIOLATION_THRESHOLD;
        if suspended {
            op.status = OperatorStatus::Suspended;
        }

        emit!(SlaViolationRecorded {
            operator:       op.key(),
            node_index,
            sla_violations: op.sla_violations,
            new_reputation: op.reputation_score,
            suspended,
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // begin_exit
    //
    // Starts the 14-day unbonding clock.  The operator can no longer claim new
    // jobs after this call (enforced by the job program checking status).
    //
    // NOTE: In V2, add a CPI to ochain-job to verify no active jobs remain
    //       before allowing exit.
    // ------------------------------------------------------------------------
    pub fn begin_exit(ctx: Context<BeginExit>) -> Result<()> {
        let op = &mut ctx.accounts.operator_account;

        require!(op.status == OperatorStatus::Active, RegistryError::NotActive);

        let slot                  = Clock::get()?.slot;
        op.status                 = OperatorStatus::Exiting;
        op.exit_initiated_slot    = slot;

        emit!(ExitBegun {
            operator:            op.key(),
            exit_initiated_slot: slot,
            earliest_exit_slot:  slot + UNBONDING_SLOTS,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // finalize_exit
    //
    // Callable after unbonding period elapses.  Closes the StakeVault (returns
    // all lamports including rent to the operator) and marks the operator as
    // ready to have their OperatorAccount closed in a follow-up transaction.
    // ------------------------------------------------------------------------
    pub fn finalize_exit(ctx: Context<FinalizeExit>) -> Result<()> {
        let op   = &mut ctx.accounts.operator_account;
        let slot = Clock::get()?.slot;

        require!(op.status == OperatorStatus::Exiting, RegistryError::NotExiting);
        require!(
            slot >= op.exit_initiated_slot + UNBONDING_SLOTS,
            RegistryError::UnbondingNotComplete
        );

        // ── Transfer stake lamports back ──────────────────────────────────
        // The `close = operator_authority` constraint on stake_vault moves all
        // lamports to the operator and zeroes the account.  We record how much
        // was in the vault before the close happens.
        let stake_returned = ctx.accounts.stake_vault.to_account_info().lamports();

        // close is handled by the Anchor constraint on StakeVault; we only
        // need to zero out the tracked amount here.
        op.stake_amount = 0;

        emit!(ExitFinalised {
            operator: op.key(),
            stake_returned,
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // update_attestation_pubkey
    //
    // Called when a node restarts and generates a fresh attestation key inside
    // the new TEE session.  The caller provides the new key and the measurement
    // hash of the restarted runtime.
    //
    // V1: The operator asserts the new key is TEE-generated.  The full
    //     cryptographic binding (CPI to ochain-attestation to verify a prior
    //     AttestationRecord for this key) is added in V2 once the attestation
    //     program is deployed.
    //
    // V2 TODO: Add `attestation_record: Account<AttestationRecord>` to the
    //          context and require attestation_record.node_pubkey == new_key
    //          && attestation_record.verified == true.
    // ------------------------------------------------------------------------
    pub fn update_attestation_pubkey(
        ctx: Context<UpdateAttestationPubkey>,
        node_index: u8,
        new_attestation_pubkey: Pubkey,
        new_measurement_hash: [u8; 48],
    ) -> Result<()> {
        require!(
            new_attestation_pubkey != Pubkey::default(),
            RegistryError::InvalidAttestationPubkey
        );

        let slot = Clock::get()?.slot;
        let operator_key = ctx.accounts.operator_account.key();
        let node_index_val = ctx.accounts.node_account.node_index;

        let node = &mut ctx.accounts.node_account;
        let old_key = node.attestation_pubkey;
        node.attestation_pubkey    = new_attestation_pubkey;
        node.measurement_hash      = new_measurement_hash;
        node.last_attestation_slot = slot;

        // Keep the primary key on the OperatorAccount in sync if this is node 0.
        if node_index_val == 0 {
            ctx.accounts.operator_account.attestation_pubkey = new_attestation_pubkey;
        }

        emit!(AttestationKeyUpdated {
            operator:   operator_key,
            node_index,
            old_key,
            new_key: new_attestation_pubkey,
            slot,
        });

        Ok(())
    }
}

// ============================================================================
//  Account Contexts
// ============================================================================

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Treasury can be any account (e.g. a multisig).  The program only
    ///        sends lamports to it; it never reads or writes data here.
    pub treasury: UncheckedAccount<'info>,

    #[account(
        init,
        payer  = authority,
        space  = ProtocolState::SPACE,
        seeds  = [SEED_PROTOCOL_STATE],
        bump,
    )]
    pub protocol_state: Account<'info, ProtocolState>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(
    tee_type:           [u8; 2],
    attestation_pubkey: Pubkey,
    endpoint_url:       Vec<u8>,
    stake_amount:       u64,
)]
pub struct RegisterOperator<'info> {
    #[account(mut)]
    pub operator_authority: Signer<'info>,

    #[account(
        init,
        payer = operator_authority,
        space = OperatorAccount::SPACE,
        seeds = [SEED_OPERATOR, operator_authority.key().as_ref()],
        bump,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    /// SOL vault that holds the operator's stake.  Intentionally has no data
    /// beyond the Anchor discriminator so rent is minimised.
    #[account(
        init,
        payer = operator_authority,
        space = StakeVault::SPACE,
        seeds = [SEED_STAKE_VAULT, operator_authority.key().as_ref()],
        bump,
    )]
    pub stake_vault: Account<'info, StakeVault>,

    #[account(
        mut,
        seeds  = [SEED_PROTOCOL_STATE],
        bump   = protocol_state.bump,
    )]
    pub protocol_state: Account<'info, ProtocolState>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(node_index: u8)]
pub struct AddNode<'info> {
    #[account(mut)]
    pub operator_authority: Signer<'info>,

    #[account(
        mut,
        seeds      = [SEED_OPERATOR, operator_authority.key().as_ref()],
        bump       = operator_account.bump,
        constraint = operator_account.authority == operator_authority.key() @ RegistryError::NotActive,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    #[account(
        init,
        payer  = operator_authority,
        space  = NodeAccount::SPACE,
        seeds  = [SEED_OPERATOR_NODE, operator_account.key().as_ref(), &[node_index]],
        bump,
    )]
    pub node_account: Account<'info, NodeAccount>,

    #[account(
        seeds = [SEED_PROTOCOL_STATE],
        bump  = protocol_state.bump,
    )]
    pub protocol_state: Account<'info, ProtocolState>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(node_index: u8)]
pub struct Heartbeat<'info> {
    pub operator_authority: Signer<'info>,

    #[account(
        seeds      = [SEED_OPERATOR, operator_authority.key().as_ref()],
        bump       = operator_account.bump,
        constraint = operator_account.authority == operator_authority.key() @ RegistryError::NotActive,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    #[account(
        mut,
        seeds  = [SEED_OPERATOR_NODE, operator_account.key().as_ref(), &[node_index]],
        bump   = node_account.bump,
        constraint = node_account.operator == operator_account.key(),
    )]
    pub node_account: Account<'info, NodeAccount>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(node_index: u8)]
pub struct CheckSlaViolation<'info> {
    /// Permissionless: anyone may be the payer/caller.
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_OPERATOR, operator_account.authority.as_ref()],
        bump  = operator_account.bump,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    #[account(
        mut,
        seeds = [SEED_OPERATOR_NODE, operator_account.key().as_ref(), &[node_index]],
        bump  = node_account.bump,
        constraint = node_account.operator == operator_account.key(),
    )]
    pub node_account: Account<'info, NodeAccount>,

    #[account(
        seeds = [SEED_PROTOCOL_STATE],
        bump  = protocol_state.bump,
    )]
    pub protocol_state: Account<'info, ProtocolState>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct BeginExit<'info> {
    pub operator_authority: Signer<'info>,

    #[account(
        mut,
        seeds      = [SEED_OPERATOR, operator_authority.key().as_ref()],
        bump       = operator_account.bump,
        constraint = operator_account.authority == operator_authority.key() @ RegistryError::NotActive,
    )]
    pub operator_account: Account<'info, OperatorAccount>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct FinalizeExit<'info> {
    #[account(mut)]
    pub operator_authority: Signer<'info>,

    #[account(
        mut,
        seeds      = [SEED_OPERATOR, operator_authority.key().as_ref()],
        bump       = operator_account.bump,
        constraint = operator_account.authority == operator_authority.key() @ RegistryError::NotExiting,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    /// close = operator_authority drains all lamports (stake + rent) to the
    /// operator and garbage-collects the account.
    #[account(
        mut,
        seeds  = [SEED_STAKE_VAULT, operator_authority.key().as_ref()],
        bump   = operator_account.stake_vault_bump,
        close  = operator_authority,
    )]
    pub stake_vault: Account<'info, StakeVault>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(node_index: u8)]
pub struct UpdateAttestationPubkey<'info> {
    pub operator_authority: Signer<'info>,

    #[account(
        mut,
        seeds      = [SEED_OPERATOR, operator_authority.key().as_ref()],
        bump       = operator_account.bump,
        constraint = operator_account.authority == operator_authority.key() @ RegistryError::NotActive,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    #[account(
        mut,
        seeds  = [SEED_OPERATOR_NODE, operator_account.key().as_ref(), &[node_index]],
        bump   = node_account.bump,
        constraint = node_account.operator == operator_account.key(),
    )]
    pub node_account: Account<'info, NodeAccount>,
    // V2: add AttestationRecord account here and verify via CPI to
    //     ochain-attestation before accepting the new key.
}
