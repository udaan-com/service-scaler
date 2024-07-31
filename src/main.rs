use std::sync::Arc;
use std::io::Write;
use chrono::{Local};
use env_logger::Builder;
use kube::{Api, Client, Resource, ResourceExt};
use kube::runtime::Controller;
use kube::runtime::controller::Action;
use kube::runtime::watcher::Config;
use tokio::time::Duration;
use futures::stream::StreamExt;
use crate::crd::{ServiceScaler};
use crate::hpa::HpaOperator;
use crate::scale::Scale;
use crate::util::{key, LABEL_SELECTOR, RECONCILIATION_PERIOD};
use log::{error, info, LevelFilter};

pub mod crd;
mod finalizer;
mod hpa;
mod util;
mod scale;

#[tokio::main]
async fn main() {
    // init logger
    Builder::new()
        .format(|buf, record| {
            writeln!(buf,
                     "{} [{}] - {}",
                     Local::now().fixed_offset().format("%d-%m-%yT%H:%MZ%z"),
                     record.level(),
                     record.args()
            )
        })
        .filter(None, LevelFilter::Info)
        .init();
    // client creation
    let kubernetes_client: Client = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable.");


    // Preparation of resources used by the `kube_runtime::Controller`
    let crd_api: Api<ServiceScaler> = Api::all(kubernetes_client.clone());
    let context: Arc<ContextData> = Arc::new(ContextData::new(kubernetes_client.clone()));

    // The controller comes from the `kube_runtime` crate and manages the reconciliation process.
    // It requires the following information:
    // - `kube::Api<T>` this controller "owns". In this case, `T = ServiceScaler`, as this controller owns the `ServiceScaler` resource,
    // - `kube::runtime::watcher::Config` can be adjusted for precise filtering of `ServiceScaler` resources before the actual reconciliation, e.g. by label,
    // - `reconcile` function with reconciliation logic to be called each time a resource of `ServiceScaler` kind is created/updated/deleted,
    // - `on_error` function to call whenever reconciliation fails.
    Controller::new(crd_api.clone(), Config::default().labels(LABEL_SELECTOR.as_str()))
        .run(reconcile, on_error, context)
        .for_each(|reconciliation_result| async move {
            match reconciliation_result {
                Ok(service_scaler_resource) => {
                    info!("Reconciliation successful! resource: {:?}", service_scaler_resource);
                }
                Err(reconciliation_err) => {
                    error!("Reconciliation error: {:?}", reconciliation_err)
                }
            }
        })
        .await;
}

/// Context injected with each `reconcile` and `on_error` method invocation.
struct ContextData {
    /// Kubernetes client to make Kubernetes API requests with. Required for K8S resource management.
    client: Client,
}

impl ContextData {
    /// Constructs a new instance of ContextData.
    ///
    /// # Arguments:
    /// - `client`: A Kubernetes client to make Kubernetes REST API requests with. Resources
    /// will be created and deleted with this client.
    pub fn new(client: Client) -> Self {
        ContextData { client }
    }
}

#[derive(Debug)]
enum ServiceScalerAction {
    Create,
    Update,
    Delete,
}

fn classify_action(service_scaler: &ServiceScaler) -> ServiceScalerAction {
    return if service_scaler.meta().deletion_timestamp.is_some() {
        ServiceScalerAction::Delete
    } else if service_scaler.meta()
        .finalizers
        .as_ref()
        .map_or(true, |finalizers| finalizers.is_empty())
    {
        ServiceScalerAction::Create
    } else {
        ServiceScalerAction::Update
    };
}

async fn reconcile(service_scaler: Arc<ServiceScaler>, context: Arc<ContextData>) -> Result<Action, Error> {
    let client: Client = context.client.clone();
    let namespace: String = match service_scaler.namespace() {
        None => {
            // If there is no namespace defined, reconciliation ends with an error immediately.
            return Err(Error::UserInputError(
                "Expected ServiceScaler resource to be namespaced. Can't deploy to an unknown namespace."
                    .to_owned(),
            ));
        }
        Some(namespace) => namespace,
    };
    let name = service_scaler.name_any();
    let hpa_operator = HpaOperator { client: client.clone() };
    let scale_operator = Scale { hpa_operator: hpa_operator.clone() };
    return match classify_action(&service_scaler) {
        ServiceScalerAction::Create => {
            finalizer::add(client.clone(), &namespace, &name).await?;
            info!("[{}] added finalizers!", key(&namespace, &name));
            hpa_operator.create(&namespace, &name, &service_scaler.spec.hpa, &service_scaler.meta()).await?;
            info!("[{}] Reconciled object! action: {}",  key(&namespace, &name), "CREATE");
            Ok(Action::requeue(Duration::from_secs(RECONCILIATION_PERIOD)))
        }
        ServiceScalerAction::Delete => {
            hpa_operator.delete(&namespace, &name).await;
            finalizer::delete(client, &namespace, &name).await?;
            info!("[{}] deleted finalizers!", key(&namespace, &name));
            info!("[{}] Reconciled object! action: {}",  key(&namespace, &name), "DELETE");
            // Makes no sense to delete after a successful delete, as the resource is gone
            Ok(Action::await_change())
        }
        ServiceScalerAction::Update => {
            let scale_op = scale_operator.act(&namespace, &name, &service_scaler).await;
            match scale_op {
                Ok(_scale_op) => {
                    info!("[{}] Reconciled object! action: {}",  key(&namespace, &name), "UPDATE/NO-OP");
                }
                Err(e) => {
                    error!("[{}] Reconciled object! action: {} err: {:?}",  key(&namespace, &name), "UPDATE/NO-OP", e);
                }
            }

            Ok(Action::requeue(Duration::from_secs(RECONCILIATION_PERIOD)))
        }
    };
}

/// Actions to be taken when a reconciliation fails - for whatever reason.
/// Prints out the error to `stderr` and requeues the resource for another reconciliation after
/// 'x' seconds.
///
/// # Arguments
/// - `ServiceScaler`: The erroneous resource.
/// - `error`: A reference to the `kube::Error` that occurred during reconciliation.
/// - `_context`: Unused argument. Context Data "injected" automatically by kube-rs.
fn on_error(service_scaler: Arc<ServiceScaler>, error: &Error, _context: Arc<ContextData>) -> Action {
    error!("Reconciliation error:\n{:?}.\n{:?}", error, service_scaler);
    Action::requeue(Duration::from_secs(RECONCILIATION_PERIOD))
}


/// All errors possible to occur during reconciliation
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Any error originating from the `kube-rs` crate
    #[error("Kubernetes reported error: {source}")]
    KubeError {
        #[from]
        source: kube::Error,
    },
    /// Error in user input or ServiceScaler resource definition, typically missing fields.
    #[error("Invalid ServiceScaler CRD: {0}")]
    UserInputError(String),
}