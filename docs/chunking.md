# Chunk Tuning and Re-Ingestion

DocBrain splits documents into chunks before embedding. Chunk size is the single biggest lever for RAG quality — too small and you lose context; too large and retrieved chunks contain noise that dilutes the answer.

---

## How Chunking Works

DocBrain uses a **header-aware** strategy:

1. Split the document at Markdown header boundaries (`#`, `##`, `###`, …)
2. Each section (header + body) becomes a chunk if it fits within `CHUNK_SIZE`
3. Sections larger than `CHUNK_SIZE` are split further at paragraph boundaries (`\n\n`)
4. Chunks smaller than 50 characters are discarded
5. Pages that produce **zero chunks** (macro-only pages, empty bodies) are stored as an empty-version marker in OpenSearch so they are not re-fetched on subsequent ingest runs

This preserves semantic coherence — a chunk about "Deployment → Kubernetes" stays together rather than being cut mid-sentence.

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `CHUNK_SIZE` | `1500` | Target chunk size in characters |
| `CHUNK_OVERLAP` | `200` | Overlap between adjacent paragraph-split chunks (reserved for future use) |

```env
CHUNK_SIZE=1500
CHUNK_OVERLAP=200
```

> **Character vs token count.** `CHUNK_SIZE` is measured in characters, not tokens. At ~4 chars/token, 1500 chars ≈ 375 tokens — well within the embedding model context window (Cohere Embed v4 supports 512 tokens, OpenAI `text-embedding-3-small` supports 8191).

---

## Choosing a Chunk Size

| Scenario | Recommended `CHUNK_SIZE` |
|---|---|
| Dense technical docs (API refs, config tables) | `800` – `1200` |
| General documentation (default) | `1500` |
| Long-form prose (runbooks, architecture docs) | `2000` – `2500` |
| Mixed content | `1500` (default) — good all-rounder |

**Signs of under-chunking (too small):**
- RAG answers feel disconnected or miss obvious context
- High chunk counts (30+ chunks per average doc)
- Answers cite many chunks from the same document

**Signs of over-chunking (too large):**
- Low chunk counts (1–3 per average doc)
- Answers are accurate but retrieval scores are lower
- Single chunks contain content from unrelated sub-sections

---

## Re-Ingesting After a Chunk Size Change

Changing `CHUNK_SIZE` only affects **new ingest runs**. Existing chunks in OpenSearch retain their old size. To apply the new size across your entire corpus:

```bash
# 1. Set the new chunk size
export CHUNK_SIZE=1500

# 2. Run a full re-ingest
# The ingest binary deletes and recreates all chunks for every page it processes.
cargo run --bin docbrain-ingest

# Or with the release binary:
./target/release/docbrain-ingest
```

The ingest run:
- Deletes existing OpenSearch chunks for each processed page
- Re-chunks with the new `CHUNK_SIZE`
- Re-embeds and re-indexes all chunks

For 10,000 documents this typically takes 20–60 minutes depending on concurrency (`INGEST_CONCURRENCY`, default 10) and embedding latency.

### Check chunk counts before and after

```bash
# Before: count chunks in OpenSearch
curl -s "http://localhost:9200/docbrain-chunks/_count" | jq .count

# After re-ingest: count again
curl -s "http://localhost:9200/docbrain-chunks/_count" | jq .count
```

At `CHUNK_SIZE=1500` you should see roughly **5–8 chunks per document** on average. At the previous default of `1000`, you'd expect 3–5 per doc.

---

## Webhook-Synced Pages

For pages updated via the Confluence webhook (real-time sync), the chunk size is read from the `CHUNK_SIZE` environment variable at the time the webhook fires. No restart is needed — the server reads the env var on each webhook event.
