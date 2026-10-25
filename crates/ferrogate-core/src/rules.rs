// SPDX-License-Identifier: Apache-2.0

//! Rule definitions (as written in a policy) and their checks.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Violation;
use crate::error::CompileError;
use crate::workload::{Container, ImageRef, PodSpec, pod_spec};

/// Maximum list length inside a rule; keeps evaluation cost bounded.
const MAX_LIST: usize = 128;

/// One rule of a policy, as written by the user. Exactly one check field must be set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
// The API server enforces "exactly one check" too, so mistakes are rejected at `kubectl apply`.
#[schemars(extend("x-kubernetes-validations" = [{
    "rule": "[has(self.requireLabels), has(self.requireAnnotations), has(self.allowedRegistries), has(self.disallowLatestTag), has(self.requireResourceLimits), has(self.requireResourceRequests), has(self.disallowPrivileged), has(self.disallowHostNamespaces), has(self.disallowHostPath), has(self.disallowHostPorts), has(self.allowedCapabilities), has(self.disallowPrivilegeEscalation), has(self.requireRunAsNonRoot), has(self.requireDropAllCapabilities), has(self.requireSeccompProfile)].filter(x, x).size() == 1",
    "message": "exactly one check must be set per rule"
}]))]
pub struct RuleSpec {
    /// Unique name within the policy, used in reports and messages.
    #[schemars(length(min = 1, max = 63))]
    pub name: String,

    /// Optional message shown instead of the built-in one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 1024))]
    pub message: Option<String>,

    /// The resource must carry every listed label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    pub require_labels: Option<Vec<String>>,

    /// The resource must carry every listed annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    pub require_annotations: Option<Vec<String>>,

    /// Every container image must come from one of these registries or registry paths,
    /// for example `ghcr.io/acme` or `registry.k8s.io`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    pub allowed_registries: Option<Vec<String>>,

    /// Images must not use the `latest` tag, or no tag at all. Digests are always allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disallow_latest_tag: Option<bool>,

    /// Every container must set these resource limits, for example `["cpu", "memory"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 16))]
    pub require_resource_limits: Option<Vec<String>>,

    /// Every container must set these resource requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 16))]
    pub require_resource_requests: Option<Vec<String>>,

    /// Containers must not be privileged. (Pod Security Standards: baseline)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disallow_privileged: Option<bool>,

    /// Pods must not use the host network, PID or IPC namespaces. (baseline)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disallow_host_namespaces: Option<bool>,

    /// Pods must not mount `hostPath` volumes. (baseline)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disallow_host_path: Option<bool>,

    /// Containers must not use host ports. (baseline)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disallow_host_ports: Option<bool>,

    /// Containers may only add these Linux capabilities. An empty list allows none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub allowed_capabilities: Option<Vec<String>>,

    /// Containers must set `allowPrivilegeEscalation: false`. (restricted)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disallow_privilege_escalation: Option<bool>,

    /// Containers must run as non-root (`runAsNonRoot: true`, pod or container level). (restricted)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_run_as_non_root: Option<bool>,

    /// Containers must drop `ALL` capabilities. (restricted)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_drop_all_capabilities: Option<bool>,

    /// A seccomp profile of type `RuntimeDefault` or `Localhost` must be set. (restricted)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_seccomp_profile: Option<bool>,
}

/// The check a compiled rule performs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    RequireLabels(Vec<String>),
    RequireAnnotations(Vec<String>),
    AllowedRegistries(Vec<String>),
    DisallowLatestTag,
    RequireResourceLimits(Vec<String>),
    RequireResourceRequests(Vec<String>),
    DisallowPrivileged,
    DisallowHostNamespaces,
    DisallowHostPath,
    DisallowHostPorts,
    AllowedCapabilities(Vec<String>),
    DisallowPrivilegeEscalation,
    RequireRunAsNonRoot,
    RequireDropAllCapabilities,
    RequireSeccompProfile,
}

/// A validated rule, ready to evaluate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub message: Option<String>,
    pub check: Check,
}

