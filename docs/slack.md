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

## Step 3b: Add Message Shortcut

This allows users to capture Slack threads via a right-click shortcut — useful because Slack blocks slash commands inside threads.

1. In the left sidebar, click **Interactivity & Shortcuts** → **Shortcuts** → **Create New Shortcut**.
2. Select **On messages**.
3. Fill in:

| Field | Value |
|-------|-------|
| Name | `Capture to DocBrain` |
| Callback ID | `docbrain_capture` |
| Short Description | `Index this thread into DocBrain` |

4. Click **Save**.

## Step 3c: Enable Event Subscriptions (optional)

This enables `@DocBrain capture` mentions as an alternative way to trigger thread capture, and unlocks **auto-listen in threads** — un-mentioned follow-ups inside a thread the bot already replied in are treated as continued questions, so users don't have to re-mention the bot every turn.

1. In the left sidebar, click **Event Subscriptions** → toggle **On**.
2. Set the **Request URL** to:

```
https://<your-domain>/slack/events
```

3. Under **Subscribe to bot events**, add:

| Event | Why |
|-------|-----|
| `app_mention` | Explicit `@DocBrain` mentions |
| `message.channels` | Auto-listen follow-ups in public-channel threads |
| `message.groups` | Auto-listen follow-ups in private channels |
| `message.im` | Auto-listen follow-ups in DMs |
| `message.mpim` | Auto-listen follow-ups in group DMs |

   The `message.*` events are only needed if you want auto-listen; `app_mention` alone is enough for explicit-mention-only behavior.
4. Go to **OAuth & Permissions** → **Bot Token Scopes** and add `app_mentions:read` and the `*:history` scopes (see Step 4) if not already present.
5. Click **Save Changes**.

## Step 4: Add OAuth Scopes

1. In the left sidebar, click **OAuth & Permissions**.
2. Under **Bot Token Scopes**, add:

| Scope | Why |
|-------|-----|
| `commands` | Handle `/docbrain` slash commands |
| `chat:write` | Post answers and notifications |
| `users:read.email` | Look up doc authors by email for stale-doc DMs |
| `app_mentions:read` | Handle `@DocBrain capture` mentions in threads |
| `channels:history` | Read thread messages for auto-listen / capture in public channels |
| `groups:history` | Read thread messages for auto-listen / capture in private channels |
| `im:history` | Read thread messages for auto-listen / capture in DMs |
| `mpim:history` | Read thread messages for auto-listen / capture in group DMs |

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

### Capture a thread

Slack blocks slash commands in threads, so `/docbrain capture` only works as a top-level command. Use one of these alternatives to capture a thread:

**Message shortcut (recommended):** Right-click any message in a thread → **Shortcuts** → **Capture to DocBrain**

**Bot mention:** Type `@DocBrain capture` in a thread (requires Event Subscriptions — see Step 3c)

**Slash command (top-level only):** `/docbrain capture` works outside of threads but is blocked by Slack inside threads.

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

### Conversation memory & auto-listen

DocBrain remembers prior turns in a conversation, so follow-up questions resolve in context instead of starting from scratch:

- **Threaded mentions** share history within the thread — everyone in the thread builds on the same conversation.
- **Top-level mentions** share history per `(channel, user)` — your follow-ups continue your own conversation.

**Auto-listen.** Once the `message.*` events are subscribed (Step 3c) and the bot is a member of the channel, you no longer need to re-mention it on every turn — DocBrain answers un-mentioned follow-ups in any thread it already participated in. Top-level (non-thread) messages still require an explicit `@mention` to start a conversation.

**Safety.** The bot ignores its own and other bots' messages (no reply loops) and only responds in threads it actually joined. It must be a member of the channel to receive messages at all.

This conversation-memory behavior is consistent across surfaces — the same follow-up-in-context experience applies on the web UI and the CLI.

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
