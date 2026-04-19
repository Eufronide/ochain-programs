// config.rs – CLI argument parsing + Anchor.toml fallback for program IDs

use anyhow::{bail, Context, Result};
use clap::Parser;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

// ── Placeholder IDs from the un-deployed workspace ───────────────────────────

const PLACEHOLDER_REGISTRY:    &str = "11111111111111111111111111111112";
const PLACEHOLDER_ATTESTATION: &str = "11111111111111111111111111111113";
const PLACEHOLDER_JOB:         &str = "11111111111111111111111111111114";

// ── Clap argument struct ──────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name  = "ochain-demo",
    about = "End-to-end Ochain flow demo: register operator → attest → post job → claim → submit result",
    long_about = None,
)]
pub struct Args {
    /// Solana RPC URL.
    /// Defaults to devnet; pass https://api.mainnet-beta.solana.com for mainnet.
    #[arg(
        long,
        env = "OCHAIN_RPC_URL",
        default_value = "https://api.devnet.solana.com"
    )]
    pub rpc_url: String,

    /// Path to the fee-payer / verifier keypair JSON file.
    /// This wallet also acts as the job client in the demo.
    #[arg(
        long,
        env = "OCHAIN_WALLET",
        default_value = "~/.config/solana/id.json"
    )]
    pub wallet: String,

    /// ochain-registry program ID.
    /// Falls back to the devnet entry in ../Anchor.toml if omitted.
    #[arg(long, env = "OCHAIN_REGISTRY_ID")]
    pub registry_id: Option<String>,

    /// ochain-attestation program ID.
    #[arg(long, env = "OCHAIN_ATTESTATION_ID")]
    pub attestation_id: Option<String>,

    /// ochain-job program ID.
    #[arg(long, env = "OCHAIN_JOB_ID")]
    pub job_id: Option<String>,

    /// Minimum operator stake in lamports (default: 1 SOL).
    #[arg(long, default_value_t = 1_000_000_000)]
    pub min_stake_lamports: u64,

    /// Epoch duration in slots used when initialising the protocol (default: 54 000 ≈ 6 h).
    #[arg(long, default_value_t = 54_000)]
    pub epoch_duration_slots: u64,

    /// SOL the demo client locks into the job vault (default: 0.02 SOL).
    #[arg(long, default_value_t = 20_000_000)]
    pub payment_lamports: u64,

    /// SOL bond the demo operator must post to claim the job (default: 0.01 SOL).
    #[arg(long, default_value_t = 10_000_000)]
    pub required_bond_lamports: u64,
}

// ── Resolved configuration ────────────────────────────────────────────────────

pub struct Config {
    pub rpc_url:                String,
    pub wallet_path:            String,
    pub registry_id:            Pubkey,
    pub attestation_id:         Pubkey,
    pub job_id:                 Pubkey,
    pub min_stake_lamports:     u64,
    pub epoch_duration_slots:   u64,
    pub payment_lamports:       u64,
    pub required_bond_lamports: u64,
}

pub fn load() -> Result<Config> {
    let args = Args::parse();

    let registry_id    = resolve_id(&args.registry_id,    "ochain_registry",    PLACEHOLDER_REGISTRY)?;
    let attestation_id = resolve_id(&args.attestation_id, "ochain_attestation", PLACEHOLDER_ATTESTATION)?;
    let job_id         = resolve_id(&args.job_id,         "ochain_job",         PLACEHOLDER_JOB)?;

    Ok(Config {
        rpc_url:                args.rpc_url,
        wallet_path:            expand_tilde(&args.wallet),
        registry_id,
        attestation_id,
        job_id,
        min_stake_lamports:     args.min_stake_lamports,
        epoch_duration_slots:   args.epoch_duration_slots,
        payment_lamports:       args.payment_lamports,
        required_bond_lamports: args.required_bond_lamports,
    })
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Resolve a program ID: explicit flag → Anchor.toml devnet → Anchor.toml localnet.
/// Returns an error if the resolved ID is still a placeholder (not yet deployed).
fn resolve_id(flag: &Option<String>, toml_key: &str, placeholder: &str) -> Result<Pubkey> {
    // 1. Explicit flag / env-var wins.
    if let Some(raw) = flag {
        return Pubkey::from_str(raw)
            .with_context(|| format!("invalid pubkey for {toml_key}: {raw}"));
    }

    // 2. Read from ../Anchor.toml (relative to cwd; works when run from cli/ or workspace root).
    for cluster in &["devnet", "localnet"] {
        if let Some(raw) = read_anchor_toml(toml_key, cluster) {
            if raw != placeholder {
                return Pubkey::from_str(&raw)
                    .with_context(|| format!("malformed pubkey in Anchor.toml for {toml_key}"));
            }
        }
    }

    // 3. Nothing usable.
    bail!(
        "Program ID for `{toml_key}` is still the placeholder {placeholder}.\n\
         Run `./deploy.sh` first, or pass --{} <PUBKEY>.",
        toml_key.replace('_', "-")
    );
}

/// Extract a program ID string from `[programs.<cluster>]` in Anchor.toml.
/// Searches `../Anchor.toml` relative to the current working directory.
fn read_anchor_toml(program_key: &str, cluster: &str) -> Option<String> {
    // Try workspace root first, then one level up (when cwd is cli/).
    let candidates = ["Anchor.toml", "../Anchor.toml"];
    for path in &candidates {
        if let Ok(content) = std::fs::read_to_string(path) {
            let section_header = format!("[programs.{}]", cluster);
            let mut in_section = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed == section_header {
                    in_section = true;
                    continue;
                }
                if in_section && trimmed.starts_with('[') {
                    break; // left the section
                }
                if in_section {
                    // Match lines like:  ochain_registry    = "ABC123..."
                    if let Some(rest) = trimmed.strip_prefix(program_key) {
                        let rest = rest.trim_start();
                        if rest.starts_with('=') {
                            let val = rest[1..].trim().trim_matches('"');
                            if !val.is_empty() {
                                return Some(val.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Expand `~/` to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        if !home.is_empty() {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}
