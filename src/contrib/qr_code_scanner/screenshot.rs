use ashpd::desktop::screenshot;
use gtk::gio;
use gtk::prelude::*;
use matrix_sdk::encryption::verification::QrVerificationData;

pub async fn capture(root: &gtk::Root) -> Option<QrVerificationData> {
    let identifier = ashpd::WindowIdentifier::from_native(root).await;
    let uri = screenshot::take(&identifier, true, true).await.ok()?;
    let screenshot = gio::File::for_uri(&uri);
    let (data, _) = screenshot.load_contents(gio::NONE_CANCELLABLE).ok()?;
    let image = image::load_from_memory(&data).ok()?;

    QrVerificationData::from_image(image).ok()
}
