  <script lang="ts">
  import Symbol from "$lib/components/Symbol.svelte";
    import { onMount, tick } from "svelte";
    import { connectAccount, deleteAccount, saveSettings, getOlaresMailStatus } from "$lib/services/tauri";
    import type { OlaresMailStatus } from "$lib/services/tauri";
    import type { AccountInfo } from "$lib/stores/accounts";
    import { t, lang, setLang, translate, localizeError } from "$lib/i18n";

    interface Props {
      oncomplete: (acct: AccountInfo) => void;
    }

    let { oncomplete }: Props = $props();

    let splashStep = $state<"intro" | "setup_mail" | "setup_llm">("intro");

    // Values Olares injected via the chart's env mapping (secrets only present/absent).
    let olares = $state<OlaresMailStatus | null>(null);
    let splashUseOlaresPassword = $state(false);
    let hasOlaresMail = $derived(!!olares && olares.configured);

    onMount(() => {
      getOlaresMailStatus().then((s) => { olares = s; }).catch(() => { olares = null; });
    });

    let splashAcctName = $state("");
    let splashImapHost = $state("");
    let splashImapPort = $state(993);
    let splashImapSsl = $state(true);
    let splashImapInsecure = $state(false);
    let splashSmtpHost = $state("");
    let splashSmtpPort = $state(587);
    let splashSmtpTls = $state(true);
    let splashAcctUser = $state("");
    let splashAcctPass = $state("");
    let splashSmtpUser = $state("");
    let splashSmtpPass = $state("");
    let splashAdvancedMail = $state(false);
    let splashSenderName = $state("");
    let splashSenderMail = $state("");
    let splashAcctConnecting = $state(false);
    let splashAcctError = $state<string | null>(null);
    let splashCreatedAccount = $state<any>(null);

    let splashAiUrl = $state("https://llm.aimighty.de/v1");
    let splashAiKey = $state("ollama");
    let splashAiModel = $state("chat");
    let splashAiSaving = $state(false);
    let splashAiError = $state<string | null>(null);

    async function handleSplashConnectAccount() {
      if (!splashAcctName || !splashImapHost || !splashSmtpHost || !splashAcctUser || !splashSenderMail) {
        splashAcctError = translate("error.fillRequired");
        return;
      }
      splashAcctConnecting = true;
      splashAcctError = null;
      try {
        const smtpUser = splashSmtpUser || splashAcctUser;
        const smtpPass = splashSmtpPass || splashAcctPass;
        const acct = await connectAccount(
          splashAcctName,
          splashImapHost,
          splashImapPort,
          splashImapSsl,
          splashSmtpHost,
          splashSmtpPort,
          splashSmtpTls,
          splashAcctUser,
          splashAcctPass,
          smtpUser,
          smtpPass,
          splashSenderName,
          splashSenderMail,
          splashImapInsecure,
          splashUseOlaresPassword,
        );
        splashCreatedAccount = acct;
        splashStep = "setup_llm";
      } catch (e: unknown) {
        splashAcctError = localizeError(e instanceof Error ? e.message : String(e));
      } finally {
        splashAcctConnecting = false;
      }
    }

    /** Fill the setup form from the values Olares injected. */
    function prefillFromOlares() {
      const o = olares;
      if (!o) return;
      splashAcctName = o.account_name || o.identity.email || o.identity.first_name || "";
      if (o.imap.server) splashImapHost = o.imap.server;
      splashImapPort = o.imap.port ?? 993;
      splashImapSsl = o.imap_ssl ?? true;
      splashAcctUser = o.imap.username || o.identity.email || "";
      if (o.smtp.server) splashSmtpHost = o.smtp.server;
      splashSmtpPort = o.smtp.port ?? 587;
      splashSmtpTls = (o.smtp_security ?? "starttls") !== "ssl";
      splashSmtpUser = o.smtp.username || o.imap.username || o.identity.email || "";
      splashSenderName =
        [o.identity.first_name, o.identity.last_name].filter(Boolean).join(" ") ||
        o.identity.username || "";
      splashSenderMail = o.smtp_from_address || o.identity.email || "";
    }

    /** "Automatisch übernehmen": prefill and — when complete — connect using the
        passwords held by Olares (they never reach the browser). */
    async function adoptFromOlares() {
      prefillFromOlares();
      splashUseOlaresPassword = !!olares?.complete;
      splashStep = "setup_mail";
      if (olares?.complete) {
        await tick();
        await handleSplashConnectAccount();
      }
    }

    async function handleSplashBackToMail() {
      if (splashCreatedAccount) {
        try {
          await deleteAccount(splashCreatedAccount.id);
        } catch (e) {
          console.warn("Konnte temporäres Konto nicht löschen", e);
        }
        splashCreatedAccount = null;
      }
      splashStep = "setup_mail";
    }

    async function handleSplashCompleteSetup() {
      splashAiSaving = true;
      splashAiError = null;
      try {
        await saveSettings(splashAiUrl, splashAiKey, splashAiModel);
        if (splashCreatedAccount) {
          oncomplete(splashCreatedAccount);
        }
      } catch (e: unknown) {
        splashAiError = e instanceof Error ? e.message : String(e);
      } finally {
        splashAiSaving = false;
      }
    }
  </script>

  <div class="splash-screen">
    <div class="splash-card">
      <div class="lang-toggle" role="group" aria-label={$t("splash.language")}>
        <button type="button" class:active={$lang === "de"} onclick={() => setLang("de")}>DE</button>
        <button type="button" class:active={$lang === "en"} onclick={() => setLang("en")}>EN</button>
      </div>
      {#if splashStep === "intro"}
        <div class="splash-intro">
          <h1>{$t("splash.welcome")}</h1>
          <p class="splash-subtitle">{$t("splash.subtitle")}</p>
          
          <div class="feature-grid">
            <div class="feature-card">
              <h3>{$t("splash.featureMonitoring")}</h3>
              <p>{$t("splash.featureMonitoringDesc")}</p>
            </div>
            
            <div class="feature-card">
              <h3>{$t("splash.featureGeneration")}</h3>
              <p>{$t("splash.featureGenerationDesc")}</p>
            </div>
            
            <div class="feature-card">
              <h3>{$t("splash.featureLocal")}</h3>
              <p>{$t("splash.featureLocalDesc")}</p>
            </div>
          </div>
          
          {#if hasOlaresMail}
            <div class="olares-hint">
              <p class="olares-hint-title">{$t("splash.olaresFound")}</p>
              <ul class="olares-hint-list">
                {#if olares?.identity.email}<li>{$t("splash.olaresEmail")}: {olares.identity.email}</li>{/if}
                {#if olares?.identity.first_name || olares?.identity.last_name}
                  <li>{$t("splash.olaresName")}: {[olares.identity.first_name, olares.identity.last_name].filter(Boolean).join(" ")}</li>
                {/if}
                {#if olares?.imap.server}
                  <li>IMAP: {olares.imap.server}{olares.imap.username ? ` · ${olares.imap.username}` : ""}</li>
                {/if}
                {#if olares?.smtp.server}<li>SMTP: {olares.smtp.server}</li>{/if}
                {#if olares?.company.name}<li>{$t("splash.olaresCompany")}: {olares.company.name}</li>{/if}
              </ul>
              {#if !olares?.complete && olares?.missing.length}
                <p class="olares-hint-missing">{$t("splash.olaresMissing")}: {olares.missing.join(", ")}</p>
              {/if}
              <div class="olares-actions">
                <button type="button" class="btn-splash-secondary" onclick={() => (splashStep = "setup_mail")}>
                  {$t("splash.setupManual")}
                </button>
                <button type="button" class="btn-splash-primary" onclick={adoptFromOlares}>
                  {$t("splash.setupAuto")}
                </button>
              </div>
            </div>
          {:else}
            <button type="button" class="btn-splash-primary" onclick={() => (splashStep = "setup_mail")}>
              {$t("splash.setupNow")}
            </button>
          {/if}
        </div>
      {:else if splashStep === "setup_mail"}
        <div class="splash-form-view">
          <h2>{$t("splash.connectTitle")}</h2>
          <p class="splash-subtitle">{$t("splash.step1Of2")}</p>
          
          <div class="splash-form">
            <div class="form-group span-2">
              <label for="splash-acct-name">{$t("splash.displayName")}</label>
              <input id="splash-acct-name" bind:value={splashAcctName} placeholder={$t("splash.displayNamePlaceholder")} />
            </div>

            <div class="form-group">
              <label for="splash-acct-user">{$t("splash.username")}</label>
              <input id="splash-acct-user" bind:value={splashAcctUser} placeholder="max@gmx.de" />
            </div>
            <div class="form-group">
              <label for="splash-acct-pass">{$t("splash.password")}</label>
              <input id="splash-acct-pass" type="password" bind:value={splashAcctPass} />
            </div>

            <div class="form-group">
              <label for="splash-sender-name">{$t("splash.senderName")}</label>
              <input id="splash-sender-name" bind:value={splashSenderName} placeholder={$t("mail.pnameExample")} />
            </div>
            <div class="form-group">
              <label for="splash-sender-mail">{$t("splash.senderMail")}</label>
              <input id="splash-sender-mail" type="text" inputmode="email" bind:value={splashSenderMail} placeholder="max@gmx.de" />
            </div>

            <div class="form-group">
              <label for="splash-imap-host">{$t("splash.imapServer")}</label>
              <input id="splash-imap-host" bind:value={splashImapHost} placeholder="imap.gmx.net" />
            </div>
            <div class="form-group">
              <label for="splash-imap-port">{$t("splash.imapPort")}</label>
              <div class="port-ssl-row">
                <input id="splash-imap-port" type="number" bind:value={splashImapPort} />
                <label class="toggle-label">
                  <input type="checkbox" class="toggle" bind:checked={splashImapSsl} />
                  <span class="toggle-track" aria-hidden="true"></span>
                  <span class="toggle-text">SSL</span>
                </label>
              </div>
            </div>
            <div class="form-group span-2">
              <label class="toggle-label">
                <input type="checkbox" class="toggle" bind:checked={splashImapInsecure} />
                <span class="toggle-track" aria-hidden="true"></span>
                <span class="toggle-text">{$t("splash.allowInsecure")}</span>
              </label>
            </div>

            <div class="form-group">
              <label for="splash-smtp-host">{$t("splash.smtpServer")}</label>
              <input id="splash-smtp-host" bind:value={splashSmtpHost} placeholder="mail.gmx.net" />
            </div>
            <div class="form-group">
              <label for="splash-smtp-port">{$t("splash.smtpPort")}</label>
              <div class="port-ssl-row">
                <input id="splash-smtp-port" type="number" bind:value={splashSmtpPort} />
                <label class="toggle-label">
                  <input type="checkbox" class="toggle" bind:checked={splashSmtpTls} />
                  <span class="toggle-track" aria-hidden="true"></span>
                  <span class="toggle-text">TLS</span>
                </label>
              </div>
            </div>

            <div class="form-group span-2">
              <button type="button" class="btn-link" onclick={() => (splashAdvancedMail = !splashAdvancedMail)}>
                {#if splashAdvancedMail}<Symbol name="chevron-hoch" size={16} />{:else}<Symbol name="chevron" size={16} />{/if}
                {splashAdvancedMail ? $t("splash.hideAdvanced") : $t("splash.showAdvanced")}
              </button>
            </div>
            {#if splashAdvancedMail}
              <div class="form-group">
                <label for="splash-smtp-user">{$t("splash.smtpUsername")}</label>
                <input id="splash-smtp-user" bind:value={splashSmtpUser} placeholder={$t("splash.optionalImapUser")} />
              </div>
              <div class="form-group">
                <label for="splash-smtp-pass">{$t("splash.smtpPassword")}</label>
                <input id="splash-smtp-pass" type="password" bind:value={splashSmtpPass} placeholder={$t("splash.optionalImapPassword")} />
              </div>
            {/if}

            {#if splashAcctError}
              <div class="error-message span-2">{splashAcctError}</div>
            {/if}

            <div class="splash-actions span-2">
              <button type="button" class="btn-splash-secondary" onclick={() => (splashStep = "intro")}>
                {$t("common.back")}
              </button>
              <button type="button" class="btn-splash-primary" onclick={handleSplashConnectAccount} disabled={splashAcctConnecting}>
                {splashAcctConnecting ? $t("splash.connecting") : $t("common.next")}
              </button>
            </div>
          </div>
        </div>
      {:else if splashStep === "setup_llm"}
        <div class="splash-form-view">
          <h2>{$t("splash.llmTitle")}</h2>
          <p class="splash-subtitle">{$t("splash.step2Of2")}</p>
          
          <div class="splash-form">
            <div class="form-group span-2">
              <label for="splash-ai-url">{$t("splash.apiUrl")}</label>
              <input id="splash-ai-url" type="text" inputmode="url" bind:value={splashAiUrl} placeholder="https://llm.aimighty.de/v1" />
            </div>
            <div class="form-group">
              <label for="splash-ai-key">{$t("splash.apiKey")}</label>
              <input id="splash-ai-key" type="password" bind:value={splashAiKey} placeholder="ollama" />
            </div>
            <div class="form-group">
              <label for="splash-ai-model">{$t("splash.modelId")}</label>
              <input id="splash-ai-model" type="text" bind:value={splashAiModel} placeholder="chat" />
            </div>

            {#if splashAiError}
              <div class="error-message span-2">{splashAiError}</div>
            {/if}

            <div class="splash-actions span-2">
              <button type="button" class="btn-splash-secondary" onclick={handleSplashBackToMail}>
                {$t("common.back")}
              </button>
              <button type="button" class="btn-splash-primary" onclick={handleSplashCompleteSetup} disabled={splashAiSaving}>
                {splashAiSaving ? $t("common.saving") : $t("splash.finish")}
              </button>
            </div>
          </div>
        </div>
      {/if}
    </div>
  </div>

<style>
  .splash-screen {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: var(--am-flaeche-1);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    overflow-y: auto;
    padding: 24px;
  }
  .splash-card {
    background: var(--am-seite);
    border: 1px solid var(--am-rand);
    border-radius: 12px;
    padding: 48px;
    width: 100%;
    max-width: 720px;
    box-shadow: none;
    display: flex;
    flex-direction: column;
    position: relative;
    animation: fadeIn 0.25s ease-out;
  }
  .lang-toggle {
    position: absolute;
    top: 16px;
    right: 16px;
    display: flex;
    gap: 4px;
    border: 1px solid var(--am-rand);
    border-radius: var(--am-radius-mittel);
    padding: 2px;
    background: var(--am-flaeche-1);
  }
  .lang-toggle button {
    background: transparent;
    border: none;
    color: var(--am-text-gedaempft);
    font-size: 0.75rem;
    font-weight: 600;
    padding: 4px 10px;
    border-radius: 4px;
    cursor: pointer;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .lang-toggle button.active {
    background: var(--am-handlung-ruhend);
    color: var(--am-handlung-text);
  }
  .lang-toggle button:focus-visible {
    outline: 2px solid var(--am-handlung-ruhend);
    outline-offset: 1px;
  }
  @keyframes fadeIn {
    from { opacity: 0; transform: scale(0.98); }
    to { opacity: 1; transform: scale(1); }
  }
  .splash-intro {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }
  .splash-intro h1 {
    font-size: 1.875rem;
    font-weight: 700;
    margin-bottom: 8px;
    color: var(--am-text-primaer);
  }
  .splash-subtitle {
    font-size: 0.9375rem;
    color: var(--am-text-gedaempft);
    margin-bottom: 48px;
    max-width: 500px;
  }
  .feature-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 24px;
    width: 100%;
    margin-bottom: 48px;
  }
  .feature-card {
    border-top: 1px solid var(--am-rand);
    padding-top: 16px;
    text-align: left;
    display: flex;
    flex-direction: column;
  }
  .feature-card h3 {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--am-text-primaer);
    margin-bottom: 8px;
  }
  .feature-card p {
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
    line-height: 1.5;
  }
  .olares-hint {
    width: 100%;
    text-align: left;
    border: 1px solid var(--am-rand);
    border-radius: 8px;
    padding: 16px 20px;
    margin-bottom: 24px;
    background: var(--am-flaeche-1);
  }
  .olares-hint-title { font-size: 0.875rem; font-weight: 600; color: var(--am-text-primaer); margin-bottom: 8px; }
  .olares-hint-list {
    list-style: none;
    margin: 0 0 12px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 0.8125rem;
    color: var(--am-text-gedaempft);
  }
  .olares-hint-missing { font-size: 0.75rem; color: var(--am-text-gedaempft); margin-bottom: 12px; }
  .olares-actions { display: flex; justify-content: flex-end; gap: 8px; }

  .btn-link {
    background: none;
    border: none;
    color: var(--am-handlung-ruhend);
    cursor: pointer;
    text-decoration: underline;
    font-size: 0.8125rem;
    padding: 8px 0;
  }
  .btn-link:hover {
    color: var(--am-handlung-hover);
  }

  .btn-splash-primary {
    background: var(--am-handlung-ruhend);
    color: var(--am-handlung-text);
    font-size: 0.875rem;
    font-weight: 600;
    padding: 10px 24px;
    border: none;
    border-radius: var(--am-radius-mittel);
    cursor: pointer;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .btn-splash-primary:hover:not(:disabled) {
    background: var(--am-handlung-hover);
  }
  .btn-splash-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-splash-secondary {
    background: transparent;
    border: 1px solid var(--am-rand);
    color: var(--am-text-primaer);
    font-size: 0.875rem;
    font-weight: 600;
    padding: 10px 20px;
    border-radius: var(--am-radius-mittel);
    cursor: pointer;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .btn-splash-secondary:hover {
    background: var(--am-flaeche-1);
  }
  .splash-form-view {
    display: flex;
    flex-direction: column;
  }
  .splash-form-view h2 {
    font-size: 1.375rem;
    font-weight: 700;
    margin-bottom: 8px;
    color: var(--am-text-primaer);
  }
  .splash-form {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 20px 24px;
    text-align: left;
    margin-top: 20px;
  }
  .form-group.span-2 {
    grid-column: span 2;
  }
  .port-ssl-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 41px;
    width: 100%;
  }
  .splash-form .form-group .port-ssl-row input[type="number"] {
    width: 75px;
    flex-shrink: 0;
  }
  .toggle-label {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--am-text-primaer);
    user-select: none;
    height: 100%;
  }
  .toggle-label .toggle {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .toggle-label .toggle-track {
    position: relative;
    width: 34px;
    height: 20px;
    border-radius: 999px;
    background: var(--am-rand);
    transition: background var(--am-dauer-mittel) var(--am-kurve);
    flex-shrink: 0;
  }
  .toggle-label .toggle-track::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--am-text-auf-farbe);
    box-shadow: none;
    transition: transform var(--am-dauer-mittel) var(--am-kurve);
  }
  .toggle-label .toggle:checked + .toggle-track {
    background: var(--am-handlung-ruhend);
  }
  .toggle-label .toggle:checked + .toggle-track::after {
    transform: translateX(14px);
  }
  .toggle-label .toggle:focus-visible + .toggle-track {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--am-handlung-ruhend) 20%, transparent);
  }
  .toggle-text {
    line-height: 1;
  }
  .splash-form .form-group label:not(.check-label):not(.toggle-label) {
    display: block;
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--am-text-gedaempft);
    margin-bottom: 6px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .splash-form .form-group input {
    width: 100%;
    padding: 10px 14px;
    border: 1px solid var(--am-rand);
    border-radius: 6px;
    font-size: 0.875rem;
    color: var(--am-text-primaer);
    background: var(--am-seite);
    box-shadow: none;
    transition: all var(--am-dauer-schnell) var(--am-kurve);
  }
  .splash-form .form-group input:focus {
    border-color: var(--am-handlung-ruhend);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--am-handlung-ruhend) 12%, transparent);
    background: var(--am-seite);
  }
  .splash-actions {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 12px;
    border-top: 1px solid var(--am-rand);
    padding-top: 24px;
  }
  .splash-actions.span-2 {
    grid-column: span 2;
  }
  .error-message.span-2 {
    grid-column: span 2;
  }
</style>
