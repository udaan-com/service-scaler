use std::collections::BTreeMap;
use k8s_openapi::api::autoscaling::v2beta2::{CrossVersionObjectReference, MetricSpec, MetricTarget, ResourceMetricSource};
use k8s_openapi::api::autoscaling::v2beta2::{HorizontalPodAutoscaler, HorizontalPodAutoscalerSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{Api, Client, Error};
use kube::api::{DeleteParams, Patch, PatchParams, PostParams};
use kube::error::{ErrorResponse};
use serde_json::{json, Value};
use log::{info};
use crate::crd::HpaSpec;
use crate::util::{K8S_AUTOSCALING_VERSION, K8S_DEPLOYMENT_VERSION, key, SERVICE_SCALER_MANAGED_ANNOTATION, SERVICE_SCALER_NOTE_KEY, SERVICE_SCALER_NOTE_VALUE};


static DEFAULT_CPU_UTILIZATION: u32 = 80;

#[derive(Clone)]
pub struct HpaOperator {
    pub client: Client,
}

impl HpaOperator {
    pub async fn get(&self, namespace: &str, name: &str) -> Result<HorizontalPodAutoscaler, Error> {
        let api: Api<HorizontalPodAutoscaler> = Api::namespaced(self.client.clone(), namespace);
        api.get(name).await
    }

    pub async fn create(&self, namespace: &str, name: &str, hpa_spec: &HpaSpec, service_scaler_metadata: &ObjectMeta) -> Result<HorizontalPodAutoscaler, Error> {
        let api: Api<HorizontalPodAutoscaler> = Api::namespaced(self.client.clone(), namespace);
        let existing = api.get(name).await;
        if existing.is_ok() {
            info!("[{}] hpa already exists!", key(namespace, name));
            // add service scaler managed annotation
            let annotations_patch: Value = json!({
                "metadata": {
                    "annotations": {
                        SERVICE_SCALER_MANAGED_ANNOTATION: "true",
                        SERVICE_SCALER_NOTE_KEY: SERVICE_SCALER_NOTE_VALUE
                    }
                }
            });
            api.patch_metadata(name, &PatchParams::default(), &Patch::Merge(&annotations_patch)).await.expect("patch_metadata errored!");
            existing
        } else {
            // copy over existing annotations and labels
            let annotations = match service_scaler_metadata.clone().annotations {
                Some(mut annotations) => {
                    annotations.insert(SERVICE_SCALER_MANAGED_ANNOTATION.to_string(), "true".to_string());
                    annotations.insert(SERVICE_SCALER_NOTE_KEY.to_string(), SERVICE_SCALER_NOTE_VALUE.to_string());
                    annotations
                }
                None => {
                    let mut annotations = BTreeMap::new();
                    annotations.insert(SERVICE_SCALER_MANAGED_ANNOTATION.to_string(), "true".to_string());
                    annotations.insert(SERVICE_SCALER_NOTE_KEY.to_string(), SERVICE_SCALER_NOTE_VALUE.to_string());
                    annotations
                }
            };

            let hpa = if hpa_spec.target_cpu_utilization.is_some() && hpa_spec.target_memory_utilization.is_some() {
                serde_json::from_value(json!({
                    "apiVersion": K8S_AUTOSCALING_VERSION,
                    "kind": "HorizontalPodAutoscaler",
                    "metadata": {
                        "name": name,
                        "namespace": namespace,
                        "annotations": annotations,
                        "labels": service_scaler_metadata.clone().labels
                    },
                    "spec": {
                        "scaleTargetRef": {
                            "apiVersion": "apps/v2beta2",
                            "kind": "Deployment",
                            "name": name
                        },
                        "minReplicas": hpa_spec.min_replicas,
                        "maxReplicas": hpa_spec.max_replicas,
                        "metrics": [
                            {
                            "type": "Resource",
                            "resource": {
                                "name": "cpu",
                                "target": {
                                    "type": "Utilization",
                                    "averageUtilization": hpa_spec.target_cpu_utilization.unwrap()
                                    }
                                }
                            },
                            {
                            "type": "Resource",
                            "resource": {
                                "name": "memory",
                                "target": {
                                    "type": "Utilization",
                                    "averageUtilization": hpa_spec.target_memory_utilization.unwrap()
                                    }
                                }
                            }]
                        }
                    }))
            } else if hpa_spec.target_cpu_utilization.is_some() {
                serde_json::from_value(json!({
                    "apiVersion": K8S_AUTOSCALING_VERSION,
                    "kind": "HorizontalPodAutoscaler",
                    "metadata": {
                        "name": name,
                        "namespace": namespace,
                        "annotations": {
                            SERVICE_SCALER_MANAGED_ANNOTATION: "true",
                            SERVICE_SCALER_NOTE_KEY: SERVICE_SCALER_NOTE_VALUE
                        }
                    },
                    "spec": {
                        "scaleTargetRef": {
                            "apiVersion": "apps/v2beta2",
                            "kind": "Deployment",
                            "name": name
                        },
                        "minReplicas": hpa_spec.min_replicas,
                        "maxReplicas": hpa_spec.max_replicas,
                        "metrics": [
                            {
                            "type": "Resource",
                            "resource": {
                                "name": "cpu",
                                "target": {
                                    "type": "Utilization",
                                    "averageUtilization": hpa_spec.target_cpu_utilization.unwrap()
                                    }
                                }
                            }]
                        }
                    }))
            } else {
                serde_json::from_value(json!({
                    "apiVersion": K8S_AUTOSCALING_VERSION,
                    "kind": "HorizontalPodAutoscaler",
                    "metadata": {
                        "name": name,
                        "namespace": namespace,
                        "annotations": {
                            SERVICE_SCALER_MANAGED_ANNOTATION: "true",
                            SERVICE_SCALER_NOTE_KEY: SERVICE_SCALER_NOTE_VALUE
                        }
                    },
                    "spec": {
                        "scaleTargetRef": {
                            "apiVersion": "apps/v2beta2",
                            "kind": "Deployment",
                            "name": name
                        },
                        "minReplicas": hpa_spec.min_replicas,
                        "maxReplicas": hpa_spec.max_replicas,
                        "metrics": [
                            {
                            "type": "Resource",
                            "resource": {
                                "name": "cpu",
                                "target": {
                                    "type": "Utilization",
                                    "averageUtilization": DEFAULT_CPU_UTILIZATION
                                    }
                                }
                            }]
                        }
                    }))
            };
            let res = api.create(&PostParams::default(), &hpa.unwrap()).await;
            info!("[{}] hpa created!", key(namespace, name));
            self.patch_metadata(namespace, name, service_scaler_metadata, None).await.expect("patch_metadata errored!");
            res
        }
    }


    pub async fn patch(&self, namespace: &str, name: &str, hpa_spec: &HpaSpec) -> Result<HorizontalPodAutoscaler, Error> {
        let api: Api<HorizontalPodAutoscaler> = Api::namespaced(self.client.clone(), namespace);
        let mut metrics: Vec<MetricSpec> = vec![];
        // patch memory utilization
        match hpa_spec.target_memory_utilization {
            Some(mem_util) => metrics.push(MetricSpec {
                container_resource: None,
                external: None,
                object: None,
                pods: None,
                resource: Some(ResourceMetricSource {
                    name: "memory".to_string(),
                    target: MetricTarget {
                        average_utilization: Some(mem_util),
                        average_value: None,
                        type_: "Utilization".to_string(),
                        value: None,
                    },
                }),
                type_: "Resource".to_string(),
            }),
            None => info!("[{}] skipping memory utilization patch!", key(namespace, name))
        }

        // patch cpu utilization
        match hpa_spec.target_cpu_utilization {
            Some(cpu_util) => metrics.push(MetricSpec {
                container_resource: None,
                external: None,
                object: None,
                pods: None,
                resource: Some(ResourceMetricSource {
                    name: "cpu".to_string(),
                    target: MetricTarget {
                        average_utilization: Some(cpu_util),
                        average_value: None,
                        type_: "Utilization".to_string(),
                        value: None,
                    },
                }),
                type_: "Resource".to_string(),
            }),
            None => info!("[{}] skipping cpu utilization patch!", key(namespace, name))
        }

        // patch minReplicas and maxReplicas
        let spec = HorizontalPodAutoscalerSpec {
            behavior: None,
            max_replicas: hpa_spec.max_replicas,
            metrics: Some(metrics.to_vec()),
            min_replicas: Some(hpa_spec.min_replicas),
            scale_target_ref: CrossVersionObjectReference {
                api_version: Some(K8S_DEPLOYMENT_VERSION.to_string()),
                kind: "Deployment".to_string(),
                name: name.to_string(),
            },
        };

        // prepare patch
        let hpa_patch: Value = json!({
            "spec": spec
        });

        // apply patch
        let patch = Patch::Merge(&hpa_patch);
        let res = api.patch(name, &PatchParams::default(), &patch).await;
        info!("[{}] patched hpa!", key(namespace, name));
        res
    }

    pub async fn delete(&self, namespace: &str, name: &str) {
        let api: Api<HorizontalPodAutoscaler> = Api::namespaced(self.client.clone(), namespace);
        api.delete(name, &DeleteParams::default()).await
            .map(|_| ())
            .or_else(|err| match err {
                // Object is already deleted
                Error::Api(ErrorResponse { code: 404, .. }) => Ok(()),
                err => Err(err),
            }).unwrap_or_else(|_| panic!("[{}] deletion errored!", key(namespace, name)));
        info!("[{}] hpa deleted!", key(namespace, name));
    }

    pub async fn patch_metadata(&self, namespace: &str, name: &str, scaler_metadata: &ObjectMeta, hpa_metadata: Option<&ObjectMeta>) -> Result<(), Error> {
        let api: Api<HorizontalPodAutoscaler> = Api::namespaced(self.client.clone(), namespace);
        let mut labels: BTreeMap<String, String> = BTreeMap::new();
        if scaler_metadata.labels.is_some() {
            labels.append(&mut scaler_metadata.clone().labels.unwrap());
        }
        if hpa_metadata.is_some() && hpa_metadata.unwrap().labels.is_some() {
            labels.append(&mut hpa_metadata.unwrap().clone().labels.unwrap());
        }
        if !labels.is_empty() {
            let json_patch: Value = json!({
                "metadata": {
                    "labels": Some(labels)
                }
            });
            api.patch_metadata(name, &PatchParams::default(), &Patch::Merge(&json_patch)).await.expect("patch_metadata errored!");
            info!("[{}] patched metadata!", key(namespace, name));
        }
        Ok(())
    }
}