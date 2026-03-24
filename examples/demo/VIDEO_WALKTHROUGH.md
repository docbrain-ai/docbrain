# DocBrain Feature Showcase — Video Walkthrough Script

> **Purpose:** This document is the scene-by-scene script for recording a long-form demo video. Each scene explains what to show, what to click, and what to say. The goal: anyone watching understands exactly what DocBrain does and why it matters.

---

## Pre-Recording Setup

```bash
# 1. Start DocBrain
docker compose up -d

# 2. Wait for services and ingest sample docs
bash scripts/setup.sh

# 3. Seed demo data (questions, fragments, style rules, connector)
export DOCBRAIN_API_KEY="<your-admin-key>"
bash examples/demo/seed.sh

# 4. Open browser to http://localhost:3001
# 5. Log in with admin@acme.com / DemoPassword123!
```

---

## PART 1: THE PROBLEM (2-3 minutes)

### Scene 1.1 — The Documentation Problem

**What to show:** A simple slide or text overlay (or just speak to camera)

**Script:**
> "Every engineering team has the same problem. Knowledge is created in one place — a PR review, a Slack thread, an incident war room — and documented in another place, if it gets documented at all.
>
> A senior engineer explains retry logic in a PR comment. Three people learn it. Two months later, nobody can find that comment. The knowledge existed — but it was never captured.
>
> Someone asks 'how do I deploy to staging?' in Slack. A colleague writes a 4-paragraph answer. It's accurate today. In 6 months, the process changed and nobody updated the Slack answer. It's now wrong AND still being referenced.
>
> Leadership schedules a documentation sprint. Engineers write docs for two weeks. Six months later, 40% of those docs are stale.
>
> This is the fundamental problem DocBrain solves. Not with better search. Not with another chatbot on top of your wiki. DocBrain captures knowledge at the moment it's created — before anyone has to remember to write it down."

---

## PART 2: SETUP & FIRST IMPRESSIONS (3-4 minutes)

### Scene 2.1 — Five-Minute Setup

**What to show:** Terminal

**Script:**
> "DocBrain is a single 25-megabyte Rust binary. Let me show you how fast you can get it running."

**Actions:**
1. Show the terminal: `docker compose up -d`
2. Point out the services starting: PostgreSQL, OpenSearch, Redis, the DocBrain server, the web UI
3. Show `docker compose exec server cat /app/admin-bootstrap-key.txt` to get the admin key
4. Open `http://localhost:3001` in the browser

> "That's it. Five services, one command. The server started in under 500 milliseconds. Let's log in."

### Scene 2.2 — Home Dashboard

**What to show:** Web UI → Home page

**Script:**
> "This is the DocBrain dashboard. Right away you can see the health of your documentation:
> - How many documents are indexed
> - Gap forecast — how many documentation gaps are projected to grow
> - Capture activity — knowledge being captured from PRs, Slack, and CI
> - Knowledge health score
>
> Think of this as your documentation observability dashboard. Just like you monitor your services with Grafana, DocBrain monitors your knowledge base."

---

## PART 3: ASK — INTELLIGENT Q&A (4-5 minutes)

### Scene 3.1 — Asking a Question

**What to show:** Web UI → Ask page

**Actions:**
1. Navigate to the **Ask** page
2. Type: `How do I deploy to production?`
3. Show the streaming response appearing in real-time

**Script:**
> "Let's ask DocBrain a question: 'How do I deploy to production?'
>
> Watch what happens. DocBrain searches across all your documentation — it's using hybrid search, combining vector similarity AND keyword matching. Then it generates an answer with citations.
>
> See those source badges? Every claim in the answer links back to a specific document. And notice the freshness indicator — it tells you whether the source is fresh, needs review, or is stale. You never get an answer without knowing how trustworthy the sources are."

### Scene 3.2 — Follow-Up Question (Working Memory)

**Actions:**
1. Without starting a new conversation, type: `What about rolling back if something goes wrong?`