impl Rule {
    /// Validates a rule definition. Exactly one check must be set; boolean checks must be `true`.
    pub fn compile(spec: &RuleSpec) -> Result<Self, CompileError> {
        let name = spec.name.trim();
        if name.is_empty() {
            return Err(CompileError::EmptyRuleName);
        }
        let err = |reason: &str| CompileError::InvalidRule {
            rule: name.to_owned(),
            reason: reason.to_owned(),
        };

        let mut checks = requested_checks(spec).map_err(|reason| err(&reason))?;

        match checks.len() {
            0 => Err(err("no check is set")),
            1 => Ok(Self {
                name: name.to_owned(),
                message: spec.message.clone().filter(|m| !m.trim().is_empty()),
                check: checks.remove(0),
            }),
            _ => Err(err("more than one check is set; use one rule per check")),
        }
    }

    /// Returns every way `object` (of the given `kind`) fails this rule.
    ///
    /// Pod-level checks do not apply to objects without a pod spec, so those pass.
    pub fn check(&self, kind: &str, object: &Value) -> Vec<Violation> {
        let found = match &self.check {
            Check::RequireLabels(keys) => missing_keys(object, "labels", keys),
            Check::RequireAnnotations(keys) => missing_keys(object, "annotations", keys),
            check => match pod_spec(kind, object) {
                Some(pod) => check_pod(check, &pod),
                None => Vec::new(),
            },
        };
        found
            .into_iter()
            .map(|(message, path)| Violation {
                rule: self.name.clone(),
                message: self.message.clone().unwrap_or(message),
                path,
            })
            .collect()
    }
}

type ListCheck = fn(Vec<String>) -> Check;

/// Every check set in `spec`, validated. Errors are reasons for `CompileError::InvalidRule`.
// Long only because each check is one table row.
#[allow(clippy::too_many_lines)]
fn requested_checks(spec: &RuleSpec) -> Result<Vec<Check>, String> {
    let lists: [(&str, &Option<Vec<String>>, bool, ListCheck); 6] = [
        (
            "requireLabels",
            &spec.require_labels,
            false,
            Check::RequireLabels,
        ),
        (
            "requireAnnotations",
            &spec.require_annotations,
            false,
            Check::RequireAnnotations,
        ),
        (
            "allowedRegistries",
            &spec.allowed_registries,
            false,
            Check::AllowedRegistries,
        ),
        (
            "requireResourceLimits",
            &spec.require_resource_limits,
            false,
            Check::RequireResourceLimits,
        ),
        (
            "requireResourceRequests",
            &spec.require_resource_requests,
            false,
            Check::RequireResourceRequests,
        ),
        (
            "allowedCapabilities",
            &spec.allowed_capabilities,
            true,
            |caps| {
                Check::AllowedCapabilities(caps.iter().map(|c| normalize_capability(c)).collect())
            },
        ),
    ];
    let flags = [
        (
            "disallowLatestTag",
            spec.disallow_latest_tag,
            Check::DisallowLatestTag,
        ),
        (
            "disallowPrivileged",
            spec.disallow_privileged,
            Check::DisallowPrivileged,
        ),
        (
            "disallowHostNamespaces",
            spec.disallow_host_namespaces,
            Check::DisallowHostNamespaces,
        ),
        (
            "disallowHostPath",
            spec.disallow_host_path,
            Check::DisallowHostPath,
        ),
        (
            "disallowHostPorts",
            spec.disallow_host_ports,
            Check::DisallowHostPorts,
        ),
        (
            "disallowPrivilegeEscalation",
            spec.disallow_privilege_escalation,
            Check::DisallowPrivilegeEscalation,
        ),
        (
            "requireRunAsNonRoot",
            spec.require_run_as_non_root,
            Check::RequireRunAsNonRoot,
        ),
        (
            "requireDropAllCapabilities",
            spec.require_drop_all_capabilities,
            Check::RequireDropAllCapabilities,
        ),
        (
            "requireSeccompProfile",
            spec.require_seccomp_profile,
            Check::RequireSeccompProfile,
        ),
    ];

    let mut checks = Vec::new();
    for (field, values, allow_empty, make) in lists {
        let Some(values) = values else { continue };
        if values.len() > MAX_LIST {
            return Err(format!("{field} has more than {MAX_LIST} entries"));
        }
        if !allow_empty && values.is_empty() {
            return Err(format!("{field} must not be empty"));
        }
        if values.iter().any(|v| v.trim().is_empty()) {
            return Err(format!("{field} contains an empty entry"));
        }
        checks.push(make(values.clone()));
    }
    for (field, value, check) in flags {
        match value {
            Some(true) => checks.push(check),
            Some(false) => {
                return Err(format!(
                    "{field} must be true (remove the rule to disable it)"
                ));
            }
            None => {}
        }
    }
    Ok(checks)
}

