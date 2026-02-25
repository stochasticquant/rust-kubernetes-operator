use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "guardian.io", version = "v1", kind = "GuardianPolicy")]
pub struct GuardianPolicySpec {
    pub severity: String,
}
