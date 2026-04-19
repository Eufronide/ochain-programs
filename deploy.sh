#!/usr/bin/env bash
# =============================================================================
# deploy.sh  ─  Deploy ochain-programs to Solana devnet
# =============================================================================
#
# WHAT THIS SCRIPT DOES (high-level)
# ─────────────────────────────────────────────────────────────────────────────
# 1.  Checks that all required CLI tools are installed.
# 2.  Generates a fresh ed25519 keypair for every program that does not yet
#     have one under target/deploy/.  (Keypairs are reused on re-runs so your
#     program addresses stay stable.)
# 3.  Reads each keypair's public key and writes it into:
#       • programs/<name>/src/lib.rs   – the declare_id!() macro
#       • Anchor.toml                  – both [programs.localnet] and
#                                        [programs.devnet] tables
# 4.  Compiles every program with `anchor build`.
# 5.  Airdrops devnet SOL to your wallet if the balance is below the minimum
#     needed to cover deployment rent.
# 6.  Deploys all five programs to devnet one by one.
# 7.  Uploads each program's IDL on-chain so explorers and clients can decode
#     transactions without a local copy of the source.
# 8.  Prints a summary table of every deployed program ID.
#
#
# SOLANA DEPLOYMENT CONCEPTS (for Rust developers new to Solana)
# ─────────────────────────────────────────────────────────────────────────────
#
# Program vs Account
#   On Solana every piece of data lives in an "account" – a blob of bytes with
#   a public key, an owner program, and a lamport (SOL) balance.  Programs are
#   just accounts whose `executable` flag is set to true.  Deploying a program
#   means uploading your compiled BPF bytecode into such an account.
#
# Program ID
#   A program's public key IS its address on-chain.  Clients call your program
#   by sending transactions to that address.  The `declare_id!()` macro in
#   lib.rs bakes the expected address into the binary so the runtime can verify
#   it is executing from the correct account.  Mismatch → runtime error.
#
# Keypair files
#   target/deploy/<program>-keypair.json holds the SECRET key that "owns" the
#   program account.  Only the holder of this key can upgrade (redeploy) the
#   program later.  Keep it safe – losing it means you can never upgrade.
#
# Upgrade authority
#   Solana programs are upgradeable by default.  The account that deployed the
#   program is stored as the "upgrade authority".  You can freeze a program
#   (make it immutable) by setting upgrade authority to null via:
#     solana program set-upgrade-authority <PROGRAM_ID> --final
#
# Rent
#   Storing data on-chain costs a one-time "rent-exempt" deposit of SOL
#   proportional to account size.  For a 200 KB program binary this is
#   roughly 2–3 SOL on devnet.  The deposit is returned if you close the
#   program account later.
#
# IDL (Interface Description Language)
#   Anchor generates a JSON file (target/idl/<program>.json) that describes
#   every instruction, account, and type in your program – analogous to an
#   ABI in Ethereum.  Uploading it on-chain (`anchor idl init`) lets tools
#   like Anchor Explorer, Solana FM, and the TypeScript SDK decode your
#   transactions automatically.
#
#
# PREREQUISITES
# ─────────────────────────────────────────────────────────────────────────────
#   • Rust + cargo          https://rustup.rs
#   • Solana CLI ≥ 1.18     https://docs.solana.com/cli/install-solana-cli-tools
#   • Anchor CLI 0.30.1     cargo install --git https://github.com/coral-xyz/anchor \
#                             avm --locked && avm install 0.30.1 && avm use 0.30.1
#   • Node ≥ 18 + Yarn      https://nodejs.org  /  npm i -g yarn
#   • A funded devnet wallet at ~/.config/solana/id.json
#       Create:   solana-keygen new --outfile ~/.config/solana/id.json
#       Fund:     solana airdrop 5 --url devnet   (run 2–3 times; faucet caps ~5 SOL/request)
#
#
# USAGE
# ─────────────────────────────────────────────────────────────────────────────
#   chmod +x deploy.sh
#
#   First deploy:
#     ./deploy.sh
#
#   Re-deploy after code changes (keypairs are reused – program IDs stay the same):
#     ./deploy.sh
#
#   Deploy a single program without touching the others:
#     solana program deploy target/deploy/ochain_registry.so \
#       --program-id target/deploy/ochain_registry-keypair.json \
#       --url devnet
#
#   Verify a deployed program:
#     solana program show <PROGRAM_ID> --url devnet
#
# =============================================================================

set -euo pipefail

# ── colour output ─────────────────────────────────────────────────────────────

RED='\033[0;31m'; YELLOW='\033[1;33m'; GREEN='\033[0;32m'
CYAN='\033[0;36m'; BOLD='\033[1m'; RESET='\033[0m'