type Found = Vec<(String, Option<String>)>;

fn missing_keys(object: &Value, field: &str, keys: &[String]) -> Found {
    let present = object
        .get("metadata")
        .and_then(|m| m.get(field))
        .and_then(Value::as_object);
    let missing: Vec<&str> = keys
        .iter()
        .filter(|key| present.is_none_or(|map| !map.contains_key(key.as_str())))
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        Vec::new()
    } else {
        let noun = if field == "labels" {
            "label"
        } else {
            "annotation"
        };
        vec![(
            format!("missing required {noun}(s): {}", missing.join(", ")),
            Some(format!("metadata.{field}")),
        )]
    }
}

fn is_true(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool) == Some(true)
}

fn check_pod(check: &Check, pod: &PodSpec<'_>) -> Found {
    let containers = pod.containers();
    let mut found = Found::new();
    match check {
        Check::DisallowHostNamespaces => {
            for field in ["hostNetwork", "hostPID", "hostIPC"] {
                if is_true(pod.field(field)) {
                    found.push((
                        format!("{field} must not be true"),
                        Some(format!("{}.{field}", pod.path)),
                    ));
                }
            }
        }
        Check::DisallowHostPath => {
            let volumes = pod.field("volumes").and_then(Value::as_array);
            for (index, volume) in volumes.into_iter().flatten().enumerate() {
                if volume.get("hostPath").is_some() {
                    let name = volume.get("name").and_then(Value::as_str).unwrap_or("");
                    found.push((
                        format!("hostPath volume \"{name}\" is not allowed"),
                        Some(format!("{}.volumes[{index}].hostPath", pod.path)),
                    ));
                }
            }
        }
        Check::RequireRunAsNonRoot => {
            let pod_level = is_true(pod.security("runAsNonRoot"));
            for c in &containers {
                let effective = match c.security("runAsNonRoot").and_then(Value::as_bool) {
                    Some(value) => value,
                    None => pod_level,
                };
                if !effective {
                    found.push((
                        format!("container \"{}\" must set runAsNonRoot: true", c.name),
                        Some(format!("{}.securityContext.runAsNonRoot", c.path)),
                    ));
                }
            }
        }
        Check::RequireSeccompProfile => {
            let pod_level = seccomp_ok(pod.security("seccompProfile"));
            for c in &containers {
                let effective = match c.security("seccompProfile") {
                    Some(profile) => seccomp_ok(Some(profile)),
                    None => pod_level,
                };
                if !effective {
                    found.push((
                        format!(
                            "container \"{}\" must use seccomp profile RuntimeDefault or Localhost",
                            c.name
                        ),
                        Some(format!("{}.securityContext.seccompProfile", c.path)),
                    ));
                }
            }
        }
        container_check => {
            for c in &containers {
                found.extend(check_container(container_check, c));
            }
        }
    }
    found
}

fn seccomp_ok(profile: Option<&Value>) -> bool {
    matches!(
        profile.and_then(|p| p.get("type")).and_then(Value::as_str),
        Some("RuntimeDefault" | "Localhost")
    )
}

fn normalize_capability(cap: &str) -> String {
    let upper = cap.trim().to_ascii_uppercase();
    upper
        .strip_prefix("CAP_")
        .map_or(upper.clone(), str::to_owned)
}

/// A violation at a field of container `c`.
fn at(c: &Container<'_>, message: String, field: &str) -> Found {
    vec![(message, Some(format!("{}.{field}", c.path)))]
}

