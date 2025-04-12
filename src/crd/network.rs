use crate::{helper::*, Context, Error, Result};
use k8s_openapi::{
    api::{
        apps::v1::{DaemonSet, DaemonSetSpec},
        core::v1::{
            Container, ContainerPort, EnvVar, EnvVarSource, HostPathVolumeSource, ObjectFieldSelector, PodSpec,
            PodTemplateSpec, SecurityContext, Volume, VolumeMount,
        },
    },
    apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta},
};
use kube::{
    api::{Api, Patch, PatchParams, ResourceExt},
    runtime::{
        controller::Action,
        events::{Event, EventType},
    },
    CustomResource, Resource,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

pub static NETWORK_FINALIZER: &str = "networks.named-data.net/finalizer";
pub static NETWORK_MANAGER_NAME: &str = "network-controller";
pub static CONTAINER_CONFIG_DIR: &str = "/etc/ndnd";
pub static CONTAINER_SOCKET_DIR: &str = "/run/ndnd";
pub static HOST_CONFIG_DIR: &str = "/etc/ndnd";
pub static HOST_SOCKET_DIR: &str = "/run/ndnd";

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(group = "named-data.net", version = "v1alpha1", kind = "Network", namespaced, shortname = "nw")]
#[kube(status = "NetworkStatus")]
pub struct NetworkSpec {
    pub prefix: String,
    pub udp_unicast_port: i32,
    pub node_selector: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatus {
    ds_created: Option<bool>,
}

impl Network {
    pub async fn reconcile(&self, ctx: Arc<Context>) -> Result<Action> {
        let api_nw: Api<Network> = Api::namespaced(ctx.client.clone(), &self.namespace().unwrap());
        let api_ds: Api<DaemonSet> = Api::namespaced(ctx.client.clone(), &self.namespace().unwrap());
        let serverside = PatchParams::apply(NETWORK_MANAGER_NAME);
        let my_pod_spec = get_my_pod(ctx.client.clone())
            .await
            .expect("Failed to get my pod")
            .spec
            .expect("Failed to get pod spec");
        let my_image = my_pod_spec.containers.first().expect("Failed to get my container").image.clone();
        let ds_data = self.create_owned_daemonset(my_image, my_pod_spec.service_account_name);
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
        Ok(Action::await_change())
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

    fn socket_file_name(&self) -> String {
        format!("{}.sock", self.name_any())
    }

    pub fn container_socket_path(&self) -> String {
        format!("{}/{}", CONTAINER_SOCKET_DIR, self.socket_file_name())
    }

    pub fn host_socket_path(&self) -> String {
        format!("{}/{}", HOST_SOCKET_DIR, self.socket_file_name())
    }

    fn config_file_name(&self) -> String {
        format!("{}.yml", self.name_any())
    }

    pub fn container_config_path(&self) -> String {
        format!("{}/{}", CONTAINER_CONFIG_DIR, self.config_file_name())
    }

    pub fn host_config_path(&self) -> String {
        format!("{}/{}", HOST_CONFIG_DIR, self.config_file_name())
    }

    pub fn create_owned_daemonset(&self, image: Option<String>, service_account: Option<String>) -> DaemonSet {
        let oref = self.controller_owner_ref(&()).unwrap();
        let mut labels = BTreeMap::new();
        labels.insert("network".to_string(), self.name_any());
        let container_config_path = self.container_config_path();
        let container_socket_path = self.container_socket_path();
        DaemonSet {
            metadata: ObjectMeta {
                name: Some(self.name_any()),
                owner_references: Some(vec![oref]),
                labels: Some(labels.clone()),
                ..ObjectMeta::default()
            },
            spec: Some(DaemonSetSpec {
                selector: LabelSelector {
                    match_labels: Some(labels.clone()),
                    ..LabelSelector::default()
                },
                template: PodTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels: Some(labels.clone()),
                        ..ObjectMeta::default()
                    }),
                    spec: Some(PodSpec {
                        service_account_name: service_account,
                        host_network: Some(true),
                        dns_policy: Some("ClusterFirstWithHostNet".to_string()),
                        node_selector: self.spec.node_selector.clone(),
                        init_containers: Some(vec![Container {
                            name: "init".to_string(),
                            image: image.clone(),
                            command: vec!["/init".to_string(), "--output".to_string(), container_config_path.clone()].into(),
                            env: Some(vec![
                                EnvVar {
                                    name: "NDN_NETWORK_NAME".to_string(),
                                    value: Some(self.name_any()),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_UDP_UNICAST_PORT".to_string(),
                                    value: Some(self.spec.udp_unicast_port.to_string()),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_NETWORK_NAMESPACE".to_string(),
                                    value_from: Some(EnvVarSource {
                                        field_ref: Some(ObjectFieldSelector {
                                            field_path: "metadata.namespace".to_string(),
                                            ..ObjectFieldSelector::default()
                                        }),
                                        ..EnvVarSource::default()
                                    }),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_ROUTER_NAME".to_string(),
                                    value_from: Some(EnvVarSource {
                                        field_ref: Some(ObjectFieldSelector {
                                            field_path: "spec.nodeName".to_string(),
                                            ..ObjectFieldSelector::default()
                                        }),
                                        ..EnvVarSource::default()
                                    }),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_NODE_NAME".to_string(),
                                    value_from: Some(EnvVarSource {
                                        field_ref: Some(ObjectFieldSelector {
                                            field_path: "spec.nodeName".to_string(),
                                            ..ObjectFieldSelector::default()
                                        }),
                                        ..EnvVarSource::default()
                                    }),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_SOCKET_PATH".to_string(),
                                    value: Some(container_socket_path.clone()),
                                    ..EnvVar::default()
                                },
                            ]),
                            security_context: Some(SecurityContext {
                                privileged: Some(true),
                                ..SecurityContext::default()
                            }),
                            volume_mounts: Some(vec![
                                VolumeMount {
                                    name: "config".to_string(),
                                    mount_path: CONTAINER_CONFIG_DIR.to_string(),
                                    read_only: Some(false),
                                    ..VolumeMount::default()
                                },
                            ]),
                            ..Container::default()
                        }]),
                        containers: vec![Container {
                            name: "network".to_string(),
                            image: Some("ghcr.io/named-data/ndnd:20250405".to_string()),
                            command: vec!["/ndnd".to_string()].into(),
                            args: Some(vec!["daemon".to_string(), container_config_path.to_string()].into()),
                            security_context: Some(SecurityContext {
                                privileged: Some(true),
                                ..SecurityContext::default()
                            }),
                            ports: Some(vec![
                                ContainerPort {
                                    container_port: self.spec.udp_unicast_port,
                                    host_port: Some(self.spec.udp_unicast_port),
                                    protocol: Some("UDP".to_string()),
                                    ..ContainerPort::default()
                                },
                            ]),
                            env: Some(vec![
                                EnvVar {
                                    name: "NDN_CLIENT_TRANSPORT".to_string(),
                                    value: Some(format!("unix://{}", container_socket_path.clone())),
                                    ..EnvVar::default()
                                },
                            ]),
                            volume_mounts: Some(vec![
                                VolumeMount {
                                    name: "config".to_string(),
                                    mount_path: CONTAINER_CONFIG_DIR.to_string(),
                                    read_only: Some(true),
                                    ..VolumeMount::default()
                                },
                                VolumeMount {
                                    name: "run-ndnd".to_string(),
                                    mount_path: CONTAINER_SOCKET_DIR.to_string(),
                                    ..VolumeMount::default()
                                },
                            ]),
                            ..Container::default()
                        },
                        Container {
                            name: "watch".to_string(),
                            image: image,
                            command: vec!["/sidecar".to_string()].into(),
                            env: Some(vec![
                                EnvVar {
                                    name: "NDN_NETWORK_NAME".to_string(),
                                    value: Some(self.name_any()),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_NETWORK_NAMESPACE".to_string(),
                                    value_from: Some(EnvVarSource {
                                        field_ref: Some(ObjectFieldSelector {
                                            field_path: "metadata.namespace".to_string(),
                                            ..ObjectFieldSelector::default()
                                        }),
                                        ..EnvVarSource::default()
                                    }),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_ROUTER_NAME".to_string(),
                                    value_from: Some(EnvVarSource {
                                        field_ref: Some(ObjectFieldSelector {
                                            field_path: "spec.nodeName".to_string(),
                                            ..ObjectFieldSelector::default()
                                        }),
                                        ..EnvVarSource::default()
                                    }),
                                    ..EnvVar::default()
                                },
                                EnvVar {
                                    name: "NDN_CLIENT_TRANSPORT".to_string(),
                                    value: Some(format!("unix://{}", container_socket_path)),
                                    ..EnvVar::default()
                                },
                            ]),
                            volume_mounts: Some(vec![
                                VolumeMount {
                                    name: "run-ndnd".to_string(),
                                    mount_path: CONTAINER_SOCKET_DIR.to_string(),
                                    ..VolumeMount::default()
                                },
                            ]),
                            ..Container::default()
                        }],
                        volumes: Some(vec![
                            Volume {
                                name: "config".to_string(),
                                host_path: Some(HostPathVolumeSource {
                                    path: HOST_CONFIG_DIR.to_string(),
                                    type_: Some("DirectoryOrCreate".to_string()),
                                    ..HostPathVolumeSource::default()
                                }),
                                ..Volume::default()
                            },
                            Volume {
                                name: "run-ndnd".to_string(),
                                host_path: Some(HostPathVolumeSource {
                                    path: HOST_SOCKET_DIR.to_string(),
                                    type_: Some("DirectoryOrCreate".to_string()),
                                    ..HostPathVolumeSource::default()
                                }),
                                ..Volume::default()
                            },
                        ]),
                        ..PodSpec::default()
                    }),
                },
                ..DaemonSetSpec::default()
                
            }),
            ..Default::default()
        }
    }
}

use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use std::convert::TryFrom;

impl TryFrom<OwnerReference> for Network {
    type Error = String;

    fn try_from(owner_ref: OwnerReference) -> Result<Self, Self::Error> {
        if owner_ref.kind != "Network" {
            return Err(format!(
                "Expected kind 'Network', found '{}'",
                owner_ref.kind
            ));
        }

        if owner_ref.api_version != "named-data.net/v1alpha1" {
            return Err(format!(
                "Expected apiVersion 'named-data.net/v1alpha1', found '{}'",
                owner_ref.api_version
            ));
        }

        let name = owner_ref.name;
        if name.is_empty() {
            return Err("OwnerReference name is empty".to_string());
        }

        Ok(Network {
            metadata: kube::api::ObjectMeta {
                name: Some(name),
                ..Default::default()
            },
            spec: NetworkSpec {
                prefix: String::new(),
                udp_unicast_port: 0,
                node_selector: None,
            },
            status: None,
        })
    }
}