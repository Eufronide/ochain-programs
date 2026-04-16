use anchor_lang::prelude::*;
use anchor_lang::system_program;

pub mod constants;
pub mod errors;
pub mod events;
pub mod state;

use constants::*;
use errors::JobError;
use events::*;
use state::*;

use ochain_attestation::state::AttestationRecord;
use ochain_registry::state::{OperatorAccount, OperatorStatus};

declare_id!("11111111111111111111111111111115");
// ↑ Placeholder — replace with:
//   anchor build && solana-keygen pubkey target/deploy/ochain_job-keypair.json

// ============================================================================
//  Program
// ============================================================================

#[program]
pub mod ochain_job {
    use super::*;

    // ------------------------------------------------------------------------
    // post_job
    //
    // Client locks a SOL payment into a per-job vault PDA and records the job
    // parameters on-chain.  The client sets required_bond_lamports — operators
    // must match this deposit when claiming, aligning incentives.
    //
    // job_nonce is client-managed (e.g. a monotonic counter or random u64) so
    // clients can have many concurrent jobs without a global counter.
    // ------------------------------------------------------------------------
    pub fn post_job(
        ctx: Context<PostJob>,
        job_nonce: u64,
        payload_hash: [u8; PAYLOAD_HASH_LEN],
        deadline_slot: u64,
        required_tee_type: [u8; 2],
        payment_lamports: u64,
        required_bond_lamports: u64,
    ) -> Result<()> {
        require!(
            required_tee_type == *b"TD" || required_tee_type == *b"SE",
            JobError::InvalidTeeType
        );
        require!(payload_hash != [0u8; PAYLOAD_HASH_LEN], JobError::InvalidPayloadHash);

        let slot = Clock::get()?.slot;
        require!(deadline_slot > slot, JobError::DeadlineInPast);
        require!(payment_lamports >= MIN_PAYMENT_LAMPORTS, JobError::PaymentTooLow);
        require!(required_bond_lamports > 0, JobError::BondTooLow);

        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.client.to_account_info(),
                    to:   ctx.accounts.job_vault.to_account_info(),
                },
            ),
            payment_lamports,
        )?;

        let job_key    = ctx.accounts.job_account.key();
        let client_key = ctx.accounts.client.key();

        let job                    = &mut ctx.accounts.job_account;
        job.client                 = client_key;
        job.operator               = Pubkey::default();
        job.payment_lamports       = payment_lamports;
        job.required_bond_lamports = required_bond_lamports;
        job.claim_bond_lamports    = 0;
        job.payload_hash           = payload_hash;
        job.result_hash            = [0u8; RESULT_HASH_LEN];
        job.deadline_slot          = deadline_slot;
        job.required_tee_type      = required_tee_type;
        job.status                 = JobStatus::Open;
        job.posted_slot            = slot;
        job.claimed_slot           = 0;
        job.job_nonce              = job_nonce;
        job.bump                   = ctx.bumps.job_account;
        job.vault_bump             = ctx.bumps.job_vault;

        emit!(JobPosted {
            job:                    job_key,
            client:                 client_key,
            payment_lamports,
            required_bond_lamports,
            deadline_slot,
            required_tee_type,
            payload_hash,
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // claim_job
    //
    // Operator claims an Open job by:
    //   1. Proving Active status in ochain-registry (cross-program account read).
    //   2. Proving a Verified attestation exists for their TEE node in
    //      ochain-attestation (cross-program account read — the core gating step).
    //   3. Depositing the required claim bond into the job vault.
    //
    // The attestation read trusts ochain-attestation's program ownership: only
    // that program can write an AttestationRecord, so a Verified status in that
    // account means the off-chain verifier service inspected the raw TEE quote
    // and confirmed it is genuine.
    // ------------------------------------------------------------------------
    pub fn claim_job(ctx: Context<ClaimJob>, job_nonce: u64) -> Result<()> {
        let _ = job_nonce; // consumed by #[instruction] for PDA seeds

        let slot = Clock::get()?.slot;

        require!(ctx.accounts.job_account.status == JobStatus::Open, JobError::JobNotOpen);
        require!(slot < ctx.accounts.job_account.deadline_slot, JobError::DeadlineExpired);

        // ── Registry check: operator must be Active ─────────────────────────
        require!(
            ctx.accounts.operator_account.status == OperatorStatus::Active,
            JobError::OperatorNotActive
        );

        // ── TEE type must match job requirement ─────────────────────────────
        require!(
            ctx.accounts.operator_account.tee_type == ctx.accounts.job_account.required_tee_type,
            JobError::TeeTypeMismatch
        );

        // ── Attestation check: node must be Verified ────────────────────────
        //
        // AttestationRecord is owned by ochain-attestation (Anchor enforces
        // this on Account deserialization via seeds::program).  A Verified
        // status means the external verifier service validated the raw TEE
        // quote off-chain.  Uncertified or revoked nodes cannot claim jobs.
        require!(
            ctx.accounts.attestation_record.is_verified(),
            JobError::AttestationNotVerified
        );
        require!(
            ctx.accounts.attestation_record.tee_type == ctx.accounts.job_account.required_tee_type,
            JobError::TeeTypeMismatch
        );

        // ── Transfer claim bond into the vault ──────────────────────────────
        let bond        = ctx.accounts.job_account.required_bond_lamports;
        let node_pubkey = ctx.accounts.attestation_record.node_pubkey;
        let op_key      = ctx.accounts.operator_authority.key();
        let job_key     = ctx.accounts.job_account.key();

        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.operator_authority.to_account_info(),
                    to:   ctx.accounts.job_vault.to_account_info(),
                },
            ),
            bond,
        )?;

        let job             = &mut ctx.accounts.job_account;
        job.operator            = op_key;
        job.claim_bond_lamports = bond;
        job.status              = JobStatus::Claimed;
        job.claimed_slot        = slot;

        emit!(JobClaimed {
            job:                 job_key,
            operator:            op_key,
            node_pubkey,
            claim_bond_lamports: bond,
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // submit_result
    //
    // Operator submits the result hash and proves it originated from a verified
    // TEE by requiring the TEE attestation key (operator_account.attestation_pubkey)
    // to co-sign the transaction.  The result_hash is stored on-chain for audit.
    //
    // On success the job vault (payment + bond + vault rent) is closed to the
    // operator via the `close = operator_authority` constraint.
    // ------------------------------------------------------------------------
    pub fn submit_result(
        ctx: Context<SubmitResult>,
        job_nonce: u64,
        result_hash: [u8; RESULT_HASH_LEN],
    ) -> Result<()> {
        let _ = job_nonce;

        require!(result_hash != [0u8; RESULT_HASH_LEN], JobError::InvalidResultHash);

        let slot    = Clock::get()?.slot;
        let job_key = ctx.accounts.job_account.key();
        let op_key  = ctx.accounts.operator_authority.key();
        let payout  = ctx.accounts.job_vault.to_account_info().lamports();

        {
            let job = &ctx.accounts.job_account;
            require!(job.status == JobStatus::Claimed, JobError::JobNotClaimed);
            require!(slot <= job.deadline_slot, JobError::DeadlineExpired);
        }

        // tee_attestation_key is a Signer on this tx; the account constraint
        // (see SubmitResult struct) verified it matches operator_account.attestation_pubkey.
        // Together: only the live TEE enclave can produce a valid submit tx.

        let job         = &mut ctx.accounts.job_account;
        job.result_hash = result_hash;
        job.status      = JobStatus::Completed;

        emit!(JobCompleted {
            job: job_key,
            operator: op_key,
            result_hash,
            payout,
            slot,
        });

        // Anchor `close = operator_authority` on job_vault fires here, draining
        // all vault lamports to the operator.

        Ok(())
    }

    // ------------------------------------------------------------------------
    // slash_timeout
    //
    // Permissionless.  If the operator claimed a job but failed to submit a
    // result before deadline_slot, anyone may call this to:
    //   • Close the job vault to the client (they recover payment + forfeited bond).
    //   • Mark the job Slashed on-chain.
    //
    // Slashing the operator's registry stake requires a future
    // `slash_by_job_program` instruction in ochain-registry (V2).
    // ------------------------------------------------------------------------
    pub fn slash_timeout(ctx: Context<SlashTimeout>, job_nonce: u64) -> Result<()> {
        let _ = job_nonce;

        let slot    = Clock::get()?.slot;
        let job_key = ctx.accounts.job_account.key();

        let (refund, slash, op) = {
            let job = &ctx.accounts.job_account;
            require!(job.status == JobStatus::Claimed, JobError::JobNotClaimed);
            require!(slot > job.deadline_slot, JobError::DeadlineNotPassed);
            (job.payment_lamports, job.claim_bond_lamports, job.operator)
        };

        ctx.accounts.job_account.status = JobStatus::Slashed;

        emit!(JobSlashed {
            job: job_key,
            operator: op,
            refund,
            slash,
            slot,
        });

        // Anchor `close = client` on job_vault fires here, draining all vault
        // lamports (payment + bond + rent) to the client as compensation.

        Ok(())
    }
}

// ============================================================================
//  Account Contexts
// ============================================================================

#[derive(Accounts)]
#[instruction(
    job_nonce:              u64,
    payload_hash:           [u8; PAYLOAD_HASH_LEN],
    deadline_slot:          u64,
    required_tee_type:      [u8; 2],
    payment_lamports:       u64,
    required_bond_lamports: u64,
)]
pub struct PostJob<'info> {
    #[account(mut)]
    pub client: Signer<'info>,

    #[account(
        init,
        payer = client,
        space = JobAccount::SPACE,
        seeds = [SEED_JOB, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump,
    )]
    pub job_account: Account<'info, JobAccount>,

    #[account(
        init,
        payer = client,
        space = JobVault::SPACE,
        seeds = [SEED_JOB_VAULT, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump,
    )]
    pub job_vault: Account<'info, JobVault>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(job_nonce: u64)]
