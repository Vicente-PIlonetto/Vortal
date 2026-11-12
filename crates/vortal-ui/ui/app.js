const localeFiles = {
  "pt-BR": "locales/pt-BR.json",
  en: "locales/en.json",
};

const storageKey = "vortal-ui-settings-v1";
let locale = {};
let settings = loadSettings();
if (settings.clipboard === undefined) settings.clipboard = true;
const linuxDesktop = !window.vortalDesktop && window.location.protocol === "vortal:";
settings.inputSettings = {
  sensitivity: settings.inputSettingsVersion === 2 && Number.isInteger(settings.inputSettings?.sensitivity)
    && settings.inputSettings.sensitivity >= 5 && settings.inputSettings.sensitivity <= 100
    ? settings.inputSettings.sensitivity : 100,
  keyboard: settings.inputSettings?.keyboard !== false,
  layout: ["auto", "us", "abnt2"].includes(settings.inputSettings?.layout)
    ? settings.inputSettings.layout : "auto",
  scrollSensitivity: Number.isInteger(settings.inputSettings?.scrollSensitivity)
    && settings.inputSettings.scrollSensitivity >= 25 && settings.inputSettings.scrollSensitivity <= 300
    ? settings.inputSettings.scrollSensitivity : 100,
  returnShortcut: /^(?:(?:ctrl|shift|alt)\+){2,3}[A-Z0-9]$/.test(settings.inputSettings?.returnShortcut)
    ? settings.inputSettings.returnShortcut : "ctrl+shift+Q",
};
let peers = Array.isArray(settings.peers)
  ? settings.peers.filter((peer) => typeof peer === "string").map(normalizeAddress).filter(Boolean)
  : [];
let remoteState = "remoteIdle";
let remoteDetail = "";
let peerReady = false;
let edgeIsActive = false;
let inputSettingsRevision = 0;
let audioContext = null;
let nextAudioTime = 0;
const heldKeys = new Set();
const heldButtons = new Set();

function desktopError(error) {
  return locale[String(error?.message || "")] || "";
}

function loadSettings() {
  try {
    return JSON.parse(localStorage.getItem(storageKey)) || {};
  } catch {
    return {};
  }
}

if (linuxDesktop) {
  document.querySelector("#desktop-nav").hidden = false;
  document.querySelector("#nav-home").hidden = true;
  document.querySelector("#nav-portals").hidden = true;
  document.querySelector("#nav-settings").addEventListener("click", () => showDesktopPage("settings"));
  document.querySelectorAll("#linux-pairing + .input-settings-panel .input-setting").forEach((row) => {
    if (!row.querySelector("#startup-enabled, #clipboard-enabled, #audio-enabled")) row.hidden = true;
  });
  showDesktopPage("settings");
  document.querySelector("#linux-pairing").hidden = false;
  const updateLinuxSetting = async (name, enabled) => {
    const response = await fetch(`vortal://app/api/${name}?enabled=${enabled ? "1" : "0"}`);
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || locale.inputSettingsFailed);
    return result.enabled;
  };
  void fetch("vortal://app/api/settings").then(async (response) => {
    if (!response.ok) throw new Error(locale.inputSettingsFailed);
    const state = await response.json();
    document.querySelector("#startup-enabled").checked = state.startup;
    document.querySelector("#clipboard-enabled").checked = state.clipboard;
    document.querySelector("#audio-enabled").checked = state.audio;
  }).catch((error) => { document.querySelector("#input-settings-status").textContent = error.message; });
  for (const [id, name] of [["#startup-enabled", "startup"], ["#clipboard-enabled", "clipboard"], ["#audio-enabled", "audio"]]) {
    document.querySelector(id).addEventListener("change", async (event) => {
      try {
        event.target.checked = await updateLinuxSetting(name, event.target.checked);
        document.querySelector("#input-settings-status").textContent = locale.inputSettingsSaved;
      } catch (error) {
        event.target.checked = !event.target.checked;
        document.querySelector("#input-settings-status").textContent = error.message;
      }
    });
  }
  let pendingNonce = null;
  const refreshPairing = async () => {
    try {
      const response = await fetch("vortal://app/api/pairing");
      const state = await response.json();
      if (!response.ok) throw new Error(state.error);
      document.querySelector("#linux-id").textContent = `${state.id.slice(0, 2)} ${state.id.slice(2)}`;
      pendingNonce = state.pending?.nonce || null;
      document.querySelector("#pairing-request").hidden = !pendingNonce;
      if (pendingNonce) {
        document.querySelector("#pairing-peer").textContent = state.pending.name || "";
        document.querySelector("#pairing-fingerprint").textContent = state.pending.fingerprint || "";
      }
    } catch (error) { document.querySelector("#input-settings-status").textContent = error.message; }
  };
  for (const [id, action] of [["#allow-pairing", "allow"], ["#deny-pairing", "deny"]]) {
    document.querySelector(id).addEventListener("click", async () => {
      if (!pendingNonce) return;
      try {
        const response = await fetch(`vortal://app/api/pairing/${action}?nonce=${pendingNonce}`);
        const result = await response.json();
        if (!response.ok) throw new Error(result.error);
        document.querySelector("#input-settings-status").textContent = locale[action === "allow" ? "pairingAllowed" : "pairingDeniedLocal"];
        document.querySelector("#pairing-request").hidden = true;
        pendingNonce = null;
      } catch (error) { document.querySelector("#input-settings-status").textContent = error.message; }
    });
  }
  void refreshPairing();
  window.setInterval(() => { void refreshPairing(); }, 1000);
}

