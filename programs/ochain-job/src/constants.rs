pub const SEED_JOB:       &[u8] = b"job";
pub const SEED_JOB_VAULT: &[u8] = b"job_vault";

pub const PAYLOAD_HASH_LEN: usize = 32;
pub const RESULT_HASH_LEN:  usize = 32;

/// Anti-spam floor: 0.005 SOL
pub const MIN_PAYMENT_LAMPORTS: u64 = 5_000_000;
