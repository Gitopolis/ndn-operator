use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client};
use crate::{Result, Error};

pub fn get_my_namespace() -> Result<String> {
    std::fs::read_to_string("/var/run/secrets/kubernetes.io/serviceaccount/namespace")
        .map_err(Error::IoError)
}

pub fn get_my_pod_name() -> Result<String> {
    std::fs::read_to_string("/etc/hostname").map_err(Error::IoError)
}

pub async fn get_my_pod(client: Client) -> Result<Pod> {
    let namespace_raw = get_my_namespace()?;
    let namespace = namespace_raw.trim_end_matches('\n');
    let api_pods = Api::<Pod>::namespaced(client, namespace);
    let pod_name_raw = get_my_pod_name()?;
    let pod_name = pod_name_raw.trim_end_matches('\n');
    api_pods.get(pod_name).await.map_err(Error::KubeError)
}