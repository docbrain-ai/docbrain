# Data Privacy & GDPR Compliance

## Overview

Acme Platform processes personal data for users across the EU, US, and APAC regions. All services must comply with GDPR, CCPA, and our internal data governance policies. This guide covers classification, handling, retention, and deletion requirements.

## Data Classification

| Level | Definition | Examples | Encryption |
|-------|-----------|----------|------------|
| **Public** | No restrictions | Marketing content, public docs | Optional |
| **Internal** | Company only | Team OKRs, architecture docs | At rest |
| **Confidential** | Need-to-know | Customer data, financials | At rest + in transit |
| **Restricted** | Regulated | Payment cards, health data | At rest + in transit + field-level |

## PII Fields in Our System

| Service | PII Fields | Retention | Legal Basis |
|---------|-----------|-----------|-------------|
| Users | email, name, phone | Account lifetime + 30 days | Contract |
| Payments | card_last4, billing_address | 7 years (tax) | Legal obligation |
| Analytics | IP address, user_agent | 90 days | Legitimate interest |
| Support | conversation transcripts | 2 years | Contract |

## Right to Deletion (GDPR Art. 17)

When a user requests deletion:

1. Customer Success receives the request and verifies identity
2. They file a Jira ticket in the PRIVACY project
3. The automated deletion pipeline runs within 72 hours:
   ```bash
   acme gdpr delete-user --user-id $USER_ID --dry-run  # preview
   acme gdpr delete-user --user-id $USER_ID             # execute
   ```
4. Data is removed from:
   - Users service (profile, preferences)
   - Analytics (query logs anonymized)
   - Support (transcripts deleted)
   - Payments (retained for tax compliance, but PII stripped)
5. Confirmation sent to the user within 30 days (GDPR requirement)

## Data Retention Automation

Retention policies are enforced by a daily cron job:

```sql
-- Analytics: delete records older than 90 days
DELETE FROM analytics_events
WHERE created_at < NOW() - INTERVAL '90 days';

-- Support: anonymize transcripts older than 2 years
UPDATE support_conversations
SET user_email = 'anonymized@deleted.local',
    transcript = '[redacted]'
WHERE created_at < NOW() - INTERVAL '2 years';
```

## Cross-Border Data Transfer

- EU data stays in eu-west-1 (Ireland)
- US data in us-east-1 (Virginia)
- APAC data in ap-southeast-1 (Singapore)
- Cross-region replication excludes PII fields

## Security Contacts

- Data Protection Officer: privacy@acme-platform.com
- Privacy engineering: #privacy-eng on Slack
- Incident reporting: Within 72 hours per GDPR Art. 33
