use k8s_openapi::api::core::v1::Node;
use kube::{api::ObjectMeta, CustomResource, Resource, ResourceExt};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;

use super::Network;

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "named-data.net", version = "v1alpha1", kind = "Router", namespaced)]
#[kube(status = "RouterStatus")]
pub struct RouterSpec {
    prefix: String,
    node: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct RouterStatus {
    faces: Vec<String>,
}

pub fn create_owned_router(source: &Network, node: &Node) -> Router {
    let oref = source.controller_owner_ref(&()).unwrap();
    Router {
        metadata: ObjectMeta {
            name: Some(format!("{}-{}", source.name_any(), node.name_any())),
            namespace: source.namespace(),
            owner_references: Some(vec![oref]),
            labels: Some(source.labels().clone()),
            annotations: Some(source.annotations().clone()),
            ..ObjectMeta::default()
        },
        spec: RouterSpec {
            prefix: source.spec.prefix.clone(),
            node: node.name_any(),
        },
        status: None,
    }
}