function saveSettings() {
  try {
    localStorage.setItem(storageKey, JSON.stringify({
      useVyLink: document.querySelector("#vylink-enabled").checked,
      server: document.querySelector("#vylink-server").value,
      network: document.querySelector("#vylink-network").value,
      peers,
      portal: settings.portal,
      inputSettings: settings.inputSettings,
      inputSettingsVersion: 2,
      autoConnect: Boolean(settings.autoConnect),
      clipboard: Boolean(settings.clipboard),
      audio: Boolean(settings.audio),
    }));
  } catch {
    showToast(locale.storageUnavailable);
  }
}

async function setLanguage(language) {
  if (window.VORTAL_LOCALES?.[language]) {
    locale = window.VORTAL_LOCALES[language];
  } else if (window.vortalDesktop) {
    locale = await window.vortalDesktop.locale(language);
  } else {
    const response = await fetch(localeFiles[language]);
    locale = await response.json();
  }
  document.documentElement.lang = language;
  document.querySelector("#language").value = language;
  document.querySelectorAll("[data-i18n]").forEach((element) => {
    const value = locale[element.dataset.i18n];
    if (value) element.textContent = value;
  });
  document.querySelectorAll("[data-i18n-aria-label]").forEach((element) => {
    const value = locale[element.dataset.i18nAriaLabel];
    if (value) element.setAttribute("aria-label", value);
  });
  document.querySelector("#language").setAttribute("aria-label", locale.languageLabel);
  document.querySelector(".rail")?.setAttribute("aria-label", locale.navHome);
  if (window.vortalDesktop) {
    document.querySelector("[data-i18n='footerStage']").textContent = locale.remoteStage;
  }
  if (linuxDesktop) {
    document.querySelector("#input-settings-title").textContent = locale.preferencesTitle;
    document.querySelector("#input-settings-page .lead").textContent = locale.linuxSettingsIntro;
    document.querySelector("#startup-enabled").closest(".input-setting").querySelector(".field-help").textContent = locale.startupHelpLinux;
  }
  renderMode();
  renderPeers();
  renderRemoteStatus();
  if (window.vortalDesktop && document.querySelector("#portal-monitor").options.length) {
    document.querySelector("#portal-status").textContent = locale[edgeIsActive ? "portalActive" : "portalSaved"];
  }
}

function renderRemoteStatus() {
  const status = document.querySelector("#remote-status");
  status.textContent = `${locale[remoteState] || ""}${remoteDetail}`;
  status.hidden = remoteState === "remoteIdle";
  if (window.vortalDesktop) {
    document.querySelector(".status-card").dataset.state = remoteState;
    document.querySelector(".status-card strong").textContent = ["remoteConnected", "portalActive"].includes(remoteState)
      ? locale.remoteBadgeConnected
      : ["remoteConnecting", "remoteReconnecting"].includes(remoteState)
        ? locale.remoteConnecting : locale.notConnected;
  }
  renderConnectionActions();
}

function renderConnectionActions() {
  if (!window.vortalDesktop) return;
  const active = ["remoteConnecting", "remoteConnected", "remoteReconnecting", "portalActive"].includes(remoteState);
  document.querySelector("#start-control").hidden = !peerReady || !peers.length || active;
  document.querySelector("#stop-control").hidden = !active;
}

