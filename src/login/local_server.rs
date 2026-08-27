#[cfg(not(target_os = "android"))]
use gettextrs::gettext;
#[cfg(not(target_os = "android"))]
use gtk::gio;
#[cfg(target_os = "android")]
use matrix_sdk::utils::local_server::QueryString;
#[cfg(not(target_os = "android"))]
use matrix_sdk::utils::local_server::{LocalServerBuilder, LocalServerResponse};
#[cfg(not(target_os = "android"))]
use tracing::error;
use url::Url;

#[cfg(target_os = "android")]
use crate::utils::android;
#[cfg(not(target_os = "android"))]
use crate::{APP_NAME, spawn_tokio};

/// The HTML template for the landing page.
#[cfg(not(target_os = "android"))]
const LOCAL_SERVER_LANDING_PAGE_TEMPLATE: &str = include_str!("local_server_landing_page.html");

/// The redirect URI OAuth 2.0 and Matrix SSO login use on Android.
///
/// A custom URI scheme rather than the loopback address other platforms use —
/// see [`RedirectHandle`]. The application id, `io.github.steeb_k.Commune`,
/// cannot be reused directly: a URI scheme is `ALPHA *( ALPHA / DIGIT / "+" /
/// "-" / "." )` (RFC 3986 §3.1) and does not allow the underscore the app id
/// has, which `url` confirms by refusing to parse it. This uses the domain the
/// app id is derived from instead, `steeb-k.github.io`, reversed and with its
/// hyphen intact — which is also, unlike the app id, not a workaround for
/// anything.
#[cfg(target_os = "android")]
pub(crate) const ANDROID_REDIRECT_URI: &str = "io.github.steeb-k.commune:/oauth2redirect";

/// A handle to wait for the end-user to be redirected back after logging in.
///
/// Everywhere but Android this is exactly [`LocalServerRedirectHandle`]: a
/// local HTTP server bound to loopback, which the browser is sent to and
/// which shows the landing page below.
///
/// No browser on Android will follow a redirect back to another application's
/// loopback listener, so there the redirect URI is a custom scheme instead,
/// and this wraps a channel fed by `Application::process_uri` when the
/// matching `Intent` arrives — see `doc/android.md`.
#[cfg(not(target_os = "android"))]
pub(super) use matrix_sdk::utils::local_server::LocalServerRedirectHandle as RedirectHandle;

/// See [`RedirectHandle`].
#[cfg(target_os = "android")]
#[derive(Debug)]
pub(super) struct RedirectHandle(tokio::sync::oneshot::Receiver<String>);

#[cfg(target_os = "android")]
impl std::future::IntoFuture for RedirectHandle {
    type Output = Option<QueryString>;
    type IntoFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let uri = self.0.await.ok()?;
            let query = Url::parse(&uri).ok()?.query()?.to_owned();
            Some(QueryString(query))
        })
    }
}

/// Spawn a local server for listening to redirects.
#[cfg(not(target_os = "android"))]
pub(super) async fn spawn_local_server() -> Result<(Url, RedirectHandle), ()> {
    spawn_tokio!(async move {
        LocalServerBuilder::new()
            .response(local_server_landing_page())
            .spawn()
            .await
    })
    .await
    .expect("task was not aborted")
    .map_err(|error| {
        error!("Could not spawn local server: {error}");
    })
}

/// Start waiting for the redirect to arrive as a custom-scheme `Intent`.
///
/// There is no server to spawn: the redirect URI is fixed, and
/// `Application::process_uri` delivers whatever comes back on it to
/// [`android::deliver_oauth_redirect`].
#[cfg(target_os = "android")]
pub(super) async fn spawn_local_server() -> Result<(Url, RedirectHandle), ()> {
    let uri = Url::parse(ANDROID_REDIRECT_URI).expect("Android redirect URI should be a valid URL");

    Ok((uri, RedirectHandle(android::await_oauth_redirect())))
}

/// The landing page, after the user performed the authentication and is
/// redirected to the local server.
#[cfg(not(target_os = "android"))]
fn local_server_landing_page() -> LocalServerResponse {
    let mut html = LOCAL_SERVER_LANDING_PAGE_TEMPLATE.to_owned();

    replace_html_variable(&mut html, "app_name", APP_NAME);
    replace_html_variable(&mut html, "title", &gettext("Authorization Completed"));
    replace_html_variable(
        &mut html,
        "message",
        &gettext(
            "The authorization step is complete. You can close this page and go back to Commune.",
        ),
    );
    replace_html_variable(&mut html, "icon", &svg_icon());

    LocalServerResponse::Html(html)
}

/// Replace the variable with the given name by the given value in the given
/// HTML.
///
/// The syntax for a variable is `@name@`. This is the same format as meson's
/// `configure_file` function.
///
/// Logs an error if the variable is not found.
#[cfg(not(target_os = "android"))]
fn replace_html_variable(html: &mut String, name: &str, value: &str) {
    let pattern = format!("@{name}@");

    // This is a programmer error.
    assert!(
        html.contains(&pattern),
        "Variable `{pattern}` should be present in HTML template"
    );

    *html = html.replace(&pattern, value);
}

/// Get the application SVG icon, ready to be embedded in HTML code.
///
/// Panics if the icon is not found or is invalid in some way.
#[cfg(not(target_os = "android"))]
fn svg_icon() -> String {
    // Load the icon from the application resources.
    let bytes = gio::resources_lookup_data(
        "/org/gnome/Fractal/icons/scalable/apps/org.gnome.Fractal.svg",
        gio::ResourceLookupFlags::NONE,
    )
    .expect("Application SVG icon should be present in GResources");

    // Convert the bytes to a string, since it should be SVG.
    let icon = String::from_utf8(bytes.to_vec())
        .expect("Application SVG icon content should be a UTF-8 string");

    // Remove the XML prologue, if there is one, to inline the SVG directly into the
    // HTML. Whether an icon has one says nothing about whether it can be drawn, so
    // it is not worth refusing to serve the page over.
    let icon = icon.trim();
    match icon.split_once("?>") {
        Some((prologue, rest)) if prologue.starts_with("<?xml") => rest.trim_start().to_owned(),
        _ => icon.to_owned(),
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use assert_matches2::assert_matches;
    use gtk::gio;
    use matrix_sdk::utils::local_server::LocalServerResponse;

    use super::local_server_landing_page;
    use crate::config::tests::BUILD_DIR;

    #[gtk::test]
    fn generate_local_server_landing_page() {
        let resources_file = format!("{BUILD_DIR}/data/resources/resources.gresource");
        let res = gio::Resource::load(resources_file).expect("Could not load gresource file");
        gio::resources_register(&res);

        // Check that the variables were all replaced.
        assert_matches!(local_server_landing_page(), LocalServerResponse::Html(html));
        assert!(!html.is_empty());
        assert!(!html.contains("@app_name@"));
        assert!(!html.contains("@title@"));
        assert!(!html.contains("@message@"));
        assert!(!html.contains("@icon@"));
    }
}
