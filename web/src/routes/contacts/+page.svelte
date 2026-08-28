<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    listContacts, createContact, updateContact, deleteContact,
    type ContactInfo, type ContactInput,
  } from "$lib/services/tauri";
  import ModuleLogo from "$lib/components/ModuleLogo.svelte";
  import ModuleIcons from "$lib/components/ModuleIcons.svelte";

  let contacts = $state<ContactInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let search = $state("");
  let busy = $state(false);

  // Editor state
  let editorOpen = $state(false);
  let editingUid = $state<string | null>(null);
  let form = $state<ContactInput>({
    given_name: "", family_name: "", display_name: "",
    email: "", phone: "", organization: "",
  });

  async function loadContacts() {
    loading = true;
    error = null;
    try {
      contacts = await listContacts(search);
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => { loadContacts(); });

  function openCreate() {
    editingUid = null;
    form = { given_name: "", family_name: "", display_name: "", email: "", phone: "", organization: "" };
    editorOpen = true;
  }

  function openEdit(c: ContactInfo) {
    editingUid = c.vcard_uid;
    form = {
      given_name: c.given_name ?? "",
      family_name: c.family_name ?? "",
      display_name: c.display_name ?? "",
      email: c.email ?? "",
      phone: c.phone ?? "",
      organization: c.organization ?? "",
    };
    editorOpen = true;
  }

  async function saveContact() {
    busy = true;
    error = null;
    try {
      if (editingUid) {
        await updateContact(editingUid, form);
      } else {
        await createContact(form);
      }
      editorOpen = false;
      await loadContacts();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function removeContact(c: ContactInfo) {
    const name = c.display_name || c.email || "diesen Kontakt";
    if (!confirm(`Kontakt „${name}" wirklich löschen?`)) return;
    busy = true;
    error = null;
    try {
      await deleteContact(c.vcard_uid);
      await loadContacts();
    } catch (e: unknown) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  function initials(c: ContactInfo): string {
    const g = (c.given_name ?? "").trim();
    const f = (c.family_name ?? "").trim();
    const d = (c.display_name ?? "").trim();
    const first = g ? g[0] : (d ? d[0] : "");
    const last = f ? f[0] : "";
    return (first + last).toUpperCase() || "?";
  }
</script>

<div class="ct-app">
  <aside class="ct-sidebar">
    <div class="ct-sidebar-header">
      <ModuleLogo to="/" label="Kontakte" />
    </div>

    <div class="ct-tools">
      <button type="button" class="ct-btn ct-btn-primary" onclick={openCreate}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>
        Neuer Kontakt
      </button>
    </div>

    <div class="ct-count">{contacts.length} Kontakte</div>

    <div class="ct-sidebar-footer">
      <div class="ct-search-bar">
        <input
          type="text"
          class="ct-search"
          placeholder="Suchen…"
          aria-label="Kontakte durchsuchen"
          bind:value={search}
          oninput={loadContacts}
        />
      </div>
      <div class="ct-module-row">
        <ModuleIcons active="contacts" />
      </div>
    </div>
  </aside>

  <main class="ct-main">
    {#if loading}
      <div class="ct-state">Lade Kontakte…</div>
    {:else if error}
      <div class="ct-state ct-state-error">
        <p>{error}</p>
        <button type="button" class="ct-btn ct-btn-ghost" onclick={loadContacts}>Erneut laden</button>
      </div>
    {:else if contacts.length === 0}
      <div class="ct-state">
        <p>{search ? "Keine Kontakte gefunden." : "Noch keine Kontakte."}</p>
        {#if !search}
          <button type="button" class="ct-btn ct-btn-ghost" onclick={openCreate}>Kontakt anlegen</button>
        {/if}
      </div>
    {:else}
      <ul class="ct-list">
        {#each contacts as c (c.vcard_uid)}
          <li class="ct-item">
            <div class="ct-avatar">{initials(c)}</div>
            <div class="ct-item-body">
              <span class="ct-item-name">{c.display_name || c.email || "Unbenannt"}</span>
              <span class="ct-item-sub">
                {#if c.email}<a class="ct-link" href="mailto:{c.email}">{c.email}</a>{/if}
                {#if c.email && c.phone}<span class="ct-sep">·</span>{/if}
                {#if c.phone}<span>{c.phone}</span>{/if}
                {#if c.organization}<span class="ct-sep">·</span><span>{c.organization}</span>{/if}
              </span>
            </div>
            <div class="ct-item-actions">
              <button type="button" class="ct-icon-btn" onclick={() => openEdit(c)} title="Bearbeiten">
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
              </button>
              <button type="button" class="ct-icon-btn ct-icon-btn-danger" onclick={() => removeContact(c)} title="Löschen">
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </main>

  {#if editorOpen}
    <div
      class="ct-modal-backdrop"
      role="button"
      tabindex="0"
      aria-label="Dialog schließen"
      onclick={(e) => { if (e.target === e.currentTarget && !busy) editorOpen = false; }}
      onkeydown={(e) => { if (e.key === "Escape" || e.key === "Enter") !busy && (editorOpen = false); }}
    >
      <div class="ct-modal" role="dialog" aria-modal="true" tabindex="-1" aria-label={editingUid ? "Kontakt bearbeiten" : "Neuer Kontakt"}>
        <h2>{editingUid ? "Kontakt bearbeiten" : "Neuer Kontakt"}</h2>
        <label>Anrede / Vorname
          <input type="text" bind:value={form.given_name} placeholder="Max" />
        </label>
        <label>Nachname
          <input type="text" bind:value={form.family_name} placeholder="Mustermann" />
        </label>
        <label>Anzeigename
          <input type="text" bind:value={form.display_name} placeholder="Max Mustermann" />
        </label>
        <label>E-Mail
          <input type="email" bind:value={form.email} placeholder="max@example.com" />
        </label>
        <label>Telefon
          <input type="tel" bind:value={form.phone} placeholder="+49 123 4567890" />
        </label>
        <label>Organisation
          <input type="text" bind:value={form.organization} placeholder="Beispiel GmbH" />
        </label>
        <div class="ct-modal-actions">
          <button type="button" class="ct-btn ct-btn-ghost" onclick={() => editorOpen = false} disabled={busy}>Abbrechen</button>
          <button type="button" class="ct-btn ct-btn-primary" onclick={saveContact} disabled={busy}>
            {busy ? "Speichern…" : "Speichern"}
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .ct-app {
    display: flex;
    height: 100vh;
    background: var(--color-list);
    color: var(--color-text);
  }
  .ct-sidebar {
    width: 240px;
    min-width: 240px;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
  }
  .ct-sidebar-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--color-border);
  }
  .ct-back {
    background: none;
    border: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-s);
  }
  .ct-back:hover { color: var(--color-text); background: var(--color-active-wash); }
  .ct-brand { font-weight: 600; font-size: 15px; }

  .ct-tools { padding: 12px 12px 4px; display: flex; flex-direction: column; gap: 8px; }
  .ct-search {
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    background: var(--color-list);
    color: var(--color-text);
    font-size: 13px;
  }
  .ct-search:focus { outline: none; border-color: var(--color-accent); }
  .ct-count {
    padding: 10px 16px;
    font-size: 12px;
    color: var(--color-text-secondary);
    border-top: 1px solid var(--color-border);
    margin-top: 8px;
  }
  .ct-sidebar-footer {
    margin-top: auto;
    padding: 12px;
    border-top: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ct-search-bar { position: relative; }
  .ct-module-row { display: flex; justify-content: center; }

  .ct-main { flex: 1; overflow-y: auto; padding: 20px 24px; }
  .ct-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    height: 100%;
    color: var(--color-text-secondary);
    font-size: 14px;
  }
  .ct-state-error { color: var(--color-danger); }

  .ct-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .ct-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    background: var(--color-card, var(--color-list));
    border: 1px solid var(--color-border);
    border-radius: var(--radius-m);
  }
  .ct-item:hover { border-color: var(--color-accent); }
  .ct-avatar {
    width: 40px;
    height: 40px;
    min-width: 40px;
    border-radius: 50%;
    background: var(--color-active-wash);
    color: var(--color-text);
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 600;
    font-size: 14px;
  }
  .ct-item-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .ct-item-name { font-weight: 600; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ct-item-sub { font-size: 12px; color: var(--color-text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ct-link { color: var(--color-accent); text-decoration: none; }
  .ct-link:hover { text-decoration: underline; }
  .ct-sep { margin: 0 4px; opacity: 0.5; }

  .ct-item-actions { display: flex; gap: 4px; }
  .ct-icon-btn {
    background: none;
    border: none;
    color: var(--color-text-secondary);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--radius-s);
  }
  .ct-icon-btn:hover { color: var(--color-text); background: var(--color-active-wash); }
  .ct-icon-btn-danger:hover { color: var(--color-danger); }

  .ct-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 14px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    background: var(--color-card, var(--color-list));
    color: var(--color-text);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }
  .ct-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .ct-btn-primary { background: var(--color-accent); border-color: var(--color-accent); color: #fff; }
  .ct-btn-ghost { border-color: transparent; background: transparent; color: var(--color-text-secondary); }
  .ct-btn-ghost:hover { background: var(--color-active-wash); }

  .ct-modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .ct-modal {
    width: 420px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 64px);
    overflow-y: auto;
    background: var(--color-card, var(--color-list));
    border: 1px solid var(--color-border);
    border-radius: var(--radius-l);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ct-modal h2 { margin: 0 0 4px; font-size: 17px; }
  .ct-modal label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--color-text-secondary); }
  .ct-modal input {
    padding: 8px 10px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-s);
    background: var(--color-list);
    color: var(--color-text);
    font-size: 13px;
  }
  .ct-modal input:focus { outline: none; border-color: var(--color-accent); }
  .ct-modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
</style>
