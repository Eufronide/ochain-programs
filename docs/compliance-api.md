# Ochain Enterprise Compliance API
## EU AI Act Article 12 — Technical Design

---

## Overview

High-risk AI systems under EU AI Act Annex III must automatically record events
throughout operation (Article 12). This document defines the Ochain compliance
layer: the data flow from AI agent execution to tamper-resistant audit log to a
PDF report a regulator can independently verify.

The architecture separates *what gets anchored* (cryptographic fingerprints,
publicly verifiable) from *what stays in your infrastructure* (actual inputs,
outputs, and personal data). GDPR and Article 12 are simultaneously satisfied.

---

## Data Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           ENTERPRISE RUNTIME                                │
│                                                                             │
│  1. Business system calls AI agent                                          │
│     POST /v1/executions  { model_id, input_hash, context_id, user_id_hash }│
│                │                                                            │
│  2. API Gateway validates, assigns execution_id, forwards to TEE            │
│                │                                                            │
│  3. TEE Enclave executes model                                              │
│     ┌─────────────────────────────┐                                         │
│     │  AI Model (TDX/SEV-SNP)    │                                         │
│     │  input_hash  verified      │                                         │
│     │  model_hash  pinned        │                                         │
│     │  output generated          │                                         │
│     │  execution_receipt signed  │                                         │
│     └──────────────┬─────────────┘                                         │
│                    │                                                        │
│  4. TEE emits ExecutionReceipt (never leaves TEE with raw data)            │
│     {                                                                       │
│       execution_id:     "exec_01J...",                                      │
│       model_id:         "credit-scorer-v2.4.1",                            │
│       model_hash:       sha256(model binary),                               │
│       input_hash:       sha256(raw_input),                                  │
│       output_hash:      sha256(raw_output),                                 │
│       confidence_score: 0.94,                                               │
│       human_override:   false,                                              │
│       tee_type:         "TD",                                               │
│       measurement_hash: MRTD of the enclave,                               │
│       slot:             current_solana_slot,                                │
│       tee_signature:    Ed25519(receipt, node_key)                         │
│     }                                                                       │
│                                                                             │
│  5. Compliance Service receives receipt                                     │
│     ├── Writes encrypted full log to off-chain store (AES-256-GCM)         │
│     │     key stored in enterprise HSM                                      │
│     └── Enqueues receipt for on-chain anchoring                            │
│                                                                             │
└──────────────────────────────┬──────────────────────────────────────────────┘
                               │
                    ┌──────────▼──────────────┐
                    │   OCHAIN ANCHOR LAYER   │
                    │                         │
                    │  6. Batch assembly      │
                    │     Every N seconds,    │
                    │     collect receipts,   │
                    │     build Merkle tree   │
                    │                         │
                    │  7. ochain-attestation  │
                    │     submit_attestation  │
                    │     → AttestationRecord │
                    │     PDA on Solana       │
                    │                         │
                    │  8. ComplianceCheckpoint│
                    │     { merkle_root,      │
                    │       period_start,     │
                    │       period_end,       │
                    │       record_count,     │
                    │       model_ids[] }     │
                    │     anchored on-chain   │
                    │                         │
                    └──────────┬──────────────┘
                               │
                    ┌──────────▼──────────────┐
                    │   REPORT GENERATION     │
                    │                         │
                    │  9. Audit trigger       │
                    │     POST /v1/audits     │
                    │                         │
                    │ 10. Report builder:     │
                    │     fetch receipts,     │
                    │     compute Merkle      │
                    │     proofs per record,  │
                    │     generate PDF,       │
                    │     sign PDF hash,      │
                    │     anchor report_hash  │
                    │                         │
                    │ 11. Regulator downloads │
                    │     PDF + verifies via  │
                    │     verification portal │
                    └─────────────────────────┘
