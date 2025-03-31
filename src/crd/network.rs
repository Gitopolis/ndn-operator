use std::sync::Arc;
use crate::{daemonset::*, helper::*, Context, Error, Result};
use k8s_openapi::api::apps::v1::DaemonSet;
use kube::{
    api::{Api, Patch, PatchParams, ResourceExt},
    runtime::{
        controller::Action,
        events::{Event, EventType},
    }, CustomResource, Resource
};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use serde_json::json;
use tokio::time::Duration;

pub static NETWORK_FINALIZER: &str = "networks.named-data.net/finalizer";
pub static MANAGER_NAME: &str = "ndnd-controller";

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "named-data.net", version = "v1alpha1", kind = "Network", namespaced, shortname = "ndn")]
#[kube(status = "NetworkStatus")]
pub struct NetworkSpec {
    pub prefix: String,
    pub node_selector: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct NetworkStatus {
    ds_created: Option<bool>,
}

impl Network {
    pub async fn reconcile(&self, ctx: Arc<Context>) -> Result<Action> {
        let api_nw: Api<Network> = Api::namespaced(ctx.client.clone(), &self.namespace().unwrap());
        let api_ds: Api<DaemonSet> = Api::namespaced(ctx.client.clone(), &self.namespace().unwrap());
        let serverside = PatchParams::apply(MANAGER_NAME);
        let my_pod_spec = get_my_pod(ctx.client.clone())
            .await
            .expect("Failed to get my pod")
            .spec
            .expect("Failed to get pod spec");
        let my_image = my_pod_spec.containers.first().expect("Failed to get my container").image.clone();
        let ds_data = create_owned_daemonset(&self, my_image, my_pod_spec.service_account_name);
        let ds = api_ds.patch(&self.name_any(), &serverside, &Patch::Apply(ds_data)).await.map_err(Error::KubeError)?;
        // Publish event
        ctx.recorder
            .publish(
                &Event {
                    type_: EventType::Normal,
                    reason: "DaemonSetCreated".into(),
                    note: Some(format!("Created `{}` DaemonSet for `{}` Network", ds.name_any(), self.name_any())),
                    action: "Created".into(),
                    secondary: None,
                },
                &self.object_ref(&()),
            )
            .await
            .map_err(Error::KubeError)?;
        // Update the status of the Network
        let status = json!({
            "status": NetworkStatus {
                ds_created: Some(true),
            }
        });
        let _o = api_nw
            .patch_status(&self.name_any(), &serverside, &Patch::Merge(&status))
            .await
            .map_err(Error::KubeError)?;
        Ok(Action::requeue(Duration::from_secs(5 * 60)))
    }

    pub async fn cleanup(&self, ctx: Arc<Context>) -> Result<Action> {
        let oref = self.object_ref(&());
        ctx.recorder
            .publish(
                &Event {
                    type_: EventType::Normal,
                    reason: "DeleteRequested".into(),
                    note: Some(format!("Delete `{}`", self.name_any())),
                    action: "Deleting".into(),
                    secondary: None,
                },
                &oref,
            )
            .await
            .map_err(Error::KubeError)?;
        Ok(Action::await_change())
    }
}
