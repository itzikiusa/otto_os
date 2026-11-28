<script lang="ts">
  // Built-in Voice / TTS Studio (`#/design/tts`) under unified Product:
  // macOS Built-in Voices out-of-the-box + extensible HTTP TTS providers,
  // character voice presets for game NPCs/narrator, dialogue script batch
  // synthesis, interactive waveform player, clip history, and 1-click export to workspace.
  import { onDestroy, untrack } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import ListDetail from '../../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../../lib/components/side/ListPane.svelte';
  import SideSection from '../../../lib/components/side/SideSection.svelte';
  import SideRow from '../../../lib/components/side/SideRow.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import ProductModeTabs from '../../../lib/components/ProductModeTabs.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { downloadBlob } from '../../../lib/downloadBlob';
  import { loadErrorText } from '../../../lib/loadError';
  import { plural } from '../../../lib/plural';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { keyboard } from '../../../lib/stores/keyboard.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import * as mapi from '../../../lib/api/media';
  import type { TtsClip, TtsFormat, TtsProvider, TtsVoice } from '../../../lib/api/types';
  import WorkspaceExportModal from '../studio3d/WorkspaceExportModal.svelte';
  import TtsProviderManagerModal from './TtsProviderManagerModal.svelte';
  import TtsFields from './TtsFields.svelte';
  import WaveformPlayer from './WaveformPlayer.svelte';
  import {
    formatAudioBytes,
    formatDurationMs,
    loadCharacterPresets,
    matchPresetByCharacter,
    parseDialogueScript,
    saveCharacterPresets,
    resolveVoiceId,
    slugifyClipName,
    uniqueVoices,
    type CharacterVoicePreset,
  } from './ttsPresets';

  const canEdit = $derived(auth.can('product', 'edit') || auth.can('design', 'edit'));
  const canAdmin = $derived(auth.can('product', 'admin') || auth.can('design', 'admin'));
  const wsId = $derived(ws.currentId);

  let providers = $state<TtsProvider[]>([]);
  let voices = $state<TtsVoice[]>([]);
  let clips = $state<TtsClip[]>([]);
  let loadingInit = $state(true);
  let loadingVoices = $state(false);
  let voicesError = $state<string | null>(null);
  let initError = $state<string | null>(null);
  let manageOpen = $state(false);
  let exportClip = $state<TtsClip | null>(null);

  // Synthesis controls
  let providerId = $state<string>('macos-say');
  let voiceId = $state<string>('Samantha');
  let speed = $state<number>(1.0);
  let format = $state<TtsFormat>('wav');
  let title = $state('');
  let characterName = $state('');
  let scriptText = $state('');
  let batchMode = $state(false);
  let generating = $state(false);
  let batchProgress = $state<{ done: number; total: number } | null>(null);

  // Character voice presets
  let presets = $state<CharacterVoicePreset[]>([]);
  // A phone shows the editor first; "‹ Voices" goes to the presets and clips.
  let phoneOpen = $state(true);
  let activePresetId = $state<string | null>(null);

  // Active clip audio playback
  let selectedClipId = $state<string | null>(null);
  let activeAudioUrl = $state<string | null>(null);
  let activeAudioBlob = $state<Blob | null>(null);
  let audioSeq = 0;
  // Bumped per load, so a late answer for a previous workspace/provider is dropped.
  let loadSeq = 0;
  let voiceSeq = 0;

  const activeProvider = $derived(providers.find((p) => p.id === providerId) ?? providers[0] ?? null);
  const supportedFormats = $derived<TtsFormat[]>(activeProvider?.supported_formats?.length ? activeProvider.supported_formats : ['wav', 'm4a', 'aiff']);
  const selectedClip = $derived(clips.find((c) => c.id === selectedClipId) ?? null);
  const parsedLines = $derived(batchMode ? parseDialogueScript(scriptText) : []);

  function revokeAudio(): void {
    if (activeAudioUrl) {
      URL.revokeObjectURL(activeAudioUrl);
      activeAudioUrl = null;
    }
    activeAudioBlob = null;
  }

  // A load still in flight answers a page that is gone: drop it (no blob URL to leak, no toast).
  onDestroy(() => {
    loadSeq++;
    audioSeq++;
    revokeAudio();
  });

  async function loadAll(targetWsId: string | null = wsId): Promise<void> {
    const my = ++loadSeq;
    loadingInit = true;
    initError = null;
    try {
      const ps = await mapi.listTtsProviders();
      if (my !== loadSeq) return;
      providers = ps;
      if (!ps.some((p) => p.id === providerId)) {
        const ready = ps.find((p) => p.enabled && p.ready) ?? ps[0];
        if (ready) providerId = ready.id;
      }
      if (targetWsId) {
        const list = await mapi.listTtsClips({ workspace_id: targetWsId, limit: 100 });
        if (my !== loadSeq) return;
        clips = list;
        if (!selectedClipId && list[0]) void selectClip(list[0]);
      }
    } catch (e) {
      if (my === loadSeq) initError = loadErrorText(e);
    } finally {
      if (my === loadSeq) loadingInit = false;
    }
  }

  async function loadVoices(pid: string): Promise<void> {
    if (!pid) return;
    const my = ++voiceSeq;
    loadingVoices = true;
    voicesError = null;
    try {
      const list = uniqueVoices(await mapi.listTtsVoices(pid));
      if (my !== voiceSeq) return;
      voices = list;
      if (list.length > 0) {
        voiceId = resolveVoiceId(list, voiceId) ?? resolveVoiceId(list, activeProvider?.default_voice) ?? list[0].id;
      }
    } catch (e) {
      if (my !== voiceSeq) return;
      voices = [];
      voicesError = loadErrorText(e);
    } finally {
      if (my === voiceSeq) loadingVoices = false;
    }
  }

  $effect(() => {
    const id = wsId;
    untrack(() => {
      selectedClipId = null;
      clips = [];
      audioSeq++;
      revokeAudio();
      presets = loadCharacterPresets(id);
      void loadAll(id);
    });
  });

  $effect(() => {
    const pid = providerId;
    if (!pid || providers.length === 0) return;
    untrack(() => {
      if (!supportedFormats.includes(format)) format = supportedFormats[0] ?? 'wav';
      void loadVoices(pid);
    });
  });

  async function selectClip(clip: TtsClip): Promise<void> {
    selectedClipId = clip.id;
    const my = ++audioSeq;
    try {
      const blob = await mapi.fetchTtsClipAudioBlob(clip.id);
      if (my !== audioSeq) return;
      revokeAudio();
      activeAudioBlob = blob;
      activeAudioUrl = URL.createObjectURL(blob);
    } catch (e) {
      if (my === audioSeq) toastError('Couldn’t load clip audio', e);
    }
  }

  function applyPreset(p: CharacterVoicePreset): void {
    activePresetId = p.id;
    characterName = p.name;
    if (providers.some((pr) => pr.id === p.provider_id)) providerId = p.provider_id;
    voiceId = resolveVoiceId(voices, p.voice_id) ?? p.voice_id;
    speed = p.speed;
    format = p.format;
  }

  async function saveCurrentAsPreset(): Promise<void> {
    const name = await confirmer.promptText('Name this game character or narrator preset.', {
      title: 'Save character preset',
      confirmLabel: 'Save preset',
      initial: characterName || '',
      placeholder: 'Tavern Keeper',
    });
    if (!name) return;
    const id = slugifyClipName(name, `preset_${Date.now()}`);
    const next: CharacterVoicePreset = {
      id,
      name,
      role: 'Custom character voice',
      provider_id: providerId,
      voice_id: voiceId,
      speed,
      format,
      style_notes: '',
    };
    const updated = [...presets.filter((p) => p.id !== id), next];
    presets = updated;
    activePresetId = id;
    characterName = name;
    saveCharacterPresets(wsId, updated);
    toasts.success('Saved character preset', name);
  }

  async function synthesizeOne(): Promise<void> {
    const ws = wsId;
    if (!ws) {
      toasts.warn('Pick a workspace first', 'Generated voice clips belong to the active workspace.');
      return;
    }
    const text = scriptText.trim();
    if (!text || generating) return;
    generating = true;
    try {
      if (batchMode) {
        const lines = parseDialogueScript(text);
        if (lines.length === 0) {
          toasts.warn('No dialogue lines found', 'Write lines as CHARACTER: Dialogue text.');
          return;
        }
        batchProgress = { done: 0, total: lines.length };
        let last: TtsClip | null = null;
        for (const line of lines) {
          // A workspace switch mid-script stops it: the rest would land in the other workspace.
          if (wsId !== ws) break;
          const matched = matchPresetByCharacter(presets, line.character);
          const clip = await mapi.synthesizeTts({
            workspace_id: ws,
            provider_id: matched?.provider_id ?? providerId,
            voice_id: matched?.voice_id ?? voiceId,
            text: line.text,
            speed: matched?.speed ?? speed,
            format: matched?.format ?? format,
            title: line.suggestedTitle,
            character_name: line.character ?? (characterName.trim() || null),
          });
          if (wsId !== ws) break;
          clips = [clip, ...clips];
          last = clip;
          batchProgress = { done: batchProgress.done + 1, total: lines.length };
        }
        if (last && wsId === ws) {
          void selectClip(last);
          toasts.success('Dialogue script generated', plural(lines.length, 'voice clip'));
        }
      } else {
        const clip = await mapi.synthesizeTts({
          workspace_id: ws,
          provider_id: providerId,
          voice_id: voiceId,
          text,
          speed,
          format,
          title: title.trim() || text.slice(0, 48),
          character_name: characterName.trim() || null,
        });
        if (wsId !== ws) return;
        clips = [clip, ...clips];
        await selectClip(clip);
        toasts.success('Voice clip generated', clip.title);
      }
    } catch (e) {
      toastError('Couldn’t generate speech', e);
    } finally {
      generating = false;
      batchProgress = null;
    }
  }

  async function deleteClip(clip: TtsClip): Promise<void> {
    const ok = await confirmer.ask(`Delete “${clip.title}”? This removes the saved audio clip.`, {
      title: 'Delete audio clip',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await mapi.deleteTtsClip(clip.id);
      clips = clips.filter((c) => c.id !== clip.id);
      if (selectedClipId === clip.id) {
        selectedClipId = null;
        revokeAudio();
      }
      toasts.success('Deleted clip', clip.title);
    } catch (e) {
      toastError('Couldn’t delete the clip', e);
    }
  }

  // Keyboard up over the script field: keep the counter and Generate in view under it
  // (Chrome scrolls only the field itself into view, after its own resize).
  let ebarEl = $state<HTMLElement | null>(null);
  function showGenerate(): void {
    if (keyboard.open && document.activeElement?.id === 'tts-script') ebarEl?.scrollIntoView({ block: 'nearest' });
  }
  $effect(() => {
    if (!keyboard.open) return;
    const t = setTimeout(showGenerate, 350);
    return () => clearTimeout(t);
  });

  function downloadSelected(): void {
    if (!selectedClip || !activeAudioBlob) return;
    const stem = slugifyClipName(selectedClip.title);
    downloadBlob(activeAudioBlob, `${stem}.${selectedClip.format}`);
  }
</script>

<div class="tts-page" data-testid="tts-studio-page">
  <PageHeader title="Product" subtitle="Game dialogue, character voices and narration" tabsPlacement="start">
    {#snippet tabs()}
      <ProductModeTabs active="voice" />
    {/snippet}
    {#snippet actions()}
      <button type="button" class="btn small" onclick={() => (manageOpen = true)} data-testid="tts-manage-providers">
        <Icon name="gear" size={12} /> Providers
      </button>
    {/snippet}
  </PageHeader>

  <PageBody fill padded={false}>
    <LoadState what="Voice Studio" loading={loadingInit} error={initError} empty={providers.length === 0} onretry={() => void loadAll()}>
      <ListDetail module="design" pane="voice" name="Voices" open={phoneOpen} onback={() => (phoneOpen = false)} testid="tts-grid">
        {#snippet list()}
          <!-- Character presets and generated clips: the studio's list pane. -->
          <ListPane primary={{ icon: 'plus', label: 'Save current voice as character preset', onclick: () => void saveCurrentAsPreset() }} testid="tts-rail">
            <SideSection title="Character presets" count={presets.length || null}>
              {#each presets as p (p.id)}
                <SideRow icon="user" title={p.name} meta="{p.voice_id} · {p.speed}×" selected={activePresetId === p.id} onclick={() => { applyPreset(p); phoneOpen = true; }} />
              {:else}
                <p class="tts-none">Save the current voice as a preset with ＋.</p>
              {/each}
            </SideSection>
            <SideSection title="Generated clips" count={clips.length}>
              {#if clips.length === 0}
                <EmptyState variant="panel" icon="volume" title="No voice clips yet" body="Enter dialogue in the editor and click Generate Speech." />
              {:else}
                <div class="tts-clips" data-testid="tts-clip-history">
                  {#each clips as clip (clip.id)}
                    <SideRow
                      icon="volume"
                      title={clip.title}
                      meta="{clip.character_name ? `${clip.character_name} · ` : ''}{clip.voice_id} · {clip.format.toUpperCase()} · {formatDurationMs(clip.duration_ms)} · {formatAudioBytes(clip.size_bytes)}{clip.visemes?.cues?.length ? ` · ${plural(clip.visemes.cues.length, 'viseme')}` : ''}"
                      tip={clip.exported_path ? `Exported to ${clip.exported_path}` : undefined}
                      selected={selectedClipId === clip.id}
                      menu={canEdit ? () => [{ label: 'Delete clip…', icon: 'trash', danger: true, action: () => void deleteClip(clip) }] : undefined}
                      onclick={() => { void selectClip(clip); phoneOpen = true; }}
                    />
                  {/each}
                </div>
              {/if}
            </SideSection>
          </ListPane>
        {/snippet}

        <section class="tts-main" aria-label="Voice synthesis editor">
          <TtsFields
            {providers}
            {voices}
            {loadingVoices}
            {voicesError}
            onretryvoices={() => void loadVoices(providerId)}
            {supportedFormats}
            bind:providerId
            bind:voiceId
            bind:characterName
            bind:title
            bind:speed
            bind:format
          />

          <div class="tts-editor">
            <div class="tts-shead">
              <label class="k" for="tts-script">Dialogue / narration script</label>
              <label class="tts-check">
                <input type="checkbox" bind:checked={batchMode} />
                <span>Script mode (CHARACTER: line)</span>
              </label>
            </div>
            <textarea
              id="tts-script"
              class="input"
              dir="auto"
              rows="6"
              maxlength="10000"
              bind:value={scriptText}
              oninput={showGenerate}
              placeholder={batchMode
                ? 'Narrator: Beyond the northern ridge lies the sunken forge.\nHero: Keep your lantern lit — something moves in the mist.'
                : 'Enter dialogue, NPC barks, or narration to synthesize…'}
              data-testid="tts-script-input"
            ></textarea>
            <div class="tts-ebar" bind:this={ebarEl}>
              <span class="tts-dim">
                {#if batchProgress}
                  Generating line {batchProgress.done} of {batchProgress.total}…
                {:else if batchMode}
                  {plural(parsedLines.length, 'dialogue line')} ready · matches Character Presets automatically
                {:else}
                  {scriptText.length.toLocaleString()} / 10,000 chars
                {/if}
              </span>
              <button
                type="button"
                class="btn primary"
                disabled={!canEdit || !wsId || !scriptText.trim() || generating}
                onclick={() => void synthesizeOne()}
                data-testid="tts-generate-btn"
              >
                <Icon name="volume" size={13} />
                {generating ? 'Synthesizing…' : batchMode ? `Generate ${plural(parsedLines.length, 'line')}` : 'Generate speech'}
              </button>
            </div>
          </div>

          {#if selectedClip}
            <WaveformPlayer
              src={activeAudioUrl}
              title={selectedClip.title}
              subtitle={`${selectedClip.character_name ? `${selectedClip.character_name} · ` : ''}${selectedClip.voice_id} · ${selectedClip.format.toUpperCase()} · ${formatAudioBytes(selectedClip.size_bytes)}${selectedClip.visemes?.cues?.length ? ` · ${plural(selectedClip.visemes.cues.length, 'viseme cue')}` : ''}`}
              ondownload={downloadSelected}
              onexport={() => (exportClip = selectedClip)}
            />
          {/if}
        </section>
      </ListDetail>
    </LoadState>
  </PageBody>
</div>

{#if manageOpen}
  <TtsProviderManagerModal {canAdmin} onclose={() => (manageOpen = false)} onchanged={() => void loadAll()} />
{/if}

{#if exportClip}
  {@const c = exportClip}
  <WorkspaceExportModal
    title="Export audio clip to workspace"
    defaultPath={`assets/audio/${slugifyClipName(c.character_name ? `${c.character_name}_${c.title}` : c.title)}.${c.format}`}
    hint="Writes the audio file safely inside the active workspace root."
    onexport={(rel, overwrite) => mapi.exportTtsClipToWorkspace(c.id, { relative_path: rel, overwrite })}
    onexported={(resp) => {
      clips = clips.map((row) => (row.id === c.id ? { ...row, exported_path: resp.path } : row));
    }}
    onclose={() => (exportClip = null)}
  />
{/if}

<style>
  .tts-page {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
    container-type: inline-size;
  }
  /* Presets and clips are the list pane (ListDetail); the editor is the content. */
  .tts-clips {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .tts-none {
    margin: 0;
    padding-block: 4px;
    padding-inline: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tts-main {
    container: tts-main / inline-size;
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
  }
  .tts-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tts-editor textarea {
    inline-size: 100%;
    padding: 10px 12px;
    border-radius: var(--radius-m);
  }
  /* Keyboard up: a shorter field, so it and Generate fit the strip of screen left, with a
     few pixels between Generate and the keyboard. */
  :global(html[data-kb='open']) .tts-editor textarea {
    max-block-size: max(72px, calc(var(--app-h, 100vh) * 0.3));
  }
  .tts-ebar {
    scroll-margin-block-end: 6px;
  }
  /* Only a short strip left over the keyboard: the label row steps aside; on a wide one (a
     phone on its side keeps ~70 px) the field and its counter + Generate sit side by side. */
  :global(html[data-kb-short]) .tts-main {
    padding-block: 4px;
  }
  :global(html[data-kb-short]) .tts-editor .tts-shead {
    display: none;
  }
  @container tts-main (min-width: 560px) {
    :global(html[data-kb-short]) .tts-editor {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      align-items: end;
      gap: 2px 8px;
    }
    :global(html[data-kb-short]) .tts-editor textarea {
      max-block-size: calc(var(--app-h, 100vh) - 14px);
    }
    :global(html[data-kb-short]) .tts-ebar {
      flex-direction: column;
      align-items: end;
      gap: 2px;
      scroll-margin-block-end: 0;
    }
  }
  .tts-ebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }
  .tts-dim {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tts-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
  }
  @container tts-main (max-width: 560px) {
    .tts-main {
      padding-block: 12px;
      padding-inline: 14px;
    }
  }
  @media (pointer: coarse) {
    .tts-page :global(.input:not(textarea)) {
      block-size: 40px;
    }
    .tts-editor textarea {
      resize: none;
    }
    .tts-check {
      min-block-size: 40px;
    }
    .tts-check input {
      inline-size: 20px;
      block-size: 20px;
    }
    .tts-page :global(.btn) {
      min-block-size: 40px;
    }
    .tts-page :global(.icon-btn) {
      min-inline-size: 40px;
      min-block-size: 40px;
    }
  }
</style>
