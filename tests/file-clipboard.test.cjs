const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { createFileClipboard, parseArchive } = require('../electron/file-clipboard.cjs');

function archive(entries, content) {
  const header = Buffer.from(JSON.stringify(entries));
  const length = Buffer.alloc(4);
  length.writeUInt32LE(header.length);
  return Buffer.concat([length, header, Buffer.from(content)]);
}

test('accepts multiple files and rejects path traversal', () => {
  const valid = archive([{ name: 'um.txt', size: 3 }, { name: 'dois.txt', size: 2 }], 'abcde');
  assert.equal(parseArchive(valid).entries.length, 2);
  assert.equal(parseArchive(archive([{ name: '../secreto', size: 1 }], 'x')), null);
  assert.equal(parseArchive(archive([{ name: 'pasta\\secreto', size: 1 }], 'x')), null);
  assert.equal(parseArchive(archive([{ name: 'um.txt', size: 9 }], 'x')), null);
});

test('sends copied files and restores the receiving clipboard', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'vortal-files-'));
  try {
    const first = path.join(directory, 'um.txt');
    const second = path.join(directory, 'dois.txt');
    fs.writeFileSync(first, 'abc');
    fs.writeFileSync(second, 'defg');
    const messages = [];
    const sender = createFileClipboard({ dataDirectory: directory, canTransfer: () => true,
      readPaths: async () => [first, second], send: async (message) => { messages.push(message); return true; } });
    await sender.checkNow();
    assert.equal(messages[0].type, 'clipboardOffer');
    assert.equal(messages.at(-1).finalChunk, true);
    let resolvePaste;
    const pasted = new Promise((resolve) => { resolvePaste = resolve; });
    const receiver = createFileClipboard({ dataDirectory: directory, canTransfer: () => true,
      send: async () => true, writePaths: async (paths) => resolvePaste(paths) });
    for (const message of messages) receiver.receive(message);
    const paths = await pasted;
    assert.deepEqual(paths.map((file) => fs.readFileSync(file, 'utf8')), ['abc', 'defg']);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test('sends a Print Screen image once as PNG clipboard data', async () => {
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lXcAAAAASUVORK5CYII=', 'base64');
  const messages = [];
  const sender = createFileClipboard({ dataDirectory: os.tmpdir(), canTransfer: () => true,
    readPaths: async () => [], readImage: () => png,
    send: async (message) => { messages.push(message); return true; } });
  await sender.checkNow();
  const sentCount = messages.length;
  await sender.checkNow();
  assert.equal(messages.length, sentCount);
  assert.equal(messages[0].type, 'clipboardOffer');
  const bytes = Buffer.concat(messages.slice(1).map((message) => Buffer.from(message.data, 'base64')));
  const parsed = parseArchive(bytes);
  assert.equal(parsed.entries[0].kind, 'image');
  assert.deepEqual(bytes.subarray(parsed.offset), png);
});
