// SPDX-License-Identifier: Apache-2.0

//! Finding the pod spec and containers inside workload objects, and parsing image references.

use serde_json::Value;

/// Where the pod spec lives for well-known kinds, as JSON pointer and field path prefix.
const POD: (&str, &str) = ("/spec", "spec");
const TEMPLATE: (&str, &str) = ("/spec/template/spec", "spec.template.spec");
const CRONJOB: (&str, &str) = (
    "/spec/jobTemplate/spec/template/spec",
    "spec.jobTemplate.spec.template.spec",
);

/// The pod spec of a workload object, with the field path used to report violations.
#[derive(Clone, Copy, Debug)]
pub struct PodSpec<'a> {
    pub spec: &'a Value,
    pub path: &'static str,
}

/// One container (regular, init or ephemeral) of a pod spec.
#[derive(Clone, Debug)]
pub struct Container<'a> {
    pub value: &'a Value,
    pub path: String,
    pub name: &'a str,
    /// Ephemeral containers cannot set resources, so resource rules skip them.
    pub ephemeral: bool,
}

impl Container<'_> {
    pub fn image(&self) -> &str {
        self.value
            .get("image")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// Field of the container's `securityContext`.
    pub fn security(&self, field: &str) -> Option<&Value> {
        self.value.get("securityContext")?.get(field)
    }
}

/// Finds the pod spec of a `Pod`, a workload with a pod template, or a `CronJob`.
///
/// Unknown kinds (for example custom workload resources) are probed for the common shapes, so
/// `spec.template.spec` objects such as Argo Rollouts are covered too.
pub fn pod_spec<'a>(kind: &str, object: &'a Value) -> Option<PodSpec<'a>> {
    let candidates: &[(&str, &'static str)] = match kind {
        "Pod" => &[POD],
        "Deployment"
        | "StatefulSet"
        | "DaemonSet"
        | "ReplicaSet"
        | "Job"
        | "ReplicationController" => &[TEMPLATE],
        "CronJob" => &[CRONJOB],
        _ => &[TEMPLATE, CRONJOB],
    };
    candidates.iter().find_map(|(pointer, path)| {
        let spec = object.pointer(pointer)?;
        // A pod spec always has a containers list; this also rejects unrelated `spec` objects.
        spec.get("containers")?.as_array()?;
        Some(PodSpec { spec, path })
    })
}

impl<'a> PodSpec<'a> {
    pub fn containers(&self) -> Vec<Container<'a>> {
        let mut out = Vec::new();
        for (field, ephemeral) in [
            ("initContainers", false),
            ("containers", false),
            ("ephemeralContainers", true),
        ] {
            let Some(list) = self.spec.get(field).and_then(Value::as_array) else {
                continue;
            };
            for (index, value) in list.iter().enumerate() {
                out.push(Container {
                    value,
                    path: format!("{}.{field}[{index}]", self.path),
                    name: value.get("name").and_then(Value::as_str).unwrap_or(""),
                    ephemeral,
                });
            }
        }
        out
    }

    /// Field of the pod-level `securityContext`.
    pub fn security(&self, field: &str) -> Option<&'a Value> {
        self.spec.get("securityContext")?.get(field)
    }

    pub fn field(&self, field: &str) -> Option<&'a Value> {
        self.spec.get(field)
    }
}

/// A parsed container image reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef<'a> {
    /// Registry host, `docker.io` when not given.
    pub registry: String,
    /// Repository path, with `library/` added for official Docker Hub images.
    pub repository: String,
    pub tag: Option<&'a str>,
    pub digest: Option<&'a str>,
}

