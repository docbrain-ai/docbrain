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

DocBrain uses a fixed pre-trained `sentence-transformers` model (configurable via `LEARNING_BASE_MODEL_ID`, default `all-MiniLM-L6-v2`). Feedback is still collected and drives Autopilot gap detection — it just doesn't feed back into the embedding model.

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
Data quality check (poison detection)
    │
    ▼
Fine-tuning run (sentence-transformers, MultipleNegativesRankingLoss)
    │
    ▼
ONNX export
    │
    ▼
Model enters "shadow" state (admin reviews in UI)
    │
    ▼
Admin promotes model → re-embed all chunks → better retrieval
```

The model trains only on what your team found helpful or unhelpful — not on document content directly. This means the model learns ranking preferences, not facts.

---

## Embed Provider and Fine-Tuned Models

**Important:** Fine-tuned models only take effect when `EMBED_PROVIDER=local`. If you are using a cloud embedding provider (`bedrock`, `openai`, etc.), the learning pipeline trains models but they are not used for inference.

| `EMBED_PROVIDER` | What serves embeddings | Fine-tuned models used? |
|---|---|---|
| `bedrock` | AWS Bedrock (Cohere, Titan, etc.) | No |
| `openai` | OpenAI API | No |
| `local` | Trainer sidecar ONNX model | Yes |

Switching from a cloud provider to `local` (or vice versa) **requires a full re-index** of all documents because the embedding dimensions change (e.g. Cohere embed-v4 is 1024-dim; all-MiniLM-L6-v2 is 384-dim). Trigger a manual re-index after changing `EMBED_PROVIDER`.

---

## Safety Mechanisms

Two safety systems protect against degraded retrieval quality and feedback manipulation:

### Training Data Quality Guards

Before any training run, the feedback corpus is validated. If more than 80% of the training pairs originate from a single user, the corpus is rejected and training is aborted. This prevents coordinated false feedback from degrading retrieval quality for the whole team.

The threshold is configurable: `LEARNING_MAX_SINGLE_USER_FRACTION` (default `0.80`).

### Automatic Rollback

After a model is promoted, the circuit breaker monitors retrieval confidence. If the new model's confidence scores drop more than 5% compared to the baseline, rollback triggers automatically:

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
| `pending` | Training job submitted, awaiting completion |
| `shadow` | Training complete, ONNX exported, ready to promote |
| `canary` | Serving a fraction of embedding requests for quality comparison |
| `promoted` | Active model; all embedding requests use this model |
| `retired` | Superseded by a newer promoted model |
| `failed` | Training failed or quality regression rollback |

Transitions:
- `pending` → `shadow` (trainer completes) or `failed` (trainer errors)
- `shadow` → `promoted` (admin promotes, skipping canary) or `failed`
- `shadow` → `canary` → `promoted` or `failed`
- `promoted` → `retired` (when a newer model is promoted)

---

## Enabling the Learning Pipeline

### Docker Compose

Add the trainer service to `docker-compose.yml` (commented out by default) and set these environment variables in your `.env`:

```bash
LEARNING_ENABLED=true
EMBED_PROVIDER=local
LEARNING_TRAINER_URL=http://trainer:8765
TRAINER_API_KEY=<generate with: openssl rand -hex 32>
LEARNING_STORAGE_BACKEND=local   # or s3, gcs, azure
LEARNING_LOCAL_PATH=/data/models  # must match TRAINER_LOCAL_MODEL_ROOT in trainer service
```

**Critical for `local` storage:** `LEARNING_LOCAL_PATH` on the server and `TRAINER_LOCAL_MODEL_ROOT` on the trainer **must point to the same mounted volume**. In Docker Compose, both services mount the same named volume to this path.

See `docker-compose.yml` for the full trainer service definition.

### Kubernetes (Helm)

For Kubernetes, use an object storage backend (S3, GCS, or Azure). The `local` backend requires a `ReadWriteMany` persistent volume shared between the server and trainer pods, which is complex to operate.

```yaml
# values.yaml
learning:
  enabled: true
  storage:
    backend: s3
    s3Bucket: "my-docbrain-models"
    s3Region: "us-east-1"
  training:
    baseModelId: "sentence-transformers/all-MiniLM-L6-v2"  # must match trainer.baseModelName

trainer:
  enabled: true
  baseModelName: "sentence-transformers/all-MiniLM-L6-v2"  # must match learning.training.baseModelId
  resources:
    requests:
      memory: "2Gi"
    limits:
      memory: "8Gi"
  persistence:
    enabled: true
    size: 20Gi
