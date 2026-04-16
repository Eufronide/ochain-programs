use anchor_lang::prelude::*;

#[error_code]
pub enum JobError {
    #[msg("tee_type must be b\"TD\" (TDX) or b\"SE\" (SEV-SNP)")]
    InvalidTeeType,

    #[msg("payload_hash must be a non-zero 32-byte SHA-256 digest")]
    InvalidPayloadHash,

    #[msg("deadline_slot must be in the future")]
    DeadlineInPast,

    #[msg("payment_lamports is below the protocol minimum")]
    PaymentTooLow,

    #[msg("required_bond_lamports must be greater than zero")]
    BondTooLow,

    #[msg("Job is not in Open status")]
    JobNotOpen,

    #[msg("Job is not in Claimed status")]
    JobNotClaimed,

    #[msg("Job deadline has already passed")]
    DeadlineExpired,

    #[msg("Job deadline has not yet passed")]
    DeadlineNotPassed,

    #[msg("Caller is not the operator assigned to this job")]
    NotAssignedOperator,

    #[msg("Operator's registry status is not Active")]
    OperatorNotActive,

    #[msg("Operator's attestation record is not Verified")]
    AttestationNotVerified,

    #[msg("Operator's TEE type does not match the job requirement")]
    TeeTypeMismatch,

    #[msg("result_hash must be a non-zero 32-byte digest")]
    InvalidResultHash,

    #[msg("Transaction signer does not match the operator's attested TEE key")]
    InvalidTeeKey,
}
