// ochain-demo  –  End-to-end Ochain flow on Solana devnet
//
// WHAT THIS DEMO DOES
// ───────────────────
// Plays all roles (operator, client, verifier) from a single funded wallet,
// using ephemeral keypairs so every run starts clean.
//
// Step 0  Fund ephemeral operator key      (system transfer from main wallet)
// Step 1  initialize_protocol              (skip if already initialised)
// Step 2  initialize_verifier              (skip if already initialised)
// Step 3  register_operator                ochain-registry
// Step 4  add_node  (index 0)              ochain-registry
// Step 5  submit_attestation               ochain-attestation  → Pending
// Step 6  verify_attestation               ochain-attestation  → Verified
// Step 7  post_job                         ochain-job
// Step 8  claim_job                        ochain-job  (checks Active + Verified)
// Step 9  submit_result                    ochain-job  (TEE key co-signs)
//
// PREREQUISITES
// ─────────────
//   1. anchor build  (programs compiled; program IDs in target/deploy/)
//   2. ./deploy.sh   (programs live on devnet; Anchor.toml updated)
//   3. Funded devnet wallet at ~/.config/solana/id.json
//        solana airdrop 5 --url devnet   # run 3–4× if balance is below 5 SOL
//
// USAGE
// ─────
//   cargo run --bin ochain-demo
//   cargo run --bin ochain-demo -- --help
//   cargo run --bin ochain-demo -- --rpc-url https://api.devnet.solana.com

mod config;
mod display;
mod ix;
mod pda;

use anyhow::{Context, Result};
use display::StepResult;
use sha2::{Digest, Sha256};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};

// ── entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cfg = config::load()?;
    run_demo(&cfg)
}

// ── demo orchestration ────────────────────────────────────────────────────────

