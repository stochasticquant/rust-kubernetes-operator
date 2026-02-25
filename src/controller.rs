use futures::StreamExt;
use kube::runtime::controller::Controller;
use kube::runtime::watcher::Config;
use kube::{Api, Client};
use std::sync::Arc;

use crate::crd::GuardianPolicy;
use crate::reconcile::{error_policy, reconcile};

pub async fn run_controller(client: Client) -> Result<(), Box<dyn std::error::Error>> {
    let api: Api<GuardianPolicy> = Api::all(client);

    Controller::new(api, Config::default())
        .run(reconcile, error_policy, Arc::new(()))
        .for_each(|res| async move {
            match res {
                Ok(obj) => println!("Reconciled: {:?}", obj),
                Err(e) => eprintln!("Reconcile error: {:?}", e),
            }
        })
        .await;

    Ok(())
}
