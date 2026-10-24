// SPDX-License-Identifier: Apache-2.0

//! Compiles a policy to CEL for a Kubernetes `ValidatingAdmissionPolicy`.
//!
//! The generated expressions mirror the engine's checks so that native enforcement, webhook
//! enforcement and background scans reach the same verdict. Rules or kinds that cannot be expressed
//! faithfully are rejected with a reason instead of being approximated.

use crate::rules::Check;
use crate::selector::{LabelSelector, SelectorOperator};
use crate::{Policy, Rule};

/// Kinds whose pod spec location is known: `spec`.
const POD_KINDS: &[&str] = &["Pod"];
/// Kinds with a pod template at `spec.template.spec`.
const TEMPLATE_KINDS: &[&str] = &[
    "Deployment",
    "StatefulSet",
    "DaemonSet",
    "ReplicaSet",
    "Job",
    "ReplicationController",
];
/// Kinds with a pod template at `spec.jobTemplate.spec.template.spec`.
const CRONJOB_KINDS: &[&str] = &["CronJob"];

/// One CEL validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeValidation {
    pub rule: String,
    pub expression: String,
    pub message: String,
}

/// One CEL match condition (all must be true for the policy to apply).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeCondition {
    pub name: String,
    pub expression: String,
}

/// A policy compiled to CEL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePolicy {
    /// Applied as `matchConstraints.namespaceSelector`.
    pub namespace_selector: Option<LabelSelector>,
    pub match_conditions: Vec<NativeCondition>,
    pub validations: Vec<NativeValidation>,
}

/// A CEL string literal.
fn lit(value: &str) -> String {
    // JSON string escapes (\" \\ \n \uXXXX) are valid CEL string escapes.
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// A CEL list of string literals.
fn list(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|v| lit(v)).collect();
    format!("[{}]", items.join(", "))
}

/// CEL that is true when the object's labels match `selector` (Kubernetes semantics).
fn selector_cel(selector: &LabelSelector) -> String {
    const LABELS: &str = "(has(object.metadata.labels) ? object.metadata.labels : {})";
    let mut terms: Vec<String> = selector
        .match_labels
        .iter()
        .map(|(k, v)| {
            format!(
                "({k} in {LABELS} && {LABELS}[{k}] == {v})",
                k = lit(k),
                v = lit(v)
            )
        })
        .collect();
    for expr in &selector.match_expressions {
        let key = lit(&expr.key);
        let values = list(&expr.values);
        terms.push(match expr.operator {
            SelectorOperator::In => format!("({key} in {LABELS} && {LABELS}[{key}] in {values})"),
            SelectorOperator::NotIn => {
                format!("(!({key} in {LABELS}) || !({LABELS}[{key}] in {values}))")
            }
            SelectorOperator::Exists => format!("({key} in {LABELS})"),
            SelectorOperator::DoesNotExist => format!("!({key} in {LABELS})"),
        });
    }
    if terms.is_empty() {
        "true".to_owned()
    } else {
        terms.join(" && ")
    }
}

fn is_pod_check(check: &Check) -> bool {
    !matches!(
        check,
        Check::RequireLabels(_) | Check::RequireAnnotations(_)
    )
}

/// `pred(c)` holds for every container in the pod spec at path `ps`.
fn all_containers(ps: &str, include_ephemeral: bool, pred: &dyn Fn(&str) -> String) -> String {
    let p = pred("c");
    let mut parts = vec![
        format!("(!has({ps}.containers) || {ps}.containers.all(c, {p}))"),
        format!("(!has({ps}.initContainers) || {ps}.initContainers.all(c, {p}))"),
    ];
    if include_ephemeral {
        parts.push(format!(
            "(!has({ps}.ephemeralContainers) || {ps}.ephemeralContainers.all(c, {p}))"
        ));
    }
    format!("({})", parts.join(" && "))
}

/// `field` of the container's securityContext, guarded: `has(c.securityContext) && has(...)`.
fn sc_has(c: &str, field: &str) -> String {
    format!("has({c}.securityContext) && has({c}.securityContext.{field})")
}

/// CEL normalizing a capability name like the engine does (`cap_sys_admin` → `SYS_ADMIN`).
fn normalized_cap(var: &str) -> String {
    format!(
        "({var}.trim().upperAscii().startsWith('CAP_') ? {var}.trim().upperAscii().substring(4) : {var}.trim().upperAscii())"
    )
}

