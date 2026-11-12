use std::{cell::RefCell, path::PathBuf, process::Command, rc::Rc};

use gtk::prelude::*;
use sha2::{Digest, Sha256};

fn label(locale: &serde_json::Value, key: &str) -> String {
    locale
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or(key)
        .to_owned()
}

fn state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|base| base.join("vortal"))
}

fn setting_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|base| base.join("vortal").join(name))
}

fn setting_enabled(name: &str) -> bool {
    setting_path(name)
        .and_then(|path| std::fs::read(path).ok())
        .is_some_and(|value| value == b"1")
}

fn set_setting(name: &str, enabled: bool) -> anyhow::Result<()> {
    let path = setting_path(name).ok_or_else(|| anyhow::anyhow!("config directory unavailable"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, if enabled { b"1" } else { b"0" })?;
    Ok(())
}

fn set_startup(enabled: bool) -> anyhow::Result<()> {
    let action = if enabled { "enable" } else { "disable" };
    anyhow::ensure!(
        Command::new("systemctl")
            .args(["--user", action, "vortal-input.service"])
            .status()?
            .success(),
        "could not update receiver startup"
    );
    set_setting("startup-enabled", enabled)
}

fn device_id(certificate: &[u8]) -> String {
    let hash = Sha256::digest(certificate);
    let number = u32::from_be_bytes([hash[2], hash[3], hash[4], hash[5]]) % 1_000_000;
    format!(
        "{}{} {number:06}",
        (b'A' + hash[0] % 26) as char,
        (b'A' + hash[1] % 26) as char
    )
}

fn ensure_linux_identity() -> anyhow::Result<()> {
    let state = state_dir().ok_or_else(|| anyhow::anyhow!("state directory unavailable"))?;
    let identity = state.join("identity");
    let key = identity.join("key.der");
    let certificate = identity.join("cert.der");
    if key.exists() && certificate.exists() {
        return Ok(());
    }
    anyhow::ensure!(
        !key.exists() && !certificate.exists(),
        "identity files are incomplete"
    );
    let device_name = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty() && name.len() <= 128)
        .unwrap_or_else(|| "linux".to_owned());
    vortal_net::generate_identity(&identity, &device_name)?;
    anyhow::ensure!(
        Command::new("systemctl")
            .args(["--user", "restart", "vortal-input.service"])
            .output()?
            .status
            .success(),
        "could not start the Linux receiver"
    );
    Ok(())
}

fn apply_style() -> anyhow::Result<()> {
    let provider = gtk::CssProvider::new();
    let css = concat!(
        include_str!("../ui/native-tokens.css"),
        include_str!("../ui/native-app.css")
    );
    provider.load_from_data(css.as_bytes())?;
    let screen = gdk_screen()?;
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    Ok(())
}

fn gdk_screen() -> anyhow::Result<gtk::gdk::Screen> {
    gtk::gdk::Screen::default().ok_or_else(|| anyhow::anyhow!("display unavailable"))
}

fn build_setting(
    content: &gtk::Box,
    locale: &serde_json::Value,
    title: &str,
    name: &str,
    initial: bool,
    status: &gtk::Label,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.style_context().add_class("setting-row");
    let text = gtk::Label::new(Some(&label(locale, title)));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    let toggle = gtk::Switch::new();
    toggle.set_active(initial);
    toggle.set_valign(gtk::Align::Center);
    row.pack_start(&text, true, true, 0);
    row.pack_end(&toggle, false, false, 0);
    content.pack_start(&row, false, false, 0);

    let name = name.to_owned();
    let status = status.clone();
    let saved = label(locale, "inputSettingsSaved");
    toggle.connect_state_set(move |_, enabled| {
        let result = if name == "startup-enabled" {
            set_startup(enabled)
        } else {
            set_setting(&name, enabled)
        };
        match result {
            Ok(()) => {
                status.set_text(&saved);
                gtk::glib::Propagation::Proceed
            }
            Err(error) => {
                status.set_text(&error.to_string());
                gtk::glib::Propagation::Stop
            }
        }
    });
}

