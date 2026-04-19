// display.rs – Terminal output helpers

use colored::Colorize;

// ── Explorer link ─────────────────────────────────────────────────────────────

/// Returns the Solana Explorer URL for a transaction signature.
pub fn explorer_tx(rpc_url: &str, sig: &str) -> String {
    let cluster = if rpc_url.contains("devnet") {
        "devnet"
    } else if rpc_url.contains("testnet") {
        "testnet"
    } else if rpc_url.contains("localhost") || rpc_url.contains("127.0.0.1") {
        "custom"
    } else {
        "mainnet-beta"
    };
    format!("https://explorer.solana.com/tx/{sig}?cluster={cluster}")
}

pub fn explorer_account(rpc_url: &str, pubkey: &str) -> String {
    let cluster = if rpc_url.contains("devnet") {
        "devnet"
    } else if rpc_url.contains("testnet") {
        "testnet"
    } else {
        "mainnet-beta"
    };
    format!("https://explorer.solana.com/address/{pubkey}?cluster={cluster}")
}

// ── Section / step printers ───────────────────────────────────────────────────

pub fn banner() {
    println!();
    println!("{}", "╔══════════════════════════════════════════════════════════════╗".cyan().bold());
    println!("{}", "║         Ochain End-to-End Demo  –  Solana Devnet             ║".cyan().bold());
    println!("{}", "╚══════════════════════════════════════════════════════════════╝".cyan().bold());
    println!();
    println!("  Flow: register operator → add TEE node → submit & verify attestation");
    println!("        → post job → claim job → submit result (TEE co-signed)");
    println!();
}

pub fn section(title: &str) {
    println!();
    println!("{}", format!("──── {title} ").bold());
}

/// Print a transaction result with signature and Explorer link.
pub fn tx_ok(rpc_url: &str, sig: &str, note: &str) {
    println!("  {} {}", "✔".green().bold(), note);
    println!("    sig  : {}", sig.dimmed());
    println!("    link : {}", explorer_tx(rpc_url, sig).blue().underline());
}

/// Print a skipped step.
pub fn skipped(reason: &str) {
    println!("  {} {}", "→".yellow(), reason);
}

/// Print an error that is non-fatal (demo continues).
pub fn soft_error(msg: &str) {
    println!("  {} {}", "✗".red(), msg);
}

// ── Final summary table ───────────────────────────────────────────────────────

pub struct StepResult<'a> {
    pub label: &'a str,
    pub sig:   String,
}

pub fn summary(rpc_url: &str, steps: &[StepResult]) {
    println!();
    println!("{}", "═══════════════════════════════════════════════════════════════".cyan().bold());
    println!("{}", "  Demo complete – transaction summary".cyan().bold());
    println!("{}", "═══════════════════════════════════════════════════════════════".cyan().bold());
    println!();

    let pad = steps.iter().map(|s| s.label.len()).max().unwrap_or(20);

    for (i, step) in steps.iter().enumerate() {
        println!(
            "  {:>2}.  {:<pad$}  {}",
            i + 1,
            step.label.bold(),
            step.sig.dimmed(),
            pad = pad,
        );
        println!("        {}", explorer_tx(rpc_url, &step.sig).blue().underline());
        println!();
    }
}
