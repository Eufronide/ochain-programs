use anchor_lang::prelude::*;

#[event]
pub struct ProtocolInitialised {
    pub authority:           Pubkey,
    pub min_stake_lamports:  u64,
    pub epoch_duration_slots: u64,
    pub slot:                u64,
}

#[event]
pub struct OperatorRegistered {
    pub operator:   Pubkey,
    pub authority:  Pubkey,
    pub stake:      u64,
    pub tee_type:   [u8; 2],
    pub slot:       u64,
}

#[event]
pub struct NodeAdded {
    pub operator:          Pubkey,
    pub node_pubkey:       Pubkey,
    pub node_index:        u8,
    pub measurement_hash:  [u8; 48],
    pub slot:              u64,
}

#[event]
pub struct HeartbeatReceived {
    pub operator:    Pubkey,
    pub node_pubkey: Pubkey,
    pub node_index:  u8,
    pub slot:        u64,
}

#[event]
pub struct SlaViolationRecorded {
    pub operator:        Pubkey,
    pub node_index:      u8,
    pub sla_violations:  u8,
    pub new_reputation:  u16,
    /// True if this violation pushed the operator into Suspended status.
    pub suspended:       bool,
    pub slot:            u64,
}

#[event]
pub struct ExitBegun {
    pub operator:             Pubkey,
    pub exit_initiated_slot:  u64,
    pub earliest_exit_slot:   u64,
}

#[event]
pub struct ExitFinalised {
    pub operator:       Pubkey,
    pub stake_returned: u64,
    pub slot:           u64,
}

#[event]
pub struct AttestationKeyUpdated {
    pub operator:     Pubkey,
    pub node_index:   u8,
    pub old_key:      Pubkey,
    pub new_key:      Pubkey,
    pub slot:         u64,
}

/// Emitted by the governance CPI call to slash an operator.
#[event]
pub struct OperatorSlashed {
    pub operator:      Pubkey,
    pub slash_amount:  u64,
    pub reason:        u8,
    pub slot:          u64,
}
