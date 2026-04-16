use anchor_lang::prelude::*;

#[event]
pub struct JobPosted {
    pub job:                    Pubkey,
    pub client:                 Pubkey,
    pub payment_lamports:       u64,
    pub required_bond_lamports: u64,
    pub deadline_slot:          u64,
    pub required_tee_type:      [u8; 2],
    pub payload_hash:           [u8; 32],
    pub slot:                   u64,
}

#[event]
pub struct JobClaimed {
    pub job:                 Pubkey,
    pub operator:            Pubkey,
    /// The TEE node key from the AttestationRecord (audit trail).
    pub node_pubkey:         Pubkey,
    pub claim_bond_lamports: u64,
    pub slot:                u64,
}

#[event]
pub struct JobCompleted {
    pub job:         Pubkey,
    pub operator:    Pubkey,
    pub result_hash: [u8; 32],
    /// Total lamports paid out to the operator (payment + bond + vault rent).
    pub payout:      u64,
    pub slot:        u64,
}

#[event]
pub struct JobSlashed {
    pub job:      Pubkey,
    pub operator: Pubkey,
    /// Lamports refunded to the client (full vault: payment + bond + rent).
    pub refund:   u64,
    /// Operator's forfeited bond (informational; included in refund).
    pub slash:    u64,
    pub slot:     u64,
}
