use std::{collections::{BTreeMap,BTreeSet}, sync::Arc};

use futures::TryFutureExt;
use kube::{api::{ListParams, ObjectMeta, Patch, PatchParams}, runtime::{controller::Action, events::{Event, EventType}}, Api, CustomResource, Resource, ResourceExt};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use serde_json::json;
use serde_with::skip_serializing_none;
use tracing::*;
use super::Network;
use crate::{Context, Error, Result};

pub static NETWORK_LABEL_KEY: &str = "network.named-data.net/name";
pub static ROUTER_FINALIZER: &str = "routers.named-data.net/finalizer";
pub static ROUTER_MANAGER_NAME: &str = "router-controller";
pub static UDP_UNICAST_PORT: i32 = 6363;

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(group = "named-data.net", version = "v1alpha1", kind = "Router", namespaced)]
#[kube(status = "RouterStatus")]
pub struct RouterSpec {
    prefix: String,
    node: String,
    pub faces: RouterFaces,
}

#[skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct RouterFaces {
    udp4: Option<String>,
    tcp4: Option<String>,
    udp6: Option<String>,
    tcp6: Option<String>,
}

impl RouterFaces {
    pub fn to_btree_set(&self) -> BTreeSet<String> {
        let mut faces = BTreeSet::new();
        if let Some(ref udp4) = self.udp4 {
            faces.insert(udp4.clone());
        }
        if let Some(ref tcp4) = self.tcp4 {
            faces.insert(tcp4.clone());
        }
        if let Some(ref udp6) = self.udp6 {
            faces.insert(udp6.clone());
        }
        if let Some(ref tcp6) = self.tcp6 {
            faces.insert(tcp6.clone());
        }
        faces
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
pub struct RouterStatus {
    pub online: bool,
    pub neighbors: BTreeSet<String>,
}

impl Router {
    pub async fn reconcile(&self, ctx: Arc<Context>) -> Result<Action> {

        // Update status.neighbors of all other routers in the network
        let api_router = Api::<Router>::namespaced(ctx.client.clone(), &self.namespace().unwrap());
        let lp = ListParams::default()
            .labels(&format!("{}={}", NETWORK_LABEL_KEY, self.name_any()));
        api_router
            .list(&lp)
            .await
            .map_err(Error::KubeError)?
            .iter()
            .filter(|router| router.name_any() != self.name_any())
            .for_each(|router| {
                let current_neighbors = match &router.status {
                    Some(status) => status.neighbors.clone(),
                    None => BTreeSet::new(),
                };
                // add self.faces to the neighbors
                let mut new_neighbors = current_neighbors.clone();
                let faces = self.spec.faces.to_btree_set();
                for face in faces {
                    new_neighbors.insert(face);
                }
                let status = json!({
                    "status": RouterStatus{
                        online: true,
                        neighbors: new_neighbors,
                    }
                });
                info!("Updating status of router {}...", router.name_any());
                let serverside = PatchParams::apply(ROUTER_MANAGER_NAME);
                let _ = api_router.patch_status(&router.name_any(), &serverside, &Patch::Merge(&status))
                    .map_err(Error::KubeError);
            });
        // Publish event
        ctx.recorder
            .publish(
                &Event {
                    type_: EventType::Normal,
                    reason: "RouterUpdated".into(),
                    note: Some(format!("Updated `{}` Router", self.name_any())),
                    action: "Updated".into(),
                    secondary: None,
                },
                &self.object_ref(&()),
            )
            .await
            .map_err(Error::KubeError)?;
        Ok(Action::await_change())
    }

    pub async fn cleanup(&self, ctx: Arc<Context>) -> Result<Action> {

        // Update status.neighbors of all other routers in the network
        let api_router = Api::<Router>::namespaced(ctx.client.clone(), &self.namespace().unwrap());
        let lp = ListParams::default()
            .labels(&format!("{}={}", NETWORK_LABEL_KEY, self.name_any()));
        api_router
            .list(&lp)
            .await
            .map_err(Error::KubeError)?
            .iter()
            .filter(|router| router.name_any() != self.name_any())
            .for_each(|router| {
                let current_neighbors = match &router.status {
                    Some(status) => status.neighbors.clone(),
                    None => BTreeSet::new(),
                };
                // remove self.faces from the neighbors
                let mut new_neighbors = current_neighbors.clone();
                let faces = self.spec.faces.to_btree_set();
                for face in faces {
                    new_neighbors.remove(&face);
                }
                let status = json!({
                    "status": RouterStatus{
                        online: false,
                        neighbors: new_neighbors,
                    }
                });
                info!("Updating status of router {}...", router.name_any());
                let serverside = PatchParams::apply(ROUTER_MANAGER_NAME);
                let _ = api_router.patch_status(&router.name_any(), &serverside, &Patch::Merge(&status))
                    .map_err(Error::KubeError);
            });

        // Publish event
        ctx.recorder
            .publish(
                &Event {
                    type_: EventType::Normal,
                    reason: "RouterDeleted".into(),
                    note: Some(format!("Deleted `{}` Router", self.name_any())),
                    action: "Deleted".into(),
                    secondary: None,
                },
                &self.object_ref(&()),
            )
            .await
            .map_err(Error::KubeError)?;
        Ok(Action::await_change())
    }
}

pub fn create_owned_router(source: &Network, name: String, node_name: String, ip4: Option<String>, ip6: Option<String>, udp_unicast_port: i32) -> Router {
    let oref = source.controller_owner_ref(&()).unwrap();
    Router {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: source.namespace(),
            owner_references: Some(vec![oref]),
            labels: {
                let mut labels = source.labels().clone();
                labels.extend(BTreeMap::from([(NETWORK_LABEL_KEY.to_string(), source.name_any())]));
                Some(labels)
            },
            annotations: Some(source.annotations().clone()),
            ..ObjectMeta::default()
        },
        spec: RouterSpec {
            prefix: source.spec.prefix.clone(),
            node: node_name,
            faces: RouterFaces {
                udp4: {
                    if let Some(ip4) = ip4 {
                        Some(format!("udp://{ip4}:{udp_unicast_port}"))
                    } else {
                        None
                    }
                },
                tcp4: None,
                udp6: {
                    if let Some(ip6) = ip6 {
                        Some(format!("udp://[{ip6}]:{udp_unicast_port}"))
                    } else {
                        None
                    }
                },
                tcp6: None,
            },
        },
        status: Some(RouterStatus {
            online: false,
            neighbors: BTreeSet::new(),
        }),
    }
}