fn check_container(check: &Check, c: &Container<'_>) -> Found {
    let name = c.name;
    match check {
        Check::AllowedRegistries(_) | Check::DisallowLatestTag => check_image(check, c),
        Check::RequireResourceLimits(names) => check_resources(c, "limits", names),
        Check::RequireResourceRequests(names) => check_resources(c, "requests", names),
        Check::AllowedCapabilities(_) | Check::RequireDropAllCapabilities => {
            check_capabilities(check, c)
        }
        Check::DisallowPrivileged if is_true(c.security("privileged")) => at(
            c,
            format!("container \"{name}\" must not be privileged"),
            "securityContext.privileged",
        ),
        Check::DisallowPrivilegeEscalation
            if c.security("allowPrivilegeEscalation")
                .and_then(Value::as_bool)
                != Some(false) =>
        {
            at(
                c,
                format!("container \"{name}\" must set allowPrivilegeEscalation: false"),
                "securityContext.allowPrivilegeEscalation",
            )
        }
        Check::DisallowHostPorts => {
            let ports = c.value.get("ports").and_then(Value::as_array);
            ports
                .into_iter()
                .flatten()
                .enumerate()
                .filter(|(_, port)| {
                    port.get("hostPort")
                        .and_then(Value::as_i64)
                        .is_some_and(|p| p != 0)
                })
                .map(|(index, _)| {
                    (
                        format!("container \"{name}\" must not use a host port"),
                        Some(format!("{}.ports[{index}].hostPort", c.path)),
                    )
                })
                .collect()
        }
        // Passing container checks, and pod-level and metadata checks handled by the callers.
        _ => Vec::new(),
    }
}

fn check_image(check: &Check, c: &Container<'_>) -> Found {
    let image = ImageRef::parse(c.image());
    let failed = match check {
        Check::AllowedRegistries(prefixes) => !prefixes.iter().any(|p| image.is_from(p)),
        _ => image.is_latest(),
    };
    if !failed {
        return Vec::new();
    }
    let problem = if matches!(check, Check::AllowedRegistries(_)) {
        "is not from an allowed registry"
    } else {
        "must use a fixed tag or digest, not latest"
    };
    at(
        c,
        format!(
            "image \"{}\" of container \"{}\" {problem}",
            c.image(),
            c.name
        ),
        "image",
    )
}

fn check_resources(c: &Container<'_>, kind: &str, names: &[String]) -> Found {
    if c.ephemeral {
        return Vec::new();
    }
    let set = c
        .value
        .get("resources")
        .and_then(|r| r.get(kind))
        .and_then(Value::as_object);
    let missing: Vec<&str> = names
        .iter()
        .filter(|n| set.is_none_or(|s| !s.contains_key(n.as_str())))
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        return Vec::new();
    }
    at(
        c,
        format!(
            "container \"{}\" must set resource {kind}: {}",
            c.name,
            missing.join(", ")
        ),
        &format!("resources.{kind}"),
    )
}

