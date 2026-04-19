// ix.rs – Raw instruction builders for the three Ochain programs.
//
// DESIGN
// ──────
// Anchor encodes every instruction as:
//   bytes[0..8]  = sha256("global:<instruction_name>")[0..8]   (discriminator)
//   bytes[8..]   = Borsh-serialised arguments in the order they appear
//                  in the Rust function signature (not the Accounts struct)
//
// Account lists mirror the #[derive(Accounts)] structs in each program.
// The ordering, writable (mut) and signer flags must match exactly or the
// runtime will reject the transaction.
//
// Pubkeys are stored as [u8; 32] in Borsh structs (avoiding a cross-crate
// BorshSerialize impl dependency on solana_sdk::pubkey::Pubkey).

use borsh::BorshSerialize;
use sha2::{Digest, Sha256};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_program,
};

// ── discriminator helper ──────────────────────────────────────────────────────

fn disc(name: &str) -> [u8; 8] {
    let hash = Sha256::digest(format!("global:{name}"));
    hash[..8].try_into().unwrap()
}

fn build_data(discriminator: [u8; 8], args: impl BorshSerialize) -> Vec<u8> {
    let mut data = discriminator.to_vec();
    args.serialize(&mut data).expect("borsh serialisation failed");
    data
}

// ── ochain-registry ───────────────────────────────────────────────────────────

pub mod registry {
    use super::*;

    // initialize_protocol(min_stake_lamports, slash_basis_points, epoch_duration_slots, max_nodes_per_operator)
    #[derive(BorshSerialize)]
    struct InitProtocolArgs {
        min_stake_lamports:      u64,
        slash_basis_points:      u16,
        epoch_duration_slots:    u64,
        max_nodes_per_operator:  u8,
    }

    pub fn initialize_protocol(
        authority:             &Pubkey,
        treasury:              &Pubkey,
        protocol_state:        &Pubkey,
        program_id:            &Pubkey,
        min_stake_lamports:    u64,
        slash_basis_points:    u16,
        epoch_duration_slots:  u64,
        max_nodes_per_operator: u8,
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*authority, true),          // signer, writable (payer)
                AccountMeta::new_readonly(*treasury, false), // just a pubkey stored in state
                AccountMeta::new(*protocol_state, false),    // writable (init)
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("initialize_protocol"), InitProtocolArgs {
                min_stake_lamports,
                slash_basis_points,
                epoch_duration_slots,
                max_nodes_per_operator,
            }),
        }
    }

    // register_operator(tee_type, attestation_pubkey, endpoint_url, stake_amount)
    //
    // Argument order mirrors the Rust function signature exactly:
    //   fn register_operator(ctx, tee_type, attestation_pubkey, endpoint_url, stake_amount)
    #[derive(BorshSerialize)]
    struct RegisterOperatorArgs {
        tee_type:           [u8; 2],
        attestation_pubkey: [u8; 32], // Pubkey serialises as 32 raw bytes
        endpoint_url:       Vec<u8>,  // borsh: u32-LE-len prefix + bytes
        stake_amount:       u64,
    }

    pub fn register_operator(
        operator_authority: &Pubkey,
        operator_account:   &Pubkey,
        stake_vault:        &Pubkey,
        protocol_state:     &Pubkey,
        program_id:         &Pubkey,
        tee_type:           [u8; 2],
        attestation_pubkey: Pubkey,
        endpoint_url:       Vec<u8>,
        stake_amount:       u64,
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*operator_authority, true), // signer, writable (payer + transfers stake)
                AccountMeta::new(*operator_account, false),  // writable (init)
                AccountMeta::new(*stake_vault, false),       // writable (init + receives stake)
                AccountMeta::new(*protocol_state, false),    // writable (operator_count++)
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("register_operator"), RegisterOperatorArgs {
                tee_type,
                attestation_pubkey: attestation_pubkey.to_bytes(),
                endpoint_url,
                stake_amount,
            }),
        }
    }

    // add_node(node_index, attestation_pubkey, measurement_hash)
    #[derive(BorshSerialize)]
    struct AddNodeArgs {
        node_index:         u8,
        attestation_pubkey: [u8; 32],
        measurement_hash:   [u8; 48], // TDX MRTD size; SEV-SNP zero-padded to 48
    }

    pub fn add_node(
        operator_authority: &Pubkey,
        operator_account:   &Pubkey,
        node_account:       &Pubkey,
        protocol_state:     &Pubkey,
        program_id:         &Pubkey,
        node_index:         u8,
        attestation_pubkey: Pubkey,
        measurement_hash:   [u8; 48],
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*operator_authority, true), // signer, writable (payer)
                AccountMeta::new(*operator_account, false),  // writable (node_count++)
                AccountMeta::new(*node_account, false),      // writable (init)
                AccountMeta::new_readonly(*protocol_state, false),
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("add_node"), AddNodeArgs {
                node_index,
                attestation_pubkey: attestation_pubkey.to_bytes(),
                measurement_hash,
            }),
        }
    }
}

// ── ochain-attestation ────────────────────────────────────────────────────────

pub mod attestation {
    use super::*;

    // initialize_verifier(verifier_authority)
    #[derive(BorshSerialize)]
    struct InitVerifierArgs {
        verifier_authority: [u8; 32],
    }