info()    { echo -e "${CYAN}[info]${RESET}  $*"; }
success() { echo -e "${GREEN}[ok]${RESET}    $*"; }
warn()    { echo -e "${YELLOW}[warn]${RESET}  $*"; }
die()     { echo -e "${RED}[error]${RESET} $*" >&2; exit 1; }
header()  { echo -e "\n${BOLD}══ $* ══${RESET}"; }

# ── configuration ─────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

CLUSTER="devnet"
RPC_URL="https://api.devnet.solana.com"
WALLET="${SOLANA_WALLET:-$HOME/.config/solana/id.json}"
DEPLOY_DIR="target/deploy"
IDL_DIR="target/idl"

# Minimum wallet balance in SOL before we start deploying.
# Each program needs ~2–3 SOL for rent; 5 programs × 3 SOL = 15 SOL.
# We require 16 SOL as a safety margin.
MIN_BALANCE_SOL=16

# Programs in dependency order: registry and attestation first (others may CPI into them).
declare -a PROGRAMS=(
  "ochain_registry"
  "ochain_attestation"
  "ochain_job"
  "ochain_identity"
  "ochain_governance"
)

# Map program name → path to lib.rs
declare -A LIB_RS=(
  [ochain_registry]="programs/ochain-registry/src/lib.rs"
  [ochain_attestation]="programs/ochain-attestation/src/lib.rs"
  [ochain_job]="programs/ochain-job/src/lib.rs"
  [ochain_identity]="programs/ochain-identity/src/lib.rs"
  [ochain_governance]="programs/ochain-governance/src/lib.rs"
)

# ── helpers ───────────────────────────────────────────────────────────────────

# Portable in-place sed (macOS needs the empty-string backup argument).
sed_i() {
  if [[ "$(uname)" == "Darwin" ]]; then
    sed -i '' "$@"
  else
    sed -i "$@"
  fi
}

# Return the SOL balance of an address as an integer (floor).
sol_balance() {
  local addr="$1"
  solana balance "$addr" --url "$RPC_URL" 2>/dev/null \
    | grep -oE '[0-9]+(\.[0-9]+)?' | head -1 | cut -d. -f1
}

# Airdrop up to $1 SOL in chunks of 5, retrying on failure.
airdrop_to() {
  local target_sol="$1"
  local attempts=0
  while (( $(sol_balance "$DEPLOYER_PUBKEY") < target_sol )); do
    (( attempts++ ))
    (( attempts > 6 )) && die "Could not reach ${target_sol} SOL after 6 airdrop attempts.\nFund manually: solana airdrop 5 $DEPLOYER_PUBKEY --url devnet"
    info "Requesting 5 SOL airdrop (attempt ${attempts})…"
    solana airdrop 5 "$DEPLOYER_PUBKEY" --url "$RPC_URL" || warn "Airdrop request failed – retrying after 10 s"
    sleep 10
  done
}

# Replace the declare_id!() value in a lib.rs file.
patch_declare_id() {
  local file="$1"
  local new_id="$2"
  # Matches: declare_id!("ANY_BASE58_STRING");
  sed_i "s/declare_id!(\"[1-9A-HJ-NP-Za-km-z]*\");/declare_id!(\"${new_id}\");/" "$file"
}

# Replace a program's ID inside one Anchor.toml section.
# Usage: patch_anchor_toml_section <section> <program_name> <new_id>
# Example: patch_anchor_toml_section "programs.devnet" "ochain_registry" "ABC123..."
patch_anchor_toml_section() {
  local section="$1"
  local prog="$2"
  local new_id="$3"
  # awk rewrites only lines inside the target section.
  awk -v sec="[${section}]" -v prog="$prog" -v id="$new_id" '
    /^\[/ { in_section = ($0 == sec) }
    in_section && $0 ~ "^"prog"[[:space:]]*=" {
      sub(/"[^"]*"/, "\"" id "\"")
    }
    { print }
  ' Anchor.toml > Anchor.toml.tmp && mv Anchor.toml.tmp Anchor.toml
}

# ── step 0: prerequisites ─────────────────────────────────────────────────────

header "Checking prerequisites"

for cmd in solana solana-keygen anchor cargo node yarn; do
  if ! command -v "$cmd" &>/dev/null; then
    die "'${cmd}' not found in PATH. See PREREQUISITES at the top of this script."
  fi
  success "$cmd  $(${cmd} --version 2>&1 | head -1)"
done

[[ -f "$WALLET" ]] || die "Wallet not found at ${WALLET}.\nCreate one with: solana-keygen new --outfile ${WALLET}"

DEPLOYER_PUBKEY="$(solana-keygen pubkey "$WALLET")"
success "Deployer wallet: ${DEPLOYER_PUBKEY}"

# ── step 1: generate program keypairs ─────────────────────────────────────────

header "Generating program keypairs"

mkdir -p "$DEPLOY_DIR"

