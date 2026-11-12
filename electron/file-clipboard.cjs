const fs = require('node:fs');
const path = require('node:path');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { createHash } = require('node:crypto');

const execFileAsync = promisify(execFile);
const MAX_BYTES = 32 * 1024 * 1024;
const CHUNK_BYTES = 16 * 1024;
const SEND_CHUNK_BYTES = 8 * 1024;

function validName(name) {
  return typeof name === 'string' && name.length > 0 && Buffer.byteLength(name) <= 240
    && name !== '.' && name !== '..' && !/[\\/<>:"|?*\x00-\x1f]/.test(name);
}

async function powershell(command) {
  const result = await execFileAsync('powershell.exe', ['-NoProfile', '-NonInteractive', '-STA', '-Command', command],
    { windowsHide: true, timeout: 4000, maxBuffer: 64 * 1024 });
  return result.stdout.trim().replace(/^\uFEFF/, '');
}

async function readFilePaths() {
  try {
    const output = await powershell('[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); Add-Type -AssemblyName System.Windows.Forms; $list=[Windows.Forms.Clipboard]::GetFileDropList(); $paths=@(); foreach($file in $list) { $paths += [string]$file }; ConvertTo-Json -InputObject ([string[]]$paths) -Compress');
    if (output) {
      const paths = JSON.parse(output);
      if (Array.isArray(paths) && paths.length) return paths;
      if (typeof paths === 'string' && paths) return [paths];
    }
  } catch { /* A different clipboard owner may block this read. */ }
  try {
    const { clipboard } = require('electron');
    const buffer = clipboard.readBuffer('FileNameW');
    const file = buffer.toString('utf16le').split('\0')[0];
    if (file && fs.existsSync(file)) return [file];
  } catch { /* The native fallback may be unavailable. */ }
  return [];
}

async function writeFilePaths(paths) {
  const encoded = Buffer.from(JSON.stringify(paths), 'utf8').toString('base64');
  const command = `Add-Type -AssemblyName System.Windows.Forms; $items=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('${encoded}')) | ConvertFrom-Json; $list=New-Object System.Collections.Specialized.StringCollection; foreach($item in $items) { [void]$list.Add([string]$item) }; [Windows.Forms.Clipboard]::SetFileDropList($list)`;
  await powershell(command);
}

function parseArchive(archive) {
  if (archive.length < 4 || archive.length > MAX_BYTES) return null;
  const headerLength = archive.readUInt32LE();
  if (headerLength > 16 * 1024 || headerLength + 4 > archive.length) return null;
  let entries;
  try { entries = JSON.parse(archive.subarray(4, 4 + headerLength).toString('utf8')); }
  catch { return null; }
  if (!Array.isArray(entries) || !entries.length || entries.length > 16
    || entries.some((item) => !validName(item.name) || !Number.isSafeInteger(item.size) || item.size < 0
      || (item.kind !== undefined && item.kind !== 'image'))
    || (entries.some((item) => item.kind === 'image') && entries.length !== 1)) return null;
  if (entries.reduce((sum, item) => sum + item.size, 0) !== archive.length - headerLength - 4) return null;
  return { entries, offset: 4 + headerLength };
}

function readClipboardImage() {
  const { clipboard } = require('electron');
  const image = clipboard.readImage();
  return image.isEmpty() ? null : image.toPNG();
}

function createFileClipboard({ dataDirectory, canTransfer, send, readPaths = readFilePaths,
  readImage = readClipboardImage, writePaths = writeFilePaths }) {
  let timer = null;
  let generation = 0;
  let lastSignature = null;
  let incoming = null;
  let filesActive = false;

  async function sendArchive(entries, buffers, currentGeneration) {
    const header = Buffer.from(JSON.stringify(entries), 'utf8');
    if (header.length > 16 * 1024) return;
    const length = Buffer.alloc(4);
    length.writeUInt32LE(header.length);
    const archive = Buffer.concat([length, header, ...buffers]);
    if (archive.length > MAX_BYTES || !canTransfer() || generation !== currentGeneration) return;
    const transferId = Date.now() * 1000 + Math.floor(Math.random() * 1000);
    const chunkCount = Math.ceil(archive.length / SEND_CHUNK_BYTES);
    if (!await send({ type: 'clipboardOffer', transferId, totalSize: archive.length, chunkCount })) return;
    for (let index = 0; index < chunkCount; index += 1) {
      if (!canTransfer() || generation !== currentGeneration) return;
      const data = archive.subarray(index * SEND_CHUNK_BYTES, (index + 1) * SEND_CHUNK_BYTES).toString('base64');
      if (!await send({ type: 'clipboardData', transferId, chunkIndex: index, data, finalChunk: index + 1 === chunkCount })) return;
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
  }

  async function sendSelection(paths, currentGeneration) {
    if (!paths.length || paths.length > 16) return;
    const entries = [];
    const buffers = [];
    let size = 0;
    for (const file of paths) {
      const name = path.basename(file);
      if (!validName(name)) return;
      const stat = await fs.promises.stat(file);
      if (!stat.isFile() || size + stat.size > MAX_BYTES - 16 * 1024) return;
      const buffer = await fs.promises.readFile(file);
      entries.push({ name, size: buffer.length });
      buffers.push(buffer);
      size += buffer.length;
    }
    await sendArchive(entries, buffers, currentGeneration);
  }

  async function poll() {
    if (!canTransfer()) return;
    try {
      const paths = await readPaths();
      let signature;
      let sendCurrent;
      if (paths.length) {
        signature = `files:${JSON.stringify(paths)}`;
        sendCurrent = (currentGeneration) => sendSelection(paths, currentGeneration);
      } else {
        const png = await readImage();
        if (!Buffer.isBuffer(png) || png.length < 8 || png.length > MAX_BYTES - 16 * 1024) {
          filesActive = false;
          lastSignature = null;
          return;
        }
        signature = `image:${createHash('sha256').update(png).digest('hex')}`;
        const name = `Screenshot-${new Date().toISOString().replace(/[:.]/g, '-')}.png`;
        sendCurrent = (currentGeneration) => sendArchive([{ name, size: png.length, kind: 'image' }], [png], currentGeneration);
      }
      filesActive = true;
      if (signature === lastSignature) return;
      lastSignature = signature;
      generation += 1;
      await sendCurrent(generation);
    } catch { /* The clipboard may be temporarily locked by another app. */ }
  }

  async function saveIncoming(batch) {
    const archive = Buffer.concat(batch.parts);
    if (archive.length !== batch.totalSize) return;
    const parsed = parseArchive(archive);
    if (!parsed) return;
    const { entries } = parsed;
    const root = path.join(dataDirectory, 'clipboard-files', String(batch.transferId));
    const paths = [];
    let { offset } = parsed;
    for (const [index, entry] of entries.entries()) {
      const directory = path.join(root, String(index));
      await fs.promises.mkdir(directory, { recursive: true });
      const file = path.join(directory, entry.name);
      await fs.promises.writeFile(file, archive.subarray(offset, offset + entry.size));
      offset += entry.size;
      paths.push(file);
    }
    await writePaths(paths);
    lastSignature = JSON.stringify(paths);
    filesActive = true;
  }

  function receive(message) {
    if (message.type === 'clipboardOffer') {
      incoming = null;
      if (Number.isSafeInteger(message.transferId) && Number.isInteger(message.totalSize)
        && message.totalSize > 0 && message.totalSize <= MAX_BYTES
        && Number.isInteger(message.chunkCount) && message.chunkCount > 0 && message.chunkCount <= 4096) {
        incoming = { transferId: message.transferId, totalSize: message.totalSize,
          chunkCount: message.chunkCount, nextIndex: 0, size: 0, parts: [] };
      }
      return;
    }
    if (message.type !== 'clipboardData' || !incoming) return;
    const batch = incoming;
    if (typeof message.data !== 'string' || message.data.length > 22000) { incoming = null; return; }
    const data = Buffer.from(message.data, 'base64');
    if (message.transferId !== batch.transferId || message.chunkIndex !== batch.nextIndex
      || data.length > CHUNK_BYTES || batch.size + data.length > batch.totalSize) { incoming = null; return; }
    batch.parts.push(data);
    batch.size += data.length;
    batch.nextIndex += 1;
    if (message.finalChunk) {
      incoming = null;
      if (batch.nextIndex === batch.chunkCount && batch.size === batch.totalSize) {
        void saveIncoming(batch).catch(() => {});
      }
    }
  }

  return {
    start() { if (!timer) { generation += 1; void poll(); timer = setInterval(() => { void poll(); }, 1200); timer.unref(); } },
    stop() { generation += 1; if (timer) clearInterval(timer); timer = null; incoming = null; filesActive = false; lastSignature = null; },
    receive,
    hasFiles() { return filesActive; },
    checkNow: poll,
  };
}

module.exports = { createFileClipboard, parseArchive };