    pub fn initialize_verifier(
        deployer:           &Pubkey,
        verifier_state:     &Pubkey,
        program_id:         &Pubkey,
        verifier_authority: Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*deployer, true),           // signer, writable (payer)
                AccountMeta::new(*verifier_state, false),    // writable (init)
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("initialize_verifier"), InitVerifierArgs {
                verifier_authority: verifier_authority.to_bytes(),
            }),
        }
    }

    // submit_attestation(node_pubkey, measurement_hash, tee_type, quote_hash)
    //
    // Argument order mirrors:
    //   fn submit_attestation(ctx, node_pubkey, measurement_hash, tee_type, quote_hash)
    #[derive(BorshSerialize)]
    struct SubmitAttestationArgs {
        node_pubkey:      [u8; 32],
        measurement_hash: [u8; 48],
        tee_type:         [u8; 2],
        quote_hash:       [u8; 32],
    }

    pub fn submit_attestation(
        operator_authority: &Pubkey,
        attestation_record: &Pubkey,
        program_id:         &Pubkey,
        node_pubkey:        Pubkey,
        measurement_hash:   [u8; 48],
        tee_type:           [u8; 2],
        quote_hash:         [u8; 32],
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*operator_authority, true), // signer, writable (payer)
                AccountMeta::new(*attestation_record, false), // writable (init)
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("submit_attestation"), SubmitAttestationArgs {
                node_pubkey:      node_pubkey.to_bytes(),
                measurement_hash,
                tee_type,
                quote_hash,
            }),
        }
    }

    // verify_attestation()  — no instruction arguments, only accounts
    pub fn verify_attestation(
        verifier_authority: &Pubkey,
        verifier_state:     &Pubkey,
        attestation_record: &Pubkey,
        program_id:         &Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                // verifier_authority has no `#[account(mut)]` in the context →
                // is_writable = false (runtime takes fees from the tx fee-payer, not here)
                AccountMeta::new_readonly(*verifier_authority, true),
                AccountMeta::new_readonly(*verifier_state, false),
                AccountMeta::new(*attestation_record, false), // writable (status → Verified)
            ],
            data: disc("verify_attestation").to_vec(), // discriminator only, no args
        }
    }
}

// ── ochain-job ────────────────────────────────────────────────────────────────

pub mod job {
    use super::*;

    // post_job(job_nonce, payload_hash, deadline_slot, required_tee_type,
    //          payment_lamports, required_bond_lamports)
    #[derive(BorshSerialize)]
    struct PostJobArgs {
        job_nonce:              u64,
        payload_hash:           [u8; 32],
        deadline_slot:          u64,
        required_tee_type:      [u8; 2],
        payment_lamports:       u64,
        required_bond_lamports: u64,
    }

    pub fn post_job(
        client:                 &Pubkey,
        job_account:            &Pubkey,
        job_vault:              &Pubkey,
        program_id:             &Pubkey,
        job_nonce:              u64,
        payload_hash:           [u8; 32],
        deadline_slot:          u64,
        required_tee_type:      [u8; 2],
        payment_lamports:       u64,
        required_bond_lamports: u64,
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*client, true),       // signer, writable (payer + transfers payment)
                AccountMeta::new(*job_account, false), // writable (init)
                AccountMeta::new(*job_vault, false),   // writable (init + receives payment)
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("post_job"), PostJobArgs {
                job_nonce,
                payload_hash,
                deadline_slot,
                required_tee_type,
                payment_lamports,
                required_bond_lamports,
            }),
        }
    }

    // claim_job(job_nonce)
    //
    // Cross-program account reads: operator_account (registry-owned),
    // attestation_record (attestation-owned).  Both are readonly here —
    // the job program reads them for gating checks but does not write them.
    #[derive(BorshSerialize)]
    struct ClaimJobArgs {
        job_nonce: u64,
    }

    pub fn claim_job(
        operator_authority: &Pubkey,
        client:             &Pubkey,
        job_account:        &Pubkey,
        job_vault:          &Pubkey,
        operator_account:   &Pubkey,
        attestation_record: &Pubkey,
        program_id:         &Pubkey,
        job_nonce:          u64,
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*operator_authority, true),          // signer, writable (pays bond)
                AccountMeta::new_readonly(*client, false),            // seed for PDA derivation
                AccountMeta::new(*job_account, false),                // writable (update operator, status)
                AccountMeta::new(*job_vault, false),                  // writable (receives bond)
                AccountMeta::new_readonly(*operator_account, false),  // cross-program read: registry
                AccountMeta::new_readonly(*attestation_record, false), // cross-program read: attestation
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("claim_job"), ClaimJobArgs { job_nonce }),
        }
    }

    // submit_result(job_nonce, result_hash)
    //
    // IMPORTANT: both operator_authority AND tee_attestation_key must sign.
    // The constraint `tee_attestation_key.key() == operator_account.attestation_pubkey`
    // proves the result was authorised by the live TEE enclave, not just the hot wallet.
    #[derive(BorshSerialize)]
    struct SubmitResultArgs {
        job_nonce:   u64,
        result_hash: [u8; 32],
    }

    pub fn submit_result(
        operator_authority:  &Pubkey,
        tee_attestation_key: &Pubkey,
        client:              &Pubkey,
        job_account:         &Pubkey,
        job_vault:           &Pubkey,
        operator_account:    &Pubkey,
        program_id:          &Pubkey,
        job_nonce:           u64,
        result_hash:         [u8; 32],
    ) -> Instruction {
        Instruction {
            program_id: *program_id,
            accounts: vec![
                AccountMeta::new(*operator_authority, true),          // signer, writable (receives vault payout)
                AccountMeta::new_readonly(*tee_attestation_key, true), // signer (TEE proof co-signer)
                AccountMeta::new_readonly(*client, false),            // seed for PDA derivation
                AccountMeta::new(*job_account, false),                // writable (set result_hash, status)
                AccountMeta::new(*job_vault, false),                  // writable (closed → operator)
                AccountMeta::new_readonly(*operator_account, false),  // reads attestation_pubkey
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: build_data(disc("submit_result"), SubmitResultArgs {
                job_nonce,
                result_hash,
            }),
        }
    }
}
