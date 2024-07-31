use crate::crd::{HpaOverrideSpec, HpaSpec, ServiceScaler};
use k8s_openapi::api::autoscaling::v2beta2::{HorizontalPodAutoscaler};
use kube::{Error, Resource};
use kube::error::DiscoveryError;
use log::info;
use crate::hpa::HpaOperator;
use crate::util::{key, patch_status, SERVICE_SCALER_MANAGED_ANNOTATION, step, timestamp_match};

pub struct Scale {
    pub(crate) hpa_operator: HpaOperator,
}

impl Scale {
    fn early_exit(&self, hpa: &HorizontalPodAutoscaler, target_hpa_spec: &HpaOverrideSpec) -> bool {
        // kill switch
        if hpa.metadata.annotations.is_some() {
            let kill_switch = match hpa.metadata.annotations.clone().unwrap().get(SERVICE_SCALER_MANAGED_ANNOTATION) {
                Some(kill_switch_value) => {
                    kill_switch_value.to_lowercase() == "false"
                }
                None => {
                    true
                }
            };
            if kill_switch {
                return kill_switch;
            }
        }

        // current == desired
        let metrics = hpa.spec.clone().unwrap().metrics.unwrap();
        let cpu_metric = metrics.iter().filter(|metric| metric.resource.clone().unwrap().name == "cpu").last();
        let mem_metric = metrics.iter().filter(|metric| metric.resource.clone().unwrap().name == "memory").last();

        let cpu_util = if cpu_metric.is_some() {
            cpu_metric.unwrap().resource.clone().unwrap().target.average_utilization
        } else {
            None
        };
        let mem_util = if mem_metric.is_some() {
            mem_metric.unwrap().resource.clone().unwrap().target.average_utilization
        } else {
            None
        };

        let min_replicas_equivalence = hpa.spec.clone().unwrap().min_replicas == target_hpa_spec.min_replicas;
        let max_replicas_equivalence = hpa.spec.clone().unwrap().max_replicas == target_hpa_spec.max_replicas.unwrap();
        let target_cpu_util_equivalence = cpu_util == target_hpa_spec.target_cpu_utilization;
        let target_mem_util_equivalence = mem_util == target_hpa_spec.target_memory_utilization;

        return min_replicas_equivalence && max_replicas_equivalence && target_cpu_util_equivalence && target_mem_util_equivalence;
    }


    pub async fn act(&self, namespace: &str, name: &str, service_scaler: &ServiceScaler) -> Result<HorizontalPodAutoscaler, Error> {
        if service_scaler.spec.hpa.min_replicas == service_scaler.spec.hpa.max_replicas {
            info!("[{}] minReplicas==maxReplicas detected! deleting hpa!", key(namespace, name));
            self.hpa_operator.delete(namespace, name).await;
            return Err(Error::Discovery(DiscoveryError::MissingKind("minReplicas == maxReplicas".to_string())));
        }
        // get current hpa
        let hpa = match self.hpa_operator.get(namespace, name).await {
            Ok(hpa) => hpa,
            Err(_) => {
                // someone directly deletes hpa, create it back
                info!("[{}] accidental hpa deletion detected! recreating hpa with default spec!", key(namespace, name));
                // assuming create does not break
                self.hpa_operator.create(namespace, name, &service_scaler.spec.hpa, &service_scaler.meta()).await.unwrap()
            }
        };

        // get override spec
        let range_match = service_scaler.spec.time_range_spec.iter().filter(|range_spec| {
            timestamp_match(&range_spec.from, &range_spec.to, &range_spec.kind)
        }).last();

        let default_hpa_spec = service_scaler.clone().spec.hpa;
        let mut hpa_override_spec = match range_match {
            Some(range_match) => {
                info!("[{}] from_ts:{} to_ts:{} ts_match:{}", key(namespace, name), range_match.from, range_match.to, "true");
                range_match.clone().replica_spec.hpa
            }
            None => {
                info!("[{}] ts_match:{}", key(namespace, name), "false");
                HpaOverrideSpec {
                    min_replicas: Some(default_hpa_spec.min_replicas),
                    max_replicas: Some(default_hpa_spec.max_replicas),
                    target_cpu_utilization: default_hpa_spec.target_cpu_utilization,
                    target_memory_utilization: default_hpa_spec.target_memory_utilization,
                }
            }
        };

        // prepare final [HpaSpec] patch

        // minReplicas step shenanigans
        let curr_min_replicas = hpa.spec.clone().unwrap().min_replicas.unwrap();
        if !hpa_override_spec.min_replicas.is_some() {
            hpa_override_spec.min_replicas = Some(default_hpa_spec.max_replicas)
        }
        hpa_override_spec.min_replicas = Some(step(curr_min_replicas, default_hpa_spec.min_replicas, hpa_override_spec.min_replicas.unwrap(), &service_scaler.spec.time_range_spec, false).unwrap());
        info!("[{}] minReplicas - from:{} to:{}", key(namespace, name), curr_min_replicas, hpa_override_spec.min_replicas.unwrap());

        //maxReplicas step shenanigans
        let curr_max_replicas = hpa.spec.clone().unwrap().max_replicas;
        if !hpa_override_spec.max_replicas.is_some() {
            hpa_override_spec.max_replicas = Some(default_hpa_spec.max_replicas)
        }
        hpa_override_spec.max_replicas = Some(step(curr_max_replicas, default_hpa_spec.max_replicas, hpa_override_spec.max_replicas.unwrap(), &service_scaler.spec.time_range_spec, true).unwrap());
        info!("[{}] maxReplicas - from:{} to:{}", key(namespace, name), curr_max_replicas, hpa_override_spec.max_replicas.unwrap());
        // targetCPUUtil
        if !hpa_override_spec.target_cpu_utilization.is_some() {
            if default_hpa_spec.target_cpu_utilization.is_some() {
                hpa_override_spec.target_cpu_utilization = default_hpa_spec.target_cpu_utilization
            }
        }
        if hpa_override_spec.target_cpu_utilization.is_some() {
            if hpa_override_spec.target_cpu_utilization.unwrap() == 0 {
                hpa_override_spec.target_cpu_utilization = None
            }
        }

        // targetMemoryUtil
        if !hpa_override_spec.target_memory_utilization.is_some() {
            if default_hpa_spec.target_memory_utilization.is_some() {
                hpa_override_spec.target_memory_utilization = default_hpa_spec.target_memory_utilization
            }
        }
        if hpa_override_spec.target_memory_utilization.is_some() {
            if hpa_override_spec.target_memory_utilization.unwrap() == 0 {
                hpa_override_spec.target_memory_utilization = None
            }
        }

        // early exit
        if self.early_exit(&hpa, &hpa_override_spec) {
            patch_status(self.hpa_operator.client.clone(), namespace, name, range_match.is_some(), "no-op", &hpa_override_spec).await.expect("patch_status errored!");
            info!("[{}] early-exit no-op!", key(namespace, name));
            return Ok(hpa);
        }

        let res = self.hpa_operator.patch(namespace, name, &HpaSpec {
            min_replicas: hpa_override_spec.min_replicas.unwrap(),
            max_replicas: hpa_override_spec.max_replicas.unwrap(),
            target_cpu_utilization: hpa_override_spec.target_cpu_utilization,
            target_memory_utilization: hpa_override_spec.target_memory_utilization,
        }).await;
        patch_status(self.hpa_operator.client.clone(), namespace, name, range_match.is_some(), "patch", &hpa_override_spec).await.expect("patch_status errored!");
        return res;
    }
}