fn run_demo(cfg: &config::Config) -> Result<()> {
    display::banner();

    let client = RpcClient::new_with_commitment(
        cfg.rpc_url.clone(),
        CommitmentConfig::confirmed(),
    );

    // ── Keypairs ──────────────────────────────────────────────────────────────
    //
    // main_wallet  – loaded from disk; acts as fee-payer, job client, and
    //                verifier authority for the demo.
    //
    // operator_kp  – ephemeral; acts as the registered node operator.
    //                Generated fresh every run so registration never conflicts.
    //
    // node_kp      – ephemeral; represents the Ed25519 key generated inside
    //                the TEE enclave.  It co-signs submit_result to prove the
    //                result originated from the live enclave, not just the
    //                operator's hot wallet.

    let main_wallet = read_keypair_file(&cfg.wallet_path)
        .with_context(|| format!("cannot read wallet: {}", cfg.wallet_path))?;

    let operator_kp = Keypair::new();
    let node_kp     = Keypair::new();

    display::section("Keypairs");
    println!("  main wallet  (payer / client / verifier)  {}", main_wallet.pubkey());
    println!("  operator     (ephemeral)                  {}", operator_kp.pubkey());
    println!("  TEE node key (ephemeral)                  {}", node_kp.pubkey());
    println!();
    println!("  registry    program  {}", cfg.registry_id);
    println!("  attestation program  {}", cfg.attestation_id);
    println!("  job         program  {}", cfg.job_id);

    // ── Fund main wallet ──────────────────────────────────────────────────────
    //
    // Needed: stake (1 SOL) + operator rents (~0.01 SOL) + bond + payments + fees
    // Buffer: 0.1 SOL extra.  Total: ~1.15 SOL minimum.
    let min_main = cfg.min_stake_lamports
        + cfg.payment_lamports
        + 100_000_000; // 0.1 SOL buffer for rents and fees

    ensure_funded(&client, &main_wallet.pubkey(), min_main)
        .context("failed to fund main wallet; run `solana airdrop 5 --url devnet` and retry")?;

    // ── Derive all PDAs up front ──────────────────────────────────────────────
    let (protocol_state, _)     = pda::protocol_state(&cfg.registry_id);
    let (operator_account, _)   = pda::operator_account(&operator_kp.pubkey(), &cfg.registry_id);
    let (stake_vault, _)        = pda::stake_vault(&operator_kp.pubkey(), &cfg.registry_id);
    let (node_account_0, _)     = pda::node_account(&operator_account, 0, &cfg.registry_id);
    let (verifier_state, _)     = pda::verifier_state(&cfg.attestation_id);
    let (attestation_record, _) = pda::attestation_record(&node_kp.pubkey(), &cfg.attestation_id);

    // Use current unix timestamp as job nonce – unique per run, fits in u64.
    let job_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let (job_account, _) = pda::job_account(&main_wallet.pubkey(), job_nonce, &cfg.job_id);
    let (job_vault, _)   = pda::job_vault(&main_wallet.pubkey(), job_nonce, &cfg.job_id);

    // ── Demo payload / hashes ─────────────────────────────────────────────────
    //
    // In production these would be real TEE measurements and job payloads.
    // Here we use deterministic hashes of labelled strings for readability.

    // measurement_hash: 48 bytes (TDX MRTD size; SEV-SNP zero-pads to 48)
    let measurement_hash: [u8; 48] = {
        let h = Sha256::digest(b"ochain-demo-measurement-v1");
        let mut arr = [0u8; 48];
        arr[..32].copy_from_slice(&h);
        arr
    };
    // quote_hash: SHA-256 of the (simulated) raw TEE attestation quote
    let quote_hash: [u8; 32]   = Sha256::digest(b"ochain-demo-tee-quote-v1").into();
    // payload_hash: SHA-256 of the off-chain job payload
    let payload_hash: [u8; 32] = Sha256::digest(
        b"run-llm-inference:model=gpt4o-mini:prompt-id=abc123"
    ).into();
    // result_hash: SHA-256 of the TEE-computed result
    let result_hash: [u8; 32]  = Sha256::digest(
        b"llm-result:tokens=1024:output-hash=0xdeadbeef"
    ).into();

    let tee_type = *b"TD"; // Intel TDX

    // Deadline: current slot + 2 000 ≈ 22 minutes (400 ms/slot × 2000 = 800 s)
    let deadline_slot = client.get_slot()? + 2_000;

    // Collect completed (step label, signature) pairs for the final summary.
    let mut results: Vec<StepResult> = Vec::new();

    // ── Step 0: Fund operator ephemeral keypair ───────────────────────────────
    //
    // Anchor's register_operator uses `payer = operator_authority`, so
    // operator_kp must hold enough lamports to cover:
    //   • stake transfer (min_stake_lamports)
    //   • operator_account rent  (~0.003 SOL)
    //   • stake_vault rent       (~0.001 SOL)
    //   • node_account rent      (~0.002 SOL)
    //   • claim bond
    //   • transaction fees       (~0.000025 SOL × 4 txns)

    display::section("Step 0 / 9 – fund operator ephemeral keypair");
    let operator_need = cfg.min_stake_lamports
        + cfg.required_bond_lamports
        + 50_000_000; // 0.05 SOL for rents + fees

    let sig = send_tx(
        &client,
        &system_instruction::transfer(
            &main_wallet.pubkey(),
            &operator_kp.pubkey(),
            operator_need,
        ),
        &[&main_wallet],
        &main_wallet,
    ).context("failed to fund operator keypair")?;

    display::tx_ok(
        &cfg.rpc_url,
        &sig,
        &format!(
            "Transferred {:.4} SOL → operator ephemeral key",
            operator_need as f64 / LAMPORTS_PER_SOL as f64
        ),
    );
    results.push(StepResult { label: "fund operator", sig });

    // ── Step 1: initialize_protocol ───────────────────────────────────────────
    //
    // Creates the global ProtocolState singleton.  Skipped if it already
    // exists (e.g. a previous demo run on the same cluster).

    display::section("Step 1 / 9 – initialize_protocol");
    if account_exists(&client, &protocol_state)? {
        display::skipped("ProtocolState already exists – skipping initialization");
    } else {
        let sig = send_tx(
            &client,
            &ix::registry::initialize_protocol(
                &main_wallet.pubkey(),
                &main_wallet.pubkey(), // treasury = main wallet for demo
                &protocol_state,
                &cfg.registry_id,
                cfg.min_stake_lamports,
                1_000,                    // slash_basis_points = 10%
                cfg.epoch_duration_slots,
                10,                       // max_nodes_per_operator
            ),
            &[&main_wallet],
            &main_wallet,
        ).context("initialize_protocol failed")?;

        display::tx_ok(&cfg.rpc_url, &sig, "ProtocolState created");
        results.push(StepResult { label: "initialize_protocol", sig });
    }

    // ── Step 2: initialize_verifier ───────────────────────────────────────────
    //
    // Creates the VerifierState singleton and sets verifier_authority to the
    // main wallet so the demo can self-verify in step 6.

    display::section("Step 2 / 9 – initialize_verifier");
    if account_exists(&client, &verifier_state)? {
        display::skipped("VerifierState already exists – skipping initialization");

        // Safety check: the existing verifier_authority must be main_wallet.
        // If not, step 6 (verify_attestation) will fail with NotVerifier.
        println!(
            "  Note: if the existing verifier_authority ≠ {}, step 6 will fail.",
            main_wallet.pubkey()
        );
    } else {
        let sig = send_tx(
            &client,
            &ix::attestation::initialize_verifier(
                &main_wallet.pubkey(),
                &verifier_state,
                &cfg.attestation_id,
                main_wallet.pubkey(), // verifier_authority = main_wallet for demo
            ),
            &[&main_wallet],
            &main_wallet,
        ).context("initialize_verifier failed")?;

        display::tx_ok(&cfg.rpc_url, &sig, "VerifierState created (authority = main wallet)");
        results.push(StepResult { label: "initialize_verifier", sig });
    }

    // ── Step 3: register_operator ─────────────────────────────────────────────
    //
    // Creates OperatorAccount + StakeVault and locks min_stake_lamports as bond.
    // tee_type = "TD" (Intel TDX) must match the job requirement in step 7.

    display::section("Step 3 / 9 – register_operator");
    let endpoint_url = b"https://operator.example.com/tee".to_vec();

    let sig = send_tx(
        &client,
        &ix::registry::register_operator(
            &operator_kp.pubkey(),
            &operator_account,
            &stake_vault,
            &protocol_state,
            &cfg.registry_id,
            tee_type,
            node_kp.pubkey(),   // attestation_pubkey = TEE node key
            endpoint_url,
            cfg.min_stake_lamports,
        ),
        &[&operator_kp],
        &operator_kp,
    ).context("register_operator failed")?;

    display::tx_ok(
        &cfg.rpc_url,
        &sig,
        &format!(
            "Operator registered – {} SOL staked, status=Active, tee_type=TD",
            cfg.min_stake_lamports as f64 / LAMPORTS_PER_SOL as f64
        ),
    );
    println!("    operator_account : {}", display::explorer_account(&cfg.rpc_url, &operator_account.to_string()));
    results.push(StepResult { label: "register_operator", sig });

    // ── Step 4: add_node (index 0) ────────────────────────────────────────────
    //
    // Registers the first (and only) TEE node under this operator.
    // node_index must be exactly operator_account.node_count (currently 0).

    display::section("Step 4 / 9 – add_node");
    let sig = send_tx(
        &client,
        &ix::registry::add_node(
            &operator_kp.pubkey(),
            &operator_account,
            &node_account_0,
            &protocol_state,
            &cfg.registry_id,
            0,              // node_index
            node_kp.pubkey(),
            measurement_hash,
        ),
        &[&operator_kp],
        &operator_kp,
    ).context("add_node failed")?;

    display::tx_ok(
        &cfg.rpc_url,
        &sig,
        &format!("NodeAccount[0] created  node_key={}", node_kp.pubkey()),
    );
    results.push(StepResult { label: "add_node[0]", sig });

    // ── Step 5: submit_attestation ────────────────────────────────────────────
    //
    // Operator records the TEE node's attestation on-chain.
    // In production the quote_hash points to a real TDX/SEV-SNP quote stored
    // at the operator's endpoint URL.  The verifier service watches for
    // AttestationSubmitted events and fetches the full quote to inspect.

    display::section("Step 5 / 9 – submit_attestation");
    let sig = send_tx(
        &client,
        &ix::attestation::submit_attestation(
            &operator_kp.pubkey(),
            &attestation_record,
            &cfg.attestation_id,
            node_kp.pubkey(),
            measurement_hash,
            tee_type,
            quote_hash,
        ),
        &[&operator_kp],
        &operator_kp,
    ).context("submit_attestation failed")?;

    display::tx_ok(
        &cfg.rpc_url,
        &sig,
        &format!(
            "AttestationRecord created  status=Pending  node_key={}",
            node_kp.pubkey()
        ),
    );
    println!("    attestation_record : {}", display::explorer_account(&cfg.rpc_url, &attestation_record.to_string()));
    results.push(StepResult { label: "submit_attestation", sig });

    // ── Step 6: verify_attestation ────────────────────────────────────────────
    //
    // In production the off-chain verifier service signs this transaction after
    // fetching the full TEE quote from the operator and confirming:
    //   (a) the quote is a valid TDX/SEV-SNP attestation report
    //   (b) the enclave measurement matches an approved OchainRuntime binary
    //   (c) the report is fresh (not replayed)
    //
    // In this demo, main_wallet acts as the verifier authority.

    display::section("Step 6 / 9 – verify_attestation  (main wallet acts as verifier)");
    let sig = send_tx(
        &client,
        &ix::attestation::verify_attestation(
            &main_wallet.pubkey(),
            &verifier_state,
            &attestation_record,
            &cfg.attestation_id,
        ),
        &[&main_wallet],
        &main_wallet,
    ).context("verify_attestation failed – is the verifier_authority in VerifierState == main_wallet?")?;

    display::tx_ok(&cfg.rpc_url, &sig, "AttestationRecord status → Verified  (TEE quote confirmed)");
    results.push(StepResult { label: "verify_attestation", sig });

    // ── Step 7: post_job ──────────────────────────────────────────────────────
    //
    // Main wallet plays the role of job client.  Locks payment_lamports into
    // a job-specific vault PDA.  required_tee_type = "TD" means only TDX
    // operators can claim this job.

    display::section("Step 7 / 9 – post_job");
    let sig = send_tx(
        &client,
        &ix::job::post_job(
            &main_wallet.pubkey(),
            &job_account,
            &job_vault,
            &cfg.job_id,
            job_nonce,
            payload_hash,
            deadline_slot,
            tee_type, // required_tee_type = TD
            cfg.payment_lamports,
            cfg.required_bond_lamports,
        ),
        &[&main_wallet],
        &main_wallet,
    ).context("post_job failed")?;

    display::tx_ok(
        &cfg.rpc_url,
        &sig,
        &format!(
            "Job posted  nonce={}  payment={:.4} SOL  bond={:.4} SOL  deadline_slot={}",
            job_nonce,
            cfg.payment_lamports       as f64 / LAMPORTS_PER_SOL as f64,
            cfg.required_bond_lamports as f64 / LAMPORTS_PER_SOL as f64,
            deadline_slot,
        ),
    );
    println!("    job_account : {}", display::explorer_account(&cfg.rpc_url, &job_account.to_string()));
    results.push(StepResult { label: "post_job", sig });

    // ── Step 8: claim_job ─────────────────────────────────────────────────────
    //
    // The operator (operator_kp) claims the open job.  The program verifies:
    //   • operator_account.status == Active           (registry cross-read)
    //   • operator_account.tee_type == "TD"           (matches job requirement)
    //   • attestation_record.is_verified() == true    (attestation cross-read)
    //   • attestation_record.tee_type == "TD"
    // Then locks required_bond_lamports into the vault alongside the payment.

    display::section("Step 8 / 9 – claim_job");
    let sig = send_tx(
        &client,
        &ix::job::claim_job(
            &operator_kp.pubkey(),
            &main_wallet.pubkey(), // client (seed for job PDA)
            &job_account,
            &job_vault,
            &operator_account,
            &attestation_record,
            &cfg.job_id,
            job_nonce,
        ),
        &[&operator_kp],
        &operator_kp,
    ).context("claim_job failed")?;

    display::tx_ok(
        &cfg.rpc_url,
        &sig,
        &format!(
            "Job claimed  bond={:.4} SOL deposited  status→Claimed",
            cfg.required_bond_lamports as f64 / LAMPORTS_PER_SOL as f64
        ),
    );
    results.push(StepResult { label: "claim_job", sig });

    // ── Step 9: submit_result ────────────────────────────────────────────────
    //
    // Two signers are required:
    //   operator_authority  – the operator's hot wallet (proves they're authorized)
    //   tee_attestation_key – the ephemeral node_kp (proves the result came from
    //                         the live TEE enclave, not just the hot wallet)
    //
    // The program checks: tee_attestation_key.key() == operator_account.attestation_pubkey
    // Since we set attestation_pubkey = node_kp.pubkey() in steps 3 and 4, this passes.
    //
    // On success the vault (payment + bond + rent) is closed to operator_authority.

    display::section("Step 9 / 9 – submit_result  (operator + TEE key co-sign)");
    let sig = send_tx_multi(
        &client,
        &ix::job::submit_result(
            &operator_kp.pubkey(),
            &node_kp.pubkey(),     // tee_attestation_key co-signer
            &main_wallet.pubkey(), // client (seed for job PDA)
            &job_account,
            &job_vault,
            &operator_account,
            &cfg.job_id,
            job_nonce,
            result_hash,
        ),
        &[&operator_kp, &node_kp], // BOTH must sign
        &operator_kp,
    ).context("submit_result failed")?;

    let payout = cfg.payment_lamports + cfg.required_bond_lamports;
    display::tx_ok(
        &cfg.rpc_url,
        &sig,
        &format!(
            "Result submitted  payout≈{:.4} SOL → operator  status→Completed",
            payout as f64 / LAMPORTS_PER_SOL as f64
        ),
    );
    results.push(StepResult { label: "submit_result", sig });

    // ── Summary ───────────────────────────────────────────────────────────────
    display::summary(&cfg.rpc_url, &results);

    Ok(())
}