function setRemoteStatus(state, detail = "") {
  remoteState = state;
  remoteDetail = detail;
  renderRemoteStatus();
  renderPeers();
}

function normalizeAddress(value) {
  const address = value.trim();
  const ipv4 = /^(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)$/;
  const ipv4WithPort = /^(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d):([1-9]\d{0,4})$/;
  const defaultPort = 24800;

  if (ipv4.test(address)) return `${address}:${defaultPort}`;
  if (ipv4WithPort.test(address)) {
    const port = Number(address.slice(address.lastIndexOf(":") + 1));
    return port <= 65535 ? address : null;
  }
  if (address.startsWith("[")) {
    const match = address.match(/^\[([0-9a-fA-F:]+)\](?::([1-9]\d{0,4}))?$/);
    if (!match || !isIpv6(match[1])) return null;
    const port = match[2] ? Number(match[2]) : defaultPort;
    return port <= 65535 ? `[${match[1]}]:${port}` : null;
  }
  if (address.includes(":") && (address.match(/:/g) || []).length > 1) {
    if (isIpv6(address)) return `[${address}]:${defaultPort}`;
    return null;
  }
  return null;
}

function isIpv6(value) {
  try {
    return new URL(`http://[${value}]/`).hostname.length > 0;
  } catch {
    return false;
  }
}

function renderMode() {
  const enabled = document.querySelector("#vylink-enabled").checked;
  document.querySelector("#vylink-enabled").setAttribute("aria-checked", String(enabled));
  document.querySelector("#vylink-settings").hidden = window.vortalDesktop || !enabled;
}

function renderPeers() {
  const list = document.querySelector("#peer-list");
  list.replaceChildren(...peers.map((address) => {
    const row = document.createElement("div");
    row.className = "peer-item";
    const peerAddress = document.createElement("span");
    peerAddress.className = "peer-address";
    peerAddress.textContent = address;
    const state = document.createElement("span");
    state.className = "peer-state";
    state.textContent = remoteState === "remoteConnected" && address === peers[0]
      ? locale.peerConnected : locale.peerState;
    const remove = document.createElement("button");
    remove.className = "remove-peer";
    remove.type = "button";
    remove.textContent = locale.remove;
    remove.setAttribute("aria-label", `${locale.remove} ${address}`);
    remove.addEventListener("click", () => {
      peers = peers.filter((peer) => peer !== address);
      renderPeers();
      saveSettings();
    });
    row.append(peerAddress, state, remove);
    return row;
  }));
  document.querySelector(".empty-state").hidden = peers.length > 0;
  renderConnectionActions();
}

function showToast(message) {
  const toast = document.querySelector("#toast");
  toast.textContent = message;
  toast.hidden = false;
  window.clearTimeout(showToast.timer);
  showToast.timer = window.setTimeout(() => { toast.hidden = true; }, 2400);
}

document.querySelector("#language").addEventListener("change", (event) => {
  void setLanguage(event.target.value);
});

document.querySelector("#vylink-enabled").addEventListener("change", () => {
  renderMode();
  saveSettings();
});

for (const id of ["#vylink-server", "#vylink-network"]) {
  document.querySelector(id).addEventListener("input", () => {
    saveSettings();
  });
}

document.querySelector("#peer-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const input = document.querySelector("#peer-address");
  const error = document.querySelector("#peer-error");
  const address = normalizeAddress(input.value);
  if (!address || peers.includes(address)) {
    error.textContent = locale[peers.includes(address) ? "peerDuplicate" : "peerInvalid"];
    error.hidden = false;
    input.setAttribute("aria-invalid", "true");
    input.focus();
    return;
  }
  error.hidden = true;
  input.removeAttribute("aria-invalid");
  peers = [...peers, address];
  input.value = "";
  renderPeers();
  saveSettings();
  showToast(locale.peerAdded);
});

document.querySelector("#vylink-enabled").checked = Boolean(settings.useVyLink);
document.querySelector("#vylink-server").value = settings.server || "";
document.querySelector("#vylink-network").value = settings.network || "";
if (window.vortalDesktop) {
  document.querySelector("#language").hidden = true;
  document.querySelector("#language").disabled = true;
  document.querySelector("label[for='language']").hidden = true;
}
const initialLanguage = window.vortalDesktop
  ? window.vortalDesktop.systemLanguage().catch(() => "en")
  : Promise.resolve("pt-BR");