/// The check for a pod spec at path `ps`.
fn pod_check(check: &Check, ps: &str) -> Result<String, String> {
    let expr = match check {
        Check::DisallowPrivileged => all_containers(ps, true, &|c| {
            format!(
                "!({sc} && {c}.securityContext.privileged)",
                sc = sc_has(c, "privileged")
            )
        }),
        Check::DisallowPrivilegeEscalation => all_containers(ps, true, &|c| {
            format!(
                "({sc} && {c}.securityContext.allowPrivilegeEscalation == false)",
                sc = sc_has(c, "allowPrivilegeEscalation")
            )
        }),
        Check::RequireDropAllCapabilities => all_containers(ps, true, &|c| {
            format!(
                "({sc} && has({c}.securityContext.capabilities.drop) && {c}.securityContext.capabilities.drop.exists(d, {n} == 'ALL'))",
                sc = sc_has(c, "capabilities"),
                n = normalized_cap("d")
            )
        }),
        Check::AllowedCapabilities(allowed) => all_containers(ps, true, &|c| {
            format!(
                "!({sc} && has({c}.securityContext.capabilities.add)) || {c}.securityContext.capabilities.add.all(a, {n} in {allowed})",
                sc = sc_has(c, "capabilities"),
                n = normalized_cap("a"),
                allowed = list(allowed)
            )
        }),
        Check::DisallowHostPorts => all_containers(ps, true, &|c| {
            format!("(!has({c}.ports) || {c}.ports.all(p, !has(p.hostPort) || p.hostPort == 0))")
        }),
        Check::DisallowLatestTag => all_containers(ps, true, &|c| {
            // Same as the engine: a digest always passes; otherwise a tag other than `latest`
            // must follow the last `/`.
            format!(
                "(has({c}.image) && ({c}.image.contains('@') || ({c}.image.lastIndexOf(':') > {c}.image.lastIndexOf('/') && !{c}.image.endsWith(':latest'))))"
            )
        }),
        Check::RequireResourceLimits(names) | Check::RequireResourceRequests(names) => {
            let kind = if matches!(check, Check::RequireResourceLimits(_)) {
                "limits"
            } else {
                "requests"
            };
            let names = list(names);
            // Ephemeral containers cannot set resources; the engine skips them too.
            all_containers(ps, false, &|c| {
                format!(
                    "(has({c}.resources) && has({c}.resources.{kind}) && {names}.all(n, n in {c}.resources.{kind}))"
                )
            })
        }
        Check::RequireRunAsNonRoot => {
            let pod = format!(
                "(has({ps}.securityContext) && has({ps}.securityContext.runAsNonRoot) && {ps}.securityContext.runAsNonRoot)"
            );
            all_containers(ps, true, &|c| {
                format!(
                    "(({sc}) ? {c}.securityContext.runAsNonRoot : {pod})",
                    sc = sc_has(c, "runAsNonRoot")
                )
            })
        }
        Check::RequireSeccompProfile => {
            let ok = "['RuntimeDefault', 'Localhost']";
            let pod = format!(
                "(has({ps}.securityContext) && has({ps}.securityContext.seccompProfile) && {ps}.securityContext.seccompProfile.type in {ok})"
            );
            all_containers(ps, true, &|c| {
                format!(
                    "(({sc}) ? {c}.securityContext.seccompProfile.type in {ok} : {pod})",
                    sc = sc_has(c, "seccompProfile")
                )
            })
        }
        Check::DisallowHostNamespaces => format!(
            "(!(has({ps}.hostNetwork) && {ps}.hostNetwork) && !(has({ps}.hostPID) && {ps}.hostPID) && !(has({ps}.hostIPC) && {ps}.hostIPC))"
        ),
        Check::DisallowHostPath => {
            format!("(!has({ps}.volumes) || {ps}.volumes.all(v, !has(v.hostPath)))")
        }
        Check::AllowedRegistries(_) => {
            return Err("allowedRegistries is not supported with enforcement Native yet".into());
        }
        Check::RequireLabels(_) | Check::RequireAnnotations(_) => {
            return Err("not a pod check".into());
        }
    };
    Ok(expr)
}