```

When using `local` storage in Kubernetes, the Helm chart automatically mounts the trainer's PVC into the server pod. The PVC must use a `ReadWriteMany` storage class or both pods must be scheduled on the same node with a `ReadWriteOnce` class.

---

## Key Configuration Variables

### Server-side

| Variable | Default | Description |
|----------|---------|-------------|
| `LEARNING_ENABLED` | `false` | Enable the learning pipeline |
| `EMBED_PROVIDER` | `bedrock` | Set to `local` to use the trained model for inference |
| `LEARNING_TRAINER_URL` | `http://docbrain-trainer:8080` | URL of the trainer sidecar |
| `LEARNING_BASE_MODEL_ID` | `sentence-transformers/all-MiniLM-L6-v2` | HuggingFace base model for fine-tuning — **must match `TRAINER_BASE_MODEL_NAME`** |
| `LEARNING_MIN_TRIPLETS` | `200` | Minimum training pairs before a run triggers |
| `LEARNING_STORAGE_BACKEND` | `local` | `local`, `s3`, `gcs`, or `azure` |
| `LEARNING_LOCAL_PATH` | `/app/models` | Writable path for model artefacts when `backend=local` — **must match `TRAINER_LOCAL_MODEL_ROOT`** |
| `LEARNING_S3_BUCKET` | — | S3 bucket name (required when `backend=s3`) |
| `LEARNING_S3_PREFIX` | `docbrain/models` | S3 key prefix |
| `LEARNING_S3_REGION` | `us-east-1` | AWS region |
| `LEARNING_GCS_BUCKET` | — | GCS bucket name (required when `backend=gcs`) |
| `LEARNING_AZURE_CONTAINER` | — | Azure Blob container (required when `backend=azure`) |
| `LEARNING_CIRCUIT_BREAKER_THRESHOLD` | `0.05` | Max confidence drop before automatic rollback |
| `LEARNING_MAX_SINGLE_USER_FRACTION` | `0.80` | Max fraction of training data from a single user |
| `LEARNING_POISON_DETECTION` | `true` | Enable feedback quality validation before training |

### Trainer sidecar

| Variable | Default | Description |
|----------|---------|-------------|
| `TRAINER_BASE_MODEL_NAME` | `sentence-transformers/all-MiniLM-L6-v2` | HuggingFace base model — **must match `LEARNING_BASE_MODEL_ID`** |
| `TRAINER_LOCAL_MODEL_ROOT` | `/data/models` | Local path for model storage — **must match `LEARNING_LOCAL_PATH`** |
| `TRAINER_EPOCHS` | `3` | Training epochs |
| `TRAINER_BATCH_SIZE` | `16` | Batch size (reduce if OOM on CPU) |
| `TRAINER_API_KEY` | — | Shared secret between server and trainer (required) |

Full configuration reference: [Configuration Guide](configuration.md)

---

## Monitoring and Management

### Admin UI

Navigate to **Settings → Learning Pipeline** in the DocBrain web UI. The dashboard shows:

- The currently serving model (provider, model ID, and whether a fine-tuned model is active)
- All training runs with their status, triplet count, and validation loss
- A "Promote to Active" button for models in `shadow` or `canary` state

### API

```bash
# Training run history
curl -H "Authorization: Bearer db_sk_..." \
  http://localhost:3000/api/v1/admin/learning/versions

# Manually trigger a learning cycle (bypasses the scheduler — useful for testing)
curl -X POST -H "Authorization: Bearer db_sk_..." \
  http://localhost:3000/api/v1/admin/learning/trigger

# Promote a shadow or canary model to active
curl -X POST -H "Authorization: Bearer db_sk_..." \
  http://localhost:3000/api/v1/admin/learning/versions/{version_id}/promote

# Trainer sidecar health
curl http://localhost:8765/health
```

---

## Frequently Asked Questions

**Does fine-tuning change what DocBrain knows?**
No. Fine-tuning adjusts the embedding model — how documents are represented as vectors for similarity search. It doesn't change the documents themselves, the LLM used for answer generation, or the knowledge graph. It only changes which documents are retrieved as candidates.

**How much feedback do I need before it helps?**
Plan for at least 200 high-quality triplets. In practice, this means 2,000–5,000 total feedback events (since not all episodes produce clean triplets). Most teams see this volume after 3–6 months of active use.

**I'm already using a cloud embedding provider. Do I need to switch?**
You don't have to switch. You can run the learning pipeline with `EMBED_PROVIDER=bedrock` (or another cloud provider) — it will train models and store them, but they won't serve inference until you switch to `EMBED_PROVIDER=local`. The pipeline is additive: enable it now to accumulate training data, switch to local inference later when you're ready. Note that switching providers requires a full re-index.

**Can I choose a different base model?**
Yes. Set `LEARNING_BASE_MODEL_ID` (server) and `TRAINER_BASE_MODEL_NAME` (trainer) to any HuggingFace `sentence-transformers` model — they must match. Larger models (e.g. `all-mpnet-base-v2`) produce higher quality embeddings but require more memory and time to train. For CPU-only environments, `all-MiniLM-L6-v2` is the best balance of quality and speed.

**Can I roll back a promoted model?**
Automatic rollback triggers when the circuit breaker detects a quality regression. For manual rollback, promote a previously retired or shadow version via the admin UI or `POST /api/v1/admin/learning/versions/{id}/promote`. If no prior version is available, set `EMBED_PROVIDER=bedrock` (or your original provider) to bypass the local model while you investigate.

**What happens if training fails?**
The model enters `failed` state. The previously active model continues serving requests unchanged. Check the training run in the admin UI or via `GET /api/v1/admin/learning/versions` for the failure reason, fix the underlying issue (insufficient data, OOM, trainer unreachable), and the next scheduled training run will try again.

**What if the trainer can't be reached?**
The server polls the trainer every 30 seconds for pending job status. If the trainer is unreachable, pending jobs stay in `pending` state. Jobs submitted before the sync loop was running will be automatically marked `failed` (with reason "trainer has no record of this job") when the trainer returns a 404 for the job ID.

**Why are the server and trainer base model configs separate?**
The server (`LEARNING_BASE_MODEL_ID`) uses the value to register training runs in the database and correlate them. The trainer (`TRAINER_BASE_MODEL_NAME`) uses it to load the model from HuggingFace. They must match. The Helm chart wires them from the same value (`learning.training.baseModelId`) so they stay in sync automatically.