// ── RPC helpers ───────────────────────────────────────────────────────────────

/// Send a single-instruction transaction; wait for confirmation.
fn send_tx(
    client:      &RpcClient,
    instruction: &Instruction,
    signers:     &[&Keypair],
    payer:       &Keypair,
) -> Result<String> {
    send_tx_multi(client, instruction, signers, payer)
}

/// Send a transaction (inner implementation — accepts any signer slice).
fn send_tx_multi(
    client:      &RpcClient,
    instruction: &Instruction,
    signers:     &[&Keypair],
    payer:       &Keypair,
) -> Result<String> {
    let blockhash = client.get_latest_blockhash()
        .context("failed to get latest blockhash")?;

    let tx = Transaction::new_signed_with_payer(
        &[instruction.clone()],
        Some(&payer.pubkey()),
        signers,
        blockhash,
    );

    let sig = client
        .send_and_confirm_transaction_with_spinner(&tx)
        .context("transaction rejected by the cluster")?;

    Ok(sig.to_string())
}

/// Return true if an on-chain account exists and has non-empty data.
/// Used to skip idempotent init instructions on re-runs.
fn account_exists(client: &RpcClient, pubkey: &Pubkey) -> Result<bool> {
    match client.get_account_with_commitment(pubkey, CommitmentConfig::confirmed()) {
        Ok(resp) => Ok(resp.value.map(|a| !a.data.is_empty()).unwrap_or(false)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("AccountNotFound") || msg.contains("could not find account") {
                Ok(false)
            } else {
                Err(e.into())
            }
        }
    }
}