void initialLanguage.then((language) => setLanguage(language === "pt-BR" ? "pt-BR" : "en")).then(() => {
  if (window.vortalDesktop) {
    void setupPortalEditor().catch((error) => setRemoteStatus("remoteSetupError", desktopError(error)));
    void applyInputSettings();
  }
});

function keyCode(code) {
  if (/^Key[A-Z]$/.test(code)) return code.charCodeAt(3);
  if (/^Digit[0-9]$/.test(code)) return code.charCodeAt(5);
  if (/^F([1-9]|1[0-2])$/.test(code)) return 299 + Number(code.slice(1));
  const special = {
    Enter: 256, Escape: 257, Backspace: 258, Tab: 259, Space: 260,
    ArrowLeft: 261, ArrowRight: 262, ArrowUp: 263, ArrowDown: 264,
    Delete: 265, ShiftLeft: 266, ShiftRight: 267,
    ControlLeft: 268, ControlRight: 269, AltLeft: 270, AltRight: 271,
    MetaLeft: 272, MetaRight: 273,
    Home: 274, End: 275, PageUp: 276, PageDown: 277, Insert: 278, CapsLock: 279,
    Quote: 39, Comma: 44, Minus: 45, Period: 46, Slash: 47,
    Semicolon: 59, Equal: 61, BracketLeft: 91, Backslash: 92,
    BracketRight: 93, Backquote: 96,
  };
  return special[code] ?? null;
}

function captureActive() {
  return document.pointerLockElement === document.querySelector("#capture-zone");
}

function showDesktopPage(page) {
  document.querySelector(".tile-grid").hidden = page !== "home";
  document.querySelector("#portal-page").hidden = page !== "portals";
  document.querySelector("#input-settings-page").hidden = page !== "settings";
  document.querySelector(".content").dataset.page = page;
  document.querySelector("#nav-home").setAttribute("aria-current", page === "home" ? "page" : "false");
  document.querySelector("#nav-portals").setAttribute("aria-current", page === "portals" ? "page" : "false");
  document.querySelector("#nav-settings").setAttribute("aria-current", page === "settings" ? "page" : "false");
}

async function applyInputSettings() {
  const revision = ++inputSettingsRevision;
  const sensitivity = Number(document.querySelector("#mouse-sensitivity").value);
  const scrollSensitivity = Number(document.querySelector("#scroll-sensitivity").value);
  const keyboard = document.querySelector("#keyboard-enabled").checked;
  const layout = document.querySelector("#keyboard-layout").value;
  const modifiers = [["#return-ctrl", "ctrl"], ["#return-shift", "shift"], ["#return-alt", "alt"]]
    .filter(([id]) => document.querySelector(id).checked).map(([, name]) => name);
  if (modifiers.length < 2) {
    document.querySelector("#input-settings-status").textContent = locale.returnShortcutInvalid;
    return;
  }
  const returnShortcut = `${modifiers.join("+")}+${document.querySelector("#return-key").value}`;
  document.querySelector("#mouse-sensitivity-value").textContent = `${sensitivity}%`;
  document.querySelector("#scroll-sensitivity-value").textContent = `${scrollSensitivity}%`;
  document.querySelector("#keyboard-enabled").setAttribute("aria-checked", String(keyboard));
  const next = { sensitivity, scrollSensitivity, keyboard, layout, returnShortcut };
  try {
    if (!keyboard && settings.inputSettings.keyboard) releaseHeldInput();
    await window.vortalDesktop.configureInput(next);
    if (revision !== inputSettingsRevision) return;
    settings.inputSettings = next;
    saveSettings();
    document.querySelector("#input-settings-status").textContent = locale.inputSettingsSaved;
  } catch (error) {
    document.querySelector("#input-settings-status").textContent = locale.inputSettingsFailed;
  }
}

