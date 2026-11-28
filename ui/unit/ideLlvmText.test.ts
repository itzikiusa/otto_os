// Managed LLVM download confirm (rc.6 design B §1): the dialog names the
// source release, the exact pinned file and its sizes from the daemon's entry.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { llvmConfirmText, llvmSizes } from '../src/modules/ide/store/ideLlvmText.ts';

const mac = {
  version: '23.1.5',
  label: 'LLVM 23.1.5 — clang, clangd, clang-format, clang-tidy',
  lts: true,
  os: 'macos',
  arch: 'aarch64',
  url: 'https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/LLVM-23.1.5-macOS-ARM64.tar.zst',
  size_hint: '871 MB download · 444 MB on disk',
  sha256: '503de30ef90311e0b143623801b19af2250adac6d69faed65ad0051170202e3b',
};

describe('managed LLVM confirm', () => {
  it('splits the download and on-disk sizes', () => {
    assert.deepEqual(llvmSizes(mac), { download: '871 MB', disk: '444 MB' });
    assert.deepEqual(llvmSizes({ ...mac, size_hint: null }), { download: '', disk: '' });
  });

  it('names the release, the exact file and what lands on disk', () => {
    const text = llvmConfirmText(mac);
    assert.match(text, /^Source: github\.com\/llvm\/llvm-project, release llvmorg-23\.1\.5$/m);
    assert.match(text, /^File: LLVM-23\.1\.5-macOS-ARM64\.tar\.zst \(871 MB\)$/m);
    assert.match(text, /Installs 444 MB under Otto’s toolchains folder/);
    assert.match(text, /Debugging keeps using Apple’s lldb-dap/);
  });

  it('says nothing about Apple tools for a Linux build', () => {
    assert.doesNotMatch(llvmConfirmText({ ...mac, os: 'linux' }), /Apple/);
  });
});
