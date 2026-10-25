// SPDX-License-Identifier: Apache-2.0

//! Native enforcement: create, update and remove the generated `ValidatingAdmissionPolicy` and
//! binding for a policy with `enforcement: Native`.

use ferrogate_core::Policy;
use ferrogate_core::native::{self, NAME_PREFIX, ResourceType};
use ferrogate_crd::ClusterGatePolicy;
use kube::api::{DeleteParams, Patch, PatchParams};
use kube::core::{ApiResource, DynamicObject, GroupVersionKind};
use kube::{Api, Resource, ResourceExt};
use serde_json::json;

use crate::scan::Listable;
use crate::{Context, Error};

const FIELD_MANAGER: &str = "ferrogate";

fn api_resource(kind: &str, plural: &str) -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk("admissionregistration.k8s.io", "v1", kind),
        plural,
    )
}

fn policies_api(ctx: &Context) -> Api<DynamicObject> {
    Api::all_with(
        ctx.client.clone(),
        &api_resource("ValidatingAdmissionPolicy", "validatingadmissionpolicies"),
    )
}

fn bindings_api(ctx: &Context) -> Api<DynamicObject> {
    Api::all_with(
        ctx.client.clone(),
        &api_resource(
            "ValidatingAdmissionPolicyBinding",
            "validatingadmissionpolicybindings",
        ),
    )
}

/// Resource types for a kind, from API discovery.
fn resolve(available: &[Listable], groups: &[String], kind: &str) -> Vec<ResourceType> {
    available
        .iter()
        .filter(|l| l.resource.kind == kind)
        .filter(|l| groups.is_empty() || groups.iter().any(|g| g == "*" || *g == l.resource.group))
        .map(|l| ResourceType {
            group: l.resource.group.clone(),
            resource: l.resource.plural.clone(),
        })
        .collect()
}

/// Outcome of applying native enforcement.
pub enum Applied {
    /// The generated policy and binding are in place under this name.
    Active(String),
    /// The policy cannot be enforced natively; the reason goes into the `Ready` condition.
    Unsupported(String),
}

/// Creates or updates the generated objects (server-side apply, owned by the policy).
pub async fn apply(
    ctx: &Context,
    object: &ClusterGatePolicy,
    policy: &Policy,
) -> Result<Applied, Error> {
    let compiled = match native::compile_native(policy) {
        Ok(compiled) => compiled,
        Err(reason) => return Ok(Applied::Unsupported(reason)),
    };
    let available = ctx.discovery.get(&ctx.client).await?;
    let owner = json!({
        "apiVersion": ClusterGatePolicy::api_version(&()),
        "kind": ClusterGatePolicy::kind(&()),
        "name": object.name_any(),
        "uid": object.uid().unwrap_or_default(),
        "controller": true
    });
    let (vap, binding) = match native::render(
        policy,
        &compiled,
        &|groups, kind| resolve(&available, groups, kind),
        Some(&owner),
    ) {
        Ok(rendered) => rendered,
        Err(reason) => return Ok(Applied::Unsupported(reason)),
    };
    let name = format!("{NAME_PREFIX}{}", policy.name);
    let params = PatchParams::apply(FIELD_MANAGER).force();
    policies_api(ctx)
        .patch(&name, &params, &Patch::Apply(&vap))
        .await?;
    bindings_api(ctx)
        .patch(&name, &params, &Patch::Apply(&binding))
        .await?;
    Ok(Applied::Active(name))
}

/// Deletes the generated objects of `policy`, if any.
pub async fn remove(ctx: &Context, policy: &str) -> Result<(), Error> {
    let name = format!("{NAME_PREFIX}{policy}");
    for api in [bindings_api(ctx), policies_api(ctx)] {
        match api.delete(&name, &DeleteParams::default()).await {
            Ok(_) => {}
            Err(kube::Error::Api(status)) if status.code == 404 => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
