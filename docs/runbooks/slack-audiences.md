# Slack and code-host audiences

Where DocBrain answers, and what an operator must configure for it.

## What changed for operators

- **Bot scope `users:read` is required.** Member or guest is decided by `users.info`. A bot token without the scope, a failed lookup or a timeout makes the asker a guest: they receive the one static notice and nothing else, and the server logs a warning per failed lookup. Re-install the Slack app with `users:read` (the same scope `SLACK_INGEST_TOKEN` already lists in `docs/configuration.md`) before upgrading. Enterprise Grid: a full member of any workspace in the bot's Grid org is a member (`slack/audience.rs` `same_org`); anyone from another org, and any restricted or ultra-restricted account, is a guest. If the bot's own `auth.test` fails, every asker is treated as a guest (the notice, nothing else) on all four entry points: mention, thread follow-up, capture and slash commands.
- **Capture allowlists take Slack user ids.** `SLACK_CAPTURE_ALLOWED_USERS=alice` (a name) never matches the mention path, which has no username; mention captures from that user are denied. Write `U01234567`. The slash command and message shortcut still accept either form.
- **GitLab loop guard.** Set `GITLAB_BOT_USER_ID` (numeric id of the account behind `GITLAB_CAPTURE_TOKEN`) so DocBrain's own notes never trigger an ask reply. See `docs/gitlab-capture.md`.
- **PR/MR link needs `DOCBRAIN_WEB_BASE_URL`.** Unset, the reply is "Ask this in DocBrain." with no link and the server warns.

## Known behaviours (not bugs)

- A webhook delivery that GitHub or GitLab replays is processed twice and posts twice (pre-existing; no delivery-id dedupe).
- Any workspace member who replies in a thread within 30 minutes after a pointer starts their own private ask (a later release limits this to the person the pointer named).
- A member who captures a message authored by a guest indexes it under the member's capture audience (a later release scopes it to the original audience).
- Legacy `event_log.payload` and `webhook_deliveries.payload` rows keep their old bytes and are projected on read; a later release rewrites them.