fn rule_expression(rule: &Rule, kinds: &[String]) -> Result<String, String> {
    match &rule.check {
        Check::RequireLabels(keys) | Check::RequireAnnotations(keys) => {
            let field = if matches!(rule.check, Check::RequireLabels(_)) {
                "labels"
            } else {
                "annotations"
            };
            Ok(format!(
                "has(object.metadata.{field}) && {keys}.all(k, k in object.metadata.{field})",
                keys = list(keys)
            ))
        }
        check => {
            let mut parts = Vec::new();
            let shapes: [(&[&str], &str); 3] = [
                (POD_KINDS, "object.spec"),
                (TEMPLATE_KINDS, "object.spec.template.spec"),
                (CRONJOB_KINDS, "object.spec.jobTemplate.spec.template.spec"),
            ];
            for (shape_kinds, path) in shapes {
                let matched: Vec<String> = kinds
                    .iter()
                    .filter(|k| shape_kinds.contains(&k.as_str()))
                    .cloned()
                    .collect();
                if matched.is_empty() {
                    continue;
                }
                parts.push(format!(
                    "(!(request.kind.kind in {}) || {})",
                    list(&matched),
                    pod_check(check, path)?
                ));
            }
            Ok(parts.join(" && "))
        }
    }
}

fn default_message(rule: &Rule) -> String {
    let text = match &rule.check {
        Check::RequireLabels(keys) => format!("required label(s): {}", keys.join(", ")),
        Check::RequireAnnotations(keys) => format!("required annotation(s): {}", keys.join(", ")),
        Check::AllowedRegistries(_) => "images must come from an allowed registry".into(),
        Check::DisallowLatestTag => "images must use a fixed tag or digest, not latest".into(),
        Check::RequireResourceLimits(n) => {
            format!("containers must set resource limits: {}", n.join(", "))
        }
        Check::RequireResourceRequests(n) => {
            format!("containers must set resource requests: {}", n.join(", "))
        }
        Check::DisallowPrivileged => "containers must not be privileged".into(),
        Check::DisallowHostNamespaces => "hostNetwork, hostPID and hostIPC must not be true".into(),
        Check::DisallowHostPath => "hostPath volumes are not allowed".into(),
        Check::DisallowHostPorts => "containers must not use host ports".into(),
        Check::AllowedCapabilities(a) => {
            format!("containers may only add capabilities: {}", a.join(", "))
        }
        Check::DisallowPrivilegeEscalation => {
            "containers must set allowPrivilegeEscalation: false".into()
        }
        Check::RequireRunAsNonRoot => "containers must set runAsNonRoot: true".into(),
        Check::RequireDropAllCapabilities => "containers must drop ALL capabilities".into(),
        Check::RequireSeccompProfile => {
            "a seccomp profile of type RuntimeDefault or Localhost is required".into()
        }
    };
    rule.message.clone().unwrap_or(text)
}