**Script:**
> "Now watch this — I'm asking a follow-up question. I said 'rolling back' and 'something goes wrong'. DocBrain understands this is about the deployment we just discussed. That's working memory — multi-turn conversation context. It resolves 'it', 'that service', 'the same thing' automatically."

### Scene 3.3 — Feedback Loop

**Actions:**
1. Click the thumbs-up on a good answer
2. Ask a question that DocBrain can't answer well (e.g., "What is our disaster recovery plan for cross-region failover?")
3. Click thumbs-down

**Script:**
> "Feedback is how DocBrain learns. Thumbs up means the answer was helpful — DocBrain caches it for future similar questions. Thumbs down is even more powerful — it feeds the Autopilot gap detection pipeline. When enough people downvote answers about the same topic, DocBrain identifies it as a documentation gap and can automatically draft the missing doc. More on that later."

### Scene 3.4 — Intent Classification

**Actions:**
1. Ask: `Who is on-call for the payments service?` (who-owns intent)
2. Ask: `What is the difference between canary and rolling deployments?` (explain intent)

**Script:**
> "DocBrain classifies the intent of every question — is it a 'find' query, a 'how-to', a 'troubleshoot', a 'who-owns', or an 'explain'? Each gets a different answer structure. 'Who owns' queries route through the knowledge graph. 'How-to' queries generate step-by-step procedures. This isn't a generic chatbot — it understands what kind of answer you need."

---

## PART 4: SHIFT-LEFT CAPTURE (5-6 minutes)

### Scene 4.1 — Knowledge Fragments

**What to show:** Web UI → Captures page

**Script:**
> "This is the Captures page — the heart of shift-left documentation. These are knowledge fragments: small units of knowledge captured from PRs, Slack conversations, CI pipelines, and IDE sessions."

**Actions:**
1. Show the fragment list
2. Click on a fragment to expand it
3. Point out the metadata: source type (pr_merge), confidence score, space, provenance link

> "Every fragment has provenance — you can see exactly where it came from. This one was extracted from PR #1234 when a developer switched from Redis pub/sub to PostgreSQL LISTEN/NOTIFY. The confidence score is 0.92 — high enough to be auto-indexed into search immediately."

### Scene 4.2 — CI/CD Capture (API Demo)

**What to show:** Terminal + Web UI

**Actions:**
1. In the terminal, run a CI capture API call:
```bash
curl -X POST http://localhost:3000/api/v1/ci/analyze \
  -H "Authorization: Bearer $DOCBRAIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "pr_number": 2900,
    "repo": "acme/platform",
    "pr_title": "Add circuit breaker to inventory service",
    "pr_body": "Adds Hystrix-style circuit breaker to inventory service calls. Opens after 5 consecutive failures, half-opens after 30s. This prevents cascade failures when inventory service is down.",
    "diff_stat": "+180 -20",
    "changed_files": "src/inventory/client.rs",
    "labels": "reliability",
    "author": "priya@acme.com"
  }'
```
2. Show the response — fragments created
3. Switch to the Web UI and refresh the Captures page to show the new fragment

**Script:**
> "Here's shift-left in action. When a PR is merged, your CI pipeline sends the PR metadata to DocBrain. The LLM extracts decisions, caveats, and procedures — and they become searchable immediately. This engineer added a circuit breaker and DocBrain captured that as a knowledge fragment. Next time someone asks 'do we have circuit breakers?', this shows up."

### Scene 4.3 — Fragment Review Queue

**What to show:** Web UI → Captures → Review Queue tab

**Script:**
> "Not every fragment is auto-indexed. Fragments with confidence between 0.4 and 0.7 land in the review queue for a human to approve or discard. This is the quality gate — DocBrain doesn't pollute your knowledge base with noise."

**Actions:**
1. Show fragments in the review queue
2. Approve one fragment
3. Discard another with a reason

### Scene 4.4 — Fragment Clusters

**What to show:** Web UI → Captures → Clusters tab

**Script:**
> "This is one of the most powerful features. DocBrain uses DBSCAN clustering on fragment embeddings to group related fragments by topic. See this cluster? Five fragments from three different sources — a PR, a Slack thread, and a CI deploy — all about Redis configuration. When a cluster reaches critical mass, DocBrain can auto-compose them into a full document."

