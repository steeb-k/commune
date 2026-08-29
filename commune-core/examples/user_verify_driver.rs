//! The other user in a user-to-user verification: logs in, bootstraps
//! cross-signing (so the app may request verification toward it), waits
//! for the in-room `m.key.verification.request`, walks the SAS to Done.
//!
//! Usage: `cargo run --example user_verify_driver -- <homeserver> <user>
//! <password>`
#![recursion_limit = "512"]

use std::sync::Arc;

use futures_util::StreamExt;
use matrix_sdk::{
    Client,
    config::SyncSettings,
    encryption::verification::{SasState, Verification, VerificationRequestState},
    ruma::{
        api::client::uiaa,
        events::room::message::{MessageType, OriginalSyncRoomMessageEvent},
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let homeserver = args.next().expect("homeserver");
    let user = args.next().expect("user");
    let password = args.next().expect("password");

    let client = Client::builder()
        .homeserver_url(&homeserver)
        .build()
        .await?;
    client
        .matrix_auth()
        .login_username(&user, &password)
        .initial_device_display_name("user-verify-driver")
        .await?;
    println!("logged in as {:?}", client.device_id());

    let sync_client = client.clone();
    tokio::spawn(async move {
        let _ = sync_client.sync(SyncSettings::default()).await;
    });
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Cross-signing, so the requester finds an identity to verify.
    if client
        .encryption()
        .cross_signing_status()
        .await
        .is_none_or(|status| !status.has_self_signing)
    {
        println!("bootstrapping cross-signing…");
        if let Err(bootstrap_error) = client.encryption().bootstrap_cross_signing(None).await {
            let Some(response) = bootstrap_error.as_uiaa_response() else {
                return Err(bootstrap_error.into());
            };
            let mut auth = uiaa::Password::new(
                uiaa::MatrixUserIdentifier::new(user.clone()).into(),
                password.clone(),
            );
            auth.session = response.session.clone();
            client
                .encryption()
                .bootstrap_cross_signing(Some(uiaa::AuthData::Password(auth)))
                .await?;
        }
        println!("cross-signing bootstrapped");
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let tx = Arc::new(tx);
    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, client: Client| {
        let tx = tx.clone();
        async move {
            if !matches!(ev.content.msgtype, MessageType::VerificationRequest(_)) {
                return;
            }
            if client.user_id().is_some_and(|own| own == ev.sender) {
                return;
            }
            if let Some(request) = client
                .encryption()
                .get_verification_request(&ev.sender, &ev.event_id)
                .await
            {
                let _ = tx.send(request);
            }
        }
    });
    println!("waiting for an in-room verification request…");

    let request = rx.recv().await.expect("a request arrives");
    println!("request received: {}", request.flow_id());
    // Subscribe before accepting: the Ready state can land between the
    // accept and a later subscription and never fire again.
    let mut changes = request.changes();
    request.accept().await?;
    if matches!(request.state(), VerificationRequestState::Ready { .. }) {
        println!("already ready; starting SAS");
        request.start_sas().await?;
    }
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
                sas.accept().await?;
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