```

---

## On-Chain vs Off-Chain

| Data | Location | Reason |
|---|---|---|
| `execution_id` | On-chain (Merkle leaf) | Record-level traceability |
| `model_id` + `model_hash` | On-chain | Proves which model version ran |
| `input_hash` | On-chain | Integrity check without exposing input |
| `output_hash` | On-chain | Integrity check without exposing output |
| `confidence_score` | On-chain | Required by Art. 12 for high-risk decisions |
| `human_override` flag | On-chain | Required by Art. 14 (human oversight) |
| TEE measurement hash | On-chain (AttestationRecord) | Hardware-verified execution environment |
| TEE attestation key signature | On-chain | Cryptographic binding receipt ↔ enclave |
| Merkle root of batch | On-chain (ComplianceCheckpoint) | Tamper-resistant anchor for entire period |
| Compliance report PDF hash | On-chain | Regulator verifies PDF wasn't altered post-generation |
| **Raw AI inputs** | Off-chain (encrypted) | GDPR — personal data must not be on public ledger |
| **Raw AI outputs** | Off-chain (encrypted) | Business sensitivity + GDPR |
| **User identifiers** | Off-chain (hashed pseudonym on-chain) | GDPR pseudonymisation |
| **Model weights** | Off-chain (hash on-chain) | IP protection; size impractical on-chain |
| **Full audit PDF** | Off-chain (S3/IPFS); hash on-chain | Size; hash provides integrity |

---

## On-Chain Program Extension

A new `ComplianceCheckpoint` account is anchored by `ochain-attestation`:

```
ComplianceCheckpoint (PDA: ["compliance", deployer, period_hash])
  deployer:         Pubkey       // enterprise authority
  model_ids_hash:   [u8; 32]    // SHA-256 of sorted model ID list
  period_start:     i64          // Unix timestamp
  period_end:       i64
  record_count:     u64
  merkle_root:      [u8; 32]
  report_hash:      [u8; 32]    // SHA-256 of the PDF; set after report generation
  tee_operator:     Pubkey       // which Ochain operator produced this batch
  attestation_ref:  Pubkey       // → AttestationRecord confirming the TEE
  status:           CheckpointStatus { Building, Anchored, Reported }
  bump:             u8
```

---

## REST API

```
BASE URL: https://compliance.yourdomain.com/api/v1
Auth:     Bearer JWT (OAuth 2.0, scope: ochain:compliance)
```

### Execution Logging

```http
POST /v1/executions
Content-Type: application/json

{
  "model_id":        "credit-scorer-v2.4.1",
  "context_id":      "loan-application-8821",
  "input_hash":      "sha256:e3b0c44298fc...",
  "tee_type":        "TD",
  "deployment_env":  "prod-eu-west-1",
  "regulation_tags": ["EU_AI_ACT_ANNEX_III_5b"]
}

→ 202 Accepted
{
  "execution_id":  "exec_01JKVM4...",
  "receipt_url":   "/v1/executions/exec_01JKVM4.../receipt",
  "anchor_eta_ms": 5000
}
```

```http
GET /v1/executions/{execution_id}
→ 200 OK
{
  "execution_id":    "exec_01JKVM4...",
  "model_id":        "credit-scorer-v2.4.1",
  "model_hash":      "sha256:4a5e1e4b...",
  "confidence":      0.94,
  "human_override":  false,
  "tee_verified":    true,
  "anchored_at":     "2025-03-15T14:22:01Z",
  "anchor_slot":     312847291,
  "merkle_position": 47,
  "batch_id":        "batch_01JKVM...",
  "status":          "anchored"
}
```

```http
GET /v1/executions/{execution_id}/proof
→ 200 OK
{
  "execution_id":  "exec_01JKVM4...",
  "leaf_hash":     "a3f2d91...",
  "merkle_proof":  ["b7c3e12...", "d94fa8...", "0011ac..."],
  "merkle_root":   "f19204b...",
  "checkpoint_id": "check_01JKVM...",
  "on_chain_slot": 312847291,
  "tee_signature": "4b2a9f...",
  "verify_url":    "https://verify.ochain.io/proof/exec_01JKVM4..."
}
```

### Compliance Checkpoints

```http
POST /v1/checkpoints
{
  "period_start": "2025-01-01T00:00:00Z",
  "period_end":   "2025-03-31T23:59:59Z",
  "model_ids":    ["credit-scorer-v2.4.1"],
  "label":        "Q1-2025-credit-decisions"
}