fn check_capabilities(check: &Check, c: &Container<'_>) -> Found {
    let list = |field: &str| -> Vec<String> {
        c.security("capabilities")
            .and_then(|caps| caps.get(field))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(normalize_capability)
            .collect()
    };
    match check {
        Check::AllowedCapabilities(allowed) => {
            let forbidden: Vec<String> = list("add")
                .into_iter()
                .filter(|cap| !allowed.contains(cap))
                .collect();
            if forbidden.is_empty() {
                return Vec::new();
            }
            at(
                c,
                format!(
                    "container \"{}\" adds capabilities that are not allowed: {}",
                    c.name,
                    forbidden.join(", ")
                ),
                "securityContext.capabilities.add",
            )
        }
        _ if list("drop").iter().any(|cap| cap == "ALL") => Vec::new(),
        _ => at(
            c,
            format!("container \"{}\" must drop ALL capabilities", c.name),
            "securityContext.capabilities.drop",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(spec: serde_json::Value) -> Rule {
        Rule::compile(&serde_json::from_value(spec).unwrap()).unwrap()
    }

    fn none() -> Vec<Violation> {
        Vec::new()
    }

    #[allow(clippy::needless_pass_by_value)]
    fn pod(container: serde_json::Value) -> serde_json::Value {
        json!({"metadata": {"name": "p"}, "spec": {"containers": [container]}})
    }

    #[test]
    fn compile_requires_exactly_one_check() {
        let none: RuleSpec = serde_json::from_value(json!({"name": "x"})).unwrap();
        assert!(Rule::compile(&none).is_err());

        let two: RuleSpec = serde_json::from_value(
            json!({"name": "x", "disallowPrivileged": true, "disallowHostPath": true}),
        )
        .unwrap();
        assert!(
            Rule::compile(&two)
                .unwrap_err()
                .to_string()
                .contains("more than one")
        );

        let off: RuleSpec =
            serde_json::from_value(json!({"name": "x", "disallowPrivileged": false})).unwrap();
        assert!(
            Rule::compile(&off)
                .unwrap_err()
                .to_string()
                .contains("true")
        );

        let empty: RuleSpec =
            serde_json::from_value(json!({"name": "x", "requireLabels": []})).unwrap();
        assert!(Rule::compile(&empty).is_err());

        let blank: RuleSpec =
            serde_json::from_value(json!({"name": " ", "disallowPrivileged": true})).unwrap();
        assert!(Rule::compile(&blank).is_err());
    }

    #[test]
    fn require_labels_reports_missing_keys() {
        let r = rule(json!({"name": "team", "requireLabels": ["team", "env"]}));
        let ok = json!({"metadata": {"labels": {"team": "a", "env": "b"}}});
        assert_eq!(r.check("ConfigMap", &ok), none());

        let bad = json!({"metadata": {"labels": {"team": "a"}}});
        let v = r.check("ConfigMap", &bad);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].message, "missing required label(s): env");
        assert_eq!(v[0].path.as_deref(), Some("metadata.labels"));
    }

    #[test]
    fn custom_message_replaces_default() {
        let r =
            rule(json!({"name": "team", "message": "add a team label", "requireLabels": ["team"]}));
        assert_eq!(r.check("Pod", &json!({}))[0].message, "add a team label");
    }

    #[test]
    fn pod_checks_skip_objects_without_pod_spec() {
        let r = rule(json!({"name": "p", "disallowPrivileged": true}));
        assert_eq!(r.check("ConfigMap", &json!({"data": {"a": "b"}})), none());
    }

    #[test]
    fn allowed_registries() {
        let r =
            rule(json!({"name": "reg", "allowedRegistries": ["ghcr.io/acme", "registry.k8s.io"]}));
        assert_eq!(
            r.check(
                "Pod",
                &pod(json!({"name": "a", "image": "ghcr.io/acme/app:1"}))
            ),
            none()
        );
        let v = r.check("Pod", &pod(json!({"name": "a", "image": "nginx:1"})));
        assert_eq!(v[0].path.as_deref(), Some("spec.containers[0].image"));
        assert!(v[0].message.contains("nginx:1"));
    }

    #[test]
    fn latest_tag() {
        let r = rule(json!({"name": "tag", "disallowLatestTag": true}));
        assert_eq!(
            r.check("Pod", &pod(json!({"name": "a", "image": "nginx"})))
                .len(),
            1
        );
        assert_eq!(
            r.check("Pod", &pod(json!({"name": "a", "image": "nginx:1.27"}))),
            none()
        );
    }

    #[test]
    fn resource_limits_skip_ephemeral_containers() {
        let r = rule(json!({"name": "lim", "requireResourceLimits": ["cpu", "memory"]}));
        let object = json!({"spec": {
            "containers": [{"name": "a", "resources": {"limits": {"cpu": "1"}}}],
            "ephemeralContainers": [{"name": "debug"}]
        }});
        let v = r.check("Pod", &object);
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "container \"a\" must set resource limits: memory"
        );
    }

    #[test]
    fn privileged_and_escalation() {
        let privileged = rule(json!({"name": "p", "disallowPrivileged": true}));
        assert_eq!(
            privileged
                .check(
                    "Pod",
                    &pod(json!({"name": "a", "securityContext": {"privileged": true}}))
                )
                .len(),
            1
        );
        assert_eq!(privileged.check("Pod", &pod(json!({"name": "a"}))), none());

        let escalation = rule(json!({"name": "e", "disallowPrivilegeEscalation": true}));
        assert_eq!(escalation.check("Pod", &pod(json!({"name": "a"}))).len(), 1);
        assert_eq!(
            escalation.check(
                "Pod",
                &pod(json!({"name": "a", "securityContext": {"allowPrivilegeEscalation": false}}))
            ),
            none()
        );
    }

    #[test]
    fn host_namespaces_paths_and_ports() {
        let ns = rule(json!({"name": "h", "disallowHostNamespaces": true}));
        let object = json!({"spec": {"hostNetwork": true, "hostPID": true, "containers": []}});
        assert_eq!(ns.check("Pod", &object).len(), 2);

        let path = rule(json!({"name": "hp", "disallowHostPath": true}));
        let object = json!({"spec": {"containers": [], "volumes": [
            {"name": "ok", "emptyDir": {}}, {"name": "root", "hostPath": {"path": "/"}}
        ]}});
        let v = path.check("Pod", &object);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path.as_deref(), Some("spec.volumes[1].hostPath"));

        let ports = rule(json!({"name": "port", "disallowHostPorts": true}));
        let object = pod(
            json!({"name": "a", "ports": [{"containerPort": 80}, {"containerPort": 81, "hostPort": 81}]}),
        );
        let v = ports.check("Pod", &object);
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].path.as_deref(),
            Some("spec.containers[0].ports[1].hostPort")
        );
    }

    #[test]
    fn capabilities() {
        let allowed = rule(json!({"name": "caps", "allowedCapabilities": ["NET_BIND_SERVICE"]}));
        let object = pod(
            json!({"name": "a", "securityContext": {"capabilities": {"add": ["CAP_NET_BIND_SERVICE", "SYS_ADMIN"]}}}),
        );
        let v = allowed.check("Pod", &object);
        assert_eq!(v.len(), 1);
        assert!(v[0].message.ends_with("SYS_ADMIN"));

        let no_caps = rule(json!({"name": "caps", "allowedCapabilities": []}));
        assert_eq!(no_caps.check("Pod", &object).len(), 1);

        let drop = rule(json!({"name": "drop", "requireDropAllCapabilities": true}));
        assert_eq!(drop.check("Pod", &object).len(), 1);
        let dropped =
            pod(json!({"name": "a", "securityContext": {"capabilities": {"drop": ["all"]}}}));
        assert_eq!(drop.check("Pod", &dropped), none());
    }

    #[test]
    fn run_as_non_root_inherits_from_pod_but_container_wins() {
        let r = rule(json!({"name": "nonroot", "requireRunAsNonRoot": true}));
        let inherited = json!({"spec": {"securityContext": {"runAsNonRoot": true}, "containers": [{"name": "a"}]}});
        assert_eq!(r.check("Pod", &inherited), none());

        let overridden = json!({"spec": {"securityContext": {"runAsNonRoot": true},
            "containers": [{"name": "a", "securityContext": {"runAsNonRoot": false}}]}});
        assert_eq!(r.check("Pod", &overridden).len(), 1);

        assert_eq!(r.check("Pod", &pod(json!({"name": "a"}))).len(), 1);
    }

    #[test]
    fn seccomp_profile() {
        let r = rule(json!({"name": "seccomp", "requireSeccompProfile": true}));
        let ok = json!({"spec": {"securityContext": {"seccompProfile": {"type": "RuntimeDefault"}}, "containers": [{"name": "a"}]}});
        assert_eq!(r.check("Pod", &ok), none());
        let unconfined = json!({"spec": {"securityContext": {"seccompProfile": {"type": "RuntimeDefault"}},
            "containers": [{"name": "a", "securityContext": {"seccompProfile": {"type": "Unconfined"}}}]}});
        assert_eq!(r.check("Pod", &unconfined).len(), 1);
    }

    #[test]
    fn workload_paths_point_into_template() {
        let r = rule(json!({"name": "tag", "disallowLatestTag": true}));
        let deploy = json!({"spec": {"template": {"spec": {"containers": [{"name": "a", "image": "nginx"}]}}}});
        let v = r.check("Deployment", &deploy);
        assert_eq!(
            v[0].path.as_deref(),
            Some("spec.template.spec.containers[0].image")
        );
    }

    #[test]
    fn malformed_objects_do_not_panic() {
        let rules = [
            rule(json!({"name": "a", "requireLabels": ["x"]})),
            rule(json!({"name": "b", "disallowLatestTag": true})),
            rule(json!({"name": "c", "requireResourceLimits": ["cpu"]})),
            rule(json!({"name": "d", "disallowHostPath": true})),
            rule(json!({"name": "e", "requireSeccompProfile": true})),
            rule(json!({"name": "f", "allowedCapabilities": []})),
            rule(json!({"name": "g", "disallowHostPorts": true})),
        ];
        let objects = [
            json!(null),
            json!(42),
            json!({"metadata": "oops"}),
            json!({"spec": {"containers": "nope"}}),
            json!({"spec": {"containers": [null, 1, {"image": 5, "securityContext": "x", "ports": [{"hostPort": "1"}]}], "volumes": "x"}}),
        ];
        for r in &rules {
            for o in &objects {
                let _ = r.check("Pod", o);
            }
        }
    }
}
