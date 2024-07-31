use kube::{CustomResource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Struct corresponding to the Specification (`spec`) part of the `Echo` resource, directly
/// reflects context of the `echoes.example.com.yaml` file to be found in this repository.
/// The `ServiceScaler` struct will be generated by the `CustomResource` derive macro.
#[derive(CustomResource, Serialize, Deserialize, Debug, PartialEq, Clone, JsonSchema)]
#[kube(
group = "scaler.udaan.io",
version = "v1",
kind = "ServiceScaler",
plural = "servicescalers",
derive = "PartialEq",
namespaced
)]
#[kube(status = "ServiceScalerStatus")]
pub struct ServiceScalerSpec {
    pub hpa: HpaSpec,
    #[serde(rename = "timeRangeSpec")]
    pub time_range_spec: Vec<TimeRangeSpec>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, PartialEq, Clone)]
pub enum TimeRangeType {
    ZonedTime,
    ZonedDateTime,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, PartialEq, Clone)]
pub struct HpaSpec {
    #[serde(rename = "minReplicas")]
    pub min_replicas: i32,
    #[serde(rename = "maxReplicas")]
    pub max_replicas: i32,
    #[serde(rename = "targetCPUUtilization")]
    pub target_cpu_utilization: Option<i32>,
    #[serde(rename = "targetMemoryUtilization")]
    pub target_memory_utilization: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, PartialEq, Clone)]
pub struct HpaOverrideSpec {
    #[serde(rename = "minReplicas")]
    pub min_replicas: Option<i32>,
    #[serde(rename = "maxReplicas")]
    pub max_replicas: Option<i32>,
    #[serde(rename = "targetCPUUtilization")]
    pub target_cpu_utilization: Option<i32>,
    #[serde(rename = "targetMemoryUtilization")]
    pub target_memory_utilization: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, PartialEq, Clone)]
pub struct ReplicaSpec {
    pub hpa: HpaOverrideSpec,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, PartialEq, Clone)]
pub struct TimeRangeSpec {
    pub kind: TimeRangeType,
    pub from: String,
    pub to: String,
    #[serde(rename = "replicaSpec")]
    pub replica_spec: ReplicaSpec,
}

#[derive(Serialize, Deserialize, Debug, JsonSchema, PartialEq, Clone)]
pub struct ServiceScalerStatus {
    #[serde(rename = "timeRangeMatch")]
    pub time_range_match: bool,
    #[serde(rename = "lastObservedGeneration")]
    pub last_observed_generation: Option<i64>,
    #[serde(rename = "lastKnownConfig")]
    pub last_known_config: HpaOverrideSpec,
    #[serde(rename = "lastUpdatedTime")]
    pub last_updated_time: String,

}