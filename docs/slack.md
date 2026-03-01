# Slack Integration

Connect DocBrain to Slack so your team can query documentation, triage incidents, and get proactive notifications — all without leaving their workspace.

## What You Get

| Command | What it does |
|---------|-------------|
| `/docbrain ask <question>` | Ask any documentation question — get a sourced, confidence-rated answer |
| `/docbrain incident <description>` | Incident mode — prioritizes runbooks and past incident docs |
| `/docbrain freshness [space]` | Freshness report for a Confluence space (or all spaces) |
| `/docbrain notify [space]` | DM stale doc owners to review their content |
| `/docbrain onboard <space>` | AI-curated reading list for new team members |
| `/docbrain help` | Show available commands |

Answers include feedback buttons (Helpful / Not helpful) that feed back into DocBrain's learning loop — the same way the web UI and API do.

## Prerequisites

- A running DocBrain server accessible via a **public HTTPS URL** (e.g. `https://docbrain.yourcompany.com`). Slack needs to reach your server over the internet.
- Admin access to your Slack workspace.

## Step 1: Create the Slack App

1. Go to [https://api.slack.com/apps](https://api.slack.com/apps) and click **Create New App** → **From scratch**.
2. Name it `DocBrain` (or whatever your team prefers).
3. Select your workspace.

## Step 2: Configure the Slash Command

1. In the left sidebar, click **Slash Commands** → **Create New Command**.
2. Fill in:

| Field | Value |
|-------|-------|
| Command | `/docbrain` |
| Request URL | `https://<your-domain>/slack/commands` |
| Short Description | Query your documentation |
| Usage Hint | `ask <question>` or `incident <description>` or `help` |

3. Click **Save**.

## Step 3: Enable Interactivity

This is required for the feedback buttons (Helpful / Not helpful) on answers.

1. In the left sidebar, click **Interactivity & Shortcuts** → toggle **On**.
2. Set the **Request URL** to:

```
https://<your-domain>/slack/interactions
```

3. Click **Save Changes**.

## Step 4: Add OAuth Scopes

1. In the left sidebar, click **OAuth & Permissions**.
2. Under **Bot Token Scopes**, add:

| Scope | Why |
|-------|-----|
| `commands` | Handle `/docbrain` slash commands |
| `chat:write` | Post answers and notifications |
| `users:read.email` | Look up doc authors by email for stale-doc DMs |

3. Click **Install to Workspace** (or **Reinstall** if already installed).
4. Copy the **Bot User OAuth Token** (`xoxb-...`) — you'll need this in the next step.

## Step 5: Get the Signing Secret

1. Go to **Basic Information** in the left sidebar.
2. Under **App Credentials**, copy the **Signing Secret**.

## Step 6: Configure DocBrain

Add these environment variables to your `.env` or deployment config:

```bash
# Required — without SLACK_SIGNING_SECRET, Slack routes are not mounted
SLACK_SIGNING_SECRET=your-signing-secret-here
SLACK_BOT_TOKEN=xoxb-your-bot-token-here

# Optional — channel for critical gap alerts (e.g. #docs-alerts)
SLACK_GAP_NOTIFICATION_CHANNEL=#docs-alerts
```

Restart DocBrain. On startup you should see:

```
[startup] Slack integration enabled
```

If `SLACK_SIGNING_SECRET` is not set, the Slack routes will not be registered and you'll see no Slack-related startup message.

## Step 7: Verify

In any Slack channel, type:

```
/docbrain help
```

You should see the list of available commands. Then try a real query:

```
/docbrain ask how do we deploy to production?
```

You'll see a "Searching documentation..." acknowledgement, followed by the full answer with sources and feedback buttons.

## Usage Examples

### Ask a question

```
/docbrain ask what's the runbook for Redis OOM errors?
```

Returns a sourced answer with confidence rating, freshness indicators on each source, and thumbs up/down buttons.

### Incident triage at 2am

```
/docbrain incident payments service returning 502 after deploy
```

Prioritizes runbooks and rollback procedures. The response is posted to the channel (visible to everyone) so the whole on-call team can see it.

### Check documentation freshness

```
/docbrain freshness PLATFORM
```

Returns a summary: how many docs are fresh, stale, or outdated in the PLATFORM space.

### Nudge doc owners

```
/docbrain notify SRE
```

Sends a Slack DM to every author in the SRE space whose docs are stale (freshness score < 40). The DM includes the doc title, how old it is, and how many times it was cited recently.

### Onboard a new hire

```
/docbrain onboard ENG
```

Generates an AI-curated reading list of the most important docs in the ENG space, sorted by relevance and freshness — takes about 30 seconds.

## Proactive Notifications

When configured, DocBrain sends automatic notifications without anyone running a command:

| Notification | Trigger | Where |
|-------------|---------|-------|
| Stale doc DMs | Background scheduler (every `NOTIFICATION_INTERVAL_HOURS`, default 24h) | DM to doc authors |
| Critical gap alerts | After each autopilot gap analysis run | `SLACK_GAP_NOTIFICATION_CHANNEL` |
| Draft ready | When autopilot generates a new draft | `SLACK_GAP_NOTIFICATION_CHANNEL` |

To restrict notifications to specific spaces:

```bash
NOTIFICATION_SPACE_FILTER=PLATFORM,SRE
```

## Configuration Reference

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SLACK_SIGNING_SECRET` | Yes | — | Signing secret from **Basic Information** → **App Credentials** |
| `SLACK_BOT_TOKEN` | Yes | — | Bot token (`xoxb-...`) from **OAuth & Permissions** |
| `SLACK_GAP_NOTIFICATION_CHANNEL` | No | — | Channel for gap/draft alerts (e.g. `#docs-alerts`) |
| `NOTIFICATION_INTERVAL_HOURS` | No | `24` | How often the stale-doc notification scheduler runs |
| `NOTIFICATION_SPACE_FILTER` | No | — | Comma-separated spaces to limit notifications |

## How Feedback Works

When DocBrain posts an answer, it includes two buttons:

- **Helpful** — records positive feedback on that answer's episode
- **Not helpful** — records negative feedback

This feedback feeds into:
- **Procedural memory** — adjusts retrieval rule success rates so future searches are better
- **Autopilot** — negative feedback contributes to gap detection and clustering
- **Consolidation** — patterns from feedback influence entity extraction and rule creation

The buttons are replaced with an acknowledgement after you click, so each person can only vote once.

## Troubleshooting

**"Dispatch failed" or command not working**
- Verify your Request URL is reachable from the internet: `curl -I https://<your-domain>/slack/commands`
- Check that `SLACK_SIGNING_SECRET` is set — without it, the routes aren't mounted

**"Slack bot token not configured"**
- Set `SLACK_BOT_TOKEN` in your environment. The signing secret enables the routes, but the bot token is needed to post messages and send DMs.

**Feedback buttons not appearing**
- Buttons only appear when the answer has an `episode_id` (i.e., the question was stored as an episode). Greetings, unclear queries, and cache hits don't produce episodes, so no buttons are shown.

**Notifications not sending**
- Stale-doc DMs require `SLACK_BOT_TOKEN` and the `users:read.email` scope. The author's email in Confluence must match their Slack email.
- Gap alerts require `SLACK_GAP_NOTIFICATION_CHANNEL` to be set and `AUTOPILOT_ENABLED=true`.

**Slow responses**
- `/docbrain ask` and `/docbrain incident` return an immediate acknowledgement and post the full answer asynchronously (within a few seconds depending on your LLM provider).
- `/docbrain onboard` can take up to 30 seconds.
- `/docbrain freshness` returns synchronously and should be under 3 seconds.