---

## PART 5: DOCUMENTATION AUTOPILOT (4-5 minutes)

### Scene 5.1 — Gap Detection

**What to show:** Web UI → Autopilot page

**Script:**
> "The Autopilot page shows documentation gaps — topics where users are asking questions but getting poor answers. DocBrain clusters these unanswered questions by semantic similarity and scores them by severity."

**Actions:**
1. Show the gap list with severity badges (critical, high, medium)
2. Click on a gap to see details: sample queries, unique user count, negative ratio
3. Point out the trend indicator (new vs recurring)

> "This critical gap has been asked about by 12 different users with a 68% negative feedback ratio. That means most people who asked about this topic were unhappy with the answer. This isn't a guess — it's data-driven gap detection."

### Scene 5.2 — Draft Generation

**Actions:**
1. Click "Generate Draft" on a gap
2. Show the draft being generated with quality score
3. Open the draft to show full content with citations

**Script:**
> "Watch this. I click 'Generate Draft' and DocBrain writes the missing documentation. But this isn't hallucination — every claim in the draft is grounded in existing documents and captured fragments. The quality score is 87 out of 100. It goes into the review workflow before anyone sees it."

### Scene 5.3 — Forecast

**What to show:** Web UI → Autopilot → Forecast section

**Script:**
> "The gap forecast shows a 30-day projection. Based on the rate of new gaps appearing and the rate of resolution, DocBrain predicts whether your documentation health is improving, stable, or worsening. This team is trending 'worsening' — more gaps are opening than closing."

---

## PART 6: QUALITY PIPELINE (4-5 minutes)

### Scene 6.1 — Quality Scores

**What to show:** Web UI → Quality page

**Script:**
> "Every document is scored on a 0-100 scale across three layers. Structural: does it have proper headings, code examples, links? Style: does it follow your team's writing rules? Semantic: is the content accurate, complete, and actionable?"

**Actions:**
1. Show the quality score list sorted by score
2. Click on a document to see the breakdown (heading structure, section completeness, etc.)
3. Point out a low-scoring document

### Scene 6.2 — Style Rules

**What to show:** Web UI → Quality → Style Rules

**Actions:**
1. Show the imported style rules
2. Point out different rule types: terminology, formatting, structure, custom pattern

**Script:**
> "These are custom style rules. Terminology rules ban words like 'simple' and 'obviously' — they discourage questions. Formatting rules cap sentence length at 40 words. Custom pattern rules catch internal URLs that shouldn't be in public docs. Every rule is YAML-exportable and version-controllable."

### Scene 6.3 — Live Linting

**Actions:**
1. Go to the Quality → Lint section (or use API)
2. Paste some text that violates rules:
```
This is a simple guide. Just follow these easy steps obviously.
Visit https://internal.acme.com for more info.
```
3. Show violations with line numbers and suggestions

**Script:**
> "The lint endpoint is CI-ready. You can block PRs that violate your style guide. Here I'm linting some text and DocBrain flags 'simple', 'just', 'easy', 'obviously', and the internal URL. Each violation has a line number, severity, and fix suggestion."

---

## PART 7: GOVERNANCE & ACCOUNTABILITY (3-4 minutes)

### Scene 7.1 — Governance Dashboard

**What to show:** Web UI → Governance page

**Script:**
> "Governance answers the question: who is responsible for this documentation? The dashboard shows ownership coverage — what percentage of your knowledge spaces have assigned owners — SLA compliance, quality distribution, and capture velocity."

**Actions:**
1. Show the governance dashboard with all sections
2. Point out unowned spaces
3. Show SLA breach summary

### Scene 7.2 — SLA Policies

**What to show:** Web UI → Governance → SLAs

**Script:**
> "SLA policies are configurable per space. Default: gaps must be acknowledged within 48 hours and resolved within 14 days. Draft reviews must happen within 72 hours. When these deadlines are missed, DocBrain fires breach events and notifications."

