use anchor_lang::prelude::*;

#[event]
pub struct VerifierInitialised {
    pub verifier_authority: Pubkey,
    pub slot:               u64,
}

#[event]
pub struct VerifierRotated {
    pub old_authority: Pubkey,
    pub new_authority: Pubkey,
    pub slot:          u64,
}

#[event]
pub struct AttestationSubmitted {
    pub operator:         Pubkey,
    pub node_pubkey:      Pubkey,
    pub tee_type:         [u8; 2],
    pub measurement_hash: [u8; 48],
    pub quote_hash:       [u8; 32],
    pub slot:             u64,
}

#[event]
pub struct AttestationVerified {
    pub node_pubkey: Pubkey,
    pub verifier:    Pubkey,
    pub slot:        u64,
}

#[event]
pub struct AttestationRevoked {
    pub node_pubkey: Pubkey,
    pub verifier:    Pubkey,
    /// Off-chain reason code (0 = measurement mismatch, 1 = quote expired,
    /// 2 = quote replay, 255 = other).
    pub reason:      u8,
    pub slot:        u64,
}

#[event]
pub struct AttestationClosed {
    pub node_pubkey: Pubkey,
    pub operator:    Pubkey,
    pub slot:        u64,
}