/// Ensure `pubkey` holds at least `min_lamports`.
/// Requests up to 5-SOL airdrops with retries when on devnet.
fn ensure_funded(
    client:       &RpcClient,
    pubkey:       &Pubkey,
    min_lamports: u64,
) -> Result<()> {
    let balance = client.get_balance(pubkey)
        .context("failed to query balance")?;

    if balance >= min_lamports {
        println!(
            "  Wallet balance: {:.4} SOL  ✔",
            balance as f64 / LAMPORTS_PER_SOL as f64
        );
        return Ok(());
    }

    println!(
        "  Wallet balance {:.4} SOL < {:.4} SOL required – requesting devnet airdrop…",
        balance as f64 / LAMPORTS_PER_SOL as f64,
        min_lamports as f64 / LAMPORTS_PER_SOL as f64,
    );

    for attempt in 1..=5u8 {
        let _ = client.request_airdrop(pubkey, 5 * LAMPORTS_PER_SOL);
        std::thread::sleep(std::time::Duration::from_secs(15));

        let new_bal = client.get_balance(pubkey).unwrap_or(0);
        if new_bal >= min_lamports {
            println!("  Airdrop confirmed: {:.4} SOL  ✔", new_bal as f64 / LAMPORTS_PER_SOL as f64);
            return Ok(());
        }

        if attempt < 5 {
            println!("  Balance {:.4} SOL – retrying airdrop ({attempt}/5)…",
                new_bal as f64 / LAMPORTS_PER_SOL as f64);
        }
    }

    anyhow::bail!(
        "Could not reach {:.4} SOL after 5 airdrop attempts.\n\
         Fund manually:  solana airdrop 5 {} --url devnet",
        min_lamports as f64 / LAMPORTS_PER_SOL as f64,
        pubkey
    )
}
