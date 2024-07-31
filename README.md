# Service Scaler
Introducing "Service Scaler”, a kubernetes operator which pro-actively monitors and controls the HPA object of a corresponding deployment enabling gradual scaling of workloads based on a time based configuration.

## The Configuration (CRD)
“Time-based” scaling is controlled by a custom configuration which looks like:
```yaml
    apiVersion: scaler.udaan.io/v1
    kind: ServiceScaler
    metadata:
      name: dummy-acorn-service
      namespace: prod
    spec:
      hpa:
        maxReplicas: 8
        minReplicas: 4
        targetCPUUtilization: 50
        targetMemoryUtilization: 75
      timeRangeSpec:
      - kind: ZonedTime
    	from: 16:00+05:30
        to: 00:00+05:30
        replicaSpec:
          hpa:
            minReplicas: 3
            targetMemoryUtilization: 0
      - kind: ZonedTime
        from: 00:00+05:30
        to: 08:00+05:30
        replicaSpec:
          hpa:
            minReplicas: 2
            targetMemoryUtilization: 0
  ```
- What does the above configuration mean?
    - between 16:00IST - 00:00IST `minReplicas` is overridden to 3 and `targetMemoryUtilzation` is removed.
    - between 00:00IST - 08:00IST `minReplicas` is overridden to 2 and `targetMemoryUtilzation` is removed.
    - defaults under `hpa:` are applied if no time range matches.

## The Control knobs
- hpa parameters
    - ``minReplicas``
    - ``maxReplicas``
    - ``targetCPUUtilization``  (`0` would mean removal of cpu based scaling)
    - ``targetMemoryUtiliization`` (`0` would mean removal of memory based scaling)
- `Defaults` under the `hpa:` section
- `Overrides` under `timeRangeSpec:` , specify any of the above parameter overrides which will be applied during the specified time range.
- Time range controls for `from:` and `to:`
    - ZonedTime: `HH:MM<tz-offset>` Ex:  `08:00+05:30`
    - ZonedDateTime: `rfc3339` format Ex: `2023-01-11T08:00:00+05:30`
- `Defaults` are applied when no time range matches.

## The Kill Switch

For those rare instances when things might not go as planned, a kill switch has been crafted. By adding a simple annotation to the HPA, the Service Scaler can be bypassed, putting control back in the hands of the user.
  ```yaml
  apiVersion: autoscaling/v1
  kind: HorizontalPodAutoscaler
  metadata:
    annotations:
      service-scaler.kubernetes.io/managed: "false" # <-- THIS LINE
    name: dummy-acorn-service
    namespace: prod
  spec:
    maxReplicas: 8
    minReplicas: 4
    scaleTargetRef:
      apiVersion: apps/v2beta2
      kind: Deployment
      name: dummy-acorn-service
    targetCPUUtilizationPercentage: 50
  ```
Once the above annotation is added, time based scaling is disabled for ``dummy-acorn-service``, users are expected to manually set hpa parameters of their choice.

## The “status” sub resource
The ``status`` block of the service scaler object shows the following:
1. What was the last active configuration of the scaler object?
2. When was the scaler object last updated?
3. Is there a time range spec match? (considering the current timestamp)
```yaml
status:
  lastKnownConfig:
    maxReplicas: 8
    minReplicas: 4
    targetCPUUtilization: 50
    targetMemoryUtilization: 75
  lastObservedGeneration: 1
  lastUpdatedTime: 2024-01-19T11:40Z+0530
  timeRangeMatch: false
```

## Installation
* Have a kubernetes cluster up and running.
* Install the CRD
    ```
    kubectl --context=<context> create -f servicescaler.scaler.udaan.io.yaml
    ```
* Ensure that rbac is setup (refer [rbac template](rbac.yaml))
* Build using ``cargo build``
* Run using ``RUST_LOG=info cargo run``
* Flexibility to watch a subset of hpas are provided via the ``LABEL_SELECTOR`` environment variable.

## Example
After installing the CRD and running the operator, to see the service scaler in action, let's create a sample deployment called ``dummy-bee-service`` with a service scaler object with the following specification:
1. ``default`` -  3 replicas
2. ``16:00 - 00:00`` - 2 replicas
3. ``00:00 - 08:00`` - 1 replica

* apply the example and examine if the replicas of ``dummy-bee-service`` are following the overrides.
```shell
kubectl --context=<context> apply -f example.yaml
```

## Points to note
- Do not specify “overlapping” time ranges as this will result in undefined behaviour.
- Refer [architecture diagram](architecture.png) to understand the mechanics of the operator.
- Battle-tested on kubernetes 1.16 and 1.22.
- For newer kubernetes clusters 
  - pin the following versions for ``kube`` and ``k8s-openapi``
    ```toml 
    kube = { version = "0.93.1", default-features = true, features = ["derive", "runtime", "config"]}
    k8s-openapi = { version = "0.20.0", features = ["latest"]} 
    ```
  - migrate from ``autoscaling/v2beta2`` to ``autoscaling/v2``
  - migrate from ``apps/v2beta2`` to ``apps/v1``

## Deployment Strategy (k8s)
1. Build the docker image.
2. Push the image to a container registry.
3. Setup a service account with the corresponding rolebinding objects with the required permissions.
4. Create a deployment object with the pushed image.

## Future Work
1. Helmify the operator for easier deployment.
2. Capability to "hibernate" services.

## References
1. [Custom Resource Definitions (CRD)](https://kubernetes.io/docs/concepts/extend-kubernetes/api-extension/custom-resources/)
2. [Horizontal pod autoscaling](https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/)
3. [Operator pattern](https://kubernetes.io/docs/concepts/extend-kubernetes/operator/)
