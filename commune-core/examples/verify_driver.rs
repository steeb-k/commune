//! The already-verified device that answers a "verify this session"
//! request from the app: it recovers cross-signing with the recovery key,
//! waits for the request, walks the SAS to Done, and signs the new device.
//!
//! Usage: `cargo run --example verify_driver -- <homeserver> <user> <password>
//! <recovery-key>`
#![recursion_limit = "512"]

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::{
    Client,
    config::SyncSettings,
    encryption::verification::{SasState, Verification, VerificationRequestState},
    ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEvent,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let homeserver = args.next().expect("homeserver");
    let user = args.next().expect("user");
    let password = args.next().expect("password");
    let recovery_key = args.next().expect("recovery key");

    let client = Client::builder()
        .homeserver_url(&homeserver)
        .build()
        .await?;
    client
        .matrix_auth()
        .login_username(&user, &password)
        .initial_device_display_name("verify-driver")
        .await?;
    println!("logged in as {:?}", client.device_id());

    // Sync in the background from the start.
    let sync_client = client.clone();
    tokio::spawn(async move {
        let _ = sync_client.sync(SyncSettings::default()).await;
    });
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Become a verified device: the recovery key brings cross-signing.
    // Recovery needs the own identity known first; give sync a moment.
    loop {
        let user_id = client.user_id().expect("logged in").to_owned();
        if client
            .encryption()
            .get_user_identity(&user_id)
            .await?
            .is_some()
        {
            break;
        }
        println!("waiting for the own identity…");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    client
        .encryption()
        .recovery()
        .recover(&recovery_key)
        .await?;
    println!("recovered; signing this device with the recovered key…");

    // The private cross-signing keys arrive with recovery; wait until the
    // store holds the self-signing key, then sign our own device — that
    // is what makes this session verified.
    loop {
        let status = client.encryption().cross_signing_status().await;
        println!("cross-signing status: {status:?}");
        if status.is_some_and(|s| s.has_self_signing) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    let own_device = client
        .encryption()
        .get_own_device()
        .await?
        .expect("own device");
    own_device.verify().await?;

    {
        use matrix_sdk::encryption::VerificationState;

        let mut states = client.encryption().verification_state();
        while let Some(state) = states.next().await {
            println!("driver verification_state: {state:?}");
            if state == VerificationState::Verified {
                break;
            }
        }
    }
    println!("driver is verified; it can answer now");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let tx = Arc::new(tx);
    client.add_event_handler(
        move |ev: ToDeviceKeyVerificationRequestEvent, client: Client| {
            let tx = tx.clone();
            async move {
                if let Some(request) = client
                    .encryption()
                    .get_verification_request(&ev.sender, &ev.content.transaction_id)
                    .await
                {
                    let _ = tx.send(request);
                }
            }
        },
    );
    println!("waiting for a verification request…");

    let request = rx.recv().await.expect("a request arrives");
    println!("request received: {}", request.flow_id());
    request.accept().await?;

    let mut changes = request.changes();
    while let Some(state) = changes.next().await {
        match state {
            VerificationRequestState::Ready { .. } => {
                println!("ready; starting SAS");
                request.start_sas().await?;
            }
            VerificationRequestState::Transitioned {
                verification: Verification::SasV1(sas),
            } => {
                println!("transitioned to SAS");
                let mut sas_changes = sas.changes();
                while let Some(sas_state) = sas_changes.next().await {
                    match sas_state {
                        SasState::KeysExchanged { emojis, .. } => {
                            if let Some(emojis) = emojis {
                                let row: Vec<String> = emojis
                                    .emojis
                                    .iter()
                                    .map(|e| format!("{} {}", e.symbol, e.description))
                                    .collect();
                                println!("EMOJIS: {}", row.join(" | "));
                            }
                            sas.confirm().await?;
                            println!("confirmed on driver side");
                        }
                        SasState::Done { .. } => {
                            println!("VERIFICATION-DONE");
                            return Ok(());
                        }
                        SasState::Cancelled(info) => {
                            println!("CANCELLED: {}", info.reason());
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
            VerificationRequestState::Cancelled(info) => {
                println!("REQUEST-CANCELLED: {}", info.reason());
                return Ok(());
            }
            _ => {}
        }
    }

    Ok(())
}
