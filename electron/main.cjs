const { app, BrowserWindow, Menu, Tray, clipboard, dialog, ipcMain, screen, shell } = require('electron');
const path = require('node:path');
const fs = require('node:fs');
const net = require('node:net');
const os = require('node:os');
const { spawn } = require('node:child_process');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const crypto = require('node:crypto');
const { createFileClipboard } = require('./file-clipboard.cjs');

app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');

let session = null;
let sessionConnected = false;
let desiredAddress = null;
let reconnectTimer = null;
let reconnectDelay = 1000;
let mainWindow = null;
let tray = null;
let quitting = false;
let edgeActive = false;
let motion = null;
let cursorShield = null;
let motionHeartbeat = null;
let edgeConfig = null;
let previousCursor = null;
let edgeCooldownUntil = 0;
let edgePollTimer = null;
let inputSettings = { sensitivity: 100, scrollSensitivity: 100, keyboard: true, layout: 'auto', returnShortcut: 'ctrl+shift+Q' };

function validReturnShortcut(value) {
  if (typeof value !== 'string') return false;
  const parts = value.split('+');
  const key = parts.pop();
  const modifiers = new Set(parts);
  return /^[A-Z0-9]$/.test(key) && parts.length >= 2 && parts.length <= 3
    && modifiers.size === parts.length && parts.every((part) => ['ctrl', 'shift', 'alt'].includes(part));
}
let mouseRemainderX = 0;
let mouseRemainderY = 0;
let scrollRemainderX = 0;
let scrollRemainderY = 0;
let clipboardEnabled = false;
let lastClipboardText = null;
let clipboardTimer = null;
let fileClipboard = null;
let audioEnabled = false;

function stopClipboard() {
  if (fileClipboard) fileClipboard.stop();
  if (clipboardTimer) clearInterval(clipboardTimer);
  clipboardTimer = null;
  lastClipboardText = null;
}

function startClipboard() {
  stopClipboard();
  if (!clipboardEnabled || !sessionConnected) return;
  if (fileClipboard) fileClipboard.start();
  clipboardTimer = setInterval(() => {
    if (fileClipboard && fileClipboard.hasFiles()) return;
    const value = clipboard.readText();
    if (value === lastClipboardText || Buffer.byteLength(value, 'utf8') > 16384) return;
    lastClipboardText = value;
    writeInput({ type: 'clipboardText', text: value });
  }, 500);
  clipboardTimer.unref();
}

function cliPath() {
  return app.isPackaged
    ? path.join(process.resourcesPath, 'bin', 'vortal.exe')
    : path.join(__dirname, '..', 'dist', 'windows', 'resources', 'bin', 'vortal.exe');
}

function motionPath() {
  return app.isPackaged
    ? path.join(process.resourcesPath, 'bin', 'vortal-motion.exe')
    : path.join(__dirname, '..', 'dist', 'windows', 'resources', 'bin', 'vortal-motion.exe');
}

function hideCursorShield() {
  const shield = cursorShield;
  cursorShield = null;
  if (shield && !shield.isDestroyed()) shield.destroy();
}

