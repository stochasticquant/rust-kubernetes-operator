# 0004 — Project name: Ferrogate

Status: accepted, 2026-10-24.

## Context

The project was called **kube-guardian**. A search in October 2026 found that the name was already used
by an established open-source Kubernetes security project (xentra-ai/kube-guardian, now kguardian),
plus several smaller "KubeGuardian" admission controllers. Sharing a name in the same space confuses
users and search results, and could force a rename later.

## Decision

Rename the project to **Ferrogate**: *ferro* (iron, a nod to Rust) and *gate* (it decides what is
admitted to the cluster).

- Binary: `ferrogate`. Crates: `ferrogate-*`. API group: `ferrogate.stochasticquant.io`.
- The GitHub repository can be renamed to `ferrogate`; GitHub redirects the old URL.

## Checks done

- No crate named `ferrogate` on crates.io.
- No Kubernetes project with 10+ stars on GitHub using the name.

Before 1.0, also check trademarks and register the name on the container registry, Artifact Hub and
crates.io (placeholder crates) to prevent squatting.

## Alternatives considered

Vigilo, Admitra, Kubegate (free); Palisade, Bulwark, Rampart, Oxigate (taken as crates).
