# Architecture

## System Overview

DocBrain is a Rust-based documentation intelligence platform that combines RAG (Retrieval-Augmented Generation) with a multi-tier memory system, freshness scoring, intent-adaptive responses, and an autonomous Documentation Autopilot that identifies gaps and generates draft content.

```
┌─────────────┐     ┌──────────────────────────┐     ┌───────────────┐
│   Web UI    │────>│       API Server          │────>│  LLM Provider │
│  (Next.js)  │     │   (Rust / Axum)           │     │  (pluggable)  │
└─────────────┘     └────────────┬──────────────┘     └───────────────┘
                                 │
                    ┌────────────┼────────────┐
                    │            │            │
              ┌─────┴────┐ ┌────┴─────┐ ┌───┴────┐
              │PostgreSQL│ │OpenSearch│ │ Redis  │
              │(memory + │ │(vectors +│ │(cache +│
              │ autopilot│ │  search) │ │session)│
              │ + scores)│ │          │ │        │
              └──────────┘ └──────────┘ └────────┘
```

## Core Components

### API Server (`docbrain-server`)
Axum-based HTTP server exposing the REST API. Handles authentication (API keys with RBAC), rate limiting, SSE streaming, and routes requests through the RAG pipeline. Serves both the Q&A endpoints and the Autopilot management API.

### RAG Pipeline (`docbrain-core`)
The intelligence layer. For each query:

1. **Intent Classification** — Determines query type (factual, procedural, troubleshooting, conceptual, comparative)
2. **Query Rewriting** — Reformulates for better retrieval using conversation context
3. **Hybrid Search** — Combines k-NN vector similarity with BM25 keyword matching
4. **Memory Enrichment** — Augments context from the 4-tier memory system
5. **Response Generation** — LLM synthesizes an answer with source attribution
6. **Caching** — Semantic cache for repeated questions

### Documentation Autopilot (`docbrain-core/autopilot`)
The autonomous documentation improvement engine. Runs on a daily schedule:

1. **Gap Analyzer** — Scans episodic memory for unanswered queries, negative feedback, and low-confidence answers from the past 30 days. Embeds the queries and clusters them using greedy cosine similarity (threshold: 0.82). Each cluster represents a documentation gap. Severity is calculated from query volume (critical: 25+, high: 11-25, medium: 4-10, low: 1-3).

2. **Doc Drafter** — Takes a gap cluster and generates a draft document. Uses sample queries from the cluster to search existing docs for partial context. Classifies the needed content type (runbook, FAQ, guide, troubleshooting, reference) via LLM, then generates a full draft with proper formatting and source attribution.

3. **Digest Builder** — Compiles weekly documentation health reports combining: query volume, unanswered rate, top gap clusters, new drafts, and stale document count. Formats as Slack Block Kit messages for team delivery.

### Ingestion Pipeline (`docbrain-ingest`)
Fetches documents from configured sources (Confluence, GitHub, local files), converts to Markdown, chunks with heading-aware splitting, generates embeddings, and indexes in OpenSearch.

### MCP Server (`docbrain-mcp`)
Model Context Protocol server for integration with AI coding tools (Claude Code, Cursor).

### CLI (`docbrain-cli`)
Command-line client for interactive Q&A sessions.

## 4-Tier Memory System

| Tier | Storage | Purpose | TTL |
|------|---------|---------|-----|
| **Working** | Redis | Current conversation context | Session-scoped |
| **Episodic** | PostgreSQL + OpenSearch | Past Q&A episodes, feedback | Permanent |
| **Semantic** | PostgreSQL | Entity graph (services, teams, concepts) | Permanent |
| **Procedural** | PostgreSQL | Learned rules from feedback patterns | Permanent |

### How Memory Works

- **Working memory** maintains conversation state across turns within a session
- **Episodic memory** finds similar past questions and their validated answers
- **Semantic memory** resolves entity references ("the auth service" -> specific service with known dependencies)
- **Procedural memory** applies learned rules (e.g., "when asked about deployments, always mention the canary process")

### How Memory Feeds Autopilot

Episodic memory is the primary data source for gap detection. Every query that receives negative feedback, a `not_found` resolution, or a confidence score below 0.4 becomes a candidate for gap analysis. This creates a closed loop: user questions that expose documentation gaps are automatically surfaced and addressed.

## 5-Signal Freshness Scoring

Each document receives a freshness score (0-100) based on:

1. **Time Decay** (30%) — How recently the document was edited
2. **Engagement** (20%) — View count, query frequency, feedback ratio
3. **Content Currency** (20%) — LLM analysis of temporal language ("as of Q1 2024")
4. **Link Health** (15%) — Percentage of working links
5. **Contradiction Detection** (15%) — Cross-document consistency analysis

Documents are classified as: Fresh (80-100), Review (60-79), Stale (40-59), Outdated (<40).

Freshness scores integrate with Autopilot: stale documents that also have high query volume are surfaced as high-priority gaps with draft updates.

## Intent-Adaptive Responses

Different query types receive tailored response formats:

| Intent | Response Style |
|--------|---------------|
| Factual | Direct answer with source citation |
| Procedural | Numbered step-by-step instructions |
| Troubleshooting | Diagnostic tree with common causes |
| Conceptual | Explanation with analogies and context |
| Comparative | Structured comparison with trade-offs |

## Data Flow

### Q&A Pipeline

```
User Question
    │
    ▼
Intent Classification ──> Query Rewriting
    │                          │
    ▼                          ▼
Memory Lookup              Hybrid Search
(episodic, semantic,       (OpenSearch: k-NN + BM25)
 procedural)                   │
    │                          │
    └──────────┬───────────────┘
               │
               ▼
        Context Assembly
        (ranked chunks + memory)
               │
               ▼
        LLM Generation
        (streaming SSE)
               │
               ▼
        Episode Storage
        (for future memory + gap analysis)
```

### Autopilot Pipeline

```
Episodic Memory (30-day window)
    │
    ▼
Filter: negative feedback, not_found, low confidence
    │
    ▼
Embed queries ──> Greedy cosine clustering (threshold: 0.82)
    │
    ▼
Label clusters via LLM ──> Persist to autopilot_gap_clusters
    │
    ▼
On demand: Generate draft ──> Search existing docs for context
    │                          │
    ▼                          ▼
Classify content type       Assemble context from related docs
    │                          │
    └──────────┬───────────────┘
               │
               ▼
        LLM Draft Generation
               │
               ▼
        Review ──> Publish ──> Re-ingest ──> Better answers
```

## Database Schema (Autopilot)

| Table | Purpose |
|-------|---------|
| `autopilot_gap_clusters` | Detected documentation gaps with severity, sample queries, and status |
| `autopilot_drafts` | Generated draft documents linked to gap clusters |
| `autopilot_digests` | Weekly digest send history for deduplication |
