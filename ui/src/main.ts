import { mount } from 'svelte';
import './lib/tokens.css';
import './app.css';
// Cousine: a monospace (Croscore) font with proper Hebrew glyphs. Used as the
// terminal's Hebrew fallback so RTL text renders crisp & aligned instead of
// falling back to a non-mono system font. latin.css enables it as a full
// primary option in the terminal font picker.
import '@fontsource/cousine/latin.css';
import '@fontsource/cousine/hebrew.css';
import App from './App.svelte';
import { mockEnabled, setupMock } from './lib/api/mock';
import { setToken, getToken } from './lib/api/client';

if (mockEnabled()) {
  setupMock();
  // mock auth: ensure a token exists so the shell loads straight away
  if (!getToken()) setToken('mock-token');
}

const app = mount(App, {
  target: document.getElementById('app')!,
});

export default app;