async function setupPortalEditor() {
  const displays = await window.vortalDesktop.displays();
  const monitor = document.querySelector("#portal-monitor");
  const side = document.querySelector("#portal-side");
  monitor.replaceChildren(...displays.map((display, index) => {
    const option = document.createElement("option");
    option.value = display.id;
    option.textContent = `${index + 1} · ${display.label} (${display.width}×${display.height})`;
    return option;
  }));
  const saved = settings.portal || {};
  monitor.value = displays.some((display) => display.id === saved.displayId)
    ? saved.displayId : displays[0]?.id || "";
  side.value = ["Left", "Right", "Top", "Bottom"].includes(saved.side) ? saved.side : "Right";
  const savePortal = async () => {
    if (!monitor.value) return;
    const portal = { displayId: monitor.value, side: side.value };
    await window.vortalDesktop.configureEdge(portal);
    settings.portal = portal;
    saveSettings();
    document.querySelector(".portal-preview").dataset.side = portal.side;
    document.querySelector("#portal-status").textContent = locale.portalSaved;
  };
  monitor.addEventListener("change", () => { void savePortal().catch((error) => setRemoteStatus("remoteSetupError", desktopError(error))); });
  side.addEventListener("change", () => { void savePortal().catch((error) => setRemoteStatus("remoteSetupError", desktopError(error))); });
  await savePortal();
}

function releaseHeldInput() {
  if (!window.vortalDesktop) return;
  for (const code of heldKeys) window.vortalDesktop.input({ type: "key", code, pressed: false });
  for (const button of heldButtons) window.vortalDesktop.input({ type: "mouseButton", button, pressed: false });
  heldKeys.clear();
  heldButtons.clear();
}

function playRemoteAudio(encoded) {
  if (!settings.audio) return;
  try {
    const binary = atob(encoded);
    if (binary.length !== 3840) return;
    if (!audioContext) audioContext = new AudioContext({ sampleRate: 48000 });
    if (audioContext.state === "suspended") void audioContext.resume();
    const buffer = audioContext.createBuffer(2, 960, 48000);
    for (let frame = 0; frame < 960; frame += 1) {
      for (let channel = 0; channel < 2; channel += 1) {
        const index = (frame * 2 + channel) * 2;
        const sample = (binary.charCodeAt(index) | (binary.charCodeAt(index + 1) << 8));
        buffer.getChannelData(channel)[frame] = (sample < 32768 ? sample : sample - 65536) / 32768;
      }
    }
    const source = audioContext.createBufferSource();
    source.buffer = buffer;
    source.connect(audioContext.destination);
    const now = audioContext.currentTime;
    if (nextAudioTime < now || nextAudioTime > now + 0.3) nextAudioTime = now + 0.06;
    source.start(nextAudioTime);
    nextAudioTime += 0.02;
  } catch {
    document.querySelector("#input-settings-status").textContent = locale.audioFailed;
  }
}

