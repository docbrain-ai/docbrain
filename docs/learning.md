# Learning Pipeline — Self-Improving Embeddings

DocBrain can fine-tune its own embedding model based on your team's feedback — making retrieval measurably better for your specific documentation vocabulary over time. This is an opt-in capability. The default configuration uses a fixed pre-trained model that works well for most teams.

---

## What Is the Learning Pipeline?

When your team answers questions with DocBrain, every thumbs-up and thumbs-down is a signal: "this document was relevant" or "this document was not." The learning pipeline mines those signals to teach the embedding model what "relevant" means in your specific context — your service names, your terminology, your organization's way of writing about technical problems.

The practical effect: after a few hundred feedback events, DocBrain retrieves documents that your team actually finds useful more often, and retrieves less relevant documents less often. The improvement compounds over time without any manual tuning.

---

## Three Tiers

The learning pipeline is designed so you only pay for what you need.

### Tier 0 — Default (no extra infrastructure)

DocBrain uses a fixed pre-trained `sentence-transformers` model (`all-MiniLM-L6-v2` by default, or whichever embedding model you've configured). Feedback is still collected and drives Autopilot gap detection — it just doesn't feed back into the embedding model.

This tier requires nothing. It's the default.

### Tier 1 — Feedback Accumulation (object storage only)

Feedback episodes are processed into training pairs and stored in object storage (S3, GCS, or Azure Blob). No fine-tuning happens yet, but you're accumulating the training data that Tier 2 will use. This tier lets you validate the data pipeline and understand your feedback volume before committing compute resources.

**Requires:** An S3, GCS, or Azure Blob bucket with write access.

**Minimum feedback volume for useful training:** approximately 200 training pairs (typically reached after 2,000–5,000 feedback events).

### Tier 2 — Full Fine-Tuning (compute required)

The `docbrain-trainer` sidecar deploys as a companion service. It trains on accumulated triplets, exports the fine-tuned model to ONNX format, and promotes it to replace the active embedding model. All documents are automatically re-embedded with the improved model. The main server hot-swaps the model without restarting.

**Requires:** The `docbrain-trainer` Docker image, object storage (from Tier 1), and a host with at least 2 vCPU and 8 GB RAM for training runs. GPU is optional but speeds up training significantly.

---

## How Fine-Tuning Works

```
Feedback episodes (thumbs up/down)
    │
    ▼
Training pair extraction
    (anchor query, positive chunk, negative chunk)
    │
    ▼
Data quality check
    │
    ▼
Fine-tuning run (sentence-transformers, MultipleNegativesRankingLoss)
    │
    ▼
ONNX export
    │
    ▼
Canary evaluation (shadow traffic)
    │
    ▼
Promote → re-embed all chunks → better retrieval
```

The model trains only on what your team found helpful or unhelpful — not on document content directly. This means the model learns ranking preferences, not facts.

---

## Safety Mechanisms

Two safety systems protect against degraded retrieval quality and feedback manipulation:

### Training Data Quality Guards

Before any training run, the feedback corpus is validated. If more than 80% of the training pairs originate from a single user, the corpus is rejected and training is aborted. This prevents coordinated false feedback from degrading retrieval quality for the whole team.

The threshold is configurable: `LEARNING_MAX_SINGLE_USER_FRACTION` (default `0.80`).

### Automatic Rollback

After a model is promoted, it serves a fraction of embedding requests alongside the previous model. If the new model's retrieval confidence scores drop more than 5% compared to the baseline on the same queries, rollback triggers automatically:

- The new model is immediately demoted.
- The previous model resumes serving all requests.
- No re-index is triggered.
- An alert is written to the stream events log.

The threshold is configurable: `LEARNING_CIRCUIT_BREAKER_THRESHOLD` (default `0.05`).

---

## Model States

Each trained model moves through a lifecycle:

| State | Meaning |
|-------|---------|
| `pending` | Training queued, awaiting triplet extraction |
| `shadow` | Training complete, ONNX exported, awaiting evaluation |
| `canary` | Serving ~10% of embedding requests for quality comparison |
| `promoted` | Active model; all embedding requests use this model |
| `retired` | Superseded by a newer promoted model |
| `failed` | Quality regression rollback or data quality rejection |

---

## Enabling the Learning Pipeline

### Docker Compose

Add the trainer service to `docker-compose.yml` (commented out by default) and set these environment variables in your `.env`:

```bash
LEARNING_ENABLED=true
EMBED_PROVIDER=local
TRAINER_URL=http://trainer:8765
TRAINER_API_KEY=<generate with: openssl rand -hex 32>
TRAINER_STORAGE_BACKEND=local   # or s3, gcs, azure
```

See `docker-compose.yml` for the full trainer service definition.

### Kubernetes (Helm)

```yaml
# values.yaml
learning:
  enabled: true
  storage:
    backend: s3
    s3Bucket: "my-docbrain-models"
    s3Region: "us-east-1"

trainer:
  enabled: true
  resources:
    requests:
      memory: "2Gi"
    limits:
      memory: "8Gi"
  persistence:
    enabled: true
    size: 20Gi
```

---

## Key Configuration Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `LEARNING_ENABLED` | `false` | Enable the learning pipeline |
| `EMBEDDING_PROVIDER` | `openai` | Set to `local` to use the trained model |
| `TRAINER_URL` | `http://localhost:8765` | URL of the trainer sidecar |
| `LEARNING_MIN_TRIPLETS` | `200` | Minimum training pairs before a run triggers |
| `LEARNING_STORAGE_BACKEND` | `local` | `local`, `s3`, `gcs`, or `azure` |
| `TRAINER_BASE_MODEL_NAME` | `sentence-transformers/all-MiniLM-L6-v2` | Base model for fine-tuning |
| `TRAINER_EPOCHS` | `3` | Training epochs |
| `TRAINER_BATCH_SIZE` | `16` | Batch size (reduce if OOM on CPU) |
| `LEARNING_CIRCUIT_BREAKER_THRESHOLD` | `0.05` | Max confidence drop before automatic rollback |
| `LEARNING_MAX_SINGLE_USER_FRACTION` | `0.80` | Max fraction of training data from a single user |

Full configuration reference: [Configuration Guide](configuration.md)

---

## Monitoring

Check the status of the learning pipeline via the admin API:

```bash
# Training run history
curl -H "Authorization: Bearer db_sk_..." \
  http://localhost:3000/api/v1/admin/learning/runs

# Active model and its lifecycle state
curl -H "Authorization: Bearer db_sk_..." \
  http://localhost:3000/api/v1/admin/learning/model

# Trainer sidecar health
curl http://localhost:8765/health
```

---

## Frequently Asked Questions

**Does fine-tuning change what DocBrain knows?**
No. Fine-tuning adjusts the embedding model — how documents are represented as vectors for similarity search. It doesn't change the documents themselves, the LLM used for answer generation, or the knowledge graph. It only changes which documents are retrieved as candidates.

**How much feedback do I need before it helps?**
Plan for at least 200 high-quality triplets. In practice, this means 2,000–5,000 total feedback events (since not all episodes produce clean triplets). Most teams see this volume after 3–6 months of active use.

**Can I roll back a promoted model?**
Rollback is handled automatically when a quality regression is detected. Manual rollback is not currently supported via API — if you need to force a rollback, set `EMBEDDING_PROVIDER=openai` (or your original provider) to bypass the local model temporarily.

**What happens if training fails?**
The model enters `failed` state. The previously active model continues serving requests unchanged. Check `GET /api/v1/admin/learning/runs` for the failure reason, fix the underlying issue, and the next scheduled training run will try again.