### Scene 7.3 — Review Workflows

**What to show:** Web UI → Governance → Workflows

**Script:**
> "Review workflows define how documentation gets approved. This space has a two-stage pipeline: SME Review by a maintainer, then Writer Review by a contributor. Each stage requires a specific number of approvals. Reviewers can approve, request changes, or reject — with threaded comments for inline feedback."

---

## PART 8: CONNECTORS — PLUG IN ANY SOURCE (3-4 minutes)

### Scene 8.1 — The Connector Protocol

**What to show:** Terminal + Web UI → Settings → Connectors

**Script:**
> "DocBrain has 13+ built-in sources — Confluence, Slack, GitHub, Jira, PagerDuty, and more. But what if you have an internal wiki or a proprietary knowledge base? That's what the Connector SDK is for."

**Actions:**
1. Show the connector code briefly (examples/connector/server.js)
2. Explain the three endpoints: `/health`, `/documents/list`, `/documents/fetch`

> "A connector is a stateless HTTP server that implements three endpoints. You serve documents, DocBrain handles scheduling, retries, circuit breaking, and ingestion. Let's test it."

### Scene 8.2 — Test the Connector

**Actions:**
1. In the Web UI, go to Settings → Connectors
2. Show the registered wiki connector
3. Click "Test" to verify health
4. Click "Sync" to trigger a manual sync
5. Show the sync results

**Script:**
> "I registered our internal wiki connector. DocBrain pings its health endpoint — healthy. Now I trigger a sync. DocBrain calls /documents/list to discover available documents, then /documents/fetch to get the content. Five documents synced, all flowing through the same quality pipeline as every other source."

### Scene 8.3 — Ask About Connector Data

**Actions:**
1. Go to Ask page
2. Ask: "What are the team OKRs for Q1 2026?"
3. Show the answer citing the wiki connector document

**Script:**
> "Now I can ask questions about the data from our custom connector. 'What are the Q1 OKRs?' — and DocBrain answers with data from the internal wiki. The connector SDK means you can plug in literally anything: Notion, Google Docs, SharePoint, an internal CMS, or even a database. Three endpoints, any language."

---

## PART 9: KNOWLEDGE GRAPH & ANALYTICS (3-4 minutes)

### Scene 9.1 — Knowledge Graph

**What to show:** Web UI → Graph page

**Actions:**
1. Search for an entity (e.g., "payments-service")
2. Show the graph visualization with connections
3. Explore dependencies and blast radius

**Script:**
> "The knowledge graph maps relationships between documents, services, people, and teams. Here's the payments service — I can see its dependencies, who owns it, which docs reference it, and most importantly: the blast radius. If the payments service changes, which documentation needs updating? DocBrain tells you before it becomes stale."

### Scene 9.2 — Expert Finder

**Actions:**
1. Search for experts on a topic (e.g., "kubernetes")

**Script:**
> "Need to find who knows about Kubernetes? The expert finder routes through the entity-to-team-to-person chain. It's not just keyword matching — it's graph traversal based on authorship, review activity, and document ownership."

### Scene 9.3 — Velocity Dashboard

**What to show:** Web UI → Velocity page

**Script:**
> "The velocity dashboard is your documentation ROI tracker. It shows: queries deflected, hours saved, cost saved in dollars, documentation velocity per team, and a grade from A to F. This makes documentation investment visible to leadership — it's not 'we feel like docs are getting better', it's 'we saved 103 hours and $7,700 last month.'"

---

## PART 10: PREDICTIVE INTELLIGENCE (2-3 minutes)

### Scene 10.1 — Cascade Staleness

**What to show:** Web UI → Predictive page

**Script:**
> "Predictive intelligence answers: what's about to break? Cascade staleness detects when one document was updated but the documents that reference it weren't. If the deployment guide changed but the onboarding guide still references the old process, DocBrain flags it."

### Scene 10.2 — Code Change Analysis

**Actions:**
1. Show the Predictive → Code Change section
2. Submit a simulated code change

