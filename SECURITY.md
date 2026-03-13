# Security Policy

## Reporting a Vulnerability

The DocBrain team takes security vulnerabilities seriously. We appreciate your efforts to responsibly disclose your findings.

**Do not file a public GitHub issue for security vulnerabilities.**

### How to Report

Email **[security@docbrain.ai](mailto:security@docbrain.ai)** with:

- A description of the vulnerability
- Steps to reproduce the issue
- Potential impact assessment
- Any suggested mitigations (if known)

### What to Expect

- **Acknowledgment** within 48 hours of your report
- **Initial assessment** within 5 business days
- **Regular updates** on our progress toward a fix
- **Credit** in the security advisory (unless you prefer to remain anonymous)

### Scope

The following are in scope for security reports:

- The DocBrain server application
- The ingestion pipeline
- Authentication and authorization mechanisms
- The MCP server
- The Slack bot integration
- The Helm chart and Docker Compose configuration
- The CLI tool

### Out of Scope

- Vulnerabilities in third-party dependencies (report these to the upstream project)
- Issues requiring physical access to the host machine
- Social engineering attacks
- Denial of service attacks against publicly accessible instances you do not own

## Security Best Practices

For operators deploying DocBrain, see the [Threat Model](THREAT_MODEL.md) for a comprehensive security analysis including assets, trust boundaries, mitigations, and an operator checklist.

## Supported Versions

Security updates are provided for the latest release only. We recommend always running the most recent version.

| Version | Supported |
|---------|-----------|
| Latest  | Yes       |
| Older   | No        |
