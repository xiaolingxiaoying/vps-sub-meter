// Run with `node tests/index-copy.mjs`; no browser or npm dependencies needed.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';

const script = await readFile(new URL('../src/subscription/copy.js', import.meta.url), 'utf8');

function fixture({ clipboard, fallback = () => true } = {}) {
  const classes = new Set();
  const btn = {
    textContent: '复制链接', dataset: {},
    classList: {
      toggle(name, enabled) { enabled ? classes.add(name) : classes.delete(name); },
      remove(name) { classes.delete(name); },
    },
    focus() { document.activeElement = this; },
  };
  const status = { hidden: true, textContent: '' };
  const children = new Set();
  const timers = new Map();
  let nextTimer = 0;
  const document = {
    activeElement: btn,
    getElementById: () => status,
    createElement: () => ({
      style: {}, focus() { document.activeElement = this; }, select() {},
    }),
    body: { appendChild: child => children.add(child), removeChild: child => children.delete(child) },
    execCommand: fallback,
  };
  const context = {
    navigator: { clipboard }, document,
    setTimeout: callback => { timers.set(++nextTimer, callback); return nextTimer; },
    clearTimeout: timer => timers.delete(timer),
  };
  runInNewContext(script, context);
  return {
    btn, status, children, document, classes,
    copy: text => context.copyUrl(btn, text),
    expire() { for (const callback of timers.values()) callback(); timers.clear(); },
  };
}

// Rapid repeated clicks used to capture "已复制" as the permanent label.
{
  const test = fixture();
  test.copy('first');
  test.copy('second');
  assert.equal(test.btn.textContent, '已复制 ✓');
  assert.equal(test.classes.has('btn-success'), true);
  assert.equal(test.status.textContent, '链接已复制。');
  test.expire();
  assert.equal(test.btn.textContent, '复制链接');
  assert.equal(test.classes.has('btn-success'), false);
  assert.equal(test.document.activeElement, test.btn);
  assert.equal(test.children.size, 0);
}

// Both clipboard denial paths need feedback and must clean up the textarea.
for (const fallback of [() => false, () => { throw new Error('denied'); }]) {
  const test = fixture({ fallback });
  test.copy('secret');
  assert.equal(test.btn.textContent, '复制失败');
  assert.equal(test.status.hidden, false);
  assert.equal(test.status.textContent, '复制失败，请手动选择并复制链接。');
  assert.equal(test.children.size, 0);
  assert.equal(test.document.activeElement, test.btn);
  test.expire();
  assert.equal(test.btn.textContent, '复制链接');
}

// A stale asynchronous clipboard response must not overwrite a newer result.
{
  const pending = [];
  const test = fixture({
    clipboard: { writeText: text => new Promise((resolve, reject) => pending.push({ text, resolve, reject })) },
    fallback: () => false,
  });
  test.copy('first');
  test.copy('second');
  assert.deepEqual(pending.map(item => item.text), ['first', 'second']);
  pending[1].reject(new Error('denied'));
  await Promise.resolve();
  assert.equal(test.btn.textContent, '复制失败');
  pending[0].resolve();
  await Promise.resolve();
  assert.equal(test.btn.textContent, '复制失败');
  assert.equal(test.classes.has('btn-success'), false);
  test.expire();
  assert.equal(test.btn.textContent, '复制链接');
}

console.log('index copy: repeated clicks, failure feedback, focus cleanup and stale promises passed');
