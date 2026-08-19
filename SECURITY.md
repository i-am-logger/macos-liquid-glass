# Security Policy

<!-- TEMPLATE: replace this paragraph with the security-relevant surface of the
     project instantiated from this template — where `unsafe` lives, what
     untrusted input it parses, what it talks to over a network. A generic
     "report vulnerabilities" page tells a reporter nothing about what is
     actually worth reporting. -->

This is a Rust CLI project. The security-relevant categories are:

- **Untrusted input handling** — any parser or deserializer reachable from
  user-supplied files or arguments.
- **Dependency vulnerabilities** — standard `RUSTSEC` advisories in the
  dependency tree.

## Reporting a Vulnerability

Please report security issues privately rather than opening a public issue —
[GitHub Security Advisories](https://github.com/i-am-logger/my-project/security/advisories/new)
if private vulnerability reporting is enabled on the repo, otherwise contact
the maintainer directly via GitHub ([@i-am-logger](https://github.com/i-am-logger)).

Include the affected version or commit, a minimal reproduction, and the impact
as you see it. There is no bug bounty — this is a solo-maintained open-source
project — but reports are taken seriously and credited unless you ask
otherwise.

## Supported Versions

Only the latest published release is supported. There is no LTS branch.
