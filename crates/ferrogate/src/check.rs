// SPDX-License-Identifier: Apache-2.0

//! `ferrogate check`: evaluate policies against local YAML files, without a cluster.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use ferrogate_core::{Decision, Policy, Resource, decide};
use ferrogate_crd::{ClusterGatePolicy, GROUP};
use serde_json::Value;

/// Largest YAML file `check` reads, to avoid surprises with huge inputs.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// One YAML document and the file it came from.
#[derive(Clone, Debug)]
pub struct Document {
    pub source: PathBuf,
    pub value: Value,
}

impl Document {
    pub fn api_version(&self) -> &str {
        self.value
            .get("apiVersion")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn kind(&self) -> &str {
        self.value.get("kind").and_then(Value::as_str).unwrap_or("")
    }

    pub fn name(&self) -> &str {
        self.value
            .pointer("/metadata/name")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn namespace(&self) -> Option<&str> {
        self.value
            .pointer("/metadata/namespace")
            .and_then(Value::as_str)
    }

    /// API group from `apiVersion` (`apps/v1` → `apps`, `v1` → core group `""`).
    pub fn group(&self) -> &str {
        self.api_version()
            .rsplit_once('/')
            .map_or("", |(group, _)| group)
    }

    pub fn is_policy(&self) -> bool {
        self.group() == GROUP && self.kind() == "ClusterGatePolicy"
    }

    /// `Kind namespace/name`, for messages.
    pub fn display(&self) -> String {
        match self.namespace() {
            Some(ns) => format!("{} {ns}/{}", self.kind(), self.name()),
            None => format!("{} {}", self.kind(), self.name()),
        }
    }

    pub fn evaluate(&self, policies: &[Policy]) -> Decision {
        let resource = Resource {
            group: self.group(),
            kind: self.kind(),
            namespace: self.namespace(),
            namespace_labels: None,
            requester: None,
            object: &self.value,
        };
        decide(policies, &resource)
    }
}

/// Reads every YAML document from `paths`. Directories are searched recursively for `.yaml` and
/// `.yml` files; files named explicitly are read whatever their extension.
pub fn load(paths: &[PathBuf]) -> anyhow::Result<Vec<Document>> {
    let mut files = Vec::new();
    for path in paths {
        let meta = std::fs::metadata(path).with_context(|| format!("read {}", path.display()))?;
        if meta.is_dir() {
            let mut found = Vec::new();
            collect_yaml(path, &mut found)?;
            found.sort();
            files.extend(found);
        } else {
            files.push(path.clone());
        }
    }
    let mut documents = Vec::new();
    for file in files {
        let size = std::fs::metadata(&file)
            .with_context(|| format!("read {}", file.display()))?
            .len();
        if size > MAX_FILE_BYTES {
            bail!("{} is larger than {MAX_FILE_BYTES} bytes", file.display());
        }
        let text =
            std::fs::read_to_string(&file).with_context(|| format!("read {}", file.display()))?;
        let values: Vec<Value> = serde_saphyr::from_multiple(&text)
            .with_context(|| format!("parse YAML in {}", file.display()))?;
        documents.extend(
            values
                .into_iter()
                .filter(Value::is_object)
                .map(|value| Document {
                    source: file.clone(),
                    value,
                }),
        );
    }
    Ok(documents)
}

fn collect_yaml(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("list {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_yaml(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Compiles every `ClusterGatePolicy` in `documents`. Invalid policies are an error.
pub fn policies(documents: &[Document]) -> anyhow::Result<Vec<Policy>> {
    documents
        .iter()
        .filter(|doc| doc.is_policy())
        .map(|doc| {
            let policy: ClusterGatePolicy = serde_json::from_value(doc.value.clone())
                .with_context(|| format!("{}: invalid {}", doc.source.display(), doc.display()))?;
            policy
                .compile()
                .with_context(|| format!("{}: invalid {}", doc.source.display(), doc.display()))
        })
        .collect()
}

/// Result of checking one resource.
pub struct Checked<'a> {
    pub document: &'a Document,
    pub decision: Decision,
}

impl Checked<'_> {
    pub fn failed(&self) -> bool {
        self.decision.failures().next().is_some()
    }
}

/// Evaluates every non-policy document against `policies`.
pub fn run<'a>(policies: &[Policy], documents: &'a [Document]) -> Vec<Checked<'a>> {
    documents
        .iter()
        .filter(|doc| !doc.is_policy())
        .map(|document| Checked {
            document,
            decision: document.evaluate(policies),
        })
        .collect()
}

/// Human-readable report.
pub fn text_report(results: &[Checked<'_>], policy_count: usize) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for checked in results {
        let doc = checked.document;
        let status = if checked.failed() { "FAIL" } else { "PASS" };
        let _ = writeln!(
            out,
            "{status}  {} ({})",
            doc.display(),
            doc.source.display()
        );
        for result in checked.decision.failures() {
            for v in &result.violations {
                let path = v
                    .path
                    .as_deref()
                    .map(|p| format!(" ({p})"))
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "      {}/{} [{} {}] {}{path}",
                    result.policy,
                    v.rule,
                    result.mode.as_str(),
                    result.severity.as_str(),
                    v.message
                );
            }
        }
    }
    let failed = results.iter().filter(|c| c.failed()).count();
    let _ = writeln!(
        out,
        "\n{} resource(s) checked against {policy_count} polic{}: {failed} failed, {} passed",
        results.len(),
        if policy_count == 1 { "y" } else { "ies" },
        results.len() - failed
    );
    out
}

/// Machine-readable report.
pub fn json_report(results: &[Checked<'_>]) -> Value {
    let resources: Vec<Value> = results
        .iter()
        .map(|checked| {
            let doc = checked.document;
            let failures: Vec<Value> = checked
                .decision
                .failures()
                .map(|r| {
                    serde_json::json!({
                        "policy": r.policy,
                        "mode": r.mode,
                        "severity": r.severity,
                        "violations": r.violations,
                    })
                })
                .collect();
            serde_json::json!({
                "source": doc.source.display().to_string(),
                "apiVersion": doc.api_version(),
                "kind": doc.kind(),
                "namespace": doc.namespace(),
                "name": doc.name(),
                "passed": !checked.failed(),
                "failures": failures,
            })
        })
        .collect();
    serde_json::json!({ "resources": resources })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn groups_from_api_version() {
        let doc = |api: &str| Document {
            source: PathBuf::new(),
            value: json!({"apiVersion": api}),
        };
        assert_eq!(doc("v1").group(), "");
        assert_eq!(doc("apps/v1").group(), "apps");
        assert_eq!(
            doc("ferrogate.stochasticquant.io/v1alpha1").group(),
            "ferrogate.stochasticquant.io"
        );
    }
}
