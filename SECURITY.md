# Security Policy

Ferrogate is an admission controller: it sits in the path of every change to a Kubernetes cluster. We
take reports about its security seriously and thank everyone who reports issues responsibly.

How Ferrogate is designed to be secure is described in
[`docs/SECURITY_ARCHITECTURE.md`](docs/SECURITY_ARCHITECTURE.md) and the
[threat model](docs/security/threat-model.md).

## Reporting a vulnerability

**Do not open a public issue, discussion or pull request for a security problem.**

Report it privately through GitHub:

1. Go to the [Security tab](https://github.com/stochasticquant/rust-kubernetes-operator/security).
2. Choose **Report a vulnerability**.
3. Describe the issue, the affected version, and how to reproduce it. A proof of concept helps.

Only the maintainers can see the report.

## What to expect

| Step | Target |
| --- | --- |
| Acknowledge your report | within 3 business days |
| Confirm the issue and rate its severity (CVSS v3.1) | within 7 days |
| Release a fix for a critical or high issue | within 30 days |
| Publish a GitHub Security Advisory and request a CVE | with the fix |

We keep you informed throughout, agree a disclosure date with you, and credit you in the advisory
unless you prefer otherwise.

## Supported versions

Ferrogate has not had a stable release yet. Until 1.0, security fixes go into the latest release only.
From 1.0 onward, fixes are backported to the two most recent minor releases.

| Version | Supported |
| --- | --- |
| `main` and latest `0.x` release | ✅ |
| older `0.x` releases | ❌ |

## Scope

In scope: the Ferrogate binaries, container images, Helm chart and manifests in this repository, and
their default configuration.

Out of scope: vulnerabilities in Kubernetes itself or in third-party dependencies (report those
upstream; we will update quickly once a fix exists), and issues that require an attacker who already
has cluster-admin rights.

## Verifying releases

From the first release, images, charts and binaries are signed with cosign and ship with an SBOM and
SLSA provenance. Verification steps will be published in the release notes.