if (window.vortalDesktop) {
  document.querySelector("#desktop-nav").hidden = false;
  document.querySelector("#nav-home").addEventListener("click", () => showDesktopPage("home"));
  document.querySelector("#nav-portals").addEventListener("click", () => showDesktopPage("portals"));
  document.querySelector("#nav-settings").addEventListener("click", () => showDesktopPage("settings"));
  document.querySelector(".brand").addEventListener("click", () => showDesktopPage("home"));
  showDesktopPage("home");
  document.querySelector("#mouse-sensitivity").value = String(settings.inputSettings.sensitivity);
  document.querySelector("#scroll-sensitivity").value = String(settings.inputSettings.scrollSensitivity);
  document.querySelector("#keyboard-enabled").checked = settings.inputSettings.keyboard;
  document.querySelector("#keyboard-layout").value = settings.inputSettings.layout;
  const shortcutParts = settings.inputSettings.returnShortcut.split("+");
  for (const [id, name] of [["#return-ctrl", "ctrl"], ["#return-shift", "shift"], ["#return-alt", "alt"]]) {
    document.querySelector(id).checked = shortcutParts.includes(name);
  }
  const returnKey = document.querySelector("#return-key");
  returnKey.replaceChildren(..."ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".split("").map((key) => {
    const option = document.createElement("option");
    option.value = key;
    option.textContent = key;
    return option;
  }));
  returnKey.value = shortcutParts.at(-1);
  document.querySelector("#mouse-sensitivity-value").textContent = `${settings.inputSettings.sensitivity}%`;
  document.querySelector("#scroll-sensitivity-value").textContent = `${settings.inputSettings.scrollSensitivity}%`;
  document.querySelector("#auto-connect").checked = Boolean(settings.autoConnect);
  document.querySelector("#clipboard-enabled").checked = Boolean(settings.clipboard);
  void window.vortalDesktop.configureClipboard(Boolean(settings.clipboard));
  document.querySelector("#audio-enabled").checked = Boolean(settings.audio);
  void window.vortalDesktop.configureAudio(Boolean(settings.audio));
  window.vortalDesktop.onAudio(playRemoteAudio);
  for (const id of ["#mouse-sensitivity", "#scroll-sensitivity", "#keyboard-enabled", "#keyboard-layout", "#return-ctrl", "#return-shift", "#return-alt", "#return-key"]) {
    document.querySelector(id).addEventListener(id === "#mouse-sensitivity" || id === "#scroll-sensitivity" ? "input" : "change", () => { void applyInputSettings(); });
  }
  void window.vortalDesktop.startup().then((enabled) => {
    document.querySelector("#startup-enabled").checked = enabled;
  }).catch((error) => showToast(desktopError(error) || locale.inputSettingsFailed));
  document.querySelector("#startup-enabled").addEventListener("change", async (event) => {
    try {
      event.target.checked = await window.vortalDesktop.startup(event.target.checked);
    } catch (error) {
      event.target.checked = !event.target.checked;
      showToast(desktopError(error) || locale.inputSettingsFailed);
    }
  });
  document.querySelector("#auto-connect").addEventListener("change", (event) => {
    settings.autoConnect = event.target.checked;
    saveSettings();
  });
  document.querySelector("#clipboard-enabled").addEventListener("change", async (event) => {
    try {
      settings.clipboard = await window.vortalDesktop.configureClipboard(event.target.checked);
      saveSettings();
    } catch (error) {
      event.target.checked = !event.target.checked;
      showToast(desktopError(error) || locale.inputSettingsFailed);
    }
  });
  document.querySelector("#audio-enabled").addEventListener("change", async (event) => {
    try {
      settings.audio = await window.vortalDesktop.configureAudio(event.target.checked);
      if (settings.audio) {
        if (!audioContext) audioContext = new AudioContext({ sampleRate: 48000 });
        await audioContext.resume();
      } else if (audioContext) {
        await audioContext.suspend();
        nextAudioTime = 0;
      }
      saveSettings();
    } catch (error) {
      event.target.checked = !event.target.checked;
      showToast(desktopError(error) || locale.inputSettingsFailed);
    }
  });
  window.vortalDesktop.onInputError((code) => {
    document.querySelector("#input-settings-status").textContent = locale[code] || locale.captureFailed;
    showDesktopPage("settings");
  });
  document.querySelector("#remote-control").hidden = false;
  document.querySelector("#settings").hidden = true;
  document.querySelector(".mode-row").hidden = true;
  document.querySelectorAll(".tile-connection > .command-block").forEach((block) => { block.hidden = true; });
  const control = document.querySelector("#capture-zone");
  const start = document.querySelector("#start-control");
  const stop = document.querySelector("#stop-control");

  void window.vortalDesktop.setup().then((setup) => {
    peerReady = setup.peerReady;
    if (setup.peerReady) setRemoteStatus("remotePeerReady");
    else renderConnectionActions();
    if (settings.autoConnect && setup.identityReady && setup.peerReady && peers[0]) start.click();
  }).catch((error) => setRemoteStatus("remoteSetupError", desktopError(error)));

  document.querySelector("#choose-peer-cert").addEventListener("click", async () => {
    try {
      if (await window.vortalDesktop.choosePeerCert()) {
        peerReady = true;
        setRemoteStatus("remotePeerReady");
      }
    } catch (error) {
      setRemoteStatus("remoteSetupError", desktopError(error));
    }
  });

  document.querySelector("#pair-id-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const button = event.currentTarget.querySelector("button");
    const id = document.querySelector("#pair-id").value;
    button.disabled = true;
    setRemoteStatus("pairingWaiting");
    try {
      const address = await window.vortalDesktop.pairById(id);
      peerReady = true;
      peers = [address, ...peers.filter((peer) => peer !== address)];
      saveSettings();
      renderPeers();
      setRemoteStatus("pairingAllowed");
      window.setTimeout(() => start.click(), 800);
    } catch (error) {
      const code = ["pairingDenied", "pairingTimedOut", "pairingInvalid", "pairingInvalidId", "idNotFound", "tailscaleUnavailable", "remoteMissingIdentity"]
        .find((candidate) => error.message.includes(candidate));
      setRemoteStatus(code || "remoteConnectionError");
    } finally { button.disabled = false; }
  });

  start.addEventListener("click", async () => {
    if (!peers[0]) { setRemoteStatus("remoteMissingAddress"); return; }
    try {
      const setup = await window.vortalDesktop.setup();
      if (!setup.identityReady) { setRemoteStatus("remoteMissingIdentity"); return; }
      if (!setup.peerReady) { setRemoteStatus("remoteMissingPeerCert"); return; }
      start.disabled = true;
      setRemoteStatus("remoteConnecting");
      await window.vortalDesktop.connect(peers[0]);
    } catch (error) {
      start.disabled = false;
      setRemoteStatus("remoteConnectionError", desktopError(error));
    }
  });

  stop.addEventListener("click", () => {
    releaseHeldInput();
    if (captureActive()) document.exitPointerLock();
    window.vortalDesktop.disconnect();
    control.hidden = true;
    stop.hidden = true;
    start.disabled = false;
    setRemoteStatus("remoteDisconnected");
  });

  window.vortalDesktop.onStatus(({ status, detail }) => {
    if (status === "connected") {
      control.hidden = false;
      stop.hidden = false;
      setRemoteStatus("remoteConnected");
    } else if (status === "connecting") {
      setRemoteStatus("remoteConnecting");
    } else if (status === "reconnecting") {
      releaseHeldInput();
      if (captureActive()) document.exitPointerLock();
      control.hidden = true;
      stop.hidden = false;
      start.disabled = true;
      setRemoteStatus("remoteReconnecting");
    } else if (status === "returned") {
      releaseHeldInput();
      if (captureActive()) document.exitPointerLock();
      setRemoteStatus("remoteConnected");
    } else {
      releaseHeldInput();
      if (captureActive()) document.exitPointerLock();
      control.hidden = true;
      stop.hidden = true;
      start.disabled = false;
      setRemoteStatus("remoteDisconnected", detail ? ` · ${detail}` : "");
    }
  });

  window.vortalDesktop.onEdgeState((active) => {
    edgeIsActive = active;
    document.querySelector("#portal-status").textContent = locale[active ? "portalActive" : "portalSaved"];
    if (active) setRemoteStatus("portalActive");
    else if (remoteState === "portalActive") setRemoteStatus("remoteConnected");
  });

  control.addEventListener("click", () => { void control.requestPointerLock(); });
  document.addEventListener("pointerlockchange", () => {
    if (captureActive()) {
      window.vortalDesktop.input({ type: "enterScreen", edge: "Right", position: 32768 });
    } else {
      releaseHeldInput();
      window.vortalDesktop.input({ type: "leaveScreen" });
    }
  });
  document.addEventListener("mousemove", (event) => {
    if (!captureActive()) return;
    const dx = Math.max(-65535, Math.min(65535, event.movementX));
    const dy = Math.max(-65535, Math.min(65535, event.movementY));
    if (dx || dy) window.vortalDesktop.input({ type: "mouseMove", dx, dy });
  });
  document.addEventListener("mousedown", (event) => {
    if (!captureActive()) return;
    const button = [1, 3, 2, 4, 5][event.button];
    if (button) { heldButtons.add(button); window.vortalDesktop.input({ type: "mouseButton", button, pressed: true }); }
    event.preventDefault();
  });
  document.addEventListener("mouseup", (event) => {
    if (!captureActive()) return;
    const button = [1, 3, 2, 4, 5][event.button];
    if (button) { heldButtons.delete(button); window.vortalDesktop.input({ type: "mouseButton", button, pressed: false }); }
    event.preventDefault();
  });
  document.addEventListener("wheel", (event) => {
    if (!captureActive()) return;
    window.vortalDesktop.input({ type: "mouseScroll", horizontal: Math.sign(event.deltaX), vertical: Math.sign(event.deltaY) });
    event.preventDefault();
  }, { passive: false });
  document.addEventListener("keydown", (event) => {
    if (!captureActive()) return;
    if (!settings.inputSettings.keyboard) return;
    const code = keyCode(event.code);
    if (code && code !== 257 && !heldKeys.has(code)) {
      heldKeys.add(code);
      window.vortalDesktop.input({ type: "key", code, pressed: true });
    }
    event.preventDefault();
  });
  document.addEventListener("keyup", (event) => {
    if (!captureActive()) return;
    if (!settings.inputSettings.keyboard) return;
    const code = keyCode(event.code);
    if (code && heldKeys.delete(code)) window.vortalDesktop.input({ type: "key", code, pressed: false });
    event.preventDefault();
  });
}