declare -A PROGRAM_IDS=()

for prog in "${PROGRAMS[@]}"; do
  keypair_file="${DEPLOY_DIR}/${prog}-keypair.json"

  if [[ -f "$keypair_file" ]]; then
    warn "Keypair already exists for ${prog} – reusing it (program ID stays the same)."
  else
    info "Generating keypair for ${prog}…"
    solana-keygen new --no-bip39-passphrase --silent --outfile "$keypair_file"
    success "Created ${keypair_file}"
  fi

  pubkey="$(solana-keygen pubkey "$keypair_file")"
  PROGRAM_IDS[$prog]="$pubkey"
  info "${prog} = ${pubkey}"
done

# ── step 2: patch declare_id!() in every lib.rs ───────────────────────────────

header "Patching declare_id!() in lib.rs files"

for prog in "${PROGRAMS[@]}"; do
  lib="${LIB_RS[$prog]}"
  new_id="${PROGRAM_IDS[$prog]}"

  [[ -f "$lib" ]] || die "lib.rs not found at ${lib}"

  old_id="$(grep -oE 'declare_id!\("[1-9A-HJ-NP-Za-km-z]+"\)' "$lib" | grep -oE '"[^"]+"' | tr -d '"')"
  if [[ "$old_id" == "$new_id" ]]; then
    success "${prog}: declare_id already correct – no change needed."
  else
    patch_declare_id "$lib" "$new_id"
    success "${prog}: declare_id updated  ${old_id}  →  ${new_id}"
  fi
done

# ── step 3: patch Anchor.toml ─────────────────────────────────────────────────

header "Patching Anchor.toml"

for prog in "${PROGRAMS[@]}"; do
  new_id="${PROGRAM_IDS[$prog]}"
  patch_anchor_toml_section "programs.localnet" "$prog" "$new_id"
  patch_anchor_toml_section "programs.devnet"   "$prog" "$new_id"
  success "Anchor.toml updated for ${prog}"
done

# ── step 4: install Node dependencies ─────────────────────────────────────────

header "Installing Node dependencies"

if [[ ! -d node_modules ]]; then
  info "Running yarn install…"
  yarn install --frozen-lockfile
  success "Node modules installed."
else
  success "node_modules present – skipping yarn install."
fi

# ── step 5: build all programs ────────────────────────────────────────────────

header "Building programs  (anchor build)"
info "This compiles Rust to BPF bytecode – expect 2–5 minutes on first build."

anchor build

success "Build complete. Artifacts written to target/deploy/ and target/idl/."

# Sanity-check: ensure every .so file was produced.
for prog in "${PROGRAMS[@]}"; do
  so_file="${DEPLOY_DIR}/${prog}.so"
  [[ -f "$so_file" ]] || die "Expected ${so_file} but it was not produced by anchor build."
done

# ── step 6: fund the deployer wallet ──────────────────────────────────────────

header "Checking deployer balance"

current_bal="$(sol_balance "$DEPLOYER_PUBKEY")"
info "Current balance: ${current_bal} SOL  (minimum required: ${MIN_BALANCE_SOL} SOL)"

if (( current_bal < MIN_BALANCE_SOL )); then
  warn "Balance too low – requesting airdrops from devnet faucet."
  info "Note: devnet faucet allows ~5 SOL per request. Multiple requests will be made."
  airdrop_to "$MIN_BALANCE_SOL"
  success "Balance after airdrop: $(sol_balance "$DEPLOYER_PUBKEY") SOL"
else
  success "Balance sufficient: ${current_bal} SOL"
fi

# ── step 7: deploy programs to devnet ─────────────────────────────────────────

header "Deploying to ${CLUSTER}"
info "Each program is deployed individually so a failure in one does not block others."

declare -A DEPLOY_STATUS=()

for prog in "${PROGRAMS[@]}"; do
  so_file="${DEPLOY_DIR}/${prog}.so"
  keypair_file="${DEPLOY_DIR}/${prog}-keypair.json"
  prog_id="${PROGRAM_IDS[$prog]}"

  echo ""
  info "Deploying ${prog}  (${prog_id})…"

  if solana program deploy "$so_file" \
       --program-id   "$keypair_file" \
       --keypair      "$WALLET" \
       --url          "$RPC_URL" \
       --commitment   confirmed; then
    DEPLOY_STATUS[$prog]="deployed"
    success "${prog} deployed successfully."
  else
    DEPLOY_STATUS[$prog]="FAILED"
    warn "Deploy FAILED for ${prog}. Continuing with remaining programs."
    warn "Retry manually:  solana program deploy ${so_file} --program-id ${keypair_file} --keypair ${WALLET} --url ${RPC_URL}"
  fi
done

# ── step 8: upload IDLs on-chain ──────────────────────────────────────────────
#
# The IDL lets Anchor Explorer and other tools decode your on-chain transactions
# without needing a local copy of the source code.
#
# `anchor idl init`   – first-time upload; fails if the IDL account already exists
# `anchor idl upgrade` – update an existing IDL account
#
# We auto-detect which command to use.

header "Uploading IDLs to devnet"

for prog in "${PROGRAMS[@]}"; do
  [[ "${DEPLOY_STATUS[$prog]:-}" == "deployed" ]] || { warn "Skipping IDL upload for ${prog} (deploy failed)."; continue; }

  idl_file="${IDL_DIR}/${prog}.json"
  prog_id="${PROGRAM_IDS[$prog]}"

  [[ -f "$idl_file" ]] || { warn "IDL file not found at ${idl_file} – skipping."; continue; }

  info "Uploading IDL for ${prog}…"

  # Try `anchor idl init`; if the account already exists, fall back to `upgrade`.
  if anchor idl init \
       --filepath "$idl_file" \
       --provider.cluster "$CLUSTER" \
       --provider.wallet  "$WALLET" \
       "$prog_id" 2>/dev/null; then
    success "IDL initialised for ${prog}."
  elif anchor idl upgrade \
         --filepath "$idl_file" \
         --provider.cluster "$CLUSTER" \
         --provider.wallet  "$WALLET" \
         "$prog_id" 2>/dev/null; then
    success "IDL upgraded for ${prog}."
  else
    warn "IDL upload failed for ${prog} – programs still functional, but explorers may not decode txns."
  fi
done

# ── step 9: summary ───────────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}══════════════════════════════════════════════════════════════════${RESET}"
echo -e "${BOLD}  Deployment summary – Solana ${CLUSTER}${RESET}"
echo -e "${BOLD}══════════════════════════════════════════════════════════════════${RESET}"
printf "  %-24s  %-44s  %s\n" "Program" "Program ID" "Status"
printf "  %-24s  %-44s  %s\n" "────────────────────────" "────────────────────────────────────────────" "────────"

all_ok=true
for prog in "${PROGRAMS[@]}"; do
  status="${DEPLOY_STATUS[$prog]:-skipped}"
  id="${PROGRAM_IDS[$prog]}"
  if [[ "$status" == "deployed" ]]; then
    colour="$GREEN"
  else
    colour="$RED"
    all_ok=false
  fi
  printf "  %-24s  %-44s  ${colour}%s${RESET}\n" "$prog" "$id" "$status"
done

echo ""
echo -e "  Deployer wallet : ${DEPLOYER_PUBKEY}"
echo -e "  Cluster         : ${RPC_URL}"
echo -e "  Keypairs stored : ${DEPLOY_DIR}/<program>-keypair.json"
echo ""
echo -e "${BOLD}  NEXT STEPS${RESET}"
echo -e "  1. Run the initialise transactions (once per program, see below)."
echo -e "  2. Transfer upgrade authority to a governance multisig when ready."
echo -e "  3. Verify programs on explorer:"
echo -e "     https://explorer.solana.com/address/<PROGRAM_ID>?cluster=devnet"
echo ""
echo -e "${BOLD}  INITIALISE INSTRUCTIONS (run after first deploy only)${RESET}"
echo ""
echo -e "  # Set your deployer as the default wallet for anchor CLI:"
echo -e "  export ANCHOR_WALLET=${WALLET}"
echo -e "  export ANCHOR_PROVIDER_URL=${RPC_URL}"
echo ""
echo -e "  # Initialize the registry protocol state:"
echo -e "  ts-node scripts/initialize_registry.ts   # create this script from the IDL"
echo ""
echo -e "  # Initialize the attestation verifier:"
echo -e "  ts-node scripts/initialize_attestation.ts"
echo ""
echo -e "${BOLD}  USEFUL COMMANDS${RESET}"
echo ""
echo -e "  # Check a deployed program:"
echo -e "  solana program show ${PROGRAM_IDS[ochain_registry]} --url devnet"
echo ""
echo -e "  # View program logs in real time:"
echo -e "  solana logs ${PROGRAM_IDS[ochain_registry]} --url devnet"
echo ""
echo -e "  # Redeploy after code changes (keypairs reused → same program IDs):"
echo -e "  anchor build && solana program deploy target/deploy/ochain_registry.so \\"
echo -e "    --program-id target/deploy/ochain_registry-keypair.json --url devnet"
echo ""
echo -e "  # Freeze a program (IRREVERSIBLE – removes upgrade authority):"
echo -e "  solana program set-upgrade-authority <PROGRAM_ID> --final --url devnet"
echo ""

if $all_ok; then
  echo -e "${GREEN}${BOLD}  All programs deployed successfully.${RESET}"
else
  echo -e "${RED}${BOLD}  One or more programs failed to deploy. Check warnings above.${RESET}"
  exit 1
fi
