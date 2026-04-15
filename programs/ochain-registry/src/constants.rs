/// ~14 days at 400 ms/slot (400ms × 3_024_000 = 1_209_600 s ≈ 14 days)
pub const UNBONDING_SLOTS: u64 = 3_024_000;

/// SLA: two missed epochs trigger a violation check
pub const SLA_MISS_MULTIPLIER: u64 = 2;

/// Reputation scores
pub const INITIAL_REPUTATION_SCORE:   u16 = 5_000;
pub const MAX_REPUTATION_SCORE:       u16 = 10_000;
pub const REPUTATION_GAIN_PER_JOB:    u16 = 10;
pub const REPUTATION_PENALTY_SLA:     u16 = 500;
pub const REPUTATION_PENALTY_TIMEOUT: u16 = 200;

/// How many SLA violations before auto-suspension
pub const SUSPENSION_VIOLATION_THRESHOLD: u8 = 3;

/// URL byte limits
pub const MAX_ENDPOINT_URL_LEN: usize = 200;

/// Default minimum stake: 1 SOL in lamports
pub const DEFAULT_MIN_STAKE_LAMPORTS: u64 = 1_000_000_000;

/// Default slash: 10 % (1000 bps)
pub const DEFAULT_SLASH_BASIS_POINTS: u16 = 1_000;

/// Default epoch: ~6 hours at 400 ms/slot
pub const DEFAULT_EPOCH_DURATION_SLOTS: u64 = 54_000;

/// PDA seed bytes
pub const SEED_PROTOCOL_STATE: &[u8] = b"protocol_state";
pub const SEED_OPERATOR:        &[u8] = b"operator";
pub const SEED_STAKE_VAULT:     &[u8] = b"stake_vault";
pub const SEED_OPERATOR_NODE:   &[u8] = b"operator_node";