pub struct ClaimJob<'info> {
    #[account(mut)]
    pub operator_authority: Signer<'info>,

    /// CHECK: only used for PDA seed derivation; correctness is enforced by
    ///        Anchor recomputing the job PDA from this key and rejecting
    ///        any mismatch with the on-chain account address.
    pub client: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [SEED_JOB, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump  = job_account.bump,
    )]
    pub job_account: Account<'info, JobAccount>,

    #[account(
        mut,
        seeds = [SEED_JOB_VAULT, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump  = job_account.vault_bump,
    )]
    pub job_vault: Account<'info, JobVault>,

    /// Operator's registry account — read-only cross-program account check.
    /// Anchor verifies this PDA is owned by ochain-registry via seeds::program.
    #[account(
        seeds          = [b"operator", operator_authority.key().as_ref()],
        bump           = operator_account.bump,
        seeds::program = ochain_registry::ID,
        constraint     = operator_account.authority == operator_authority.key()
            @ JobError::OperatorNotActive,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    /// Attestation record for the operator's TEE node.
    ///
    /// This is the cross-program TEE verification step: ochain-attestation is
    /// the sole program that can write AttestationRecord accounts (Anchor verifies
    /// via seeds::program), so a Verified status here is unforgeable on-chain
    /// proof that the TEE quote passed the off-chain verifier service.
    ///
    /// The operator passes their node_pubkey implicitly by providing this account;
    /// Anchor re-derives the PDA using the loaded node_pubkey and rejects any
    /// account that does not satisfy the seed equation.
    #[account(
        seeds          = [b"attestation", attestation_record.node_pubkey.as_ref()],
        bump           = attestation_record.bump,
        seeds::program = ochain_attestation::ID,
        constraint     = attestation_record.operator == operator_authority.key()
            @ JobError::AttestationNotVerified,
    )]
    pub attestation_record: Account<'info, AttestationRecord>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(job_nonce: u64)]