**Script:**
> "Submit a PR diff and DocBrain tells you which documentation needs updating. Wire this into CI and you can block merges when docs are impacted. Shift-left documentation means you don't wait for docs to become stale — you catch it at the PR."

---

## PART 11: EVENTS & WEBHOOKS (2-3 minutes)

### Scene 11.1 — Real-Time Event Stream

**What to show:** Web UI → Events page

**Script:**
> "Every significant action in DocBrain emits an event: document ingested, fragment captured, gap detected, SLA breached. The Events page shows the real-time stream. You can filter by event type and time range."

### Scene 11.2 — Webhook Subscriptions

**Actions:**
1. Show webhook subscriptions
2. Point out the event types subscribed
3. Show the delivery log

**Script:**
> "Webhooks push events to external systems — Slack bots, CI/CD pipelines, PagerDuty, custom dashboards. Every delivery is HMAC-signed for security and retried with exponential backoff. Circuit breakers auto-disable broken subscriptions."

---

## PART 12: SETTINGS & INTEGRATIONS (2-3 minutes)

### Scene 12.1 — LLM Provider Agnostic

**What to show:** Terminal or .env file

**Script:**
> "DocBrain supports 14 LLM providers. Switching from Anthropic to OpenAI to a fully local Ollama setup is a single environment variable change. No code changes, no redeploy of application logic."

### Scene 12.2 — MCP IDE Integration

**What to show:** Show the MCP config file (examples/mcp-configs/cursor.json)

**Script:**
> "For IDE users: DocBrain has 10 MCP tools for Cursor, Claude Code, and any MCP-compatible editor. Ask questions, annotate code with knowledge, capture decisions at commit time — without leaving your IDE."

---

## PART 13: THE COMPLETE LOOP (2-3 minutes)

### Scene 13.1 — Closing Summary

**Script:**
> "Let me tie it all together. This is the knowledge lifecycle DocBrain automates:
>
> 1. **Users ask questions** — about deployments, debugging, architecture
> 2. **Gaps are detected** — from negative feedback and unanswered queries
> 3. **Drafts are generated** — AI composes missing docs from existing knowledge
> 4. **Admins review** — multi-stage approval ensures quality
> 5. **Published and re-ingested** — next user gets the answer, retrieval improves
>
> Meanwhile, knowledge is continuously captured from PRs, Slack, CI, and IDE sessions. Fragments cluster, compose into docs, get quality-scored, and published. The system gets smarter with every interaction.
>
> That's shift-left documentation. Knowledge captured where the work happens. Documentation that writes itself. Quality enforced automatically. Accountability built in.
>
> DocBrain. Stop writing docs after the fact. Capture knowledge where it happens."

---

## Recording Tips

1. **Pace:** Go slowly. Pause after each feature to let it sink in.
2. **Mouse:** Move the cursor deliberately. Highlight what you're pointing to.
3. **Resolution:** Record at 1920x1080 minimum. Use browser zoom if text is small.
4. **Terminal:** Use a large font (16pt+) and a clean theme.
5. **Web UI:** Close unnecessary browser tabs. Use full-screen mode.
6. **Pauses:** After each major section, pause for 2-3 seconds before moving on.
7. **Errors:** If something fails, explain why and move on. Real demos have real problems.

## Estimated Total Runtime

| Part | Duration |
|------|----------|
| Part 1: The Problem | 2-3 min |
| Part 2: Setup & First Impressions | 3-4 min |
| Part 3: Ask — Intelligent Q&A | 4-5 min |
| Part 4: Shift-Left Capture | 5-6 min |
| Part 5: Documentation Autopilot | 4-5 min |
| Part 6: Quality Pipeline | 4-5 min |
| Part 7: Governance & Accountability | 3-4 min |
| Part 8: Connectors | 3-4 min |
| Part 9: Knowledge Graph & Analytics | 3-4 min |
| Part 10: Predictive Intelligence | 2-3 min |
| Part 11: Events & Webhooks | 2-3 min |
| Part 12: Settings & Integrations | 2-3 min |
| Part 13: The Complete Loop | 2-3 min |
| **Total** | **~40-52 min** |