impl<'a> ImageRef<'a> {
    /// Parses an image reference the way container runtimes do.
    pub fn parse(image: &'a str) -> Self {
        let (name_tag, digest) = match image.split_once('@') {
            Some((name, digest)) => (name, Some(digest)),
            None => (image, None),
        };
        let last_slash = name_tag.rfind('/');
        let (name, tag) = match name_tag.rfind(':') {
            Some(colon) if last_slash.is_none_or(|slash| colon > slash) => {
                (&name_tag[..colon], Some(&name_tag[colon + 1..]))
            }
            _ => (name_tag, None),
        };
        let (registry, repository) = match name.split_once('/') {
            Some((first, rest))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first.to_ascii_lowercase(), rest.to_owned())
            }
            _ => ("docker.io".to_owned(), name.to_owned()),
        };
        let repository = if registry == "docker.io" && !repository.contains('/') {
            format!("library/{repository}")
        } else {
            repository
        };
        Self {
            registry,
            repository,
            tag,
            digest,
        }
    }

    /// `registry/repository`, for example `docker.io/library/nginx`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.registry, self.repository)
    }

    /// True for `:latest` and for images with neither tag nor digest (which default to latest).
    pub fn is_latest(&self) -> bool {
        self.digest.is_none() && self.tag.is_none_or(|tag| tag == "latest")
    }

    /// True if the image comes from `prefix`, which is a registry (`ghcr.io`) or a registry and
    /// repository path (`ghcr.io/acme`). Matching is on whole path segments: `ghcr.io/acme` does
    /// not match `ghcr.io/acme-evil/app`.
    pub fn is_from(&self, prefix: &str) -> bool {
        let prefix = prefix.trim_end_matches('/');
        let (prefix_registry, prefix_path) = match prefix.split_once('/') {
            Some((registry, path)) => (registry, Some(path)),
            None => (prefix, None),
        };
        if !self.registry.eq_ignore_ascii_case(prefix_registry) {
            return false;
        }
        match prefix_path {
            None => true,
            Some(path) => {
                self.repository == path
                    || self
                        .repository
                        .strip_prefix(path)
                        .is_some_and(|rest| rest.starts_with('/'))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_image_references() {
        let cases = [
            ("nginx", "docker.io", "library/nginx", None, None),
            (
                "nginx:1.27",
                "docker.io",
                "library/nginx",
                Some("1.27"),
                None,
            ),
            (
                "bitnami/redis:7",
                "docker.io",
                "bitnami/redis",
                Some("7"),
                None,
            ),
            (
                "ghcr.io/acme/app:v1",
                "ghcr.io",
                "acme/app",
                Some("v1"),
                None,
            ),
            ("localhost:5000/app", "localhost:5000", "app", None, None),
            (
                "registry.k8s.io/pause@sha256:abc",
                "registry.k8s.io",
                "pause",
                None,
                Some("sha256:abc"),
            ),
            (
                "Quay.IO/org/app:1@sha256:abc",
                "quay.io",
                "org/app",
                Some("1"),
                Some("sha256:abc"),
            ),
        ];
        for (image, registry, repository, tag, digest) in cases {
            let parsed = ImageRef::parse(image);
            assert_eq!(parsed.registry, registry, "{image}");
            assert_eq!(parsed.repository, repository, "{image}");
            assert_eq!(parsed.tag, tag, "{image}");
            assert_eq!(parsed.digest, digest, "{image}");
        }
    }

    #[test]
    fn latest_detection() {
        assert!(ImageRef::parse("nginx").is_latest());
        assert!(ImageRef::parse("nginx:latest").is_latest());
        assert!(ImageRef::parse("localhost:5000/app").is_latest());
        assert!(!ImageRef::parse("nginx:1.27").is_latest());
        assert!(!ImageRef::parse("nginx@sha256:abc").is_latest());
        assert!(!ImageRef::parse("nginx:latest@sha256:abc").is_latest());
    }

    #[test]
    fn registry_prefixes_match_whole_segments() {
        let image = ImageRef::parse("ghcr.io/acme/app:1");
        assert!(image.is_from("ghcr.io"));
        assert!(image.is_from("ghcr.io/acme"));
        assert!(image.is_from("ghcr.io/acme/"));
        assert!(image.is_from("GHCR.io/acme/app"));
        assert!(!image.is_from("ghcr.io/ac"));
        assert!(!ImageRef::parse("ghcr.io/acme-evil/app").is_from("ghcr.io/acme"));
        assert!(!image.is_from("docker.io"));
        // A registry name in the repository path must not fool the matcher.
        assert!(!ImageRef::parse("evil.io/ghcr.io/acme/app").is_from("ghcr.io"));
        assert!(ImageRef::parse("nginx").is_from("docker.io/library"));
    }

    #[test]
    fn finds_pod_spec_for_each_workload_shape() {
        let pod = json!({"spec": {"containers": [{"name": "a"}]}});
        let deploy = json!({"spec": {"template": {"spec": {"containers": [{"name": "a"}]}}}});
        let cron =
            json!({"spec": {"jobTemplate": {"spec": {"template": {"spec": {"containers": []}}}}}});

        assert_eq!(pod_spec("Pod", &pod).unwrap().path, "spec");
        assert_eq!(
            pod_spec("Deployment", &deploy).unwrap().path,
            "spec.template.spec"
        );
        assert_eq!(
            pod_spec("CronJob", &cron).unwrap().path,
            "spec.jobTemplate.spec.template.spec"
        );
        assert_eq!(
            pod_spec("Rollout", &deploy).unwrap().path,
            "spec.template.spec"
        );
        assert!(pod_spec("ConfigMap", &json!({"data": {}})).is_none());
        assert!(pod_spec("Service", &json!({"spec": {"ports": []}})).is_none());
        assert!(pod_spec("Deployment", &pod).is_none());
    }

    #[test]
    fn lists_all_container_kinds_with_paths() {
        let pod = json!({"spec": {
            "initContainers": [{"name": "init"}],
            "containers": [{"name": "app"}, {"name": "side"}],
            "ephemeralContainers": [{"name": "debug"}]
        }});
        let containers = pod_spec("Pod", &pod).unwrap().containers();
        let paths: Vec<_> = containers.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "spec.initContainers[0]",
                "spec.containers[0]",
                "spec.containers[1]",
                "spec.ephemeralContainers[0]"
            ]
        );
        assert!(containers[3].ephemeral);
        assert_eq!(containers[1].name, "app");
    }
}
