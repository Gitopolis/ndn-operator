use crate::crd::Network;
use k8s_openapi::api::apps::v1::{DaemonSet, DaemonSetSpec};
use k8s_openapi::api::core::v1::{Container, EnvVar, EnvVarSource, HostPathVolumeSource, ObjectFieldSelector, PodSpec, PodTemplateSpec, SecurityContext, Volume, VolumeMount};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::{Resource, ResourceExt};
use std::collections::BTreeMap;

pub fn create_owned_daemonset(source: &Network, image: Option<String>, service_account: Option<String>) -> DaemonSet {
    let oref = source.controller_owner_ref(&()).unwrap();
    let mut labels = BTreeMap::new();
    labels.insert("network".to_string(), source.name_any());
    DaemonSet {
        metadata: ObjectMeta {
            name: Some(source.name_any()),
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
                    init_containers: Some(vec![Container {
                        name: "gencfg".to_string(),
                        image: image,
                        command: vec!["/genconfig".to_string(), "--output".to_string(), "/etc/ndnd/example.yml".to_string()].into(),
                        env: Some(vec![
                            EnvVar {
                                name: "NDN_NETWORK_NAME".to_string(),
                                value: Some(source.name_any()),
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
                                name: "NDN_SOCKET_PATH".to_string(),
                                value: Some(format!("/run/ndnd/{}.sock", source.name_any())),
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
                                mount_path: "/etc/ndnd".to_string(),
                                read_only: Some(false),
                                ..VolumeMount::default()
                            },
                        ]),
                        ..Container::default()
                    }]),
                    containers: vec![Container {
                        name: "network".to_string(),
                        image: Some("ghcr.io/named-data/ndnd:latest".to_string()),
                        command: vec!["/ndnd".to_string()].into(),
                        args: Some(vec!["daemon".to_string(), "/etc/ndnd/example.yml".to_string()]),
                        security_context: Some(SecurityContext {
                            privileged: Some(true),
                            ..SecurityContext::default()
                        }),
                        env: Some(vec![
                            EnvVar {
                                name: "NDN_CLIENT_TRANSPORT".to_string(),
                                value: Some(format!("unix:///run/ndnd/{}.sock", source.name_any())),
                                ..EnvVar::default()
                            },
                        ]),
                        volume_mounts: Some(vec![
                            VolumeMount {
                                name: "config".to_string(),
                                mount_path: "/etc/ndnd".to_string(),
                                read_only: Some(true),
                                ..VolumeMount::default()
                            },
                            VolumeMount {
                                name: "run-ndnd".to_string(),
                                mount_path: "/run/ndnd".to_string(),
                                ..VolumeMount::default()
                            },
                        ]),
                        ..Container::default()
                    }],
                    volumes: Some(vec![
                        Volume {
                            name: "config".to_string(),
                            host_path: Some(HostPathVolumeSource {
                                path: "/etc/ndnd".to_string(),
                                type_: Some("DirectoryOrCreate".to_string()),
                                ..HostPathVolumeSource::default()
                            }),
                            ..Volume::default()
                        },
                        Volume {
                            name: "run-ndnd".to_string(),
                            host_path: Some(HostPathVolumeSource {
                                path: "/run/ndnd".to_string(),
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