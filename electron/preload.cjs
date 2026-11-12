const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('vortalDesktop', {
  setup: () => ipcRenderer.invoke('vortal:setup'),
  startup: (enabled) => ipcRenderer.invoke('vortal:startup', enabled),
  configureClipboard: (enabled) => ipcRenderer.invoke('vortal:clipboard-enabled', enabled),
  configureAudio: (enabled) => ipcRenderer.invoke('vortal:audio-enabled', enabled),
  onAudio: (callback) => ipcRenderer.on('vortal:audio', (_event, data) => callback(data)),
  locale: (language) => ipcRenderer.invoke('vortal:locale', language),
  createIdentity: () => ipcRenderer.invoke('vortal:create-identity'),
  choosePeerCert: () => ipcRenderer.invoke('vortal:choose-peer-cert'),
  pairById: (id) => ipcRenderer.invoke('vortal:pair-by-id', id),
  connect: (address) => ipcRenderer.invoke('vortal:connect', address),
  disconnect: () => ipcRenderer.send('vortal:disconnect'),
  input: (event) => ipcRenderer.send('vortal:input', event),
  onStatus: (callback) => ipcRenderer.on('vortal:session', (_event, status) => callback(status)),
  displays: () => ipcRenderer.invoke('vortal:displays'),
  configureEdge: (config) => ipcRenderer.invoke('vortal:edge-config', config),
  configureInput: (config) => ipcRenderer.invoke('vortal:input-settings', config),
  returnFromEdge: () => ipcRenderer.send('vortal:edge-return'),
  onEdgeState: (callback) => ipcRenderer.on('vortal:edge-state', (_event, active) => callback(active)),
  onInputError: (callback) => ipcRenderer.on('vortal:input-error', (_event, code) => callback(code)),
});
