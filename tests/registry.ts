/**
 * Integration tests for ochain-registry.
 *
 * Prerequisites: run `anchor build` before `anchor test` so that
 * target/deploy/ and target/types/ are populated.
 *
 * Clock-sensitive tests (SLA violation, finalize_exit) use solana-bankrun
 * to warp the validator clock without waiting real time.
 */

import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import {
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { assert } from "chai";
import { startAnchor, ProgramTestContext } from "anchor-bankrun";
import { BankrunProvider } from "anchor-bankrun";
import type { OchainRegistry } from "../target/types/ochain_registry";

// ── helpers ──────────────────────────────────────────────────────────────────

function pda(seeds: Buffer[], programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(seeds, programId)[0];
}

function funded(kp: Keypair, sol = 100) {
  return {
    address: kp.publicKey,
    info: {
      executable: false,
      owner: SystemProgram.programId,
      lamports: BigInt(sol * LAMPORTS_PER_SOL),
      data: Buffer.alloc(0),
    },
  };
}

function teeType(s: "TD" | "SE"): number[] {
  return Array.from(Buffer.from(s));
}

function measurement(fill: number): number[] {
  return Array.from(Buffer.alloc(48, fill));
}

// ── constants ─────────────────────────────────────────────────────────────────

const MIN_STAKE       = new BN(LAMPORTS_PER_SOL);
const EPOCH_SLOTS     = new BN(10);   // short epoch so SLA tests are fast
const MAX_NODES       = 2;
const UNBONDING_SLOTS = 3_024_000;    // mirrors programs/ochain-registry/src/constants.rs
const SLA_MULTIPLIER  = 2;            // SLA_MISS_MULTIPLIER

// ── keypairs (declared at module scope so startAnchor can fund them) ──────────

const deployer   = Keypair.generate();
const treasury   = Keypair.generate();
const operator   = Keypair.generate();
const operator2  = Keypair.generate();
const slaChecker = Keypair.generate();

// ── suite ────────────────────────────────────────────────────────────────────

describe("ochain-registry", () => {
  let context : ProgramTestContext;
  let provider : BankrunProvider;
  let program  : Program<OchainRegistry>;

  // PDAs derived once programId is known
  let protocolState  : PublicKey;
  let operatorAcct   : PublicKey;   // operator
  let stakeVault     : PublicKey;
  let nodeAcct0      : PublicKey;
  let nodeAcct1      : PublicKey;
  let op2Acct        : PublicKey;   // operator2
  let op2Vault       : PublicKey;
  let op2Node0       : PublicKey;

  before(async () => {
    context = await startAnchor(".", [], [
      funded(deployer),
      funded(operator),
      funded(operator2),
      funded(slaChecker),
    ]);

    provider = new BankrunProvider(context);
    anchor.setProvider(provider);
    program = anchor.workspace.OchainRegistry as Program<OchainRegistry>;

    const pid = program.programId;
    protocolState = pda([Buffer.from("protocol_state")], pid);

    operatorAcct = pda([Buffer.from("operator"), operator.publicKey.toBuffer()], pid);
    stakeVault   = pda([Buffer.from("stake_vault"), operator.publicKey.toBuffer()], pid);
    nodeAcct0    = pda([Buffer.from("operator_node"), operatorAcct.toBuffer(), Buffer.from([0])], pid);
    nodeAcct1    = pda([Buffer.from("operator_node"), operatorAcct.toBuffer(), Buffer.from([1])], pid);

    op2Acct  = pda([Buffer.from("operator"), operator2.publicKey.toBuffer()], pid);
    op2Vault = pda([Buffer.from("stake_vault"), operator2.publicKey.toBuffer()], pid);
    op2Node0 = pda([Buffer.from("operator_node"), op2Acct.toBuffer(), Buffer.from([0])], pid);
  });

  // ────────────────────────────────────────────────────────────────────────────
  // initialize_protocol
  // ────────────────────────────────────────────────────────────────────────────

  describe("initialize_protocol", () => {
    it("creates ProtocolState with correct parameters", async () => {
      await program.methods
        .initializeProtocol(MIN_STAKE, new BN(1_000), EPOCH_SLOTS, MAX_NODES)
        .accounts({
          authority:     deployer.publicKey,
          treasury:      treasury.publicKey,
          protocolState,
          systemProgram: SystemProgram.programId,
        })
        .signers([deployer])
        .rpc();

      const s = await program.account.protocolState.fetch(protocolState);
      assert.ok(s.authority.equals(deployer.publicKey), "authority");
      assert.ok(s.treasury.equals(treasury.publicKey),  "treasury");
      assert.equal(s.minStakeLamports.toNumber(), LAMPORTS_PER_SOL, "min stake");
      assert.equal(s.epochDurationSlots.toNumber(), EPOCH_SLOTS.toNumber(), "epoch");
      assert.equal(s.maxNodesPerOperator, MAX_NODES, "max nodes");
      assert.equal(s.operatorCount.toNumber(), 0, "operator count starts at 0");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // register_operator
  // ────────────────────────────────────────────────────────────────────────────

  describe("register_operator", () => {
    const attestKey = Keypair.generate().publicKey;
    const endpoint  = "https://operator.example.com/tee";

    it("rejects stake below minimum", async () => {
      try {
        await program.methods
          .registerOperator(
            new BN(LAMPORTS_PER_SOL - 1),
            teeType("TD"), endpoint, attestKey, measurement(0xab),
          )
          .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, stakeVault, protocolState, systemProgram: SystemProgram.programId })
          .signers([operator])
          .rpc();
        assert.fail("expected StakeTooLow");
      } catch (err: any) {
        assert.match(err.toString(), /StakeTooLow/, "error: StakeTooLow");
      }
    });

    it("rejects invalid TEE type", async () => {
      try {
        await program.methods
          .registerOperator(
            MIN_STAKE,
            Array.from(Buffer.from("XX")), endpoint, attestKey, measurement(0xab),
          )
          .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, stakeVault, protocolState, systemProgram: SystemProgram.programId })
          .signers([operator])
          .rpc();
        assert.fail("expected InvalidTeeType");
      } catch (err: any) {
        assert.match(err.toString(), /InvalidTeeType/, "error: InvalidTeeType");
      }
    });

    it("rejects endpoint URL longer than 200 bytes", async () => {
      const longUrl = "https://" + "x".repeat(193); // 8 + 193 = 201 bytes
      try {
        await program.methods
          .registerOperator(
            MIN_STAKE,
            teeType("TD"), longUrl, attestKey, measurement(0xab),
          )
          .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, stakeVault, protocolState, systemProgram: SystemProgram.programId })
          .signers([operator])
          .rpc();
        assert.fail("expected InvalidEndpointUrl");
      } catch (err: any) {
        assert.match(err.toString(), /InvalidEndpointUrl/, "error: InvalidEndpointUrl");
      }
    });

    it("registers operator (TD) and creates accounts", async () => {
      const balBefore = await provider.connection.getBalance(operator.publicKey);

      await program.methods
        .registerOperator(MIN_STAKE, teeType("TD"), endpoint, attestKey, measurement(0xab))
        .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, stakeVault, protocolState, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      const acc = await program.account.operatorAccount.fetch(operatorAcct);
      assert.ok(acc.authority.equals(operator.publicKey), "authority");
      assert.equal(acc.stakeAmount.toNumber(), LAMPORTS_PER_SOL, "stake amount");
      assert.deepEqual(acc.teeType, teeType("TD"), "tee type TD");
      assert.ok(acc.attestationPubkey.equals(attestKey), "attestation pubkey");
      assert.equal(acc.endpointUrl, endpoint, "endpoint url");
      assert.equal(acc.nodeCount, 0, "node_count is 0");
      assert.equal(acc.reputationScore, 5_000, "initial reputation 5000");
      assert.property(acc.status, "active", "status active");

      const vaultBal = await provider.connection.getBalance(stakeVault);
      assert.isAtLeast(vaultBal, LAMPORTS_PER_SOL, "stake vault funded");

      const balAfter = await provider.connection.getBalance(operator.publicKey);
      assert.isBelow(balAfter, balBefore - LAMPORTS_PER_SOL + 1, "operator lamports decreased");

      const state = await program.account.protocolState.fetch(protocolState);
      assert.equal(state.operatorCount.toNumber(), 1, "operator_count incremented");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // add_node
  // ────────────────────────────────────────────────────────────────────────────

  describe("add_node", () => {
    const nodeKey0 = Keypair.generate().publicKey;
    const nodeKey1 = Keypair.generate().publicKey;

    it("adds node 0 and increments node_count", async () => {
      await program.methods
        .addNode(0, nodeKey0, measurement(0x01))
        .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct0, protocolState, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      const node = await program.account.nodeAccount.fetch(nodeAcct0);
      assert.ok(node.operator.equals(operatorAcct), "node.operator");
      assert.ok(node.attestationPubkey.equals(nodeKey0), "attestation pubkey");
      assert.deepEqual(Array.from(node.measurementHash), measurement(0x01), "measurement hash");
      assert.equal(node.nodeIndex, 0, "node_index 0");

      const acc = await program.account.operatorAccount.fetch(operatorAcct);
      assert.equal(acc.nodeCount, 1, "node_count 1");
    });

    it("adds node 1 and increments node_count to 2", async () => {
      await program.methods
        .addNode(1, nodeKey1, measurement(0x02))
        .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct1, protocolState, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      const acc = await program.account.operatorAccount.fetch(operatorAcct);
      assert.equal(acc.nodeCount, 2, "node_count 2");
    });

    it("rejects adding a third node beyond max_nodes_per_operator", async () => {
      const nodeAcct2 = pda(
        [Buffer.from("operator_node"), operatorAcct.toBuffer(), Buffer.from([2])],
        program.programId,
      );
      try {
        await program.methods
          .addNode(2, Keypair.generate().publicKey, measurement(0x03))
          .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct2, protocolState, systemProgram: SystemProgram.programId })
          .signers([operator])
          .rpc();
        assert.fail("expected TooManyNodes");
      } catch (err: any) {
        assert.match(err.toString(), /TooManyNodes/, "error: TooManyNodes");
      }
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // heartbeat
  // ────────────────────────────────────────────────────────────────────────────

  describe("heartbeat", () => {
    it("updates last_heartbeat_slot for node 0", async () => {
      const clock = await context.banksClient.getClock();

      await program.methods
        .heartbeat(0)
        .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct0 })
        .signers([operator])
        .rpc();

      const node = await program.account.nodeAccount.fetch(nodeAcct0);
      assert.isAtLeast(
        node.lastHeartbeatSlot.toNumber(),
        Number(clock.slot),
        "last_heartbeat_slot updated",
      );
    });

    it("updates last_heartbeat_slot for node 1", async () => {
      await program.methods
        .heartbeat(1)
        .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct1 })
        .signers([operator])
        .rpc();

      const node = await program.account.nodeAccount.fetch(nodeAcct1);
      assert.isAbove(node.lastHeartbeatSlot.toNumber(), 0, "last_heartbeat_slot > 0");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // check_sla_violation
  // ────────────────────────────────────────────────────────────────────────────

  describe("check_sla_violation", () => {
    it("rejects violation report before SLA window expires", async () => {
      try {
        await program.methods
          .checkSlaViolation(0)
          .accounts({ caller: slaChecker.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct0, protocolState })
          .signers([slaChecker])
          .rpc();
        assert.fail("expected NoSlaViolation");
      } catch (err: any) {
        assert.match(err.toString(), /NoSlaViolation/, "error: NoSlaViolation");
      }
    });

    it("records violation #1 after SLA window (warp 2x epoch past last heartbeat)", async () => {
      const node        = await program.account.nodeAccount.fetch(nodeAcct0);
      const targetSlot  = node.lastHeartbeatSlot.toNumber() + EPOCH_SLOTS.toNumber() * SLA_MULTIPLIER + 5;
      await context.warpToSlot(BigInt(targetSlot));

      const accBefore = await program.account.operatorAccount.fetch(operatorAcct);

      await program.methods
        .checkSlaViolation(0)
        .accounts({ caller: slaChecker.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct0, protocolState })
        .signers([slaChecker])
        .rpc();

      const acc = await program.account.operatorAccount.fetch(operatorAcct);
      assert.equal(acc.slaViolations, accBefore.slaViolations + 1, "sla_violations +1");
      assert.equal(
        acc.reputationScore,
        accBefore.reputationScore - 500,
        "reputation_score -500",
      );
    });

    it("rejects double-report within the same epoch", async () => {
      try {
        await program.methods
          .checkSlaViolation(0)
          .accounts({ caller: slaChecker.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct0, protocolState })
          .signers([slaChecker])
          .rpc();
        assert.fail("expected ViolationAlreadyChecked");
      } catch (err: any) {
        assert.match(err.toString(), /ViolationAlreadyChecked/, "error: ViolationAlreadyChecked");
      }
    });

    it("suspends operator after reaching 3 cumulative SLA violations", async () => {
      // Add violations #2 and #3 (violation #1 already recorded above).
      for (let i = 0; i < 2; i++) {
        const clock      = await context.banksClient.getClock();
        const targetSlot = Number(clock.slot) + EPOCH_SLOTS.toNumber() * SLA_MULTIPLIER + 5;
        await context.warpToSlot(BigInt(targetSlot));

        await program.methods
          .checkSlaViolation(0)
          .accounts({ caller: slaChecker.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct0, protocolState })
          .signers([slaChecker])
          .rpc();
      }

      const acc = await program.account.operatorAccount.fetch(operatorAcct);
      assert.equal(acc.slaViolations, 3, "3 total sla_violations");
      assert.property(acc.status, "suspended", "status suspended after threshold");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // exit flow (uses operator2 – a fresh Active operator)
  // ────────────────────────────────────────────────────────────────────────────

  describe("exit flow", () => {
    before(async () => {
      // Register operator2 (SE type) so we have a clean Active operator.
      await program.methods
        .registerOperator(
          MIN_STAKE,
          teeType("SE"),
          "https://op2.example.com/tee",
          Keypair.generate().publicKey,
          measurement(0xcc),
        )
        .accounts({
          operatorAuthority: operator2.publicKey,
          operatorAccount:   op2Acct,
          stakeVault:        op2Vault,
          protocolState,
          systemProgram:     SystemProgram.programId,
        })
        .signers([operator2])
        .rpc();
    });

    it("begins unbonding period and sets status to Exiting", async () => {
      const clock = await context.banksClient.getClock();

      await program.methods
        .beginExit()
        .accounts({ operatorAuthority: operator2.publicKey, operatorAccount: op2Acct })
        .signers([operator2])
        .rpc();

      const acc = await program.account.operatorAccount.fetch(op2Acct);
      assert.property(acc.status, "exiting", "status exiting");
      assert.isAtLeast(
        acc.exitInitiatedSlot.toNumber(),
        Number(clock.slot),
        "exit_initiated_slot set",
      );
    });

    it("rejects finalize before unbonding period ends", async () => {
      try {
        await program.methods
          .finalizeExit()
          .accounts({
            operatorAuthority: operator2.publicKey,
            operatorAccount:   op2Acct,
            stakeVault:        op2Vault,
            systemProgram:     SystemProgram.programId,
          })
          .signers([operator2])
          .rpc();
        assert.fail("expected UnbondingNotComplete");
      } catch (err: any) {
        assert.match(err.toString(), /UnbondingNotComplete/, "error: UnbondingNotComplete");
      }
    });

    it("finalizes exit and returns staked SOL after ~14-day unbonding (warp 3 024 000 slots)", async () => {
      const acc        = await program.account.operatorAccount.fetch(op2Acct);
      const targetSlot = acc.exitInitiatedSlot.toNumber() + UNBONDING_SLOTS + 1;
      await context.warpToSlot(BigInt(targetSlot));

      const balBefore = await provider.connection.getBalance(operator2.publicKey);

      await program.methods
        .finalizeExit()
        .accounts({
          operatorAuthority: operator2.publicKey,
          operatorAccount:   op2Acct,
          stakeVault:        op2Vault,
          systemProgram:     SystemProgram.programId,
        })
        .signers([operator2])
        .rpc();

      const vaultInfo = await provider.connection.getAccountInfo(op2Vault);
      assert.isNull(vaultInfo, "stake_vault closed");

      const balAfter = await provider.connection.getBalance(operator2.publicKey);
      assert.isAbove(balAfter, balBefore, "staked SOL returned to operator");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // update_attestation_pubkey  (key rotation)
  // ────────────────────────────────────────────────────────────────────────────

  describe("update_attestation_pubkey", () => {
    it("rotates node 0 key and syncs operator primary attestation_pubkey", async () => {
      const newKey         = Keypair.generate().publicKey;
      const newMeasurement = measurement(0xff);

      const nodeBefore = await program.account.nodeAccount.fetch(nodeAcct0);
      const oldKey     = nodeBefore.attestationPubkey;

      await program.methods
        .updateAttestationPubkey(0, newKey, newMeasurement)
        .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct0 })
        .signers([operator])
        .rpc();

      const node = await program.account.nodeAccount.fetch(nodeAcct0);
      assert.ok(node.attestationPubkey.equals(newKey), "node 0 key updated");
      assert.deepEqual(Array.from(node.measurementHash), newMeasurement, "measurement hash updated");
      assert.isFalse(node.attestationPubkey.equals(oldKey), "old key replaced");

      // node_index == 0 → operator_account.attestation_pubkey must sync
      const acc = await program.account.operatorAccount.fetch(operatorAcct);
      assert.ok(acc.attestationPubkey.equals(newKey), "primary key synced on OperatorAccount");
    });

    it("rotates node 1 key WITHOUT changing operator primary attestation_pubkey", async () => {
      const accBefore = await program.account.operatorAccount.fetch(operatorAcct);
      const primaryKey = accBefore.attestationPubkey;

      const newKey1 = Keypair.generate().publicKey;

      await program.methods
        .updateAttestationPubkey(1, newKey1, measurement(0xee))
        .accounts({ operatorAuthority: operator.publicKey, operatorAccount: operatorAcct, nodeAccount: nodeAcct1 })
        .signers([operator])
        .rpc();

      const node = await program.account.nodeAccount.fetch(nodeAcct1);
      assert.ok(node.attestationPubkey.equals(newKey1), "node 1 key updated");

      // node_index != 0 → primary key must NOT change
      const accAfter = await program.account.operatorAccount.fetch(operatorAcct);
      assert.ok(accAfter.attestationPubkey.equals(primaryKey), "primary key unchanged after node 1 rotation");
    });
  });
});
