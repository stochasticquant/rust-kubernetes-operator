# 0001 — Licence: Apache-2.0

Status: accepted, 2026-10-24.

## Context

The repository had no licence, and its README described it as "for educational purposes". Without a
licence nobody may legally use, change or redistribute the code, so companies cannot adopt it and
contributors cannot safely contribute.

## Decision

License the project under the **Apache License 2.0**, with a `NOTICE` file and an SPDX header
(`// SPDX-License-Identifier: Apache-2.0`) in every source file. Contributions are accepted under the
same licence, certified with a DCO sign-off (`git commit -s`).

## Why

- It is the norm in the Kubernetes and CNCF ecosystem (Kubernetes, kube-rs, Kyverno, Gatekeeper), which
  CNCF projects are expected to use.
- It includes an explicit patent grant and patent-retaliation clause, which legal teams at adopting
  companies look for.
- It is permissive: companies can run and extend it without having to publish their changes.

## Alternatives considered

- **MIT**: simpler, but no patent grant. Common in Rust, less so for Kubernetes infrastructure.
- **Dual MIT/Apache-2.0** (the Rust convention): adds little for an application rather than a library,
  and complicates the CNCF story.
- **AGPL/GPL**: would discourage enterprise adoption, which is the goal of the project.

## Consequences

- Dependencies must have licences compatible with Apache-2.0 distribution. `deny.toml` holds the allow
  list and CI enforces it.
- `NOTICE` must be kept and carried in distributions.