→ 202 Accepted
{
  "checkpoint_id": "check_01JKVM...",
  "record_count":  12847,
  "merkle_root":   "f19204b...",
  "anchor_tx":     "5xN7KqE2...",
  "status":        "anchoring"
}
```

```http
GET /v1/checkpoints/{checkpoint_id}
→ 200 OK
{
  "checkpoint_id":       "check_01JKVM...",
  "period_start":        "2025-01-01T00:00:00Z",
  "period_end":          "2025-03-31T23:59:59Z",
  "record_count":        12847,
  "merkle_root":         "f19204b...",
  "anchor_slot":         312847291,
  "anchor_time":         "2025-04-01T09:00:44Z",
  "operator_count":      15,
  "consensus_threshold": 10,
  "status":              "confirmed"
}
```

### Audit Reports

```http
POST /v1/audits
{
  "checkpoint_id":          "check_01JKVM...",
  "format":                 "pdf",
  "language":               "en",
  "include_sample_records": 25,
  "redaction_policy":       "pseudonymise_subjects",
  "recipient": {
    "name":  "Bundesamt für KI-Aufsicht",
    "email": "audit@example-authority.de"
  }
}

→ 202 Accepted
{
  "audit_id":    "audit_01JKVM...",
  "eta_seconds": 45,
  "status_url":  "/v1/audits/audit_01JKVM.../status"
}
```

```http
GET /v1/audits/{audit_id}/download
→ 200 OK  Content-Type: application/pdf
  X-Report-Hash: sha256:8f3d...
  X-Anchor-Slot: 312901847
```

### Verification (public, no auth required)

```http
GET /v1/verify/{execution_id}
→ 200 OK
{
  "status":        "verified",
  "model_id":      "credit-scorer-v2.4.1",
  "anchored_at":   "2025-03-15T14:22:01Z",
  "tee_verified":  true,
  "proof_valid":   true,
  "checkpoint_id": "check_01JKVM..."
}

GET /v1/verify/report/{report_hash}
→ 200 OK
{
  "status":       "authentic",
  "generated_at": "2025-04-01T09:00:00Z",
  "anchor_slot":  312901847,
  "tampered":     false
}
```

---

## Explaining This to a Compliance Officer

> "Every time our AI system makes a decision, it automatically creates a sealed
> record — not the personal data itself, but a unique digital fingerprint, like
> a wax seal on an envelope. You don't need to open the envelope to know it
> hasn't been tampered with.
>
> These fingerprints are registered in real time with an independent network of
> fifteen organisations across the EU, none of which we control. Think of it as
> a distributed notary service. Every quarter, all fingerprints from that period
> are consolidated into a single master seal, registered with all fifteen
> notaries simultaneously. To alter any record after the fact, you would need to
> simultaneously compromise ten of those fifteen independent organisations —
> which is not practically possible.
>
> When a regulator requests an audit, our system generates a PDF report. That
> report contains a QR code any regulator can scan to independently confirm the
> report has not been altered since it was generated. The regulator does not
> need to trust us. They can verify authenticity themselves, without our
> involvement, in about thirty seconds.
>
> Your data stays in our infrastructure, in the EU, under GDPR. The notary
> network only ever sees the fingerprints, never the underlying data."

---

## Why This Satisfies Regulators

Three questions any competent regulator will ask, and how the architecture
answers them without requiring trust in the deployer:

**"How do I know you didn't alter the logs before sending this?"**
The Merkle root was anchored before the report was generated, by fifteen
independent operators. Altering any record changes the root, which no longer
matches the anchor.

**"How do I know the AI model that ran was the one you claim?"**
The TEE measurement hash (`MRTD`) is a hardware-computed fingerprint of every
byte loaded into the execution environment. An independent auditor can download
the model binary and confirm the measurement matches.

**"How do I know the timestamps weren't backdated?"**
Network slot numbers have a deterministic relationship to wall-clock time
observable by anyone. You cannot anchor a record into a slot after that slot
has passed.
