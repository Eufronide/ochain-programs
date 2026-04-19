/**
 * Integration tests for ochain-attestation.
 *
 * Prerequisites: run `anchor build` before `anchor test`.
 */

import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import {
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { assert } from "chai";
import { startAnchor, ProgramTestContext } from "anchor-bankrun";
import { BankrunProvider } from "anchor-bankrun";
import type { OchainAttestation } from "../target/types/ochain_attestation";

// ── helpers ──────────────────────────────────────────────────────────────────

function pda(seeds: Buffer[], programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(seeds, programId)[0];
}

function attRecord(nodePubkey: PublicKey, programId: PublicKey): PublicKey {
  return pda([Buffer.from("attestation"), nodePubkey.toBuffer()], programId);
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

// ── keypairs ──────────────────────────────────────────────────────────────────

const deployer    = Keypair.generate();
const verifier    = Keypair.generate();
const newVerifier = Keypair.generate();
const operator    = Keypair.generate();

// ── suite ────────────────────────────────────────────────────────────────────

describe("ochain-attestation", () => {
  let context  : ProgramTestContext;
  let provider : BankrunProvider;
  let program  : Program<OchainAttestation>;
  let verifierState: PublicKey;

  before(async () => {
    context = await startAnchor(".", [], [
      funded(deployer),
      funded(verifier),
      funded(newVerifier),
      funded(operator),
    ]);

    provider = new BankrunProvider(context);
    anchor.setProvider(provider);
    program = anchor.workspace.OchainAttestation as Program<OchainAttestation>;

    verifierState = pda([Buffer.from("verifier_state")], program.programId);
  });

  // ────────────────────────────────────────────────────────────────────────────
  // initialize_verifier
  // ────────────────────────────────────────────────────────────────────────────

  describe("initialize_verifier", () => {
    it("creates VerifierState with the supplied authority", async () => {
      await program.methods
        .initializeVerifier(verifier.publicKey)
        .accounts({
          deployer:      deployer.publicKey,
          verifierState,
          systemProgram: SystemProgram.programId,
        })
        .signers([deployer])
        .rpc();

      const s = await program.account.verifierState.fetch(verifierState);
      assert.ok(s.verifierAuthority.equals(verifier.publicKey), "verifier_authority matches");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // submit_attestation
  // ────────────────────────────────────────────────────────────────────────────

  describe("submit_attestation", () => {
    // Shared node whose attestation we will use across subsequent suites.
    const sharedNode     = Keypair.generate();
    const measurementHash = Array.from(Buffer.alloc(48, 0xab));
    const quoteHash       = Array.from(Buffer.alloc(32, 0xcd));

    it("rejects an invalid TEE type", async () => {
      const rec = attRecord(sharedNode.publicKey, program.programId);
      try {
        await program.methods
          .submitAttestation(
            sharedNode.publicKey,
            measurementHash,
            Array.from(Buffer.from("XX")),
            quoteHash,
          )
          .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
          .signers([operator])
          .rpc();
        assert.fail("expected InvalidTeeType");
      } catch (err: any) {
        assert.match(err.toString(), /InvalidTeeType/, "error: InvalidTeeType");
      }
    });

    it("rejects an all-zero quote hash", async () => {
      const rec = attRecord(sharedNode.publicKey, program.programId);
      try {
        await program.methods
          .submitAttestation(
            sharedNode.publicKey,
            measurementHash,
            teeType("TD"),
            Array.from(Buffer.alloc(32, 0)), // all zeros = invalid
          )
          .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
          .signers([operator])
          .rpc();
        assert.fail("expected InvalidQuoteHash");
      } catch (err: any) {
        assert.match(err.toString(), /InvalidQuoteHash/, "error: InvalidQuoteHash");
      }
    });

    it("rejects an all-zero measurement hash", async () => {
      const rec = attRecord(sharedNode.publicKey, program.programId);
      try {
        await program.methods
          .submitAttestation(
            sharedNode.publicKey,
            Array.from(Buffer.alloc(48, 0)), // all zeros = invalid
            teeType("TD"),
            quoteHash,
          )
          .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
          .signers([operator])
          .rpc();
        assert.fail("expected InvalidMeasurementHash");
      } catch (err: any) {
        assert.match(err.toString(), /InvalidMeasurementHash/, "error: InvalidMeasurementHash");
      }
    });

    it("submits attestation with Pending status (TD type)", async () => {
      // Make the node pubkey available to later suites via closure.
      (sharedNode as any)._exported = true;

      const rec = attRecord(sharedNode.publicKey, program.programId);

      await program.methods
        .submitAttestation(sharedNode.publicKey, measurementHash, teeType("TD"), quoteHash)
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      const r = await program.account.attestationRecord.fetch(rec);
      assert.ok(r.operator.equals(operator.publicKey),         "operator");
      assert.ok(r.nodePubkey.equals(sharedNode.publicKey),    "node_pubkey");
      assert.deepEqual(Array.from(r.teeType), teeType("TD"),  "tee_type TD");
      assert.deepEqual(Array.from(r.measurementHash), measurementHash, "measurement_hash");
      assert.deepEqual(Array.from(r.quoteHash), quoteHash,    "quote_hash");
      assert.property(r.status, "pending",                    "status Pending");
      assert.equal(r.verifiedSlot.toNumber(), 0,              "verified_slot is 0");
    });

    // Export the shared node for sibling suites.
    after(() => {
      (describe as any)._sharedNode = sharedNode;
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // verify_attestation
  // Uses the record created in submit_attestation above (state carries over).
  // ────────────────────────────────────────────────────────────────────────────

  describe("verify_attestation", () => {
    const verifyNode = Keypair.generate();

    before(async () => {
      // Submit a fresh attestation used by this suite.
      const rec = attRecord(verifyNode.publicKey, program.programId);
      await program.methods
        .submitAttestation(
          verifyNode.publicKey,
          Array.from(Buffer.alloc(48, 0x11)),
          teeType("SE"),
          Array.from(Buffer.alloc(32, 0x22)),
        )
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();
    });

    it("rejects verification by a non-verifier signer", async () => {
      const impostor = Keypair.generate();
      context.setAccount(impostor.publicKey, {
        executable: false,
        owner: SystemProgram.programId,
        lamports: BigInt(10 * LAMPORTS_PER_SOL),
        data: Buffer.alloc(0),
      });

      const rec = attRecord(verifyNode.publicKey, program.programId);
      try {
        await program.methods
          .verifyAttestation()
          .accounts({ verifierAuthority: impostor.publicKey, verifierState, attestationRecord: rec })
          .signers([impostor])
          .rpc();
        assert.fail("expected NotVerifier");
      } catch (err: any) {
        assert.match(err.toString(), /NotVerifier/, "error: NotVerifier");
      }
    });

    it("verifies attestation and sets status to Verified", async () => {
      const clock = await context.banksClient.getClock();
      const rec   = attRecord(verifyNode.publicKey, program.programId);

      await program.methods
        .verifyAttestation()
        .accounts({ verifierAuthority: verifier.publicKey, verifierState, attestationRecord: rec })
        .signers([verifier])
        .rpc();

      const r = await program.account.attestationRecord.fetch(rec);
      assert.property(r.status, "verified", "status Verified");
      assert.isAtLeast(r.verifiedSlot.toNumber(), Number(clock.slot), "verified_slot set");
    });

    it("rejects double-verification of an already-Verified record", async () => {
      const rec = attRecord(verifyNode.publicKey, program.programId);
      try {
        await program.methods
          .verifyAttestation()
          .accounts({ verifierAuthority: verifier.publicKey, verifierState, attestationRecord: rec })
          .signers([verifier])
          .rpc();
        assert.fail("expected AlreadyVerified");
      } catch (err: any) {
        assert.match(err.toString(), /AlreadyVerified/, "error: AlreadyVerified");
      }
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // revoke_attestation
  // ────────────────────────────────────────────────────────────────────────────

  describe("revoke_attestation", () => {
    const revokeNode = Keypair.generate();
    let revokeRec: PublicKey;

    before(async () => {
      revokeRec = attRecord(revokeNode.publicKey, program.programId);
      await program.methods
        .submitAttestation(
          revokeNode.publicKey,
          Array.from(Buffer.alloc(48, 0xde)),
          teeType("TD"),
          Array.from(Buffer.alloc(32, 0xad)),
        )
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: revokeRec, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();
    });

    it("rejects revocation by a non-verifier", async () => {
      const impostor = Keypair.generate();
      context.setAccount(impostor.publicKey, {
        executable: false,
        owner: SystemProgram.programId,
        lamports: BigInt(5 * LAMPORTS_PER_SOL),
        data: Buffer.alloc(0),
      });

      try {
        await program.methods
          .revokeAttestation(0)
          .accounts({ verifierAuthority: impostor.publicKey, verifierState, attestationRecord: revokeRec })
          .signers([impostor])
          .rpc();
        assert.fail("expected NotVerifier");
      } catch (err: any) {
        assert.match(err.toString(), /NotVerifier/, "error: NotVerifier");
      }
    });

    it("revokes attestation with reason 0 (measurement mismatch) and keeps account open", async () => {
      await program.methods
        .revokeAttestation(0) // 0 = measurement mismatch
        .accounts({ verifierAuthority: verifier.publicKey, verifierState, attestationRecord: revokeRec })
        .signers([verifier])
        .rpc();

      const r = await program.account.attestationRecord.fetch(revokeRec);
      assert.property(r.status, "revoked", "status Revoked");

      // Account must stay open (operator needs to see the rejection reason).
      const info = await provider.connection.getAccountInfo(revokeRec);
      assert.isNotNull(info, "account kept open after revocation");
    });

    it("revokes with reason 1 (quote expired) on a fresh record", async () => {
      const node2 = Keypair.generate();
      const rec2  = attRecord(node2.publicKey, program.programId);

      await program.methods
        .submitAttestation(
          node2.publicKey,
          Array.from(Buffer.alloc(48, 0xfe)),
          teeType("SE"),
          Array.from(Buffer.alloc(32, 0xfe)),
        )
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec2, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      await program.methods
        .revokeAttestation(1) // 1 = quote expired
        .accounts({ verifierAuthority: verifier.publicKey, verifierState, attestationRecord: rec2 })
        .signers([verifier])
        .rpc();

      const r = await program.account.attestationRecord.fetch(rec2);
      assert.property(r.status, "revoked", "status Revoked (expired)");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // rotate_verifier
  // ────────────────────────────────────────────────────────────────────────────

  describe("rotate_verifier", () => {
    it("rejects rotation by a non-current-verifier", async () => {
      const impostor = Keypair.generate();
      context.setAccount(impostor.publicKey, {
        executable: false,
        owner: SystemProgram.programId,
        lamports: BigInt(5 * LAMPORTS_PER_SOL),
        data: Buffer.alloc(0),
      });

      try {
        await program.methods
          .rotateVerifier(newVerifier.publicKey)
          .accounts({ verifierAuthority: impostor.publicKey, verifierState })
          .signers([impostor])
          .rpc();
        assert.fail("expected NotVerifier");
      } catch (err: any) {
        assert.match(err.toString(), /NotVerifier/, "error: NotVerifier");
      }
    });

    it("rotates verifier authority to newVerifier", async () => {
      await program.methods
        .rotateVerifier(newVerifier.publicKey)
        .accounts({ verifierAuthority: verifier.publicKey, verifierState })
        .signers([verifier])
        .rpc();

      const s = await program.account.verifierState.fetch(verifierState);
      assert.ok(s.verifierAuthority.equals(newVerifier.publicKey), "verifier_authority updated");
    });

    it("old verifier can no longer verify attestations after rotation", async () => {
      const testNode = Keypair.generate();
      const rec      = attRecord(testNode.publicKey, program.programId);

      await program.methods
        .submitAttestation(
          testNode.publicKey,
          Array.from(Buffer.alloc(48, 0x33)),
          teeType("TD"),
          Array.from(Buffer.alloc(32, 0x44)),
        )
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      try {
        await program.methods
          .verifyAttestation()
          .accounts({ verifierAuthority: verifier.publicKey, verifierState, attestationRecord: rec }) // old key
          .signers([verifier])
          .rpc();
        assert.fail("expected NotVerifier for old key");
      } catch (err: any) {
        assert.match(err.toString(), /NotVerifier/, "error: NotVerifier (old key)");
      }
    });

    it("new verifier can verify attestations after rotation", async () => {
      const testNode = Keypair.generate();
      const rec      = attRecord(testNode.publicKey, program.programId);

      await program.methods
        .submitAttestation(
          testNode.publicKey,
          Array.from(Buffer.alloc(48, 0x55)),
          teeType("SE"),
          Array.from(Buffer.alloc(32, 0x66)),
        )
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      await program.methods
        .verifyAttestation()
        .accounts({ verifierAuthority: newVerifier.publicKey, verifierState, attestationRecord: rec })
        .signers([newVerifier])
        .rpc();

      const r = await program.account.attestationRecord.fetch(rec);
      assert.property(r.status, "verified", "new verifier can verify");
    });
  });

  // ────────────────────────────────────────────────────────────────────────────
  // close_attestation
  // ────────────────────────────────────────────────────────────────────────────

  describe("close_attestation", () => {
    const closeNode = Keypair.generate();
    let closeRec: PublicKey;

    before(async () => {
      closeRec = attRecord(closeNode.publicKey, program.programId);

      // Submit + verify so the record is in a closeable state.
      await program.methods
        .submitAttestation(
          closeNode.publicKey,
          Array.from(Buffer.alloc(48, 0x77)),
          teeType("TD"),
          Array.from(Buffer.alloc(32, 0x88)),
        )
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: closeRec, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      await program.methods
        .verifyAttestation()
        .accounts({ verifierAuthority: newVerifier.publicKey, verifierState, attestationRecord: closeRec })
        .signers([newVerifier])
        .rpc();
    });

    it("rejects close by a non-submitter", async () => {
      const impostor = Keypair.generate();
      context.setAccount(impostor.publicKey, {
        executable: false,
        owner: SystemProgram.programId,
        lamports: BigInt(5 * LAMPORTS_PER_SOL),
        data: Buffer.alloc(0),
      });

      try {
        await program.methods
          .closeAttestation()
          .accounts({ operatorAuthority: impostor.publicKey, attestationRecord: closeRec })
          .signers([impostor])
          .rpc();
        assert.fail("expected NotSubmitter");
      } catch (err: any) {
        assert.match(err.toString(), /NotSubmitter/, "error: NotSubmitter");
      }
    });

    it("closes a Verified record and returns rent to operator", async () => {
      const balBefore = await provider.connection.getBalance(operator.publicKey);

      await program.methods
        .closeAttestation()
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: closeRec })
        .signers([operator])
        .rpc();

      const info = await provider.connection.getAccountInfo(closeRec);
      assert.isNull(info, "attestation_record account closed");

      const balAfter = await provider.connection.getBalance(operator.publicKey);
      assert.isAbove(balAfter, balBefore, "rent lamports returned to operator");
    });

    it("closes a Revoked record (operator can reclaim rent after remediation)", async () => {
      const reclaimNode = Keypair.generate();
      const rec         = attRecord(reclaimNode.publicKey, program.programId);

      await program.methods
        .submitAttestation(
          reclaimNode.publicKey,
          Array.from(Buffer.alloc(48, 0x99)),
          teeType("SE"),
          Array.from(Buffer.alloc(32, 0xaa)),
        )
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec, systemProgram: SystemProgram.programId })
        .signers([operator])
        .rpc();

      await program.methods
        .revokeAttestation(255) // reason 255 = other
        .accounts({ verifierAuthority: newVerifier.publicKey, verifierState, attestationRecord: rec })
        .signers([newVerifier])
        .rpc();

      await program.methods
        .closeAttestation()
        .accounts({ operatorAuthority: operator.publicKey, attestationRecord: rec })
        .signers([operator])
        .rpc();

      const info = await provider.connection.getAccountInfo(rec);
      assert.isNull(info, "revoked record closed successfully");
    });
  });
});
