use crate::crd::ServiceScaler;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client, Error};
use serde_json::{json, Value};

/// adds finalizer
pub async fn add(client: Client, namespace: &str, name: &str) -> Result<ServiceScaler, Error> {
    let api: Api<ServiceScaler> = Api::namespaced(client, namespace);
    let finalizer: Value = json!({
        "metadata": {
            "finalizers": ["servicescalers.scaler.udaan.io/finalizer"]
        }
    });
    let patch: Patch<&Value> = Patch::Merge(&finalizer);
    api.patch(name, &PatchParams::default(), &patch).await
}


/// removes finalizer
pub async fn delete(client: Client, namespace: &str, name: &str) -> Result<ServiceScaler, Error> {
    let api: Api<ServiceScaler> = Api::namespaced(client, namespace);
    let finalizer: Value = json!({
        "metadata": {
            "finalizers": null
        }
    });
    let patch: Patch<&Value> = Patch::Merge(&finalizer);
    api.patch(name, &PatchParams::default(), &patch).await
}