pub struct SubmitResult<'info> {
    #[account(mut)]
    pub operator_authority: Signer<'info>,

    /// The TEE-resident Ed25519 key that must co-sign this transaction.
    /// Verified against operator_account.attestation_pubkey — proves the result
    /// was authorized by the operator's live TEE enclave, not just their hot wallet.
    pub tee_attestation_key: Signer<'info>,

    /// CHECK: used only for PDA seed derivation.
    pub client: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds      = [SEED_JOB, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump       = job_account.bump,
        constraint = job_account.operator == operator_authority.key()
            @ JobError::NotAssignedOperator,
    )]
    pub job_account: Account<'info, JobAccount>,

    /// Closed to operator_authority on success — releases payment + bond + rent.
    #[account(
        mut,
        seeds  = [SEED_JOB_VAULT, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump   = job_account.vault_bump,
        close  = operator_authority,
    )]
    pub job_vault: Account<'info, JobVault>,

    /// Registry record providing the current attestation_pubkey for TEE key check.
    #[account(
        seeds          = [b"operator", operator_authority.key().as_ref()],
        bump           = operator_account.bump,
        seeds::program = ochain_registry::ID,
        constraint     = tee_attestation_key.key() == operator_account.attestation_pubkey
            @ JobError::InvalidTeeKey,
    )]
    pub operator_account: Account<'info, OperatorAccount>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(job_nonce: u64)]
pub struct SlashTimeout<'info> {
    /// Permissionless: any payer can trigger the slash after the deadline.
    #[account(mut)]
    pub caller: Signer<'info>,

    /// CHECK: receives the full vault refund; verified via PDA seed re-derivation
    ///        (if this key is wrong, the job PDA address will not match).
    #[account(mut)]
    pub client: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [SEED_JOB, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump  = job_account.bump,
    )]
    pub job_account: Account<'info, JobAccount>,

    /// Closed to client on slash — client recovers payment + operator's forfeited bond.
    #[account(
        mut,
        seeds = [SEED_JOB_VAULT, client.key().as_ref(), &job_nonce.to_le_bytes()],
        bump  = job_account.vault_bump,
        close = client,
    )]
    pub job_vault: Account<'info, JobVault>,

    pub system_program: Program<'info, System>,
}
