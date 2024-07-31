use std::string::ToString;
use crate::crd::{HpaOverrideSpec, ServiceScaler, ServiceScalerStatus, TimeRangeSpec, TimeRangeType};
use chrono::prelude::*;
use chrono::*;
use kube::{Api, Client, Resource};
use kube::api::{Patch, PatchParams};
use log::{error, info, warn};
use crate::Error;
use std::env;
use lazy_static::lazy_static;

// runtime constants
lazy_static! {
    pub static ref LABEL_SELECTOR: String = env::var("LABEL_SELECTOR").unwrap_or("".to_string());
}

/// in seconds
pub static RECONCILIATION_PERIOD: u64 = 300;

/// labels
pub const SERVICE_SCALER_MANAGED_ANNOTATION: &str = "service-scaler.kubernetes.io/managed";
pub const SERVICE_SCALER_NOTE_KEY: &str = "service-scaler.kubernetes.io/note";
pub const SERVICE_SCALER_NOTE_VALUE: &str = "DO-NOT-EDIT-THIS--EDIT-SERVICE-SCALER-INSTEAD";

pub fn key(namespace: &str, name: &str) -> String {
    return [namespace, name].join("/");
}

fn parse_zoned_time_str(ts: &str) -> DateTime<FixedOffset> {
    let today = Local::now().format("%d-%m-%y").to_string();
    return DateTime::parse_from_str((today + ts).as_str(), "%d-%m-%y %H:%M%:z").expect("Failed to parse from ZonedDateTime");
}

fn parse_zoned_date_time_str(ts: &str) -> DateTime<FixedOffset> {
    return DateTime::parse_from_rfc3339(ts).expect("Failed to parse from ZonedDateTime");
}

pub fn timestamp_match(from: &str, to: &str, kind: &TimeRangeType) -> bool {
    let curr_ts = Local::now().fixed_offset();
    return match kind {
        TimeRangeType::ZonedTime => {
            let from_ts = parse_zoned_time_str(from);
            let mut to_ts = parse_zoned_time_str(to);
            if to_ts < from_ts {
                to_ts = to_ts + Duration::days(1);
            }
            info!("Trying from_ts:{} curr_ts:{} to_ts:{}", from_ts, curr_ts, to_ts);
            (curr_ts > from_ts) && (curr_ts < to_ts)
        }
        TimeRangeType::ZonedDateTime => {
            let from_ts = parse_zoned_date_time_str(from);
            let to_ts = parse_zoned_date_time_str(to);
            return (curr_ts > from_ts) && (curr_ts < to_ts);
        }
    };
}

fn diff_from_now(ts: &str, kind: &TimeRangeType) -> i64 {
    // get current time
    let curr_ts = Local::now().fixed_offset();
    return match kind {
        TimeRangeType::ZonedTime => {
            let mut ts = parse_zoned_time_str(ts);
            if ts < curr_ts {
                ts = ts + Duration::days(1);
            }
            (ts - curr_ts).num_seconds()
        }
        TimeRangeType::ZonedDateTime => {
            let ts = parse_zoned_date_time_str(ts);
            (ts - curr_ts).num_seconds()
        }
    };
}

/// determines the "jump" factor and the next nearest target minReplicas/maxReplicas according to the distance from the nearest matching interval
pub fn determine_next_target(default: i32, time_range_spec: &Vec<TimeRangeSpec>, is_max: bool) -> (i32, Option<i32>) {
    let mut min_diff = i64::MAX;
    let mut next_nearest_target = None;
    for time_range in time_range_spec {
        let diff_from_from = diff_from_now(&time_range.from, &time_range.kind);
        let diff_from_to = diff_from_now(&time_range.to, &time_range.kind);

        if diff_from_from < diff_from_to {
            // incase from and to are equally placed take diff_from_from
            if diff_from_from <= min_diff {
                min_diff = diff_from_from;
                if !is_max {
                    next_nearest_target = time_range.replica_spec.hpa.min_replicas
                } else {
                    next_nearest_target = time_range.replica_spec.hpa.max_replicas
                }
            }
        } else {
            if diff_from_to < min_diff {
                min_diff = diff_from_to;
                next_nearest_target = Some(default)
            }
        }
    }
    return ((min_diff / RECONCILIATION_PERIOD as i64).max(1i64) as i32, next_nearest_target);
}


/// steps from [curr] to [next nearest target], falls back to default if no next target found, falls back to fallback if not within the "ramp-up/down" duration
/// fallback is
///   * default: if ts_match=false
///   * actual_target: if ts_match=true
/// ramp-up/down duration: 30min ~(6 intervals)
pub fn step(curr: i32, default: i32, fallback: i32, time_range_spec: &Vec<TimeRangeSpec>, is_max: bool) -> Result<i32, Error> {
    let (jump_interval, next_target) = determine_next_target(default, time_range_spec, is_max);
    if !next_target.is_some() {
        warn!("unable to determine next nearest target falling back to default!");
        return Ok(default);
    }
    info!("intervals_left:{} next_target:{}", jump_interval, next_target.unwrap());
    if jump_interval > 6 {
        // falls back to default if ts_match=false, else it falls back to actual target
        warn!("greater than ramp up/down duration! falling back to {}", fallback);
        return Ok(fallback);
    }
    if next_target.unwrap() == curr {
        warn!("current already at target!");
        return Ok(curr);
    }

    let step = (next_target.unwrap() - curr) / jump_interval;
    return if curr > next_target.unwrap() {
        Ok((curr + step).max(next_target.unwrap()))
    } else {
        Ok((curr + step).min(next_target.unwrap()))
    };
}

pub async fn patch_status(client: Client, namespace: &str, name: &str, time_range_match: bool, _action: &str, hpa_spec: &HpaOverrideSpec) -> Result<(), Error> {
    let api: Api<ServiceScaler> = Api::namespaced(client, namespace);
    let curr_ts = Local::now().fixed_offset();
    match api.get(name).await {
        Ok(service_scaler) => {
            let mut patch = service_scaler.clone();
            patch.status = Some(ServiceScalerStatus {
                time_range_match: time_range_match,
                last_observed_generation: service_scaler.meta().generation,
                last_known_config: hpa_spec.clone(),
                last_updated_time: curr_ts.format("%Y-%m-%dT%H:%MZ%z").to_string(),
            });
            api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch)).await.expect("patch_status errored!");
            info!("[{}] patched status!", key(namespace, name));
            Ok(())
        }
        Err(_) => {
            error!("[{}] skipping status patch! ServiceScaler not found!", key(namespace, name));
            Ok(())
        }
    }
}