/// Compiles `policy` to CEL, or explains why it cannot be enforced natively.
pub fn compile_native(policy: &Policy) -> Result<NativePolicy, String> {
    let kinds: Vec<String> = policy
        .match_
        .resources
        .iter()
        .flat_map(|r| r.kinds.iter().cloned())
        .collect();
    let known = |k: &String| {
        [POD_KINDS, TEMPLATE_KINDS, CRONJOB_KINDS]
            .iter()
            .any(|shape| shape.contains(&k.as_str()))
    };
    if policy.rules.iter().any(|r| is_pod_check(&r.check))
        && let Some(kind) = kinds.iter().find(|k| !known(k))
    {
        return Err(format!(
            "pod checks with enforcement Native support built-in workload kinds only, not {kind:?}"
        ));
    }

    let mut conditions = Vec::new();
    let match_ = &policy.match_;
    if !match_.namespaces.is_empty() {
        conditions.push(NativeCondition {
            name: "match-namespaces".into(),
            expression: format!("request.namespace in {}", list(&match_.namespaces)),
        });
    }
    if match_.namespace_selector.is_some() {
        // The engine never matches cluster-scoped objects through a namespace selector, except
        // Namespaces themselves; the API server would.
        conditions.push(NativeCondition {
            name: "namespaced-only".into(),
            expression: "request.namespace != '' || (request.kind.group == '' && request.kind.kind == 'Namespace')".into(),
        });
    }
    if let Some(selector) = &match_.object_selector {
        conditions.push(NativeCondition {
            name: "match-object-labels".into(),
            expression: selector_cel(selector),
        });
    }
    let exclude = &policy.exclude;
    if !exclude.namespaces.is_empty() {
        conditions.push(NativeCondition {
            name: "exclude-namespaces".into(),
            expression: format!("!(request.namespace in {})", list(&exclude.namespaces)),
        });
    }
    if let Some(selector) = exclude.object_selector.as_ref().filter(|s| !s.is_empty()) {
        conditions.push(NativeCondition {
            name: "exclude-object-labels".into(),
            expression: format!("!({})", selector_cel(selector)),
        });
    }
    if !exclude.usernames.is_empty() {
        conditions.push(NativeCondition {
            name: "exclude-users".into(),
            expression: format!(
                "!(request.userInfo.username in {})",
                list(&exclude.usernames)
            ),
        });
    }
    if !exclude.groups.is_empty() {
        conditions.push(NativeCondition {
            name: "exclude-groups".into(),
            expression: format!(
                "!(has(request.userInfo.groups) && request.userInfo.groups.exists(g, g in {}))",
                list(&exclude.groups)
            ),
        });
    }

    let validations = policy
        .rules
        .iter()
        .map(|rule| {
            Ok(NativeValidation {
                rule: rule.name.clone(),
                expression: rule_expression(rule, &kinds)
                    .map_err(|e| format!("rule {:?}: {e}", rule.name))?,
                message: format!("[{}] {}: {}", policy.name, rule.name, default_message(rule)),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(NativePolicy {
        namespace_selector: match_.namespace_selector.clone(),
        match_conditions: conditions,
        validations,
    })
}

/// Prefix of generated object names: `ferrogate-<policy>`.
pub const NAME_PREFIX: &str = "ferrogate-";

/// A resource type for `matchConstraints.resourceRules`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceType {
    pub group: String,
    /// Plural resource name, for example `deployments`.
    pub resource: String,
}

/// Resource types of common built-in kinds, for callers without API discovery (the CLI).
pub fn builtin_resource(kind: &str) -> Option<ResourceType> {
    let (group, resource) = match kind {
        "Pod" => ("", "pods"),
        "ReplicationController" => ("", "replicationcontrollers"),
        "Service" => ("", "services"),
        "ConfigMap" => ("", "configmaps"),
        "Namespace" => ("", "namespaces"),
        "ServiceAccount" => ("", "serviceaccounts"),
        "PersistentVolumeClaim" => ("", "persistentvolumeclaims"),
        "Deployment" => ("apps", "deployments"),
        "StatefulSet" => ("apps", "statefulsets"),
        "DaemonSet" => ("apps", "daemonsets"),
        "ReplicaSet" => ("apps", "replicasets"),
        "Job" => ("batch", "jobs"),
        "CronJob" => ("batch", "cronjobs"),
        "Ingress" => ("networking.k8s.io", "ingresses"),
        "NetworkPolicy" => ("networking.k8s.io", "networkpolicies"),
        _ => return None,
    };
    Some(ResourceType {
        group: group.into(),
        resource: resource.into(),
    })
}

/// Renders the `ValidatingAdmissionPolicy` and its binding as JSON.
///
/// `resolve(groups, kind)` returns the resource types for a kind, limited to `groups` when that is
/// non-empty. `owner` is added as an owner reference so Kubernetes deletes the objects with the
/// policy.
pub fn render(
    policy: &Policy,
    native: &NativePolicy,
    resolve: &dyn Fn(&[String], &str) -> Vec<ResourceType>,
    owner: Option<&serde_json::Value>,
) -> Result<(serde_json::Value, serde_json::Value), String> {
    use serde_json::json;
    use std::collections::BTreeMap;

    let mut by_group: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for filter in &policy.match_.resources {
        for kind in &filter.kinds {
            let found = resolve(&filter.api_groups, kind);
            if found.is_empty() {
                return Err(format!("unknown kind {kind:?} for enforcement Native"));
            }
            for t in found {
                let list = by_group.entry(t.group).or_default();
                if !list.contains(&t.resource) {
                    list.push(t.resource);
                }
            }
        }
    }
    let resource_rules: Vec<serde_json::Value> = by_group
        .into_iter()
        .map(|(group, mut resources)| {
            resources.sort();
            json!({
                "apiGroups": [group],
                "apiVersions": ["*"],
                "resources": resources,
                "operations": ["CREATE", "UPDATE"],
                "scope": "*"
            })
        })
        .collect();

    let name = format!("{NAME_PREFIX}{}", policy.name);
    let mut metadata = json!({
        "name": name,
        "labels": {"app.kubernetes.io/managed-by": "ferrogate"},
        "annotations": {"ferrogate.stochasticquant.io/policy": policy.name}
    });
    if let Some(owner) = owner {
        metadata["ownerReferences"] = json!([owner]);
    }

    let mut constraints = json!({"resourceRules": resource_rules, "matchPolicy": "Equivalent"});
    if let Some(selector) = &native.namespace_selector {
        constraints["namespaceSelector"] =
            serde_json::to_value(selector).map_err(|e| e.to_string())?;
    }
    let (failure_policy, actions) = match policy.mode {
        // Enforce fails closed on evaluation errors; there is no network hop to fail.
        crate::Mode::Enforce => ("Fail", json!(["Deny"])),
        crate::Mode::Warn => ("Ignore", json!(["Warn"])),
        crate::Mode::Audit => ("Ignore", json!(["Audit"])),
    };
    let vap = json!({
        "apiVersion": "admissionregistration.k8s.io/v1",
        "kind": "ValidatingAdmissionPolicy",
        "metadata": metadata,
        "spec": {
            "failurePolicy": failure_policy,
            "matchConstraints": constraints,
            "matchConditions": native.match_conditions.iter()
                .map(|c| json!({"name": c.name, "expression": c.expression}))
                .collect::<Vec<_>>(),
            "validations": native.validations.iter()
                .map(|v| json!({"expression": v.expression, "message": v.message, "reason": "Forbidden"}))
                .collect::<Vec<_>>()
        }
    });
    let binding = json!({
        "apiVersion": "admissionregistration.k8s.io/v1",
        "kind": "ValidatingAdmissionPolicyBinding",
        "metadata": metadata,
        "spec": {"policyName": name, "validationActions": actions}
    });
    Ok((vap, binding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PolicySpec;
    use serde_json::json;

    fn policy(spec: serde_json::Value) -> Policy {
        let spec: PolicySpec = serde_json::from_value(spec).unwrap();
        Policy::compile("p", &spec).unwrap()
    }

    #[test]
    fn literals_are_escaped() {
        assert_eq!(lit("a'b\"c\\"), r#""a'b\"c\\""#);
        assert_eq!(list(&["x".into(), "y".into()]), r#"["x", "y"]"#);
    }

    #[test]
    fn labels_rule_works_on_any_kind() {
        let p = policy(json!({
            "match": {"resources": [{"kinds": ["ConfigMap"]}]},
            "rules": [{"name": "team", "requireLabels": ["team"]}]
        }));
        let native = compile_native(&p).unwrap();
        assert_eq!(
            native.validations[0].expression,
            r#"has(object.metadata.labels) && ["team"].all(k, k in object.metadata.labels)"#
        );
        assert_eq!(
            native.validations[0].message,
            "[p] team: required label(s): team"
        );
    }

    #[test]
    fn pod_checks_cover_each_matched_shape() {
        let p = policy(json!({
            "match": {"resources": [{"kinds": ["Pod", "Deployment", "CronJob"]}]},
            "rules": [{"name": "priv", "disallowPrivileged": true}]
        }));
        let e = &compile_native(&p).unwrap().validations[0].expression;
        assert!(e.contains(r#"!(request.kind.kind in ["Pod"]) || "#));
        assert!(e.contains("object.spec.containers.all(c,"));
        assert!(e.contains(r#"!(request.kind.kind in ["Deployment"]) || "#));
        assert!(e.contains("object.spec.template.spec.initContainers"));
        assert!(e.contains("object.spec.jobTemplate.spec.template.spec.ephemeralContainers"));
    }

    #[test]
    fn unsupported_kinds_and_rules_are_rejected() {
        let wildcard = policy(json!({
            "match": {"resources": [{"kinds": ["*"]}]},
            "rules": [{"name": "priv", "disallowPrivileged": true}]
        }));
        assert!(
            compile_native(&wildcard)
                .unwrap_err()
                .contains("built-in workload kinds")
        );

        let registries = policy(json!({
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "reg", "allowedRegistries": ["ghcr.io"]}]
        }));
        assert!(
            compile_native(&registries)
                .unwrap_err()
                .contains("allowedRegistries")
        );
    }

    #[test]
    fn match_and_exclude_become_conditions() {
        let p = policy(json!({
            "match": {
                "resources": [{"kinds": ["Pod"]}],
                "namespaces": ["prod"],
                "namespaceSelector": {"matchLabels": {"env": "prod"}},
                "objectSelector": {"matchExpressions": [{"key": "app", "operator": "Exists"}]}
            },
            "exclude": {
                "namespaces": ["kube-system"],
                "usernames": ["ci"],
                "groups": ["system:masters"],
                "objectSelector": {"matchLabels": {"skip": "true"}}
            },
            "rules": [{"name": "priv", "disallowPrivileged": true}]
        }));
        let native = compile_native(&p).unwrap();
        let names: Vec<&str> = native
            .match_conditions
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "match-namespaces",
                "namespaced-only",
                "match-object-labels",
                "exclude-namespaces",
                "exclude-object-labels",
                "exclude-users",
                "exclude-groups"
            ]
        );
        assert!(native.namespace_selector.is_some());
        let exclude_labels = &native.match_conditions[4].expression;
        assert!(exclude_labels.starts_with("!(("), "{exclude_labels}");
    }

    fn static_resolve(groups: &[String], kind: &str) -> Vec<ResourceType> {
        builtin_resource(kind)
            .filter(|t| groups.is_empty() || groups.contains(&t.group))
            .into_iter()
            .collect()
    }

    #[test]
    fn renders_policy_and_binding() {
        let p = policy(json!({
            "mode": "Enforce",
            "match": {"resources": [{"kinds": ["Pod"]}, {"apiGroups": ["apps"], "kinds": ["Deployment", "StatefulSet"]}]},
            "rules": [{"name": "tag", "disallowLatestTag": true}]
        }));
        let native = compile_native(&p).unwrap();
        let owner =
            json!({"apiVersion": "x/v1", "kind": "ClusterGatePolicy", "name": "p", "uid": "1"});
        let (vap, binding) = render(&p, &native, &static_resolve, Some(&owner)).unwrap();
        assert_eq!(vap["metadata"]["name"], "ferrogate-p");
        assert_eq!(vap["metadata"]["ownerReferences"][0]["uid"], "1");
        assert_eq!(vap["spec"]["failurePolicy"], "Fail");
        let rules = vap["spec"]["matchConstraints"]["resourceRules"]
            .as_array()
            .unwrap();
        assert_eq!(rules[0]["apiGroups"], json!([""]));
        assert_eq!(rules[0]["resources"], json!(["pods"]));
        assert_eq!(rules[1]["apiGroups"], json!(["apps"]));
        assert_eq!(
            rules[1]["resources"],
            json!(["deployments", "statefulsets"])
        );
        assert_eq!(binding["spec"]["policyName"], "ferrogate-p");
        assert_eq!(binding["spec"]["validationActions"], json!(["Deny"]));
    }

    #[test]
    fn audit_and_warn_fail_open() {
        for (mode, action) in [("Audit", "Audit"), ("Warn", "Warn")] {
            let p = policy(json!({
                "mode": mode,
                "match": {"resources": [{"kinds": ["Pod"]}]},
                "rules": [{"name": "tag", "disallowLatestTag": true}]
            }));
            let (vap, binding) =
                render(&p, &compile_native(&p).unwrap(), &static_resolve, None).unwrap();
            assert_eq!(vap["spec"]["failurePolicy"], "Ignore");
            assert_eq!(binding["spec"]["validationActions"], json!([action]));
        }
    }

    #[test]
    fn unknown_kinds_fail_to_render() {
        let p = policy(json!({
            "match": {"resources": [{"kinds": ["Widget"]}]},
            "rules": [{"name": "team", "requireLabels": ["team"]}]
        }));
        let err = render(&p, &compile_native(&p).unwrap(), &static_resolve, None).unwrap_err();
        assert!(err.contains("Widget"));
    }

    #[test]
    fn custom_messages_are_kept() {
        let p = policy(json!({
            "match": {"resources": [{"kinds": ["Pod"]}]},
            "rules": [{"name": "tag", "disallowLatestTag": true, "message": "pin your images"}]
        }));
        assert_eq!(
            compile_native(&p).unwrap().validations[0].message,
            "[p] tag: pin your images"
        );
    }
}
