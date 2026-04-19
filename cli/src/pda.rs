// pda.rs – PDA derivations mirroring the seed constants in each program.
//
// Seeds are taken directly from the program source:
//   programs/ochain-registry/src/constants.rs
//   programs/ochain-attestation/src/constants.rs
//   programs/ochain-job/src/constants.rs
//
// Each function returns (Pubkey, bump) to match `find_program_address`.

use solana_sdk::pubkey::Pubkey;

// ── ochain-registry ───────────────────────────────────────────────────────────

/// seeds: ["protocol_state"]
pub fn protocol_state(registry_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"protocol_state"], registry_id)
}

/// seeds: ["operator", authority]
pub fn operator_account(authority: &Pubkey, registry_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"operator", authority.as_ref()],
        registry_id,
    )
}

/// seeds: ["stake_vault", authority]
pub fn stake_vault(authority: &Pubkey, registry_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"stake_vault", authority.as_ref()],
        registry_id,
    )
}

/// seeds: ["operator_node", operator_account, node_index]
pub fn node_account(
    operator_account: &Pubkey,
    node_index: u8,
    registry_id: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"operator_node", operator_account.as_ref(), &[node_index]],
        registry_id,
    )
}

// ── ochain-attestation ────────────────────────────────────────────────────────

/// seeds: ["verifier_state"]
pub fn verifier_state(attestation_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"verifier_state"], attestation_id)
}

/// seeds: ["attestation", node_pubkey]
pub fn attestation_record(node_pubkey: &Pubkey, attestation_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"attestation", node_pubkey.as_ref()],
        attestation_id,
    )
}

// ── ochain-job ────────────────────────────────────────────────────────────────

/// seeds: ["job", client, job_nonce.to_le_bytes()]
pub fn job_account(client: &Pubkey, job_nonce: u64, job_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"job", client.as_ref(), &job_nonce.to_le_bytes()],
        job_id,
    )
}

/// seeds: ["job_vault", client, job_nonce.to_le_bytes()]
pub fn job_vault(client: &Pubkey, job_nonce: u64, job_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"job_vault", client.as_ref(), &job_nonce.to_le_bytes()],
        job_id,
    )
}
