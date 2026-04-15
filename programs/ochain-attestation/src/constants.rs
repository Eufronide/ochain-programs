/// PDA seed bytes
pub const SEED_VERIFIER_STATE: &[u8] = b"verifier_state";
pub const SEED_ATTESTATION:    &[u8] = b"attestation";

/// SHA-256 digest of the raw TEE attestation quote stored on-chain.
/// The full quote (typically 4–8 KB) stays off-chain; we bind it by hash.
pub const QUOTE_HASH_LEN: usize = 32;

/// TDX MRTD or SEV-SNP measurement length (zero-padded for SEV-SNP's 32-byte hash).
pub const MEASUREMENT_HASH_LEN: usize = 48;