function showCursorShield(display) {
  hideCursorShield();
  const bounds = display.bounds;
  const shield = new BrowserWindow({
    x: Math.round(bounds.x + bounds.width / 2 - 16),
    y: Math.round(bounds.y + bounds.height / 2 - 16),
    width: 32, height: 32, frame: false, transparent: true, focusable: false,
    skipTaskbar: true, hasShadow: false, show: false,
    webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false },
  });
  cursorShield = shield;
  shield.setAlwaysOnTop(true, 'screen-saver');
  shield.webContents.once('did-finish-load', () => {
    if (edgeActive && cursorShield === shield && !shield.isDestroyed()) shield.showInactive();
  });
  const html = '<!doctype html><html><head><style>html,body{margin:0;width:100%;height:100%;cursor:none!important}</style></head><body></body></html>';
  void shield.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`).catch(() => {
    if (cursorShield === shield) hideCursorShield();
  });
}

function writeInput(input) {
  if (!session || !session.stdin.writable) return;
  if (input.type === 'key' && !inputSettings.keyboard) return;
  if (input.type === 'mouseMove') {
    const scaledX = input.dx * inputSettings.sensitivity / 100 + mouseRemainderX;
    const scaledY = input.dy * inputSettings.sensitivity / 100 + mouseRemainderY;
    const dx = Math.trunc(scaledX);
    const dy = Math.trunc(scaledY);
    mouseRemainderX = scaledX - dx;
    mouseRemainderY = scaledY - dy;
    if (!dx && !dy) return;
    input = { type: 'mouseMove', dx, dy };
  }
  if (input.type === 'mouseScroll') {
    const scaledX = input.horizontal * inputSettings.scrollSensitivity / 100 + scrollRemainderX;
    const scaledY = input.vertical * inputSettings.scrollSensitivity / 100 + scrollRemainderY;
    const horizontal = Math.trunc(scaledX);
    const vertical = Math.trunc(scaledY);
    scrollRemainderX = scaledX - horizontal;
    scrollRemainderY = scaledY - vertical;
    if (!horizontal && !vertical) return;
    input = { type: 'mouseScroll', horizontal, vertical };
  }
  if (session.stdin.writableLength > 32768) {
    session.kill();
    return;
  }
  session.stdin.write(`${JSON.stringify(input)}\n`);
}

function stopEdgeCapture() {
  if (!edgeActive && !motion) { hideCursorShield(); return; }
  const helper = motion;
  edgeActive = false;
  motion = null;
  mouseRemainderX = 0;
  mouseRemainderY = 0;
  scrollRemainderX = 0;
  scrollRemainderY = 0;
  if (motionHeartbeat) { clearInterval(motionHeartbeat); motionHeartbeat = null; }
  hideCursorShield();
  edgeCooldownUntil = Date.now() + 800;
  writeInput({ type: 'leaveScreen' });
  if (helper) {
    helper.stdin.end('stop\n');
    const timer = setTimeout(() => { if (helper.exitCode === null) helper.kill(); }, 150);
    timer.unref();
  }
  if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send('vortal:edge-state', false);
}

function startEdgeCapture(display, side, point) {
  if (edgeActive || !sessionConnected) return;
  const bounds = display.bounds;
  const along = side === 'Left' || side === 'Right'
    ? (point.y - bounds.y) / Math.max(1, bounds.height - 1)
    : (point.x - bounds.x) / Math.max(1, bounds.width - 1);
  const position = Math.max(0, Math.min(65535, Math.round(along * 65535)));
  edgeActive = true;
  const helper = spawn(motionPath(), [inputSettings.keyboard ? '--keyboard' : '--no-keyboard', '--layout', inputSettings.layout,
    '--return-shortcut', inputSettings.returnShortcut], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  motion = helper;
  helper.stdin.on('error', stopEdgeCapture);
  motionHeartbeat = setInterval(() => {
    if (helper.stdin.writable) helper.stdin.write('ping\n');
  }, 75);
  motionHeartbeat.unref();
  helper.stdin.write('ping\n');
  let ready = false;
  let buffer = '';
  let helperError = '';
  helper.stderr.on('data', (chunk) => { helperError = (helperError + chunk.toString()).slice(-2048); });
  helper.stdout.on('data', (chunk) => {
    buffer += chunk.toString();
    let end;
    while ((end = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, end).trim();
      buffer = buffer.slice(end + 1);
      if (line === 'VORTAL_MOTION_READY') {
        if (!edgeActive || motion !== helper) continue;
        ready = true;
        showCursorShield(display);
        writeInput({ type: 'enterScreen', edge: side, position });
        if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send('vortal:edge-state', true);
      } else if (line === 'VORTAL_MOTION_RETURN') {
        stopEdgeCapture();
      } else if (ready && edgeActive) {
        try {
          writeInput(JSON.parse(line));
        } catch { stopEdgeCapture(); }
      }
    }
  });
  helper.on('error', stopEdgeCapture);
  helper.on('close', (code) => {
    if (motion !== helper) return;
    stopEdgeCapture();
    if (code !== 0 && mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send('vortal:input-error', helperError.includes('selected keyboard layout is not installed')
        ? 'layoutUnavailable' : 'captureFailed');
    }
  });
}

function pollScreenEdge() {
  const point = screen.getCursorScreenPoint();
  const previous = previousCursor;
  previousCursor = point;
  if (!sessionConnected || !edgeConfig || edgeActive || Date.now() < edgeCooldownUntil || !previous) return;
  const display = screen.getAllDisplays().find((item) => String(item.id) === edgeConfig.displayId);
  if (!display) return;
  const { x, y, width, height } = display.bounds;
  const inside = point.x >= x && point.x < x + width && point.y >= y && point.y < y + height;
  const wasInside = previous.x >= x && previous.x < x + width && previous.y >= y && previous.y < y + height;
  if (!inside || !wasInside) return;
  const side = edgeConfig.side;
  const reached = (side === 'Right' && point.x >= x + width - 2 && point.x > previous.x)
    || (side === 'Left' && point.x <= x + 1 && point.x < previous.x)
    || (side === 'Bottom' && point.y >= y + height - 2 && point.y > previous.y)
    || (side === 'Top' && point.y <= y + 1 && point.y < previous.y);
  if (reached) startEdgeCapture(display, side, point);
}

function updateEdgePolling() {
  if (sessionConnected && edgeConfig && !edgePollTimer) {
    edgePollTimer = setInterval(pollScreenEdge, 16);
    edgePollTimer.unref();
  } else if ((!sessionConnected || !edgeConfig) && edgePollTimer) {
    clearInterval(edgePollTimer);
    edgePollTimer = null;
    previousCursor = null;
  }
}

function identityPath() {
  return path.join(app.getPath('userData'), 'identity');
}

function peerCertPath() {
  return path.join(app.getPath('userData'), 'peer.der');
}

function pairingId(certificate) {
  const hash = crypto.createHash('sha256').update(certificate).digest();
  const number = hash.readUInt32BE(2) % 1000000;
  return `${String.fromCharCode(65 + hash[0] % 26, 65 + hash[1] % 26)}${String(number).padStart(6, '0')}`;
}

async function tailscalePeers() {
  const candidates = [
    path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Tailscale', 'tailscale.exe'),
    'tailscale.exe',
  ];
  let lastError;
  for (const binary of candidates) {
    try {
      const { stdout } = await promisify(execFile)(binary, ['status', '--json'], { timeout: 5000, maxBuffer: 2 * 1024 * 1024, windowsHide: true });
      const status = JSON.parse(stdout);
      return Object.values(status.Peer || {})
        .filter((peer) => peer.Online && peer.OS === 'linux')
        .flatMap((peer) => peer.TailscaleIPs || [])
        .filter((ip) => {
          const parts = ip.split('.').map(Number);
          return parts.length === 4 && parts[0] === 100 && parts[1] >= 64 && parts[1] <= 127;
        });
    } catch (error) { lastError = error; }
  }
  throw new Error(`tailscaleUnavailable: ${lastError?.message || ''}`);
}

function pairAtAddress(address, id, certificate) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: address, port: 24801 });
    let buffer = '';
    let settled = false;
    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      if (error) reject(error); else resolve(result);
    };
    socket.setTimeout(65000, () => finish(new Error('pairingTimedOut')));
    socket.on('connect', () => {
      socket.write(`${JSON.stringify({ id, certificate: certificate.toString('base64'), name: os.hostname().slice(0, 128) })}\n`);
    });
    socket.on('error', (error) => finish(error));
    socket.on('data', (chunk) => {
      buffer += chunk.toString();
      if (buffer.length > 32768) { finish(new Error('pairingInvalid')); return; }
      let end;
      while ((end = buffer.indexOf('\n')) >= 0) {
        let reply;
        try { reply = JSON.parse(buffer.slice(0, end)); } catch { finish(new Error('pairingInvalid')); return; }
        buffer = buffer.slice(end + 1);
        if (reply.status === 'pending') continue;
        if (reply.status !== 'approved' || typeof reply.certificate !== 'string') {
          finish(new Error(reply.status === 'denied' ? 'pairingDenied' : 'idNotFound'));
          return;
        }
        const peerCertificate = Buffer.from(reply.certificate, 'base64');
        if (!peerCertificate.length || peerCertificate.length > 16384 || pairingId(peerCertificate) !== id) {
          finish(new Error('pairingInvalid'));
          return;
        }
        finish(null, { address: `${address}:24800`, certificate: peerCertificate });
      }
    });
    socket.on('close', () => { if (!settled) finish(new Error('pairingTimedOut')); });
  });
}

function validAddress(value) {
  if (typeof value !== 'string') return false;
  const match = value.match(/^(?:\[([^\]]+)\]|([^:]+)):(\d{1,5})$/);
  return Boolean(match && net.isIP(match[1] || match[2]) && Number(match[3]) > 0 && Number(match[3]) <= 65535);
}

function sendStatus(window, status, detail = '') {
  const target = window && !window.isDestroyed() ? window : mainWindow;
  if (target && !target.isDestroyed()) target.webContents.send('vortal:session', { status, detail });
}

function cancelReconnect() {
  if (reconnectTimer) clearTimeout(reconnectTimer);
  reconnectTimer = null;
}

function scheduleReconnect(window, detail = '') {
  if (!desiredAddress || quitting || reconnectTimer) return;
  const delay = reconnectDelay;
  reconnectDelay = Math.min(reconnectDelay * 2, 10000);
  sendStatus(window, 'reconnecting', detail);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    if (desiredAddress && !session && !quitting) startSession(window, desiredAddress);
  }, delay);
}

ipcMain.handle('vortal:setup', async () => ({
  identityReady: fs.existsSync(path.join(identityPath(), 'key.der'))
    && fs.existsSync(path.join(identityPath(), 'cert.der')),
  certificate: path.join(identityPath(), 'cert.der'),
  peerReady: fs.existsSync(peerCertPath()),
}));

ipcMain.handle('vortal:startup', async (_event, enabled) => {
  if (typeof enabled === 'boolean') {
    if (!app.isPackaged) throw new Error('Startup requires the packaged app');
    app.setLoginItemSettings({ openAtLogin: enabled });
  }
  return app.getLoginItemSettings().openAtLogin;
});

ipcMain.handle('vortal:clipboard-enabled', async (_event, enabled) => {
  if (typeof enabled !== 'boolean') throw new Error('Invalid clipboard setting');
  clipboardEnabled = enabled;
  startClipboard();
  return clipboardEnabled;
});

ipcMain.handle('vortal:audio-enabled', async (_event, enabled) => {
  if (typeof enabled !== 'boolean') throw new Error('Invalid audio setting');
  audioEnabled = enabled;
  if (sessionConnected) writeInput({ type: 'audioControl', enabled });
  return audioEnabled;
});

ipcMain.handle('vortal:locale', async (_event, language) => {
  if (language !== 'pt-BR' && language !== 'en') throw new Error('Unsupported language');
  const file = path.join(__dirname, '..', 'crates', 'vortal-ui', 'ui', 'locales', `${language}.json`);
  return JSON.parse(fs.readFileSync(file, 'utf8'));
});

ipcMain.handle('vortal:displays', async () => screen.getAllDisplays().map((display, index) => ({
  id: String(display.id),
  label: display.label || `${index + 1} · ${display.bounds.width}×${display.bounds.height}`,
  width: display.bounds.width,
  height: display.bounds.height,
})));

ipcMain.handle('vortal:edge-config', async (_event, config) => {
  if (!config || !['Left', 'Right', 'Top', 'Bottom'].includes(config.side)
    || !screen.getAllDisplays().some((display) => String(display.id) === String(config.displayId))) {
    throw new Error('Invalid screen edge configuration');
  }
  stopEdgeCapture();
  edgeConfig = { displayId: String(config.displayId), side: config.side };
  updateEdgePolling();
  return true;
});

ipcMain.handle('vortal:input-settings', async (_event, config) => {
  if (!config || !Number.isInteger(config.sensitivity) || config.sensitivity < 5
    || config.sensitivity > 100 || !Number.isInteger(config.scrollSensitivity)
    || config.scrollSensitivity < 25 || config.scrollSensitivity > 300
    || typeof config.keyboard !== 'boolean' || !validReturnShortcut(config.returnShortcut)
    || !['auto', 'us', 'abnt2'].includes(config.layout)) {
    throw new Error('Invalid input settings');
  }
  if (edgeActive && (config.keyboard !== inputSettings.keyboard || config.layout !== inputSettings.layout
    || config.returnShortcut !== inputSettings.returnShortcut)) {
    stopEdgeCapture();
  }
  inputSettings = { sensitivity: config.sensitivity, scrollSensitivity: config.scrollSensitivity,
    keyboard: config.keyboard, layout: config.layout, returnShortcut: config.returnShortcut };
  mouseRemainderX = 0;
  mouseRemainderY = 0;
  scrollRemainderX = 0;
  scrollRemainderY = 0;
  return true;
});

ipcMain.on('vortal:edge-return', (event) => {
  if (mainWindow && event.sender === mainWindow.webContents) stopEdgeCapture();
});

async function ensureIdentity() {
  const key = path.join(identityPath(), 'key.der');
  const certificate = path.join(identityPath(), 'cert.der');
  if (fs.existsSync(key) && fs.existsSync(certificate)) return certificate;
  if (fs.existsSync(key) || fs.existsSync(certificate)) throw new Error('Identity files are incomplete');
  const deviceId = os.hostname().slice(0, 128) || 'windows';
  await new Promise((resolve, reject) => {
    const child = spawn(cliPath(), ['identity', deviceId, '--dir', identityPath()], { windowsHide: true });
    let error = '';
    child.stderr.on('data', (chunk) => { error += chunk.toString().slice(0, 2048); });
    child.on('error', reject);
    child.on('close', (code) => code === 0 ? resolve(true) : reject(new Error(error || `Exit code ${code}`)));
  });
  return certificate;
}

ipcMain.handle('vortal:create-identity', async () => ({ certificate: await ensureIdentity() }));

ipcMain.handle('vortal:choose-peer-cert', async (event) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  const result = await dialog.showOpenDialog(window, { properties: ['openFile'], filters: [{ name: 'DER certificate', extensions: ['der'] }] });
  if (result.canceled || !result.filePaths[0]) return false;
  fs.mkdirSync(app.getPath('userData'), { recursive: true });
  fs.copyFileSync(result.filePaths[0], peerCertPath());
  return true;
});

ipcMain.handle('vortal:pair-by-id', async (_event, rawId) => {
  const id = String(rawId || '').toUpperCase().replace(/[^A-Z0-9]/g, '');
  if (!/^[A-Z]{2}\d{6}$/.test(id)) throw new Error('pairingInvalidId');
  await ensureIdentity();
  const certificate = fs.readFileSync(path.join(identityPath(), 'cert.der'));
  const peers = await tailscalePeers();
  if (!peers.length) throw new Error('idNotFound');
  let approved;
  try {
    approved = await Promise.any(peers.map((address) => pairAtAddress(address, id, certificate)));
  } catch (error) {
    const failures = error.errors || [];
    if (failures.some((failure) => failure.message === 'pairingDenied')) throw new Error('pairingDenied');
    if (failures.some((failure) => failure.message === 'pairingTimedOut')) throw new Error('pairingTimedOut');
    throw new Error('idNotFound');
  }
  fs.mkdirSync(app.getPath('userData'), { recursive: true });
  const temporary = `${peerCertPath()}.tmp`;
  fs.writeFileSync(temporary, approved.certificate);
  fs.renameSync(temporary, peerCertPath());
  return approved.address;
});

function startSession(window, address) {
  if (!desiredAddress || session || quitting) return;
  const child = spawn(cliPath(), ['connect-input', address, '--identity', identityPath(), '--peer-cert', peerCertPath()], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  session = child;
  sessionConnected = false;
  let output = '';
  let error = '';
  child.stdin.on('error', (failure) => {
    error = failure.message;
    child.kill();
  });
  child.stdout.on('data', (chunk) => {
    if (session !== child) return;
    output += chunk.toString();
    let end;
    while ((end = output.indexOf('\n')) >= 0) {
      const line = output.slice(0, end).trim();
      output = output.slice(end + 1);
      if (line === 'VORTAL_INPUT_CONNECTED') {
        if (session !== child) continue;
        sessionConnected = true;
        updateEdgePolling();
        reconnectDelay = 1000;
        startClipboard();
        writeInput({ type: 'audioControl', enabled: audioEnabled });
        sendStatus(window, 'connected');
      } else if (line === 'VORTAL_INPUT_RETURN') {
        stopEdgeCapture();
        sendStatus(window, 'returned');
      } else if (line.startsWith('{')) {
        try {
          const message = JSON.parse(line);
          if (message.type === 'clipboardText' && clipboardEnabled && typeof message.text === 'string'
            && Buffer.byteLength(message.text, 'utf8') <= 16384) {
            lastClipboardText = message.text;
            clipboard.writeText(message.text);
          } else if (clipboardEnabled && fileClipboard
            && (message.type === 'clipboardOffer' || message.type === 'clipboardData')) {
            fileClipboard.receive(message);
          } else if (message.type === 'audioPcm' && audioEnabled && typeof message.data === 'string'
            && message.data.length <= 5200 && !window.isDestroyed()) {
            window.webContents.send('vortal:audio', message.data);
          }
        } catch { /* Ignore malformed helper output. */ }
      }
    }
  });
  child.stderr.on('data', (chunk) => { error = (error + chunk.toString()).slice(-2048); });
  child.on('error', (failure) => {
    if (session !== child) return;
    sessionConnected = false;
    updateEdgePolling();
    stopClipboard();
    stopEdgeCapture();
    if (session === child) session = null;
    scheduleReconnect(window, failure.message);
  });
  child.on('close', (code) => {
    if (session !== child) return;
    sessionConnected = false;
    updateEdgePolling();
    stopClipboard();
    stopEdgeCapture();
    if (session === child) session = null;
    scheduleReconnect(window, error.trim() || `Connection closed (${code})`);
  });
  sendStatus(window, 'connecting');
}

ipcMain.handle('vortal:connect', async (event, address) => {
  const window = BrowserWindow.fromWebContents(event.sender);
  if (!validAddress(address)) throw new Error('Invalid peer address');
  if (!fs.existsSync(peerCertPath())) throw new Error('Peer certificate missing');
  await ensureIdentity();
  if (session) throw new Error('Session already active');
  cancelReconnect();
  desiredAddress = address;
  reconnectDelay = 1000;
  startSession(window, address);
  return true;
});

ipcMain.on('vortal:input', (_event, input) => {
  if (!session || !session.stdin.writable || !input || typeof input !== 'object') return;
  if (input.type === 'leaveScreen') { writeInput({ type: 'leaveScreen' }); return; }
  if (input.type === 'enterScreen') {
    if (['Left', 'Right', 'Top', 'Bottom'].includes(input.edge)
      && Number.isInteger(input.position) && input.position >= 0 && input.position <= 65535) writeInput(input);
    return;
  }
  const allowed = {
    mouseMove: ['dx', 'dy'], mouseButton: ['button', 'pressed'],
    mouseScroll: ['horizontal', 'vertical'], key: ['code', 'pressed'],
  };
  const fields = allowed[input.type];
  if (!fields || !fields.every((field) => typeof input[field] === (field === 'pressed' ? 'boolean' : 'number') && (field === 'pressed' || Number.isInteger(input[field]) && Math.abs(input[field]) <= 65535))) return;
  writeInput(input);
});

ipcMain.on('vortal:disconnect', () => {
  desiredAddress = null;
  cancelReconnect();
  sessionConnected = false;
  updateEdgePolling();
  stopClipboard();
  stopEdgeCapture();
  if (session) {
    session.stdin.end();
    session.kill();
    session = null;
  }
});

function createWindow() {
  const window = new BrowserWindow({
    width: 1000,
    height: 640,
    minWidth: 700,
    minHeight: 500,
    show: false,
    autoHideMenuBar: true,
    backgroundColor: '#080808',
    title: 'Vortal',
    icon: path.join(__dirname, '..', 'crates', 'vortal-desktop', 'assets', 'vortal.ico'),
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });
  mainWindow = window;

  Menu.setApplicationMenu(null);
  window.loadFile(path.join(__dirname, '..', 'crates', 'vortal-ui', 'ui', 'index.html'));
  window.once('ready-to-show', () => window.show());
  window.webContents.on('will-navigate', (event, url) => {
    event.preventDefault();
    if (url === 'https://github.com/Vicente-PIlonetto/Vortal') void shell.openExternal(url);
  });
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.on('render-process-gone', () => { stopEdgeCapture(); if (session) session.kill(); });
  window.on('close', (event) => {
    if (quitting) return;
    event.preventDefault();
    window.hide();
  });
  window.on('closed', () => { mainWindow = null; });
}

function showMainWindow() {
  if (!mainWindow || mainWindow.isDestroyed()) createWindow();
  else {
    mainWindow.show();
    mainWindow.focus();
  }
}

function createTray() {
  const icon = path.join(__dirname, '..', 'crates', 'vortal-desktop', 'assets', 'vortal.ico');
  const language = app.getLocale().toLowerCase().startsWith('pt') ? 'pt-BR' : 'en';
  const file = path.join(__dirname, '..', 'crates', 'vortal-ui', 'ui', 'locales', `${language}.json`);
  const labels = JSON.parse(fs.readFileSync(file, 'utf8'));
  tray = new Tray(icon);
  tray.setToolTip('Vortal');
  tray.setContextMenu(Menu.buildFromTemplate([
    { label: labels.trayOpen, click: showMainWindow },
    { type: 'separator' },
    { label: labels.trayQuit, click: () => app.quit() },
  ]));
  tray.on('double-click', showMainWindow);
}

const hasInstanceLock = app.requestSingleInstanceLock();
if (!hasInstanceLock) app.quit();
else app.on('second-instance', showMainWindow);

app.whenReady().then(() => {
  if (!hasInstanceLock) return;
  app.setName('Vortal');
  fileClipboard = createFileClipboard({
    dataDirectory: app.getPath('userData'),
    canTransfer: () => clipboardEnabled && sessionConnected && Boolean(session?.stdin.writable),
    send: async (message) => {
      const target = session;
      if (!clipboardEnabled || !sessionConnected || !target?.stdin.writable) return false;
      for (let attempt = 0; target.stdin.writableLength > 8192; attempt += 1) {
        if (attempt >= 150 || session !== target || !sessionConnected) return false;
        await new Promise((resolve) => setTimeout(resolve, 2));
      }
      return new Promise((resolve) => {
        const timeout = setTimeout(() => resolve(false), 300);
        target.stdin.write(`${JSON.stringify(message)}\n`, (error) => {
          clearTimeout(timeout);
          resolve(!error && session === target);
        });
      });
    },
  });
  createTray();
  createWindow();
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('before-quit', () => {
  quitting = true;
  desiredAddress = null;
  sessionConnected = false;
  updateEdgePolling();
  cancelReconnect();
  stopEdgeCapture();
  stopClipboard();
  if (session) session.kill();
});

app.on('window-all-closed', () => {
  if (!tray && process.platform !== 'darwin') app.quit();
});