fn refresh_pairing(
    locale: &serde_json::Value,
    id_label: &gtk::Label,
    create_button: &gtk::Button,
    request_box: &gtk::Box,
    peer_label: &gtk::Label,
    fingerprint_label: &gtk::Label,
    nonce: &Rc<RefCell<Option<String>>>,
) {
    let state = state_dir();
    let certificate = state
        .as_ref()
        .and_then(|base| std::fs::read(base.join("identity/cert.der")).ok());
    if let Some(certificate) = certificate {
        id_label.set_text(&device_id(&certificate));
        create_button.hide();
    } else {
        id_label.set_text("—");
        create_button.show();
    }
    let pending = state
        .and_then(|base| std::fs::read(base.join("pairing-request.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let candidate = pending
        .as_ref()
        .and_then(|value| value.get("nonce"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    *nonce.borrow_mut() = candidate.map(str::to_owned);
    if let (Some(pending), Some(_)) = (pending.as_ref(), candidate) {
        peer_label.set_text(
            pending
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        );
        fingerprint_label.set_text(
            pending
                .get("fingerprint")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        );
        request_box.show();
    } else {
        peer_label.set_text(&label(locale, "remoteIdle"));
        request_box.hide();
    }
}

fn decide_pairing(nonce: &str, action: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        nonce.len() == 32 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid pairing request"
    );
    let state = state_dir().ok_or_else(|| anyhow::anyhow!("state directory unavailable"))?;
    let pending: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.join("pairing-request.json"))?)?;
    anyhow::ensure!(
        pending.get("nonce").and_then(serde_json::Value::as_str) == Some(nonce),
        "pairing request expired"
    );
    std::fs::write(state.join("pairing-decision"), format!("{nonce}:{action}"))?;
    Ok(())
}

fn build_window(application: &gtk::Application, locale: serde_json::Value) {
    let window = gtk::ApplicationWindow::new(application);
    window.set_title("Vortal");
    window.set_default_size(640, 510);
    window.set_resizable(true);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.style_context().add_class("app-content");
    let title = gtk::Label::new(Some("Vortal"));
    title.style_context().add_class("app-title");
    title.set_xalign(0.0);
    content.pack_start(&title, false, false, 0);

    let id_card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    id_card.style_context().add_class("card");
    let id_heading = gtk::Label::new(Some(&label(&locale, "myId")));
    id_heading.set_xalign(0.0);
    let id_label = gtk::Label::new(Some("—"));
    id_label.style_context().add_class("device-id");
    id_label.set_xalign(0.0);
    id_label.set_selectable(true);
    let create_button = gtk::Button::with_label(&label(&locale, "createIdentity"));
    id_card.pack_start(&id_heading, false, false, 0);
    id_card.pack_start(&id_label, false, false, 0);
    id_card.pack_start(&create_button, false, false, 0);
    content.pack_start(&id_card, false, false, 0);

    let request_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    request_box.style_context().add_class("card");
    let request_heading = gtk::Label::new(Some(&label(&locale, "pairingRequest")));
    request_heading.set_xalign(0.0);
    let peer_label = gtk::Label::new(None);
    peer_label.set_xalign(0.0);
    let fingerprint_label = gtk::Label::new(None);
    fingerprint_label.set_xalign(0.0);
    fingerprint_label.set_selectable(true);
    fingerprint_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let allow = gtk::Button::with_label(&label(&locale, "allowPairing"));
    allow.style_context().add_class("primary");
    let deny = gtk::Button::with_label(&label(&locale, "denyPairing"));
    actions.pack_start(&allow, true, true, 0);
    actions.pack_start(&deny, true, true, 0);
    request_box.pack_start(&request_heading, false, false, 0);
    request_box.pack_start(&peer_label, false, false, 0);
    request_box.pack_start(&fingerprint_label, false, false, 0);
    request_box.pack_start(&actions, false, false, 0);
    content.pack_start(&request_box, false, false, 0);

    let settings = gtk::Box::new(gtk::Orientation::Vertical, 0);
    settings.style_context().add_class("card");
    let settings_heading = gtk::Label::new(Some(&label(&locale, "preferencesTitle")));
    settings_heading.set_xalign(0.0);
    settings.pack_start(&settings_heading, false, false, 0);
    let status = gtk::Label::new(None);
    status.style_context().add_class("status");
    status.set_xalign(0.0);
    if let Err(error) = ensure_linux_identity() {
        status.set_text(&error.to_string());
    }
    let startup = Command::new("systemctl")
        .args(["--user", "is-enabled", "vortal-input.service"])
        .output()
        .is_ok_and(|result| result.status.success());
    build_setting(
        &settings,
        &locale,
        "startupLabel",
        "startup-enabled",
        startup,
        &status,
    );
    build_setting(
        &settings,
        &locale,
        "clipboardLabel",
        "clipboard-enabled",
        setting_enabled("clipboard-enabled"),
        &status,
    );
    build_setting(
        &settings,
        &locale,
        "audioLabel",
        "audio-enabled",
        setting_enabled("audio-enabled"),
        &status,
    );
    settings.pack_start(&status, false, false, 0);
    content.pack_start(&settings, false, false, 0);

    window.add(&content);
    window.show_all();

    let nonce = Rc::new(RefCell::new(None::<String>));
    for (button, action, result_key) in [
        (&allow, "allow", "pairingAllowed"),
        (&deny, "deny", "pairingDeniedLocal"),
    ] {
        let nonce = Rc::clone(&nonce);
        let status = status.clone();
        let success = label(&locale, result_key);
        let request_box = request_box.clone();
        button.connect_clicked(move |_| {
            if let Some(value) = nonce.borrow().as_deref() {
                match decide_pairing(value, action) {
                    Ok(()) => {
                        request_box.hide();
                        status.set_text(&success);
                    }
                    Err(error) => status.set_text(&error.to_string()),
                }
            }
        });
    }
    {
        let status = status.clone();
        let success = label(&locale, "remoteCertReady");
        create_button.connect_clicked(move |button| match ensure_linux_identity() {
            Ok(()) => {
                button.hide();
                status.set_text(&success);
            }
            Err(error) => status.set_text(&error.to_string()),
        });
    }
    refresh_pairing(
        &locale,
        &id_label,
        &create_button,
        &request_box,
        &peer_label,
        &fingerprint_label,
        &nonce,
    );
    gtk::glib::timeout_add_seconds_local(1, move || {
        refresh_pairing(
            &locale,
            &id_label,
            &create_button,
            &request_box,
            &peer_label,
            &fingerprint_label,
            &nonce,
        );
        gtk::glib::ControlFlow::Continue
    });
}

/// Runs the Linux receiver UI with native GTK widgets.
pub fn run_desktop() -> anyhow::Result<()> {
    let language = if std::env::var("LANG")
        .unwrap_or_default()
        .to_lowercase()
        .starts_with("pt")
    {
        include_str!("../ui/locales/pt-BR.json")
    } else {
        include_str!("../ui/locales/en.json")
    };
    let locale: serde_json::Value = serde_json::from_str(language)?;
    let application = gtk::Application::new(Some("com.vortal.desktop"), Default::default());
    application.connect_activate(move |app| {
        if let Err(error) = apply_style() {
            eprintln!("Vortal style: {error}");
        }
        build_window(app, locale.clone());
    });
    let _ = application.run();
    Ok(())
}
