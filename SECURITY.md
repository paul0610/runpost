# Security policy

RunPost is a local tool: it binds only to `127.0.0.1`, uses a random token per launch for its local
API and MCP endpoint, and keeps secrets masked for agents and, optionally, in the OS keyring.

## Reporting a vulnerability

Please report vulnerabilities **privately** through GitHub's
[private vulnerability reporting](https://docs.github.com/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
("Report a vulnerability" in the repository's Security tab). Do not open a public issue and do not
include real credentials in the report. You will get an answer within 7 days.

## Supported versions

Only the latest release receives fixes.
