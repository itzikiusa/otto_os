import { mount } from 'svelte';
import './lib/tokens.css';
import './app.css';
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
