use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod state;

use constants::*;
use errors::AttestationError;
use events::*;
use state::*;

declare_id!("11111111111111111111111111111113");
// ↑ Placeholder — replace with the output of:
//   anchor build && solana-keygen pubkey target/deploy/ochain_attestation-keypair.json

// ============================================================================
//  Program
// ============================================================================

#[program]
pub mod ochain_attestation {
    use super::*;

    // ------------------------------------------------------------------------
    // initialize_verifier
    //
    // One-time call by the deployer.  Creates the VerifierState singleton and
    // sets the initial verifier authority (the off-chain attestation service's
    // signing key).  Authority should be transferred to the governance program
    // PDA via `rotate_verifier` once governance is deployed.
    // ------------------------------------------------------------------------
    pub fn initialize_verifier(
        ctx: Context<InitializeVerifier>,
        verifier_authority: Pubkey,
    ) -> Result<()> {
        let vs = &mut ctx.accounts.verifier_state;
        vs.verifier_authority = verifier_authority;
        vs.bump               = ctx.bumps.verifier_state;

        emit!(VerifierInitialised {
            verifier_authority,
            slot: Clock::get()?.slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // rotate_verifier
    //
    // Replaces the verifier authority.  Only the current authority may call
    // this; governance upgrades will CPI into this instruction.
    // ------------------------------------------------------------------------
    pub fn rotate_verifier(
        ctx: Context<RotateVerifier>,
        new_authority: Pubkey,
    ) -> Result<()> {
        let vs     = &mut ctx.accounts.verifier_state;
        let old    = vs.verifier_authority;
        vs.verifier_authority = new_authority;

        emit!(VerifierRotated {
            old_authority: old,
            new_authority,
            slot: Clock::get()?.slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // submit_attestation
    //
    // Called by an operator to record that a specific node pubkey has been
    // generated inside a TEE enclave.  The operator provides:
    //   • node_pubkey       — the Ed25519 key the enclave generated
    //   • measurement_hash  — MRTD / SEV-SNP measurement of the runtime binary
    //   • tee_type          — b"TD" or b"SE"
    //   • quote_hash        — SHA-256 of the raw attestation quote stored
    //                         off-chain; lets auditors replay verification
    //
    // The record starts in Pending status.  The verifier service watches for
    // this event off-chain, fetches the full quote from the operator's
    // endpoint, validates it, then calls `verify_attestation` or
    // `revoke_attestation` accordingly.
    // ------------------------------------------------------------------------
    pub fn submit_attestation(
        ctx: Context<SubmitAttestation>,
        node_pubkey:      Pubkey,
        measurement_hash: [u8; MEASUREMENT_HASH_LEN],
        tee_type:         [u8; 2],
        quote_hash:       [u8; QUOTE_HASH_LEN],
    ) -> Result<()> {
        require!(
            tee_type == *b"TD" || tee_type == *b"SE",
            AttestationError::InvalidTeeType
        );
        require!(
            node_pubkey != Pubkey::default(),
            AttestationError::InvalidNodePubkey
        );
        require!(
            measurement_hash != [0u8; MEASUREMENT_HASH_LEN],
            AttestationError::InvalidMeasurementHash
        );
        require!(
            quote_hash != [0u8; QUOTE_HASH_LEN],
            AttestationError::InvalidQuoteHash
        );

        let slot = Clock::get()?.slot;
        let rec  = &mut ctx.accounts.attestation_record;
        rec.operator         = ctx.accounts.operator_authority.key();
        rec.node_pubkey      = node_pubkey;
        rec.tee_type         = tee_type;
        rec.measurement_hash = measurement_hash;
        rec.quote_hash       = quote_hash;
        rec.status           = AttestationStatus::Pending;
        rec.submitted_slot   = slot;
        rec.verified_slot    = 0;
        rec.bump             = ctx.bumps.attestation_record;

        emit!(AttestationSubmitted {
            operator: rec.operator,
            node_pubkey,
            tee_type,
            measurement_hash,
            quote_hash,
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // verify_attestation
    //
    // Called by the verifier authority after off-chain inspection of the raw
    // quote confirms it is genuine, unexpired, and the measurement matches the
    // approved OchainRuntime binary hash.
    //
    // Sets status to Verified.  The registry V2 `update_attestation_pubkey`
    // instruction will read this account and require is_verified() == true
    // before accepting the key rotation.
    // ------------------------------------------------------------------------
    pub fn verify_attestation(ctx: Context<VerifyAttestation>) -> Result<()> {
        let rec = &mut ctx.accounts.attestation_record;

        require!(rec.status != AttestationStatus::Verified, AttestationError::AlreadyVerified);
        require!(rec.status != AttestationStatus::Revoked,  AttestationError::Revoked);

        let slot        = Clock::get()?.slot;
        rec.status        = AttestationStatus::Verified;
        rec.verified_slot = slot;

        emit!(AttestationVerified {
            node_pubkey: rec.node_pubkey,
            verifier:    ctx.accounts.verifier_authority.key(),
            slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // revoke_attestation
    //
    // Called by the verifier authority when a previously verified (or pending)
    // attestation is found to be invalid — e.g. the measurement no longer
    // matches the approved binary, a replay was detected, or a CVE forced a
    // measurement revocation.
    //
    // reason codes (off-chain convention, stored in the event only):
    //   0 = measurement mismatch / deprecated binary
    //   1 = quote expired
    //   2 = quote replay / reuse
    //   255 = other / see off-chain incident log
    //
    // NOTE: The account is NOT closed here so the registry's V2 key-rotation
    //       check can still see the Revoked status and reject the rotation.
    //       The operator must re-submit a fresh attestation after remediation.
    // ------------------------------------------------------------------------
    pub fn revoke_attestation(
        ctx: Context<RevokeAttestation>,
        reason: u8,
    ) -> Result<()> {
        let rec        = &mut ctx.accounts.attestation_record;
        rec.status     = AttestationStatus::Revoked;
        rec.verified_slot = 0; // clear the verified timestamp on revoke

        emit!(AttestationRevoked {
            node_pubkey: rec.node_pubkey,
            verifier:    ctx.accounts.verifier_authority.key(),
            reason,
            slot:        Clock::get()?.slot,
        });

        Ok(())
    }

    // ------------------------------------------------------------------------
    // close_attestation
    //
    // Callable by the submitting operator to reclaim rent after a key has been
    // successfully rotated and the old record is no longer needed.  Only
    // allowed on Verified or Revoked records (not Pending — the verifier must
    // act first to prevent premature cleanup).
    // ------------------------------------------------------------------------
    pub fn close_attestation(ctx: Context<CloseAttestation>) -> Result<()> {
        let rec = &ctx.accounts.attestation_record;

        require!(
            rec.status != AttestationStatus::Pending,
            AttestationError::NotVerified,
        );

        emit!(AttestationClosed {
            node_pubkey: rec.node_pubkey,
            operator:    ctx.accounts.operator_authority.key(),
            slot:        Clock::get()?.slot,
        });

        // `close = operator_authority` on the account drains lamports and
        // zeroes the data — Anchor handles this via the constraint below.
        Ok(())
    }
}

// ============================================================================
//  Account Contexts
// ============================================================================

#[derive(Accounts)]
pub struct InitializeVerifier<'info> {
    #[account(mut)]
    pub deployer: Signer<'info>,

    #[account(
        init,
        payer  = deployer,
        space  = VerifierState::SPACE,
        seeds  = [SEED_VERIFIER_STATE],
        bump,
    )]
    pub verifier_state: Account<'info, VerifierState>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct RotateVerifier<'info> {
    /// Must be the current verifier authority.
    pub verifier_authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_VERIFIER_STATE],
        bump  = verifier_state.bump,
        constraint = verifier_state.verifier_authority == verifier_authority.key()
            @ AttestationError::NotVerifier,
    )]
    pub verifier_state: Account<'info, VerifierState>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(
    node_pubkey:      Pubkey,
    measurement_hash: [u8; MEASUREMENT_HASH_LEN],
    tee_type:         [u8; 2],
    quote_hash:       [u8; QUOTE_HASH_LEN],
)]
pub struct SubmitAttestation<'info> {
    #[account(mut)]
    pub operator_authority: Signer<'info>,

    #[account(
        init,
        payer  = operator_authority,
        space  = AttestationRecord::SPACE,
        seeds  = [SEED_ATTESTATION, node_pubkey.as_ref()],
        bump,
    )]
    pub attestation_record: Account<'info, AttestationRecord>,

    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct VerifyAttestation<'info> {
    /// Must be the registered verifier authority.
    pub verifier_authority: Signer<'info>,

    #[account(
        seeds = [SEED_VERIFIER_STATE],
        bump  = verifier_state.bump,
        constraint = verifier_state.verifier_authority == verifier_authority.key()
            @ AttestationError::NotVerifier,
    )]
    pub verifier_state: Account<'info, VerifierState>,

    #[account(
        mut,
        seeds = [SEED_ATTESTATION, attestation_record.node_pubkey.as_ref()],
        bump  = attestation_record.bump,
    )]
    pub attestation_record: Account<'info, AttestationRecord>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct RevokeAttestation<'info> {
    /// Must be the registered verifier authority.
    pub verifier_authority: Signer<'info>,

    #[account(
        seeds = [SEED_VERIFIER_STATE],
        bump  = verifier_state.bump,
        constraint = verifier_state.verifier_authority == verifier_authority.key()
            @ AttestationError::NotVerifier,
    )]
    pub verifier_state: Account<'info, VerifierState>,

    #[account(
        mut,
        seeds = [SEED_ATTESTATION, attestation_record.node_pubkey.as_ref()],
        bump  = attestation_record.bump,
    )]
    pub attestation_record: Account<'info, AttestationRecord>,
}

// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct CloseAttestation<'info> {
    #[account(mut)]
    pub operator_authority: Signer<'info>,

    #[account(
        mut,
        seeds  = [SEED_ATTESTATION, attestation_record.node_pubkey.as_ref()],
        bump   = attestation_record.bump,
        constraint = attestation_record.operator == operator_authority.key()
            @ AttestationError::NotSubmitter,
        close  = operator_authority,
    )]
    pub attestation_record: Account<'info, AttestationRecord>,
}
